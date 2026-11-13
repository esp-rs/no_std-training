// MQTT Communication (with WiFi provisioning)
// 1. Start the MQTT server from this repository root:
// cargo xtask mqtt-server
// 2. Run the app
// Host IP will be printed by xtask command, but if auto-detect fails, you can find it manually by running:
// ipconfig getifaddr en0 or ip addr show eth0
// HOST_IP="<IP>" cargo r -r

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod http;
mod mqtt;
mod network;
mod sensor;

use core::net::Ipv4Addr;

use embassy_executor::Spawner;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config, I2c},
    ram,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::WifiController;
use log::info;
use shtcx2::asynchronous::shtc3;

esp_bootloader_esp_idf::esp_app_desc!();

use crate::http::{run_captive_portal, run_dhcp, run_http_server};
use crate::mqtt::mqtt_task;
use crate::network::{NetworkStacks, connection, create_network_stacks, net_task};

// ANCHOR: gateway_ip
const GATEWAY_IP: Option<&str> = option_env!("GATEWAY_IP");
// ANCHOR_END: gateway_ip

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let i2c = I2c::new(peripherals.I2C0, Config::default())
        .expect("Failed to create I2C bus")
        .with_sda(peripherals.GPIO10)
        .with_scl(peripherals.GPIO8)
        .into_async();
    let sht = shtc3(i2c);

    let controller = WifiController::new(peripherals.WIFI, Default::default())
        .expect("Failed to create Wi-Fi controller");

    // ANCHOR: network_stacks
    let gw_ip_addr: Ipv4Addr = GATEWAY_IP
        .unwrap_or("192.168.2.1")
        .parse()
        .expect("GATEWAY_IP is not an IPv4 address");

    let NetworkStacks {
        ap_stack,
        ap_runner,
        sta_stack,
        sta_runner,
    } = create_network_stacks(gw_ip_addr);
    // ANCHOR_END: network_stacks

    // ANCHOR: spawn_tasks
    spawner.spawn(connection(controller).expect("failed to spawn connection task"));
    spawner.spawn(net_task(ap_runner).expect("failed to spawn AP network task"));
    spawner.spawn(net_task(sta_runner).expect("failed to spawn STA network task"));
    spawner.spawn(run_dhcp(ap_stack, gw_ip_addr).expect("failed to spawn DHCP task"));
    spawner.spawn(
        run_captive_portal(ap_stack, gw_ip_addr).expect("failed to spawn captive portal task"),
    );
    spawner.spawn(run_http_server(ap_stack, gw_ip_addr).expect("failed to spawn HTTP task"));
    spawner.spawn(mqtt_task(sta_stack, sht).expect("failed to spawn MQTT task"));
    // ANCHOR_END: spawn_tasks

    // ANCHOR: portal_ready
    info!("Provisioning portal: join the `esp-radio` network and open http://{gw_ip_addr}/");
    // ANCHOR_END: portal_ready
}

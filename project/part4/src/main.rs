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
use core::str::FromStr;

use embassy_executor::Spawner;
use embassy_sync::channel::Channel;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config, I2c},
    interrupt::software::SoftwareInterruptControl,
    ram,
    timer::timg::TimerGroup,
};
use log::{debug, info};
use shtcx2::asynchronous::shtc3;

esp_bootloader_esp_idf::esp_app_desc!();

use crate::http::{run_captive_portal, run_dhcp, run_http_server};
use crate::mqtt::mqtt_task;
use crate::network::{
    NetworkStacks, WifiCredentials, connection, create_network_stacks, net_task, sta_net_task,
};

const GW_IP_ADDR_ENV: Option<&'static str> = option_env!("GATEWAY_IP");

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let sda = peripherals.GPIO10;
    let scl = peripherals.GPIO8;
    let i2c = I2c::new(peripherals.I2C0, Config::default())
        .expect("Failed to create I2C bus")
        .with_sda(sda)
        .with_scl(scl)
        .into_async();
    let sht = shtc3(i2c);

    let (controller, interfaces) = esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Failed to create WiFi controller");

    // Start with AP device for provisioning
    let ap_device = interfaces.access_point;
    // Store STA device for later use
    let sta_device = interfaces.station;

    let gw_ip_addr_str = GW_IP_ADDR_ENV.unwrap_or("192.168.2.1");
    let gw_ip_addr = Ipv4Addr::from_str(gw_ip_addr_str).expect("failed to parse gateway ip");

    let NetworkStacks {
        ap_stack,
        ap_runner,
        sta_stack,
        sta_runner,
    } = create_network_stacks(ap_device, sta_device, gw_ip_addr);

    // Create WiFi credentials channel
    static WIFI_CREDENTIALS_CHANNEL_CELL: static_cell::StaticCell<
        Channel<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, WifiCredentials, 1>,
    > = static_cell::StaticCell::new();
    let wifi_credentials_channel = WIFI_CREDENTIALS_CHANNEL_CELL.uninit().write(Channel::new());

    spawner.spawn(
        connection(controller, wifi_credentials_channel).expect("failed to spawn connection task"),
    );
    spawner.spawn(net_task(ap_runner).expect("failed to spawn AP network task"));
    spawner.spawn(sta_net_task(sta_runner).expect("failed to spawn STA network task"));
    spawner.spawn(run_dhcp(ap_stack, gw_ip_addr).expect("failed to spawn DHCP task"));
    spawner.spawn(
        run_captive_portal(ap_stack, gw_ip_addr).expect("failed to spawn captive portal task"),
    );
    spawner.spawn(mqtt_task(sta_stack, sht).expect("failed to spawn MQTT task"));

    ap_stack.wait_link_up().await;
    info!("WiFi Provisioning Portal Ready");
    info!("1. Connect to the AP: `esp-radio`");
    info!("2. Navigate to: http://{gw_ip_addr_str}/");
    ap_stack.wait_config_up().await;
    ap_stack
        .config_v4()
        .inspect(|c| debug!("ipv4 config: {c:?}"));

    spawner.spawn(
        run_http_server(ap_stack, wifi_credentials_channel)
            .expect("failed to spawn HTTP server task"),
    );

    // Keep main task alive
    loop {
        Timer::after(EmbassyDuration::from_secs(60)).await;
    }
}

// OTA Update
// 1. Start the MQTT server from this repository root:
// cargo xtask mqtt-server
// 2. Flash the app, erasing otadata so it boots from the factory partition
// Host IP will be printed by xtask command, but if auto-detect fails, you can find it manually by running:
// ipconfig getifaddr en0 or ip addr show eth0
// HOST_IP="<IP>" cargo r -r -- --erase-data-parts ota
// Optional: configure the OTA interval with `OTA_CHECK_INTERVAL_SECS=<seconds>` (default: 300).
// 3. Join the AP network and navigate to http://<MCU_IP>/ to set the wifi credentials.
// Once the device stops the AP mode and starts the STA mode connected to the wifi, it will start sending sensor data to the MQTT broker and periodically check for OTA updates.
// 4. Bump `version` in Cargo.toml (the device only installs images whose version differs from the running one),
// build with the same env vars as step 2 without flashing, and generate the firmware.bin file
// HOST_IP="<IP>" cargo b -r
// mkdir -p ota
// espflash save-image --chip esp32c3 --partition-table partitions.csv target/riscv32imc-unknown-none-elf/release/no_std-training ota/firmware.bin
// 5. Run the OTA server from this repository root:
// cargo xtask ota-server --firmware project/part5/ota/firmware.bin

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
mod ota;
mod sensor;
mod status_led;

use core::net::Ipv4Addr;

use embassy_executor::Spawner;
use esp_alloc as _;
use esp_backtrace as _;
use esp_bootloader_esp_idf::partitions;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config, I2c},
    ram,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::WifiController;
use esp_storage::FlashStorage;
use log::info;
use shtcx2::asynchronous::shtc3;

esp_bootloader_esp_idf::esp_app_desc!();

use crate::http::{run_captive_portal, run_dhcp, run_http_server};
use crate::mqtt::mqtt_task;
use crate::network::{NetworkStacks, connection, create_network_stacks, net_task};
use crate::ota::http_client_task;
use crate::status_led::{LED_STATUS, LedStatus, status_led_task};

const GATEWAY_IP: Option<&str> = option_env!("GATEWAY_IP");

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // ANCHOR: flash_storage
    let mut flash = FlashStorage::new(peripherals.FLASH);
    let mut buffer = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let pt = partitions::read_partition_table(&mut flash, &mut buffer)
        .expect("Failed to read partition table");
    info!("Currently booted partition {:?}", pt.booted_partition());
    // ANCHOR_END: flash_storage

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

    LED_STATUS.signal(LedStatus::Provisioning);

    let controller = WifiController::new(peripherals.WIFI, Default::default())
        .expect("Failed to create Wi-Fi controller");

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

    spawner.spawn(connection(controller).expect("failed to spawn connection task"));
    spawner.spawn(net_task(ap_runner).expect("failed to spawn AP network task"));
    spawner.spawn(net_task(sta_runner).expect("failed to spawn STA network task"));
    spawner.spawn(run_dhcp(ap_stack, gw_ip_addr).expect("failed to spawn DHCP task"));
    spawner.spawn(
        run_captive_portal(ap_stack, gw_ip_addr).expect("failed to spawn captive portal task"),
    );
    spawner.spawn(run_http_server(ap_stack, gw_ip_addr).expect("failed to spawn HTTP task"));
    spawner.spawn(mqtt_task(sta_stack, sht).expect("failed to spawn MQTT task"));
    // ANCHOR: spawn_ota
    spawner.spawn(http_client_task(sta_stack, flash).expect("failed to spawn OTA task"));
    spawner.spawn(
        status_led_task(peripherals.RMT, peripherals.GPIO2).expect("failed to spawn LED task"),
    );
    // ANCHOR_END: spawn_ota

    info!("Provisioning portal: join the `esp-radio` network and open http://{gw_ip_addr}/");
}

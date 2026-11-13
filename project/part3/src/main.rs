// MQTT Communication (without WiFi provisioning)
// 1. Start the MQTT server from this repository root:
// cargo xtask mqtt-server
// 2. Run the app
// Host IP will be printed by xtask command, but if auto-detect fails, you can find it manually by running:
// ipconfig getifaddr en0 or ip addr show eth0
// SSID="<SSID>" PASSWORD="<PASSWORD>" HOST_IP="<IP>" cargo r -r

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

// ANCHOR: mqtt_module
mod mqtt;
// ANCHOR_END: mqtt_module
mod network;
mod sensor;

use embassy_executor::Spawner;
use embassy_net::StackResources;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config, I2c},
    ram,
    rng::Rng,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{Interface, WifiController};
use shtcx2::asynchronous::shtc3;
use static_cell::StaticCell;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

use crate::mqtt::mqtt_task;
use crate::network::{connection, net_task};

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
    let wifi_interface = Interface::station();

    let config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        STACK_RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(connection(controller).expect("failed to spawn connection task"));
    spawner.spawn(net_task(runner).expect("failed to spawn network task"));
    // ANCHOR: spawn_mqtt
    spawner.spawn(mqtt_task(stack, sht).expect("failed to spawn MQTT task"));
    // ANCHOR_END: spawn_mqtt
}

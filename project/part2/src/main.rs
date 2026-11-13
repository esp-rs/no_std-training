// WiFi Connectivity
// 1. Start local HTTP receiver:
// cargo xtask http-server
// 2. Run the app:
// SSID="<SSID>" PASSWORD="<PASSWORD>" HOST_IP="<IP>" cargo r -r
#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

// ANCHOR: alloc_crate
extern crate alloc;
// ANCHOR_END: alloc_crate

// ANCHOR: modules
mod http;
mod network;
mod sensor;
// ANCHOR_END: modules

use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_time::{Duration, Timer};
// ANCHOR: alloc_import
use esp_alloc as _;
// ANCHOR_END: alloc_import
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config, I2c},
    ram,
    rng::Rng,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{Interface, WifiController};
use log::info;
use shtcx2::asynchronous::shtc3;
use static_cell::StaticCell;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

use crate::http::send_sensor_data;
use crate::network::{connection, net_task};
use crate::sensor::read_sensor;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // ANCHOR: heap_init
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);
    // ANCHOR_END: heap_init

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let i2c = I2c::new(peripherals.I2C0, Config::default())
        .expect("Failed to create I2C bus")
        .with_sda(peripherals.GPIO10)
        .with_scl(peripherals.GPIO8)
        .into_async();
    let mut sht = shtc3(i2c);

    // ANCHOR: wifi_controller
    let controller = WifiController::new(peripherals.WIFI, Default::default())
        .expect("Failed to create Wi-Fi controller");
    let wifi_interface = Interface::station();
    // ANCHOR_END: wifi_controller

    // ANCHOR: embassy_net_config
    let config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    // ANCHOR_END: embassy_net_config

    // ANCHOR: stack_init
    static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        STACK_RESOURCES.init(StackResources::new()),
        seed,
    );
    // ANCHOR_END: stack_init
    // ANCHOR: spawn_tasks
    spawner.spawn(connection(controller).expect("failed to spawn connection task"));
    spawner.spawn(net_task(runner).expect("failed to spawn network task"));
    // ANCHOR_END: spawn_tasks

    // ANCHOR: wait_ip
    stack.wait_config_up().await;
    if let Some(config) = stack.config_v4() {
        info!("Got IP: {}", config.address);
    }
    // ANCHOR_END: wait_ip

    loop {
        // ANCHOR: main_loop
        if let Some((temperature, humidity)) = read_sensor(&mut sht).await {
            send_sensor_data(stack, temperature, humidity).await;
        }
        // ANCHOR_END: main_loop

        Timer::after(Duration::from_secs(1)).await;
    }
}

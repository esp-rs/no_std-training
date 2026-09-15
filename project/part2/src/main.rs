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
use embassy_time::Duration;
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
use log::debug;
use shtcx2::asynchronous::shtc3;

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
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    let sda = peripherals.GPIO10;
    let scl = peripherals.GPIO8;
    let i2c = I2c::new(peripherals.I2C0, Config::default())
        .expect("Failed to create I2C bus")
        .with_sda(sda)
        .with_scl(scl)
        .into_async();
    let mut sht = shtc3(i2c);

    // ANCHOR: wifi_controller
    let (controller, interfaces) = esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Failed to create WiFi controller");

    let wifi_interface = interfaces.station;
    // ANCHOR_END: wifi_controller

    // ANCHOR: embassy_net_config
    let config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    // ANCHOR_END: embassy_net_config

    // ANCHOR: stack_init
    // Init network stack
    static STACK_RESOURCES_CELL: static_cell::StaticCell<StackResources<3>> =
        static_cell::StaticCell::new();
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        STACK_RESOURCES_CELL
            .uninit()
            .write(StackResources::<3>::new()),
        seed,
    );
    // ANCHOR_END: stack_init
    // ANCHOR: spawn_tasks
    spawner.spawn(connection(controller).expect("failed to spawn connection task"));
    spawner.spawn(net_task(runner).expect("failed to spawn network task"));
    // ANCHOR_END: spawn_tasks

    // ANCHOR: wait_ip
    stack.wait_link_up().await;

    debug!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            debug!("Got IP: {}", config.address);
            break;
        }
        embassy_time::Timer::after(Duration::from_millis(500)).await;
    }
    // ANCHOR_END: wait_ip

    loop {
        // ANCHOR: main_loop
        // Read sensor
        if let Some((temp, humidity)) = read_sensor(&mut sht).await {
            // Send sensor data via HTTP
            let _ = send_sensor_data(stack, temp, humidity).await;
        }
        // ANCHOR_END: main_loop

        // Small delay before next measurement
        embassy_time::Timer::after(Duration::from_secs(1)).await;
    }
}

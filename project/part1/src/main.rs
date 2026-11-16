// Reading sensors
// 1. Run the app
// cargo r -r
#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer};
// ANCHOR: backtrace_import
use esp_backtrace as _;
// ANCHOR_END: backtrace_import
use esp_hal::clock::CpuClock;
use esp_hal::{
    i2c::master::{Config, I2c},
    timer::timg::TimerGroup,
};
use log::{error, info};
use shtcx2::asynchronous::{PowerMode, shtc3};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    // ANCHOR: logger_init
    esp_println::logger::init_logger_from_env();
    // ANCHOR_END: logger_init

    // ANCHOR: esp_hal_init
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    // ANCHOR_END: esp_hal_init

    // ANCHOR: esp_rtos_start
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);
    // ANCHOR_END: esp_rtos_start

    // ANCHOR: i2c_driver
    let sda = peripherals.GPIO10;
    let scl = peripherals.GPIO8;
    let i2c = I2c::new(peripherals.I2C0, Config::default())
        .expect("Failed to create I2C bus")
        .with_sda(sda)
        .with_scl(scl)
        .into_async();
    let mut sht = shtc3(i2c);
    // ANCHOR_END: i2c_driver

    loop {
        // ANCHOR: read_measurement
        match sht.measure(PowerMode::NormalMode, &mut Delay).await {
            // ANCHOR: log_measurement
            Ok(measurement) => info!(
                "  {:.2} °C | {:.2} %RH",
                measurement.temperature.as_degrees_celsius(),
                measurement.humidity.as_percent(),
            ),
            // ANCHOR_END: log_measurement
            Err(e) => error!("Failed to read sensor: {e:?}"),
        }
        // ANCHOR_END: read_measurement
        Timer::after(Duration::from_secs(1)).await;
    }
}

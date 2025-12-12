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
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::{
    i2c::master::{Config, I2c},
    timer::timg::TimerGroup,
};
use log::{error, info};
use shtcx::asynchronous::{PowerMode, max_measurement_duration, shtc3};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

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

    loop {
        // Read sensor
        if let Err(e) = sht.start_measurement(PowerMode::NormalMode).await {
            error!("Failed to start measurement: {:?}", e);
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }
        // Wait for 12.1 ms https://github.com/Fristi/shtcx-rs/blob/feature/async-support/src/asynchronous.rs#L413-L424
        let duration = max_measurement_duration(&sht, PowerMode::NormalMode);
        Timer::after(Duration::from_micros(duration.into())).await;
        let measurement = match sht.get_measurement_result().await {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to get measurement result: {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };

        info!(
            "  {:.2} °C | {:.2} %RH",
            measurement.temperature.as_degrees_celsius(),
            measurement.humidity.as_percent(),
        );
        Timer::after(Duration::from_secs(1)).await;
    }
}

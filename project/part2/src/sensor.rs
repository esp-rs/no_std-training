use embassy_time::{Duration, Timer};
use esp_hal::i2c::master::I2c;
use log::{error, info};
use shtcx2::asynchronous::{AsyncShtC3 as ShtC3, PowerMode, max_measurement_duration};

pub async fn read_sensor(sht: &mut ShtC3<I2c<'static, esp_hal::Async>>) -> Option<(f32, f32)> {
    // Read sensor
    if let Err(e) = sht.start_measurement(PowerMode::NormalMode).await {
        error!("Failed to start measurement: {:?}", e);
        return None;
    }
    // Wait for the maximum measurement duration reported by the sensor driver.
    let duration = max_measurement_duration(sht, PowerMode::NormalMode);
    Timer::after(Duration::from_micros(duration.into())).await;

    match sht.get_measurement_result().await {
        Ok(m) => {
            let temp = m.temperature.as_degrees_celsius();
            let humidity = m.humidity.as_percent();
            info!("  {:.2} °C | {:.2} %RH", temp, humidity);
            Some((temp, humidity))
        }
        Err(e) => {
            error!("Failed to get measurement result: {:?}", e);
            None
        }
    }
}

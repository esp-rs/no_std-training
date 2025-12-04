use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::i2c::master::I2c;
use log::{error, info};
use shtcx::asynchronous::{PowerMode, ShtC3, max_measurement_duration};

pub async fn read_sensor(sht: &mut ShtC3<I2c<'static, esp_hal::Async>>) -> Option<(f32, f32)> {
    if let Err(e) = sht.start_measurement(PowerMode::NormalMode).await {
        error!("Failed to start measurement: {:?}", e);
        return None;
    }

    // Wait for measurement to complete
    let duration = max_measurement_duration(sht, PowerMode::NormalMode);
    Timer::after(EmbassyDuration::from_micros(duration.into())).await;

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

use embassy_time::Delay;
use esp_hal::{Async, i2c::master::I2c};
use log::{error, info};
use shtcx2::asynchronous::{AsyncShtC3, PowerMode};

pub type Sensor = AsyncShtC3<I2c<'static, Async>>;

/// Returns the temperature in °C and the relative humidity in %.
pub async fn read_sensor(sht: &mut Sensor) -> Option<(f32, f32)> {
    match sht.measure(PowerMode::NormalMode, &mut Delay).await {
        Ok(m) => {
            let temperature = m.temperature.as_degrees_celsius();
            let humidity = m.humidity.as_percent();
            info!("  {temperature:.2} °C | {humidity:.2} %RH");
            Some((temperature, humidity))
        }
        Err(e) => {
            error!("Failed to read sensor: {e:?}");
            None
        }
    }
}

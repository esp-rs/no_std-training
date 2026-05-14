use embassy_sync::signal::Signal;
use esp_hal::{rmt::Rmt, time::Rate};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use smart_leds::{RGB8, SmartLedsWriteAsync, brightness, gamma};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedStatus {
    Provisioning,
    Idle,
    Updating,
}

pub static LED_STATUS: Signal<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    LedStatus,
> = Signal::new();

#[embassy_executor::task]
pub async fn status_led_task(
    rmt: esp_hal::peripherals::RMT<'static>,
    gpio2: esp_hal::peripherals::GPIO2<'static>,
) {
    let rmt: Rmt<'_, esp_hal::Async> = {
        let frequency: Rate = Rate::from_mhz(80);
        Rmt::new(rmt, frequency)
    }
    .expect("Failed to initialize RMT")
    .into_async();

    let rmt_channel = rmt.channel0;
    let mut rmt_buffer = [esp_hal::rmt::PulseCode::default(); buffer_size_async(1)];
    let mut led = SmartLedsAdapterAsync::new(rmt_channel, gpio2, &mut rmt_buffer);
    let level = 10;

    // Default state before any signal arrives.
    led.write(brightness(gamma([RGB8::new(0, 255, 0)].into_iter()), level))
        .await
        .unwrap();

    loop {
        let status = LED_STATUS.wait().await;
        let color = match status {
            LedStatus::Provisioning => RGB8::new(0, 255, 0),
            LedStatus::Idle => RGB8::new(0, 0, 0),
            LedStatus::Updating => RGB8::new(0, 0, 255),
        };

        led.write(brightness(gamma([color].into_iter()), level))
            .await
            .unwrap();
    }
}

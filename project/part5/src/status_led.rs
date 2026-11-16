use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use esp_hal::{
    Async,
    gpio::Level,
    peripherals::{GPIO2, RMT},
    rmt::{Channel, PulseCode, Rmt, Tx, TxChannelConfig, TxChannelCreator},
    time::Rate,
};
use log::warn;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedStatus {
    Provisioning,
    Idle,
    Updating,
}

pub static LED_STATUS: Signal<CriticalSectionRawMutex, LedStatus> = Signal::new();

/// Scales every color channel down from 255 to keep the LED comfortable to look at.
const BRIGHTNESS: u8 = 10;

// WS2812 bit timings at 80 MHz (12.5 ns per tick)
const BIT_0: PulseCode = PulseCode::new(Level::High, 28, Level::Low, 64); // 350 ns / 800 ns
const BIT_1: PulseCode = PulseCode::new(Level::High, 56, Level::Low, 48); // 700 ns / 600 ns

#[embassy_executor::task]
pub async fn status_led_task(rmt: RMT<'static>, pin: GPIO2<'static>) {
    let rmt = Rmt::new(rmt, Rate::from_mhz(80))
        .expect("Failed to initialize RMT")
        .into_async();
    let mut channel = rmt
        .channel0
        .configure_tx(&TxChannelConfig::default().with_clk_divider(1))
        .expect("Failed to configure RMT TX channel")
        .with_pin(pin);

    loop {
        // ANCHOR: led_colors
        let color = match LED_STATUS.wait().await {
            LedStatus::Provisioning => (0, 255, 0),
            LedStatus::Idle => (0, 0, 0),
            LedStatus::Updating => (0, 0, 255),
        };
        // ANCHOR_END: led_colors

        write_led(&mut channel, color).await;
    }
}

async fn write_led(channel: &mut Channel<'_, Async, Tx>, (red, green, blue): (u8, u8, u8)) {
    let scale = |value: u8| (value as u16 * BRIGHTNESS as u16 / 255) as u8;
    // WS2812 LEDs expect 24 bits in GRB order, most significant bit first
    let grb = u32::from_be_bytes([0, scale(green), scale(red), scale(blue)]);

    let mut data = [PulseCode::end_marker(); 25];
    for (i, pulse) in data[..24].iter_mut().enumerate() {
        *pulse = if grb & (1 << (23 - i)) == 0 {
            BIT_0
        } else {
            BIT_1
        };
    }

    if let Err(e) = channel.transmit(&data).await {
        warn!("Failed to update status LED: {e:?}");
    }
}

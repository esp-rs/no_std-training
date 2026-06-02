use embassy_sync::signal::Signal;
use esp_hal::{
    gpio::Level,
    rmt::{Channel, PulseCode, Rmt, Tx, TxChannelConfig},
    time::Rate,
};

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

const BRIGHTNESS: u8 = 10;

#[embassy_executor::task]
pub async fn status_led_task(
    rmt: esp_hal::peripherals::RMT<'static>,
    gpio2: esp_hal::peripherals::GPIO2<'static>,
) {
    let rmt: Rmt<'_, esp_hal::Async> = Rmt::new(rmt, Rate::from_mhz(80))
        .expect("Failed to initialize RMT")
        .into_async();

    let tx_config = TxChannelConfig::default()
        .with_clk_divider(1)
        .with_idle_output_level(Level::Low)
        .with_idle_output(false);
    let mut channel = esp_hal::rmt::TxChannelCreator::configure_tx(rmt.channel0, &tx_config)
        .expect("Failed to configure RMT TX channel")
        .with_pin(gpio2);

    // Default state before any signal arrives.
    write_led(&mut channel, (0, 255, 0)).await;

    loop {
        let status = LED_STATUS.wait().await;
        let color = match status {
            LedStatus::Provisioning => (0, 255, 0),
            LedStatus::Idle => (0, 0, 0),
            LedStatus::Updating => (0, 0, 255),
        };

        write_led(&mut channel, color).await;
    }
}

async fn write_led(
    channel: &mut Channel<'_, esp_hal::Async, Tx>,
    (red, green, blue): (u8, u8, u8),
) {
    let mut data = [PulseCode::end_marker(); 25];
    let mut index = 0;

    // WS2812/SK6812-style LEDs expect GRB byte order.
    encode_byte(scale(green), &mut data, &mut index);
    encode_byte(scale(red), &mut data, &mut index);
    encode_byte(scale(blue), &mut data, &mut index);
    data[index] = PulseCode::end_marker();

    if let Err(e) = channel.transmit(&data).await {
        log::warn!("Failed to update status LED: {e:?}");
    }
}

fn scale(value: u8) -> u8 {
    ((value as u16 * BRIGHTNESS as u16) / u8::MAX as u16) as u8
}

fn encode_byte(byte: u8, data: &mut [PulseCode], index: &mut usize) {
    for bit in (0..8).rev() {
        data[*index] = if (byte & (1 << bit)) == 0 {
            ws2812_bit0()
        } else {
            ws2812_bit1()
        };
        *index += 1;
    }
}

fn ws2812_bit0() -> PulseCode {
    // 0-bit: ~350 ns high, ~800 ns low at 80 MHz.
    PulseCode::new(Level::High, 28, Level::Low, 64)
}

fn ws2812_bit1() -> PulseCode {
    // 1-bit: ~700 ns high, ~600 ns low at 80 MHz.
    PulseCode::new(Level::High, 56, Level::Low, 48)
}

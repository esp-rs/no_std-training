// To easily test this you can connect GPIO2 and GPIO4
// This way we will receive was we send. (loopback)

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    dma::{Dma, DmaPriority, DmaRxBuf, DmaTxBuf},
    dma_buffers,
    gpio::Io,
    prelude::*,
    spi::{master::Spi, SpiMode},
};
use esp_println::{print, println};

#[entry]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);
    let sclk = io.pins.gpio0;
    let miso = io.pins.gpio2;
    let mosi = io.pins.gpio4;
    let cs = io.pins.gpio5;

    let mut spi =
        Spi::new(peripherals.SPI2, 100.kHz(), SpiMode::Mode0).with_pins(sclk, mosi, miso, cs);

    let delay = Delay::new();

    loop {
        // ANCHOR: transfer
        // To transfer much larger amounts of data we can use DMA and
        // the CPU can even do other things while the transfer is in progress
        let mut data = [0x01u8, 0x02, 0x03, 0x04];
        spi.transfer(&mut data).unwrap();
        // ANCHOR_END: transfer
        println!("{:x?}", data);

        delay.delay_millis(2500u32);
    }
}

// To easily test this you can connect GPIO2 and GPIO4
// This way we will receive was we send. (loopback)

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    dma::{DmaRxBuf, DmaTxBuf},
    dma_buffers, main,
    spi::{
        master::{Config, Spi},
        Mode,
    },
    time::Rate,
};
use esp_println::{print, println};

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let sclk = peripherals.GPIO0;
    let miso = peripherals.GPIO2;
    let mosi = peripherals.GPIO4;
    let cs = peripherals.GPIO5;

    let mut spi = Spi::new(
        peripherals.SPI2,
        Config::default()
            .with_frequency(Rate::from_khz(100))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(sclk)
    .with_mosi(mosi)
    .with_miso(miso)
    .with_cs(cs);

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

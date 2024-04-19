// To easily test this you can connect GPIO2 and GPIO4
// This way we will receive was we send. (loopback)

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    clock::ClockControl,
    delay::Delay,
    dma::Dma,
    dma::DmaPriority,
    dma_buffers,
    gpio::IO,
    peripherals::Peripherals,
    prelude::*,
    spi::{
        master::{prelude::*, Spi},
        SpiMode,
    },
};
use esp_println::{print, println};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let sclk = io.pins.gpio0;
    let miso = io.pins.gpio2;
    let mosi = io.pins.gpio4;
    let cs = io.pins.gpio5;

    // ANCHOR: init-dma
    // we need to create the DMA driver and get a channel
    let dma = Dma::new(peripherals.DMA);
    let dma_channel = dma.channel0;

    // DMA transfers need descriptors and buffers
    let (mut tx_buffer, mut tx_descriptors, mut rx_buffer, mut rx_descriptors) = dma_buffers!(3200);
    // ANCHOR_END: init-dma

    // ANCHOR: configure-spi
    // we can call `.with_dma` on the SPI driver to make it use DMA
    let mut spi = Spi::new(peripherals.SPI2, 100u32.kHz(), SpiMode::Mode0, &clocks)
        .with_pins(Some(sclk), Some(mosi), Some(miso), Some(cs))
        .with_dma(dma_channel.configure(
            false,
            &mut tx_descriptors,
            &mut rx_descriptors,
            DmaPriority::Priority0,
        ));
    // ANCHOR_END: configure-spi

    let delay = Delay::new(&clocks);

    // populate the tx_buffer with data to send
    tx_buffer.fill(0x42);

    loop {
        // ANCHOR: transfer
        // `dma_transfer` will move the driver and the buffers into the
        // returned transfer.
        let transfer = spi.dma_transfer(&mut tx_buffer, &mut rx_buffer).unwrap();
        // ANCHOR_END: transfer

        // here the CPU could do other things while the transfer is taking done without using the CPU
        while !transfer.is_done() {
            print!(".");
        }

        // ANCHOR: transfer-wait
        // the buffers and spi are moved into the transfer and
        // we can get it back via `wait`
        // if the transfer isn't completed this will block
        transfer.wait().unwrap();
        // ANCHOR_END: transfer-wait

        println!();
        println!(
            "Received {:x?} .. {:x?}",
            &rx_buffer[..10],
            &rx_buffer[rx_buffer.len() - 10..]
        );

        delay.delay_millis(2500u32);
    }
}

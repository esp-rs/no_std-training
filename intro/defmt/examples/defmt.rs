#![no_std]
#![no_main]

// ANCHOR: println_include
use esp_backtrace as _;
use esp_println as _;
// ANCHOR_END: println_include
use esp_hal::{clock::ClockControl, peripherals::Peripherals, prelude::*, Delay};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();
    let mut delay = Delay::new(&clocks);

    defmt::trace!("trace");
    defmt::debug!("debug");
    defmt::info!("info");
    defmt::warn!("warn");
    defmt::error!("error");

    // panic!("Very useful panic message");

    loop {
        defmt::println!("Loop...");
        delay.delay_ms(500u32);
    }
}

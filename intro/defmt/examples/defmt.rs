#![no_std]
#![no_main]

// ANCHOR: println_include
use esp_backtrace as _;
use esp_println as _;
// ANCHOR_END: println_include
use esp_hal::{delay::Delay, main};

#[main]
fn main() -> ! {
    esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    defmt::trace!("trace");
    defmt::debug!("debug");
    defmt::info!("info");
    defmt::warn!("warn");
    defmt::error!("error");

    // panic!("Very useful panic message");

    loop {
        defmt::println!("Loop...");
        delay.delay_millis(500u32);
    }
}

#![no_std]
#![no_main]

//  Build the `esp_println` and `esp_backtrace` libs

use esp_hal::{clock::ClockControl, peripherals::Peripherals, prelude::*, Delay};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();
    let mut delay = Delay::new(&clocks);

    // Print a log or a message using defmt

    // Use a panic! macro to trigger a panic

    loop {
        defmt::println!("Loop...");
        delay.delay_ms(500u32);
    }
}

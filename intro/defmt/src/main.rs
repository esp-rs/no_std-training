#![no_std]
#![no_main]

//  Build the `esp_println` and `esp_backtrace` libs

use esp_hal::{
    clock::ClockControl, delay::Delay, peripherals::Peripherals, prelude::*, system::SystemControl,
};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();
    let delay = Delay::new(&clocks);

    // Print a log or a message using defmt

    // Use a panic! macro to trigger a panic

    loop {
        defmt::println!("Loop...");
        delay.delay_millis(500u32);
    }
}

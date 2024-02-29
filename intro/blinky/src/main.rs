#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_println::println;
use esp_hal::{clock::ClockControl, peripherals::Peripherals, prelude::*, Delay, IO};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    println!("Hello world!");

    // Set GPIO7 as an output, and set its state high initially.

    // Initialize the Delay peripheral, and use it to toggle the LED state in a
    // loop.
    loop {}
}

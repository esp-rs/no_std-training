#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, Io, Level, Output, Pull},
    peripherals::Peripherals,
    prelude::*,
    system::SystemControl,
};
use esp_println::println;

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let _system = SystemControl::new(peripherals.SYSTEM);

    println!("Hello world!");

    // Set GPIO7 as an output, and set its state high initially.

    // Check the button state and set the LED state accordingly.
    loop {}
}

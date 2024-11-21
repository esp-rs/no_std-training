#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, Level, Output, Pull},
    prelude::*,
};
use esp_println::println;

#[entry]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Hello world!");

    // Set GPIO7 as an output, and set its state high initially.

    // Check the button state and set the LED state accordingly.
    loop {}
}

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Level, Output},
    prelude::*,
};
use esp_println::println;

#[entry]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Hello world!");

    // Set GPIO7 as an output, and set its state high initially.

    // Initialize the Delay peripheral, and use it to toggle the LED state in a
    // loop.

    loop {}
}

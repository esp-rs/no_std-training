#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::prelude::*;
use esp_println::println;

#[entry]
fn main() -> ! {
    esp_hal::init(esp_hal::Config::default());

    println!("Hello world!");

    panic!("This is a panic");

    loop {}
}

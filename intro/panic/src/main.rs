#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_println::println;
use hal::{peripherals::Peripherals, prelude::*};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let _system = peripherals.SYSTEM.split();

    println!("Hello world!");

    loop {}
}

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{peripherals::Peripherals, prelude::*, system::SystemControl};
use esp_println::println;

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let _system = SystemControl::new(peripherals.SYSTEM);

    println!("Hello world!");

    panic!("This is a panic");

    loop {}
}

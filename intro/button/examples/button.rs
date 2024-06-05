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
    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);
    let mut led = Output::new(io.pins.gpio7, Level::Low);
    let button = Input::new(io.pins.gpio9, Pull::Up);

    // Check the button state and set the LED state accordingly.
    loop {
        if button.is_high() {
            led.set_high();
        } else {
            led.set_low();
        }
    }
}

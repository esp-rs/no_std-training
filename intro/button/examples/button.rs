#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig, Level, Output, OutputConfig},
    main,
};
use esp_println::println;

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Hello world!");

    // Set GPIO7 as an output, and set its state high initially.
    let mut led = Output::new(peripherals.GPIO7, Level::Low, OutputConfig::default());
    let button = Input::new(peripherals.GPIO9, InputConfig::default());

    // Check the button state and set the LED state accordingly.
    loop {
        if button.is_high() {
            led.set_high();
        } else {
            led.set_low();
        }
    }
}

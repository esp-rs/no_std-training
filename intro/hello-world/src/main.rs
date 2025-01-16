#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{delay::Delay, main};
use log::info;

#[main]
fn main() -> ! {
    esp_println::logger::init_logger_from_env();

    let delay = Delay::new();
    loop {
        info!("Hello world!");
        delay.delay_millis(500);
    }
}

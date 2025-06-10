#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::main;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    esp_hal::init(esp_hal::Config::default());

    println!("Hello world!");

    loop {}
}

#![no_std]
#![no_main]

//  Build the `esp_println` and `esp_backtrace` libs

use esp_hal::{delay::Delay, main};

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // Print a log or a message using defmt

    // Use a panic! macro to trigger a panic

    loop {
        defmt::println!("Loop...");
        delay.delay_millis(500u32);
    }
}

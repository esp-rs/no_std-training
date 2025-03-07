#![no_std]
#![no_main]

use core::cell::RefCell;
use critical_section::Mutex;
use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Event, Input, InputConfig, Io, Level, Output, OutputConfig},
    handler, main,
};
use esp_println::println;

static BUTTON: Mutex<RefCell<Option<Input>>> = Mutex::new(RefCell::new(None));

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Hello world!");

    let mut io = Io::new(peripherals.IO_MUX);
    // Set the interrupt handler for GPIO interrupts.
    io.set_interrupt_handler(handler);

    // Set GPIO7 as an output, and set its state high initially.
    let mut led = Output::new(peripherals.GPIO7, Level::Low, OutputConfig::default());

    // Set GPIO9 as an input
    let mut button = Input::new(peripherals.GPIO9, InputConfig::default());

    // ANCHOR: critical_section
    critical_section::with(|cs| {
        button.listen(Event::FallingEdge);
        BUTTON.borrow_ref_mut(cs).replace(button)
    });
    // ANCHOR_END: critical_section

    let delay = Delay::new();
    loop {
        led.toggle();
        delay.delay_millis(500u32);
    }
}

#[handler]
fn handler() {
    critical_section::with(|cs| {
        println!("GPIO interrupt");
        BUTTON
            .borrow_ref_mut(cs)
            .as_mut()
            .unwrap()
            .clear_interrupt();
    });
}

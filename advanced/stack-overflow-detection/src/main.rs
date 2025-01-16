#![no_std]
#![no_main]

use core::cell::RefCell;
use core::ptr::addr_of_mut;

use critical_section::Mutex;
use esp_backtrace as _;
use esp_hal::{assist_debug::DebugAssist, handler, interrupt::InterruptConfigurable, main, ram};
use esp_println::println;

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    // get the debug assist driver
    let da = DebugAssist::new(peripherals.ASSIST_DEBUG);
    da.set_interrupt_handler(interrupt_handler);

    boom();

    loop {}
}

#[inline(never)]
fn boom() {
    deadly_recursion([0u8; 2048]);
}

#[ram]
#[allow(unconditional_recursion)]
fn deadly_recursion(data: [u8; 2048]) {
    static mut COUNTER: u32 = 0;

    println!(
        "Iteration {}, data {:02x?}...",
        unsafe { COUNTER },
        &data[0..10]
    );

    unsafe {
        COUNTER = COUNTER.wrapping_add(1);
    };

    deadly_recursion([0u8; 2048]);
}

#[handler(priority = esp_hal::interrupt::Priority::min())]
fn interrupt_handler() {}

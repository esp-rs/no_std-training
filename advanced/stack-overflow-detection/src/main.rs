#![no_std]
#![no_main]

use core::cell::RefCell;

use critical_section::Mutex;
use esp_backtrace as _;
use esp_hal::{
    assist_debug::DebugAssist, clock::ClockControl, peripherals::Peripherals, prelude::*,
};
use esp_println::println;

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let _ = ClockControl::boot_defaults(system.clock_control).freeze();

    // get the debug assist driver
    let da = DebugAssist::new(peripherals.ASSIST_DEBUG, Some(interrupt_handler));

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

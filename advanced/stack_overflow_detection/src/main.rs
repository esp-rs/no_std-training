#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_println::println;
use hal::{assist_debug::DebugAssist, peripherals::Peripherals, prelude::*};

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let mut peripheral_clock_control = system.peripheral_clock_control;

    // get the debug assist driver
    let da = DebugAssist::new(peripherals.ASSIST_DEBUG, &mut peripheral_clock_control);

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

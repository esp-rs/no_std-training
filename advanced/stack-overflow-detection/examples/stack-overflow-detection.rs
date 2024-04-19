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

    // set up stack overflow protection
    install_stack_guard(da, 4096);

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

// ANCHOR: debug_assists
static DA: Mutex<RefCell<Option<DebugAssist>>> = Mutex::new(RefCell::new(None));

fn install_stack_guard(mut da: DebugAssist<'static>, safe_area_size: u32) {
    extern "C" {
        static mut _stack_end: u32;
        static mut _stack_start: u32;
    }
    let stack_low = unsafe { (&mut _stack_end as *mut _ as *mut u32) as u32 };
    let stack_high = unsafe { (&mut _stack_start as *mut _ as *mut u32) as u32 };
    println!(
        "Safe stack {} bytes",
        stack_high - stack_low - safe_area_size
    );
    da.enable_region0_monitor(stack_low, stack_low + safe_area_size, true, true);

    critical_section::with(|cs| DA.borrow_ref_mut(cs).replace(da));
}
// ANCHOR_END: debug_assists

// ANCHOR: interrupt
// ANCHOR: handler
#[handler(priority = esp_hal::interrupt::Priority::min())]
fn interrupt_handler() {
    // ANCHOR_END: interrupt

    critical_section::with(|cs| {
        println!("\n\nPossible Stack Overflow Detected");
        let mut da = DA.borrow_ref_mut(cs);
        let da = da.as_mut().unwrap();

        if da.is_region0_monitor_interrupt_set() {
            let pc = da.get_region_monitor_pc();
            println!("PC = 0x{:x}", pc);
            da.clear_region0_monitor_interrupt();
            da.disable_region0_monitor();
            loop {}
        }
    });
}
// ANCHOR_END: handler

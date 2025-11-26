use embassy_sync::signal::Signal;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::gpio::Input;
use log::debug;

pub static BUTTON_PRESSED: Signal<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, ()> =
    Signal::new();

#[embassy_executor::task]
pub async fn button_monitor(
    button: Input<'static>,
    button_pressed: &'static Signal<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, ()>,
) {
    debug!("Button monitor: Waiting for button press...");
    let mut last_state = button.is_high();

    loop {
        Timer::after(EmbassyDuration::from_millis(50)).await;

        let current_state = button.is_high();

        // Detect falling edge (button press - goes from high to low due to pull-up)
        if last_state && !current_state {
            log::info!("Button pressed!");
            button_pressed.signal(());
        }

        last_state = current_state;
    }
}

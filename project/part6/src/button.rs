use embassy_sync::signal::Signal;
use esp_hal::gpio::Input;
use log::debug;

pub static BUTTON_PRESSED: Signal<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, ()> =
    Signal::new();

#[embassy_executor::task]
pub async fn button_monitor(
    mut button: Input<'static>,
    button_pressed: &'static Signal<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, ()>,
) {
    debug!("Button monitor: Waiting for button press...");

    loop {
        // Wait for falling edge (button press - goes from high to low due to pull-up)
        button.wait_for_falling_edge().await;
        log::info!("Button pressed!");
        button_pressed.signal(());
    }
}

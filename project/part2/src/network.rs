use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{Config as WifiConfig, Interface, WifiController, sta::StationConfig};
use log::{debug, error, info};

// ANCHOR: wifi_env
const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
// ANCHOR_END: wifi_env

// ANCHOR: connection_task
#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    debug!("start connection task");
    loop {
        if controller.is_connected() {
            let _ = controller.wait_for_disconnect_async().await;
            Timer::after(Duration::from_millis(5000)).await;
            continue;
        }

        let station_config = WifiConfig::Station(
            StationConfig::default()
                .with_ssid(SSID)
                .with_password(PASSWORD.into()),
        );
        controller
            .set_config(&station_config)
            .expect("Failed to set WiFi configuration");

        debug!("About to connect...");
        match controller.connect_async().await {
            Ok(_) => info!("Wifi connected!"),
            Err(e) => {
                error!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}
// ANCHOR_END: connection_task

// ANCHOR: net_task
#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}
// ANCHOR_END: net_task

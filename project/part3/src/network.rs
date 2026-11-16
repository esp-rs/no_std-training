use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, Interface, WifiController, sta::StationConfig,
};
use log::{error, info};

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().expect("SSID is longer than 32 bytes"))
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD
                    .try_into()
                    .expect("PASSWORD is longer than 64 bytes"),
            )),
    );
    controller
        .set_config(&station_config)
        .expect("Failed to set Wi-Fi configuration");

    loop {
        match controller.connect_async().await {
            Ok(_) => {
                info!("Wi-Fi connected!");
                let _ = controller.wait_for_disconnect_async().await;
                info!("Wi-Fi disconnected");
            }
            Err(e) => error!("Failed to connect to Wi-Fi: {e:?}"),
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}

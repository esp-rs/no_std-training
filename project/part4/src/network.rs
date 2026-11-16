use core::net::Ipv4Addr;
use embassy_net::{Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, Interface, WifiController, ap::AccessPointConfig,
    sta::StationConfig,
};
use heapless::String;
use log::{error, info};
use serde::Deserialize;
use static_cell::StaticCell;

// ANCHOR: wifi_credentials
#[derive(Debug, Deserialize)]
pub struct WifiCredentials {
    pub ssid: String<32>,
    pub password: String<64>,
}
// ANCHOR_END: wifi_credentials

// ANCHOR: credentials_signal
/// Credentials submitted through the provisioning page.
pub static WIFI_CREDENTIALS: Signal<CriticalSectionRawMutex, WifiCredentials> = Signal::new();
// ANCHOR_END: credentials_signal

pub struct NetworkStacks {
    pub ap_stack: Stack<'static>,
    pub ap_runner: Runner<'static, Interface>,
    pub sta_stack: Stack<'static>,
    pub sta_runner: Runner<'static, Interface>,
}

pub fn create_network_stacks(gw_ip_addr: Ipv4Addr) -> NetworkStacks {
    // ANCHOR: stack_configs
    let ap_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr, 24),
        gateway: Some(gw_ip_addr),
        dns_servers: Default::default(),
    });
    let sta_config = embassy_net::Config::dhcpv4(Default::default());
    // ANCHOR_END: stack_configs

    let rng = Rng::new();

    // ANCHOR: ap_stack
    // DHCP server, captive portal DNS server, HTTP server, and spare sockets
    static AP_STACK_RESOURCES: StaticCell<StackResources<6>> = StaticCell::new();
    let (ap_stack, ap_runner) = embassy_net::new(
        Interface::access_point(),
        ap_config,
        AP_STACK_RESOURCES.init(StackResources::new()),
        (rng.random() as u64) << 32 | rng.random() as u64,
    );
    // ANCHOR_END: ap_stack

    static STA_STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (sta_stack, sta_runner) = embassy_net::new(
        Interface::station(),
        sta_config,
        STA_STACK_RESOURCES.init(StackResources::new()),
        (rng.random() as u64) << 32 | rng.random() as u64,
    );

    NetworkStacks {
        ap_stack,
        ap_runner,
        sta_stack,
        sta_runner,
    }
}

// ANCHOR: runner_task
#[embassy_executor::task(pool_size = 2)]
pub async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
// ANCHOR_END: runner_task

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    // ANCHOR: ap_mode
    // Start in access point mode for provisioning
    let ap_config = Config::AccessPoint(
        AccessPointConfig::default().with_ssid("esp-radio".try_into().unwrap()),
    );
    controller
        .set_config(&ap_config)
        .expect("Failed to start the access point");
    info!("Wi-Fi access point started");
    // ANCHOR_END: ap_mode

    // ANCHOR: receive_credentials
    let credentials = WIFI_CREDENTIALS.wait().await;
    info!("Credentials received for SSID {}", credentials.ssid);

    // Give the HTTP server time to send its response before the access point stops
    Timer::after(Duration::from_secs(2)).await;
    // ANCHOR_END: receive_credentials

    // ANCHOR: station_mode
    // The conversions cannot fail: `WifiCredentials` bounds the lengths
    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(credentials.ssid.as_str().try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                credentials.password.as_str().try_into().unwrap(),
            )),
    );
    // This replaces the access point configuration and restarts Wi-Fi in station mode
    controller
        .set_config(&station_config)
        .expect("Failed to configure station mode");
    // ANCHOR_END: station_mode

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

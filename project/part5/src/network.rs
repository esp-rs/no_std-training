use core::net::Ipv4Addr;
use embassy_net::{Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_sync::channel::Channel;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    Config as WifiConfig, Interface, WifiController, ap::AccessPointConfig, sta::StationConfig,
};
use heapless::String;
use log::{debug, error, info};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct WifiCredentials {
    pub ssid: String<32>,
    pub password: String<64>,
}

pub struct NetworkStacks {
    pub ap_stack: Stack<'static>,
    pub ap_runner: Runner<'static, Interface<'static>>,
    pub sta_stack: Stack<'static>,
    pub sta_runner: Runner<'static, Interface<'static>>,
}

pub fn create_network_stacks(
    ap_device: Interface<'static>,
    sta_device: Interface<'static>,
    gw_ip_addr: Ipv4Addr,
) -> NetworkStacks {
    let ap_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr, 24),
        gateway: Some(gw_ip_addr),
        dns_servers: Default::default(),
    });
    let sta_config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack for AP (provisioning)
    static AP_STACK_RESOURCES_CELL: static_cell::StaticCell<StackResources<6>> =
        static_cell::StaticCell::new();
    let (ap_stack, ap_runner) = embassy_net::new(
        ap_device,
        ap_config,
        AP_STACK_RESOURCES_CELL
            .uninit()
            .write(StackResources::<6>::new()),
        seed,
    );

    // Init network stack for STA (client connection)
    static STA_STACK_RESOURCES_CELL: static_cell::StaticCell<StackResources<6>> =
        static_cell::StaticCell::new();
    let (sta_stack, sta_runner) = embassy_net::new(
        sta_device,
        sta_config,
        STA_STACK_RESOURCES_CELL
            .uninit()
            .write(StackResources::<6>::new()),
        seed,
    );

    NetworkStacks {
        ap_stack,
        ap_runner,
        sta_stack,
        sta_runner,
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
pub async fn sta_net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
pub async fn connection(
    mut controller: WifiController<'static>,
    wifi_credentials_channel: &'static Channel<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        WifiCredentials,
        1,
    >,
) {
    debug!("start connection task");

    // Start in AP mode first for provisioning. `set_config` starts/restarts the
    // Wi-Fi controller as needed in esp-radio 0.18.
    let ap_config = WifiConfig::AccessPoint(AccessPointConfig::default().with_ssid("esp-radio"));
    controller
        .set_config(&ap_config)
        .expect("Failed to set WiFi configuration");
    debug!("WiFi AP started!");

    // Wait for credentials
    debug!("Waiting for WiFi credentials...");
    let credentials = wifi_credentials_channel.receiver().receive().await;
    info!("Credentials received! SSID: {}", credentials.ssid);

    // Give the HTTP handler time to send the saved page before switching off AP mode.
    debug!("Delaying AP shutdown to allow HTTP response to complete...");
    Timer::after(EmbassyDuration::from_secs(2)).await;

    // Configure station mode. This replaces the AP configuration and restarts Wi-Fi.
    debug!("Configuring station mode...");
    let station_config = StationConfig::default()
        .with_ssid(credentials.ssid.as_str())
        .with_password(credentials.password.as_str().into());

    let sta_config = WifiConfig::Station(station_config);
    controller
        .set_config(&sta_config)
        .expect("Failed to set station mode WiFi configuration");
    debug!("WiFi station configured!");

    // Connect to the network
    info!("Connecting to WiFi network...");
    loop {
        match controller.connect_async().await {
            Ok(_) => {
                info!("Successfully connected to WiFi!");

                let _ = controller.wait_for_disconnect_async().await;
                info!("Disconnected from WiFi, will attempt to reconnect...");
            }
            Err(e) => {
                error!("Failed to connect: {:?}", e);
                debug!("Retrying in 5 seconds...");
                Timer::after(EmbassyDuration::from_secs(5)).await;
            }
        }
    }
}

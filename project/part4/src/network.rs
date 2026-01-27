use core::net::Ipv4Addr;
use embassy_net::{Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_sync::channel::Channel;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    AccessPointConfig, ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent,
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
    pub ap_runner: Runner<'static, WifiDevice<'static>>,
    pub sta_stack: Stack<'static>,
    pub sta_runner: Runner<'static, WifiDevice<'static>>,
}

pub fn create_network_stacks(
    ap_device: WifiDevice<'static>,
    sta_device: WifiDevice<'static>,
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
    // Increased from 3 to 6 to accommodate: DHCP UDP socket, Captive Portal UDP socket,
    // HTTP TCP socket, and some buffer for concurrent connections
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
    static STA_STACK_RESOURCES_CELL: static_cell::StaticCell<StackResources<3>> =
        static_cell::StaticCell::new();
    let (sta_stack, sta_runner) = embassy_net::new(
        sta_device,
        sta_config,
        STA_STACK_RESOURCES_CELL
            .uninit()
            .write(StackResources::<3>::new()),
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
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
pub async fn sta_net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
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
    debug!("Device capabilities: {:?}", controller.capabilities());

    // Start in AP mode first for provisioning
    let ap_config =
        ModeConfig::AccessPoint(AccessPointConfig::default().with_ssid("esp-radio".into()));
    controller
        .set_config(&ap_config)
        .expect("Failed to set WiFi configuration");
    info!("Starting WiFi in AP mode");
    controller
        .start_async()
        .await
        .expect("Failed to start WiFi");
    debug!("WiFi AP started!");

    // Wait for credentials
    debug!("Waiting for WiFi credentials...");
    let credentials = wifi_credentials_channel.receiver().receive().await;
    info!("Credentials received! SSID: {}", credentials.ssid);

    // Give the HTTP handler time to send the saved page before dropping AP
    debug!("Delaying AP shutdown to allow HTTP response to complete...");
    Timer::after(EmbassyDuration::from_secs(2)).await;

    // Stop the AP
    debug!("Stopping AP mode...");
    controller.stop_async().await.expect("Failed to stop WiFi");
    debug!("AP stopped");

    Timer::after(EmbassyDuration::from_secs(1)).await;

    // Configure and start station mode
    debug!("Configuring station mode...");
    let client_config = ClientConfig::default()
        .with_ssid(credentials.ssid.as_str().into())
        .with_password(credentials.password.as_str().into());

    let sta_config = ModeConfig::Client(client_config);
    controller
        .set_config(&sta_config)
        .expect("Failed to set station mode WiFi configuration");

    debug!("Starting WiFi in station mode...");
    controller
        .start_async()
        .await
        .expect("Failed to start WiFi");
    debug!("WiFi station started!");

    // Connect to the network
    info!("Connecting to WiFi network...");
    loop {
        match controller.connect_async().await {
            Ok(()) => {
                info!("Successfully connected to WiFi!");

                // Wait for disconnect event
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
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

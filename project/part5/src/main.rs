// MQTT Communication (with wifi provisioning)
// 1. Install tools
// cargo install --git https://github.com/bytebeamio/rumqtt rumqttd
// brew install mosquitto / https://mosquitto.org/download/
// 2. Get your IP
// ipconfig getifaddr en0 ow ip addr show eth0
// 3. Run the broker
// rumqttd
// 4. Subscribe to the topic
// mosquitto_sub -h <IP> -p 1884 -V mqttv5 -i mac-subscriber -t 'measeurement/#' -v
// 5. Run the app
// BROKER_HOST="<IP>" BROKER_PORT="1884" cargo r -r

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::{fmt::Write, net::Ipv4Addr, str::FromStr, time::Duration};

use edge_captive::io::run;
use embassy_executor::Spawner;
use embassy_net::{
    IpAddress, IpListenEndpoint, Ipv4Address, Ipv4Cidr, Runner, Stack, StackResources,
    StaticConfigV4, dns::DnsQueryType, tcp::TcpSocket,
};
use embassy_sync::{channel::Channel, signal::Signal};
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config, I2c},
    interrupt::software::SoftwareInterruptControl,
    ram,
    rng::Rng,
    timer::timg::TimerGroup,
};
use esp_println::println;
use esp_radio::{
    Controller,
    wifi::{AccessPointConfig, ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent},
};
use heapless::String;
use log::{error, info};
use rust_mqtt::{
    client::{client::MqttClient, client_config::ClientConfig as MqttClientConfig},
    packet::v5::reason_codes::ReasonCode,
    utils::rng_generator::CountingRng,
};
use shtcx::{
    self,
    asynchronous::{PowerMode, ShtC3, max_measurement_duration, shtc3},
};

esp_bootloader_esp_idf::esp_app_desc!();

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const BROKER_HOST: Option<&'static str> = option_env!("BROKER_HOST");
const BROKER_PORT: Option<&'static str> = option_env!("BROKER_PORT");

#[derive(Clone, Debug)]
struct WifiCredentials {
    ssid: heapless::String<32>,
    password: heapless::String<64>,
}

static WIFI_CREDENTIALS_CHANNEL: Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    WifiCredentials,
    1,
> = Channel::new();

static WIFI_CONNECTED: Signal<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, ()> =
    Signal::new();

const GW_IP_ADDR_ENV: Option<&'static str> = option_env!("GATEWAY_IP");
// HTML templates embedded at compile time
const HOME_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/templates/home.html"
));
const SAVED_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/templates/saved.html"
));

fn parse_ipv4_address(s: &str) -> Option<IpAddress> {
    let mut parts_iter = s.split('.');
    let a = parts_iter.next()?.parse::<u8>().ok()?;
    let b = parts_iter.next()?.parse::<u8>().ok()?;
    let c = parts_iter.next()?.parse::<u8>().ok()?;
    let d = parts_iter.next()?.parse::<u8>().ok()?;
    // Ensure there are exactly 4 parts
    if parts_iter.next().is_some() {
        return None;
    }
    Some(IpAddress::Ipv4(Ipv4Address::new(a, b, c, d)))
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let sda = peripherals.GPIO10;
    let scl = peripherals.GPIO8;
    let i2c = I2c::new(peripherals.I2C0, Config::default())
        .unwrap()
        .with_sda(sda)
        .with_scl(scl)
        .into_async();
    let mut sht = shtc3(i2c);

    println!(
        "Raw ID register: {}",
        sht.raw_id_register()
            .await
            .expect("Failed to get raw ID register")
    );

    let esp_radio_ctrl = &*mk_static!(Controller<'static>, esp_radio::init().unwrap());

    let (controller, interfaces) =
        esp_radio::wifi::new(esp_radio_ctrl, peripherals.WIFI, Default::default()).unwrap();

    // Start with AP device for provisioning
    let ap_device = interfaces.ap;
    // Store STA device for later use
    let sta_device = interfaces.sta;

    let gw_ip_addr_str = GW_IP_ADDR_ENV.unwrap_or("192.168.2.1");
    let gw_ip_addr = Ipv4Addr::from_str(gw_ip_addr_str).expect("failed to parse gateway ip");

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
    let (ap_stack, ap_runner) = embassy_net::new(
        ap_device,
        ap_config,
        mk_static!(StackResources<6>, StackResources::<6>::new()),
        seed,
    );

    // Init network stack for STA (client connection)
    let (sta_stack, sta_runner) = embassy_net::new(
        sta_device,
        sta_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(ap_runner)).ok();
    spawner.spawn(sta_net_task(sta_runner)).ok();
    spawner.spawn(run_dhcp(ap_stack, gw_ip_addr)).ok();
    spawner.spawn(run_captive_portal(ap_stack, gw_ip_addr)).ok();
    spawner.spawn(mqtt_task(sta_stack, sht)).ok();

    loop {
        if ap_stack.is_link_up() {
            break;
        }
        Timer::after(EmbassyDuration::from_millis(500)).await;
    }
    println!("WiFi Provisioning Portal Ready");
    println!("1. Connect to the AP: `esp-radio`");
    println!("2. Navigate to: http://{gw_ip_addr_str}/");
    while !ap_stack.is_config_up() {
        Timer::after(EmbassyDuration::from_millis(100)).await
    }
    ap_stack
        .config_v4()
        .inspect(|c| println!("ipv4 config: {c:?}"));

    spawner.spawn(run_http_server(ap_stack)).ok();

    // Keep main task alive
    loop {
        Timer::after(EmbassyDuration::from_secs(60)).await;
    }
}

// Define the form structure for WiFi credentials
#[derive(serde::Deserialize)]
struct WifiForm {
    ssid: heapless::String<32>,
    password: heapless::String<64>,
}

// Create router with picoserve
fn make_app() -> picoserve::Router<
    impl picoserve::routing::PathRouter<(), picoserve::routing::NoPathParameters>,
    (),
    picoserve::routing::NoPathParameters,
> {
    picoserve::Router::new()
        .route("/", picoserve::routing::get(home_handler))
        .route("/save", picoserve::routing::post(save_handler))
        .route("/generate_204", picoserve::routing::get(captive_redirect))
        .route("/gen_204", picoserve::routing::get(captive_redirect))
        .route("/ncsi.txt", picoserve::routing::get(captive_redirect))
        .route(
            "/connecttest.txt",
            picoserve::routing::get(captive_redirect),
        )
}

async fn home_handler() -> (
    picoserve::response::StatusCode,
    &'static [(&'static str, &'static str)],
    &'static str,
) {
    (
        picoserve::response::StatusCode::OK,
        &[("Content-Type", "text/html; charset=utf-8")],
        HOME_HTML,
    )
}

async fn save_handler(
    form: picoserve::extract::Form<WifiForm>,
) -> (
    picoserve::response::StatusCode,
    &'static [(&'static str, &'static str)],
    &'static str,
) {
    println!(
        "WiFi Credentials Received: SSID: {} | Password: {}",
        form.0.ssid, form.0.password
    );

    // Send credentials to the connection task
    let credentials = WifiCredentials {
        ssid: form.0.ssid,
        password: form.0.password,
    };
    println!("Sending credentials to connection task...");
    WIFI_CREDENTIALS_CHANNEL.sender().send(credentials).await;
    println!("Credentials sent!");

    (
        picoserve::response::StatusCode::OK,
        &[("Content-Type", "text/html; charset=utf-8")],
        SAVED_HTML,
    )
}

async fn captive_redirect() -> picoserve::response::Redirect {
    picoserve::response::Redirect::to("/")
}

#[embassy_executor::task]
async fn run_http_server(stack: Stack<'static>) {
    let app = make_app();

    const HTTP_PORT: u16 = 80;
    println!("Starting HTTP server on port {HTTP_PORT}");

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(EmbassyDuration::from_secs(5)),
        read_request: Some(EmbassyDuration::from_secs(5)),
        write: Some(EmbassyDuration::from_secs(3)),
    })
    .keep_connection_alive();

    loop {
        let mut rx_buffer = [0; 2048];
        let mut tx_buffer = [0; 2048];
        let mut http_buffer = [0; 2048];

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(EmbassyDuration::from_secs(10)));

        println!("HTTP: Waiting for connection...");
        let result = socket
            .accept(IpListenEndpoint {
                addr: None,
                port: HTTP_PORT,
            })
            .await;

        if let Err(e) = result {
            println!("HTTP accept error: {:?}", e);
            Timer::after(EmbassyDuration::from_millis(100)).await;
            continue;
        }

        println!("HTTP: Client connected");

        match picoserve::serve(&app, &config, &mut http_buffer, socket).await {
            Ok(handled_requests_count) => {
                println!("HTTP: Handled {} requests", handled_requests_count);
            }
            Err(e) => {
                println!("HTTP error: {:?}", e);
            }
        }

        Timer::after(EmbassyDuration::from_millis(100)).await;
    }
}

#[embassy_executor::task]
async fn run_dhcp(stack: Stack<'static>, gw_ip_addr: Ipv4Addr) {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use edge_dhcp::{
        io::{self, DEFAULT_SERVER_PORT},
        server::{Server, ServerOptions},
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let mut buf = [0u8; 1500];

    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];

    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let unbound_socket = Udp::new(stack, &buffers);
    let mut bound_socket = unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .unwrap();

    loop {
        _ = io::server::run(
            &mut Server::<_, 64>::new_with_et(gw_ip_addr),
            &ServerOptions::new(gw_ip_addr, Some(&mut gw_buf)),
            &mut bound_socket,
            &mut buf,
        )
        .await
        .inspect_err(|e| log::warn!("DHCP server error: {e:?}"));
        Timer::after(EmbassyDuration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
async fn run_captive_portal(stack: Stack<'static>, gw_ip_addr: Ipv4Addr) {
    use core::net::{SocketAddr, SocketAddrV4};
    use edge_nal_embassy::{Udp, UdpBuffers};

    const DNS_PORT: u16 = 8853;

    let mut tx_buf = [0u8; 1500];
    let mut rx_buf = [0u8; 1500];

    println!("Starting Captive Portal DNS server on port {DNS_PORT}");
    println!("All DNS queries will resolve to {gw_ip_addr}");

    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let udp_stack = Udp::new(stack, &buffers);

    loop {
        println!("Starting Captive Portal DNS server");
        _ = run(
            &udp_stack,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DNS_PORT)),
            &mut tx_buf,
            &mut rx_buf,
            gw_ip_addr,
            Duration::from_secs(60),
        )
        .await
        .inspect_err(|e| log::warn!("Captive Portal DNS server error: {e:?}"));
        Timer::after(EmbassyDuration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.capabilities());

    // Start in AP mode first for provisioning
    let ap_config =
        ModeConfig::AccessPoint(AccessPointConfig::default().with_ssid("esp-radio".into()));
    controller.set_config(&ap_config).unwrap();
    println!("Starting WiFi in AP mode");
    controller.start_async().await.unwrap();
    println!("WiFi AP started!");

    // Wait for credentials
    println!("Waiting for WiFi credentials...");
    let credentials = WIFI_CREDENTIALS_CHANNEL.receiver().receive().await;
    println!("Credentials received! SSID: {}", credentials.ssid);

    // Give the HTTP handler time to send the saved page before dropping AP
    println!("Delaying AP shutdown to allow HTTP response to complete...");
    Timer::after(EmbassyDuration::from_secs(2)).await;

    // Stop the AP
    println!("Stopping AP mode...");
    controller.stop_async().await.unwrap();
    println!("AP stopped");

    Timer::after(EmbassyDuration::from_secs(1)).await;

    // Configure and start station mode
    println!("Configuring station mode...");
    let client_config = ClientConfig::default()
        .with_ssid(credentials.ssid.as_str().into())
        .with_password(credentials.password.as_str().into());

    let sta_config = ModeConfig::Client(client_config);
    controller.set_config(&sta_config).unwrap();

    println!("Starting WiFi in station mode...");
    controller.start_async().await.unwrap();
    println!("WiFi station started!");

    // Connect to the network
    println!("Connecting to WiFi network...");
    loop {
        match controller.connect_async().await {
            Ok(()) => {
                println!("Successfully connected to WiFi!");
                // Signal that WiFi is connected
                WIFI_CONNECTED.signal(());

                // Wait for disconnect event
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                println!("Disconnected from WiFi, will attempt to reconnect...");
            }
            Err(e) => {
                println!("Failed to connect: {:?}", e);
                println!("Retrying in 5 seconds...");
                Timer::after(EmbassyDuration::from_secs(5)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn sta_net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn mqtt_task(
    stack: Stack<'static>,
    mut sht: ShtC3<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
) {
    // Wait for WiFi connection
    println!("HTTP Client: Waiting for WiFi connection...");
    WIFI_CONNECTED.wait().await;
    println!("HTTP Client: WiFi connected, waiting for network configuration...");

    // Wait for DHCP to assign an IP address
    loop {
        if stack.is_config_up() {
            break;
        }
        Timer::after(EmbassyDuration::from_millis(100)).await;
    }

    println!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            println!("Got IP: {}", config.address);
            break;
        }
        Timer::after(EmbassyDuration::from_millis(500)).await;
    }

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        Timer::after(EmbassyDuration::from_millis(1_000)).await;

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        let host = match BROKER_HOST {
            Some(h) => h,
            None => {
                error!(
                    "No BROKER_HOST set. Provide e.g. BROKER_HOST=10.0.0.10 (or hostname) and optional BROKER_PORT."
                );
                continue;
            }
        };

        // Default to rumqttd's v5 listener port (1884) unless overridden
        let port: u16 = BROKER_PORT
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(1884);

        // If host is an IPv4 literal, bypass DNS
        let address = if let Some(ip) = parse_ipv4_address(host) {
            ip
        } else {
            match stack.dns_query(host, DnsQueryType::A).await.map(|a| a[0]) {
                Ok(address) => address,
                Err(e) => {
                    error!("DNS lookup error: {e:?}");
                    continue;
                }
            }
        };

        let remote_endpoint = (address, port);
        info!("connecting to MQTT broker at {}:{}...", host, port);
        let connection = socket.connect(remote_endpoint).await;
        if let Err(e) = connection {
            error!("connect error: {:?}", e);
            continue;
        }
        info!("connected!");

        let mut config = MqttClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(20000),
        );
        config.add_max_subscribe_qos(rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1);
        config.add_client_id("esp32c3");
        config.max_packet_size = 1024;
        let mut recv_buffer = [0; 512];
        let mut write_buffer = [0; 512];

        let write_len = write_buffer.len();
        let recv_len = recv_buffer.len();
        let mut client = MqttClient::<_, 5, _>::new(
            socket,
            &mut write_buffer,
            write_len,
            &mut recv_buffer,
            recv_len,
            config,
        );

        match client.connect_to_broker().await {
            Ok(()) => {}
            Err(mqtt_error) => match mqtt_error {
                ReasonCode::NetworkError => {
                    error!("MQTT Network Error");
                    continue;
                }
                _ => {
                    error!("Other MQTT Error: {:?}", mqtt_error);
                    continue;
                }
            },
        }

        loop {
            // Read sensor
            if let Err(e) = sht.start_measurement(PowerMode::NormalMode).await {
                println!("Failed to start measurement: {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
            // Wait for 12.1 ms https://github.com/Fristi/shtcx-rs/blob/feature/async-support/src/asynchronous.rs#L413-L424
            let duration = max_measurement_duration(&sht, PowerMode::NormalMode);
            Timer::after(Duration::from_micros(duration.into())).await;
            let measurement = match sht.get_measurement_result().await {
                Ok(m) => m,
                Err(e) => {
                    println!("Failed to get measurement result: {:?}", e);
                    Timer::after(Duration::from_secs(1)).await;
                    continue;
                }
            };

            println!(
                "  {:.2} °C | {:.2} %RH",
                measurement.temperature.as_degrees_celsius(),
                measurement.humidity.as_percent(),
            );

            let mut temperature_string: String<32> = String::new();
            write!(
                temperature_string,
                "{:.2}",
                measurement.temperature.as_degrees_celsius()
            )
            .expect("write! failed!");

            // MQTT
            match client
                .send_message(
                    "measeurement/temperature",
                    temperature_string.as_bytes(),
                    rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1,
                    true,
                )
                .await
            {
                Ok(()) => {}
                Err(mqtt_error) => match mqtt_error {
                    ReasonCode::NetworkError => {
                        error!("MQTT Network Error");
                        continue;
                    }
                    _ => {
                        error!("Other MQTT Error: {:?}", mqtt_error);
                        continue;
                    }
                },
            }

            // Delay
            Timer::after(EmbassyDuration::from_secs(1)).await;
        }
    }
}

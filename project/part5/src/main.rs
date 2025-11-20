// MQTT Communication (with wifi provisioning)
// 1. Install tools
// cargo install --git https://github.com/bytebeamio/rumqtt rumqttd
// brew install mosquitto / https://mosquitto.org/download/
// 2. Get your IP
// ipconfig getifaddr en0 ow ip addr show eth0
// 3. Run the broker
// rumqttd
// 4. Subscribe to the topic
// mosquitto_sub -h <IP> -p 1884 -V mqttv5 -i mac-subscriber -t 'measurement/#' -v
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

use core::{
    fmt::Debug,
    fmt::Write as FmtWrite,
    net::{Ipv4Addr, SocketAddr},
    str::FromStr,
    time::Duration,
};

use edge_captive::io::run;
use edge_http::Method;
use edge_http::io::Error as HttpError;
use edge_http::io::server::{Connection, Handler, Server};
use edge_nal::TcpBind;
use edge_nal_embassy::{Tcp, TcpBuffers};
use embassy_executor::Spawner;
use embassy_net::{
    IpAddress, Ipv4Address, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
    dns::DnsQueryType, tcp::TcpSocket,
};
use embassy_sync::channel::Channel;
use embassy_time::{Duration as EmbassyDuration, Timer};
use embedded_io_async::{Read, Write};
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
use esp_radio::{
    Controller,
    wifi::{AccessPointConfig, ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent},
};
use heapless::String;
use log::{debug, error, info};
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

const BROKER_HOST: Option<&'static str> = option_env!("BROKER_HOST");
const BROKER_PORT: Option<&'static str> = option_env!("BROKER_PORT");

#[derive(Clone, Debug)]
struct WifiCredentials {
    ssid: heapless::String<32>,
    password: heapless::String<64>,
}

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
        .expect("Failed to create I2C bus")
        .with_sda(sda)
        .with_scl(scl)
        .into_async();
    let mut sht = shtc3(i2c);

    debug!(
        "Raw ID register: {}",
        sht.raw_id_register()
            .await
            .expect("Failed to get raw ID register")
    );

    static ESP_RADIO_CTRL_CELL: static_cell::StaticCell<Controller<'static>> =
        static_cell::StaticCell::new();
    let esp_radio_ctrl = &*ESP_RADIO_CTRL_CELL
        .uninit()
        .write(esp_radio::init().expect("Failed to initialize radio controller"));

    let (controller, interfaces) =
        esp_radio::wifi::new(esp_radio_ctrl, peripherals.WIFI, Default::default())
            .expect("Failed to create WiFi controller");

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

    // Create WiFi credentials channel
    static WIFI_CREDENTIALS_CHANNEL_CELL: static_cell::StaticCell<
        Channel<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, WifiCredentials, 1>,
    > = static_cell::StaticCell::new();
    let wifi_credentials_channel = WIFI_CREDENTIALS_CHANNEL_CELL.uninit().write(Channel::new());

    spawner
        .spawn(connection(controller, wifi_credentials_channel))
        .ok();
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
    info!("WiFi Provisioning Portal Ready");
    debug!("1. Connect to the AP: `esp-radio`");
    debug!("2. Navigate to: http://{gw_ip_addr_str}/");
    while !ap_stack.is_config_up() {
        Timer::after(EmbassyDuration::from_millis(100)).await
    }
    ap_stack
        .config_v4()
        .inspect(|c| debug!("ipv4 config: {c:?}"));

    spawner
        .spawn(run_http_server(ap_stack, wifi_credentials_channel))
        .ok();

    // Keep main task alive
    loop {
        Timer::after(EmbassyDuration::from_secs(60)).await;
    }
}

// Simple URL decoding
fn url_decode(input: &str) -> heapless::String<256> {
    let mut result = heapless::String::<256>::new();
    let mut chars = input.chars();

    while let Some(ch) = chars.next() {
        match ch {
            '+' => {
                result.push(' ').ok();
            }
            '%' => {
                let hex1 = chars.next().and_then(|c| c.to_digit(16));
                let hex2 = chars.next().and_then(|c| c.to_digit(16));
                if let (Some(d1), Some(d2)) = (hex1, hex2) {
                    result.push(char::from((d1 * 16 + d2) as u8)).ok();
                } else {
                    result.push(ch).ok();
                }
            }
            _ => {
                result.push(ch).ok();
            }
        }
    }
    result
}

// Parse form data from URL-encoded body
fn parse_form_data(body: &[u8]) -> Option<WifiCredentials> {
    let body_str = core::str::from_utf8(body).ok()?;
    let mut ssid = None;
    let mut password = heapless::String::<64>::new();

    for pair in body_str.split('&') {
        let (key, value) = pair.split_once('=')?;
        let decoded = url_decode(value);

        match key {
            "ssid" => ssid = heapless::String::from_str(&decoded).ok(),
            "password" => {
                if let Ok(pwd) = heapless::String::from_str(&decoded) {
                    password = pwd;
                }
            }
            _ => {}
        }
    }

    Some(WifiCredentials {
        ssid: ssid?,
        password,
    })
}

// HTTP Handler implementation
struct HttpHandler {
    wifi_credentials_channel: &'static Channel<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        WifiCredentials,
        1,
    >,
}

impl Handler for HttpHandler {
    type Error<E>
        = HttpError<E>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl core::fmt::Display + Copy,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write,
    {
        let headers = conn.headers()?;
        let method = headers.method;
        let path = headers.path;

        debug!(
            "HTTP: {} {}",
            match method {
                Method::Get => "GET",
                Method::Post => "POST",
                _ => "OTHER",
            },
            path
        );

        // Handle captive portal redirects
        const CAPTIVE_PATHS: &[&str] =
            &["/generate_204", "/gen_204", "/ncsi.txt", "/connecttest.txt"];
        if CAPTIVE_PATHS.contains(&path) {
            conn.initiate_response(302, Some("Found"), &[("Location", "/")])
                .await?;
            return Ok(());
        }

        // Handle routes
        match (method, path) {
            (Method::Get, "/") => {
                conn.initiate_response(
                    200,
                    Some("OK"),
                    &[("Content-Type", "text/html; charset=utf-8")],
                )
                .await?;
                conn.write_all(HOME_HTML.as_bytes()).await?;
            }
            (Method::Post, "/save") => {
                // Read request body
                let mut body = heapless::Vec::<u8, 512>::new();
                let mut buf = [0u8; 256];
                loop {
                    match conn.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            for &b in &buf[..n] {
                                if body.push(b).is_err() {
                                    break;
                                }
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }

                match parse_form_data(&body) {
                    Some(credentials) => {
                        debug!(
                            "WiFi Credentials Received: SSID: {} | Password: {}",
                            credentials.ssid, credentials.password
                        );
                        self.wifi_credentials_channel
                            .sender()
                            .send(credentials)
                            .await;
                        debug!("Credentials sent!");

                        conn.initiate_response(
                            200,
                            Some("OK"),
                            &[("Content-Type", "text/html; charset=utf-8")],
                        )
                        .await?;
                        conn.write_all(SAVED_HTML.as_bytes()).await?;
                    }
                    None => {
                        conn.initiate_response(400, Some("Bad Request"), &[])
                            .await?;
                    }
                }
            }
            _ => {
                conn.initiate_response(404, Some("Not Found"), &[]).await?;
            }
        }

        Ok(())
    }
}

#[embassy_executor::task]
async fn run_http_server(
    stack: Stack<'static>,
    wifi_credentials_channel: &'static Channel<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        WifiCredentials,
        1,
    >,
) {
    const HTTP_PORT: u16 = 80;
    info!("Starting HTTP server on port {HTTP_PORT}");

    static TCP_BUFFERS: static_cell::StaticCell<TcpBuffers<1, 2048, 2048>> =
        static_cell::StaticCell::new();
    let buffers = TCP_BUFFERS.uninit().write(TcpBuffers::new());

    let tcp = Tcp::new(stack, buffers);
    let mut acceptor = tcp
        .bind(SocketAddr::new(
            core::net::IpAddr::V4(core::net::Ipv4Addr::UNSPECIFIED),
            HTTP_PORT,
        ))
        .await
        .expect("Failed to bind TCP socket");

    let handler = HttpHandler {
        wifi_credentials_channel,
    };

    let mut server = Server::<1, 2048, 32>::new();

    loop {
        if let Err(_e) = server
            .run(Some(50000), &mut acceptor, &handler)
            .await
            .inspect_err(|e| error!("HTTP server error: {:?}", e))
        {
            Timer::after(EmbassyDuration::from_millis(100)).await;
        }
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
        .expect("Failed to bind DHCP server");

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

    info!("Starting Captive Portal DNS server on port {DNS_PORT}");
    debug!("All DNS queries will resolve to {gw_ip_addr}");

    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let udp_stack = Udp::new(stack, &buffers);

    loop {
        debug!("Starting Captive Portal DNS server");
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
async fn connection(
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
        .expect("Failed to set AP WiFi configuration");
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
    debug!("Connecting to WiFi network...");
    loop {
        match controller.connect_async().await {
            Ok(()) => {
                debug!("Successfully connected to WiFi!");

                // Wait for disconnect event
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                debug!("Disconnected from WiFi, will attempt to reconnect...");
            }
            Err(e) => {
                error!("Failed to connect: {:?}", e);
                debug!("Retrying in 5 seconds...");
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
    debug!("MQTT: Waiting for WiFi link to come up...");
    stack.wait_link_up().await;
    debug!("MQTT: WiFi link up, waiting for network configuration...");

    // Wait for DHCP to assign an IP address
    loop {
        if stack.is_config_up() {
            break;
        }
        Timer::after(EmbassyDuration::from_millis(100)).await;
    }

    debug!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            debug!("Got IP: {}", config.address);
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
        let address = if let Ok(ipv4) = host.parse::<Ipv4Address>() {
            IpAddress::Ipv4(ipv4)
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
                error!("Failed to start measurement: {:?}", e);
                Timer::after(EmbassyDuration::from_secs(1)).await;
                continue;
            }
            // Wait for 12.1 ms https://github.com/Fristi/shtcx-rs/blob/feature/async-support/src/asynchronous.rs#L413-L424
            let duration = max_measurement_duration(&sht, PowerMode::NormalMode);
            Timer::after(EmbassyDuration::from_micros(duration.into())).await;
            let measurement = match sht.get_measurement_result().await {
                Ok(m) => m,
                Err(e) => {
                    error!("Failed to get measurement result: {:?}", e);
                    Timer::after(EmbassyDuration::from_secs(1)).await;
                    continue;
                }
            };

            info!(
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

            let mut humidity_string: String<32> = String::new();
            write!(humidity_string, "{:.2}", measurement.humidity.as_percent())
                .expect("write! failed!");

            // Helper to handle MQTT send errors
            let handle_mqtt_error = |mqtt_error: ReasonCode| match mqtt_error {
                ReasonCode::NetworkError => error!("MQTT Network Error"),
                _ => error!("Other MQTT Error: {:?}", mqtt_error),
            };

            // MQTT
            if let Err(e) = client
                .send_message(
                    "measurement/temperature",
                    temperature_string.as_bytes(),
                    rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1,
                    true,
                )
                .await
            {
                handle_mqtt_error(e);
                continue;
            }

            if let Err(e) = client
                .send_message(
                    "measurement/humidity",
                    humidity_string.as_bytes(),
                    rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1,
                    true,
                )
                .await
            {
                handle_mqtt_error(e);
                continue;
            }

            // Delay
            Timer::after(EmbassyDuration::from_secs(1)).await;
        }
    }
}

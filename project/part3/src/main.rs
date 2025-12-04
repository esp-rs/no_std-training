// Wifi Provisioning
// 1. Run the app
// cargo r -r
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
    Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4, dns::DnsQueryType, tcp::TcpSocket,
};
use embassy_sync::channel::Channel;
use embassy_time::{Duration as EmbassyDuration, Timer};
use embedded_io_async::{Read, Write};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock, interrupt::software::SoftwareInterruptControl, ram, rng::Rng,
    timer::timg::TimerGroup,
};
use esp_radio::{
    Controller,
    wifi::{AccessPointConfig, ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent},
};
use log::{debug, error, info};

esp_bootloader_esp_idf::esp_app_desc!();

#[derive(Clone, Debug, serde::Deserialize)]
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
    // Stack is Copy, so we can pass it by value to each task
    spawner.spawn(run_dhcp(ap_stack, gw_ip_addr)).ok();
    spawner.spawn(run_captive_portal(ap_stack, gw_ip_addr)).ok();
    spawner.spawn(http_client_task(sta_stack)).ok();

    ap_stack.wait_link_up().await;
    info!("WiFi Provisioning Portal Ready");
    info!("1. Connect to the AP: `esp-radio`");
    info!("2. Navigate to: http://{gw_ip_addr_str}/");
    ap_stack.wait_config_up().await;
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
            (Method::Get, "/saved") => {
                conn.initiate_response(
                    200,
                    Some("OK"),
                    &[("Content-Type", "text/html; charset=utf-8")],
                )
                .await?;
                conn.write_all(SAVED_HTML.as_bytes()).await?;
            }
            (Method::Post, "/save") => {
                // Read request body
                let mut buf = [0u8; 256];
                let n = match conn.read(&mut buf).await {
                    Ok(0) => {
                        conn.initiate_response(400, Some("Bad Request"), &[])
                            .await?;
                        return Ok(());
                    }
                    Ok(n) => n,
                    Err(e) => return Err(e),
                };

                match serde_json_core::from_slice::<WifiCredentials>(&buf[..n])
                    .ok()
                    .map(|(credentials, _)| credentials)
                {
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

    debug!("Starting Captive Portal DNS server on port {DNS_PORT}");
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
    debug!("Starting WiFi in AP mode");
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
async fn http_client_task(stack: Stack<'static>) {
    use embedded_io_async::Write;

    // Prepare buffers for TCP socket
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        // Wait for network to be ready before attempting connection
        debug!("HTTP Client: Waiting for WiFi link to come up...");
        stack.wait_link_up().await;
        debug!("HTTP Client: WiFi link up, waiting for network configuration...");

        // Wait for DHCP to assign an IP address
        stack.wait_config_up().await;

        // Wait for DNS servers to be configured
        debug!("HTTP Client: Waiting for DNS servers to be configured...");
        loop {
            if let Some(config) = stack.config_v4() {
                debug!("HTTP Client: Got IP address: {:?}", config.address);
                debug!("HTTP Client: Gateway: {:?}", config.gateway);
                debug!("HTTP Client: DNS servers: {:?}", config.dns_servers);

                // Check if we have at least one DNS server
                if !config.dns_servers.is_empty() {
                    break;
                }
            }
            Timer::after(EmbassyDuration::from_millis(100)).await;
        }

        // Check if we still have a valid network config before proceeding
        if !stack.is_config_up() {
            debug!("HTTP Client: Network config lost, retrying...");
            continue;
        }

        // Wait longer for the network to stabilize and routes to be established
        debug!("HTTP Client: Waiting for network to stabilize...");
        Timer::after(EmbassyDuration::from_secs(5)).await;

        debug!("HTTP Client: Starting HTTP request...");

        debug!("Making HTTP request");

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(EmbassyDuration::from_secs(20)));

        // Connect to www.mobile-j.de
        let host = "www.mobile-j.de";
        let remote_port = 80;

        // Resolve hostname using DNS
        debug!("HTTP Client: Resolving {}...", host);
        let remote_ip = match stack.dns_query(host, DnsQueryType::A).await {
            Ok(addresses) => {
                if addresses.is_empty() {
                    error!("HTTP Client: DNS query returned no addresses for {}", host);
                    debug!("HTTP Client: Retrying in 10 seconds...");
                    Timer::after(EmbassyDuration::from_secs(10)).await;
                    continue;
                }
                let address = addresses[0];
                debug!("HTTP Client: Resolved {} to {}", host, address);
                address
            }
            Err(e) => {
                error!("HTTP Client: DNS lookup failed for {}: {:?}", host, e);
                debug!("HTTP Client: Retrying in 10 seconds...");
                Timer::after(EmbassyDuration::from_secs(10)).await;
                continue;
            }
        };

        debug!(
            "HTTP Client: Connecting to {} ({}:{})...",
            host, remote_ip, remote_port
        );
        match socket.connect((remote_ip, remote_port)).await {
            Ok(()) => {
                debug!("HTTP Client: Connected!");

                // Check network state before sending request
                if !stack.is_link_up() || !stack.is_config_up() {
                    debug!("HTTP Client: Network connection lost, reconnecting...");
                    socket.close();
                    continue;
                }

                // Send HTTP/1.0 request
                let http_request = b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n";

                if let Err(e) = socket.write_all(http_request).await {
                    error!("HTTP Client: Write error: {:?}", e);
                    socket.close();
                    Timer::after(EmbassyDuration::from_secs(10)).await;
                    continue;
                } else if let Err(e) = socket.flush().await {
                    error!("HTTP Client: Flush error: {:?}", e);
                    socket.close();
                    Timer::after(EmbassyDuration::from_secs(10)).await;
                    continue;
                } else {
                    debug!("HTTP Client: Request sent, reading response...");

                    // Read response
                    let mut response_buffer = [0u8; 512];
                    let mut total_read = 0;
                    let mut first_chunk = true;

                    loop {
                        // Check network state during read
                        if !stack.is_link_up() || !stack.is_config_up() {
                            debug!(
                                "HTTP Client: Network connection lost during read, reconnecting..."
                            );
                            break;
                        }

                        match socket.read(&mut response_buffer).await {
                            Ok(0) => {
                                debug!("HTTP Client: Connection closed by server");
                                break;
                            }
                            Ok(n) => {
                                total_read += n;
                                let response_chunk = unsafe {
                                    core::str::from_utf8_unchecked(&response_buffer[..n])
                                };

                                if first_chunk {
                                    info!("HTTP Client: Response received:");
                                    info!("{}", response_chunk);
                                    first_chunk = false;
                                } else {
                                    info!("{}", response_chunk);
                                }

                                if total_read > 2048 {
                                    info!("... (truncated, received {} bytes)", total_read);
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("HTTP Client: Read error: {:?}", e);
                                break;
                            }
                        }
                    }

                    info!(
                        "HTTP Client: Response complete ({} bytes total)",
                        total_read
                    );
                }

                socket.close();

                // Success! Wait before next request
                debug!("HTTP Client: Waiting 30 seconds before next request...");
                Timer::after(EmbassyDuration::from_secs(30)).await;
            }
            Err(e) => {
                error!("HTTP Client: Connection failed: {:?}", e);
                debug!("HTTP Client: Retrying in 10 seconds...");
                Timer::after(EmbassyDuration::from_secs(10)).await;
            }
        }
    }
}

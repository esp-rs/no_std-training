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

use core::{net::Ipv4Addr, str::FromStr, time::Duration};

use edge_captive::io::run;
use embassy_executor::Spawner;
use embassy_net::{
    IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4, tcp::TcpSocket,
};
use embassy_sync::channel::Channel;
use embassy_time::{Duration as EmbassyDuration, Timer};
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

    static ESP_RADIO_CTRL_CELL: static_cell::StaticCell<Controller<'static>> = static_cell::StaticCell::new();
    let esp_radio_ctrl = &*ESP_RADIO_CTRL_CELL.uninit().write(esp_radio::init().expect("Failed to initialize radio controller"));

    let (controller, interfaces) =
        esp_radio::wifi::new(esp_radio_ctrl, peripherals.WIFI, Default::default()).expect("Failed to create WiFi controller");

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
    static AP_STACK_RESOURCES_CELL: static_cell::StaticCell<StackResources<6>> = static_cell::StaticCell::new();
    let (ap_stack, ap_runner) = embassy_net::new(
        ap_device,
        ap_config,
        AP_STACK_RESOURCES_CELL.uninit().write(StackResources::<6>::new()),
        seed,
    );

    // Init network stack for STA (client connection)
    static STA_STACK_RESOURCES_CELL: static_cell::StaticCell<StackResources<3>> = static_cell::StaticCell::new();
    let (sta_stack, sta_runner) = embassy_net::new(
        sta_device,
        sta_config,
        STA_STACK_RESOURCES_CELL.uninit().write(StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(ap_runner)).ok();
    spawner.spawn(sta_net_task(sta_runner)).ok();
    spawner.spawn(run_dhcp(ap_stack, gw_ip_addr)).ok();
    spawner.spawn(run_captive_portal(ap_stack, gw_ip_addr)).ok();
    spawner.spawn(http_client_task(sta_stack)).ok();

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
    debug!(
        "WiFi Credentials Received: SSID: {} | Password: {}",
        form.0.ssid, form.0.password
    );

    // Send credentials to the connection task
    let credentials = WifiCredentials {
        ssid: form.0.ssid,
        password: form.0.password,
    };
    debug!("Sending credentials to connection task...");
    WIFI_CREDENTIALS_CHANNEL.sender().send(credentials).await;
    debug!("Credentials sent!");

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
    info!("Starting HTTP server on port {HTTP_PORT}");

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

        debug!("HTTP: Waiting for connection...");
        let result = socket
            .accept(IpListenEndpoint {
                addr: None,
                port: HTTP_PORT,
            })
            .await;

        if let Err(e) = result {
            error!("HTTP accept error: {:?}", e);
            Timer::after(EmbassyDuration::from_millis(100)).await;
            continue;
        }

        debug!("HTTP: Client connected");

        match picoserve::serve(&app, &config, &mut http_buffer, socket).await {
            Ok(handled_requests_count) => {
                debug!("HTTP: Handled {} requests", handled_requests_count);
            }
            Err(e) => {
                error!("HTTP error: {:?}", e);
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
async fn connection(mut controller: WifiController<'static>) {
    debug!("start connection task");
    debug!("Device capabilities: {:?}", controller.capabilities());

    // Start in AP mode first for provisioning
    let ap_config =
        ModeConfig::AccessPoint(AccessPointConfig::default().with_ssid("esp-radio".into()));
    controller.set_config(&ap_config).expect("Failed to set AP WiFi configuration");
    debug!("Starting WiFi in AP mode");
    controller.start_async().await.expect("Failed to start WiFi");
    debug!("WiFi AP started!");

    // Wait for credentials
    debug!("Waiting for WiFi credentials...");
    let credentials = WIFI_CREDENTIALS_CHANNEL.receiver().receive().await;
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
    controller.set_config(&sta_config).expect("Failed to set station mode WiFi configuration");

    debug!("Starting WiFi in station mode...");
   controller.start_async().await.expect("Failed to start WiFi");
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

    // Wait for WiFi connection
    debug!("HTTP Client: Waiting for WiFi link to come up...");
    stack.wait_link_up().await;
    debug!("HTTP Client: WiFi link up, waiting for network configuration...");

    // Wait for DHCP to assign an IP address
    loop {
        if stack.is_config_up() {
            break;
        }
        Timer::after(EmbassyDuration::from_millis(100)).await;
    }

    if let Some(config) = stack.config_v4() {
        debug!("HTTP Client: Got IP address: {:?}", config.address);
        debug!("HTTP Client: Gateway: {:?}", config.gateway);
        debug!("HTTP Client: DNS servers: {:?}", config.dns_servers);
    }

    // Wait longer for the network to stabilize and routes to be established
    debug!("HTTP Client: Waiting for network to stabilize...");
    Timer::after(EmbassyDuration::from_secs(5)).await;

    debug!("HTTP Client: Starting HTTP request...");

    // Prepare buffers for TCP socket
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        debug!("Making HTTP request");

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(EmbassyDuration::from_secs(20)));

        // Connect to www.mobile-j.de
        let remote_ip = Ipv4Addr::new(142, 250, 185, 115);
        let remote_port = 80;

        debug!(
            "HTTP Client: Connecting to www.mobile-j.de ({}:{})...",
            remote_ip, remote_port
        );

        match socket.connect((remote_ip, remote_port)).await {
            Ok(()) => {
                debug!("HTTP Client: Connected!");

                // Send HTTP/1.0 request
                let http_request = b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n";

                if let Err(e) = socket.write_all(http_request).await {
                    error!("HTTP Client: Write error: {:?}", e);
                } else if let Err(e) = socket.flush().await {
                    error!("HTTP Client: Flush error: {:?}", e);
                } else {
                    debug!("HTTP Client: Request sent, reading response...");

                    // Read response
                    let mut response_buffer = [0u8; 512];
                    let mut total_read = 0;
                    let mut first_chunk = true;

                    loop {
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
                                    debug!("HTTP Client: Response received:");
                                    debug!("{}", response_chunk);
                                    first_chunk = false;
                                } else {
                                    debug!("{}", response_chunk);
                                }

                                if total_read > 2048 {
                                    debug!("... (truncated, received {} bytes)", total_read);
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("HTTP Client: Read error: {:?}", e);
                                break;
                            }
                        }
                    }

                    debug!(
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

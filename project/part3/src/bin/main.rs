#![no_std]
#![no_main]

use core::net::Ipv4Addr;
use core::str::FromStr;
use core::time::Duration;

use edge_captive::io::run;
use embassy_executor::Spawner;
use embassy_net::{
    IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4, tcp::TcpSocket,
};
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
#[cfg(target_arch = "riscv32")]
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::{clock::CpuClock, ram, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_radio::{
    Controller,
    wifi::{AccessPointConfig, ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent},
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

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    #[cfg(target_arch = "riscv32")]
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(
        timg0.timer0,
        #[cfg(target_arch = "riscv32")]
        sw_int.software_interrupt0,
    );

    let esp_radio_ctrl = &*mk_static!(Controller<'static>, esp_radio::init().unwrap());

    let (controller, interfaces) =
        esp_radio::wifi::new(&esp_radio_ctrl, peripherals.WIFI, Default::default()).unwrap();

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
    spawner.spawn(http_client_task(sta_stack)).ok();

    loop {
        if ap_stack.is_link_up() {
            break;
        }
        Timer::after(EmbassyDuration::from_millis(500)).await;
    }
    println!("=== WiFi Provisioning Portal Ready ===");
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

#[embassy_executor::task]
async fn run_http_server(stack: Stack<'static>) {
    use embedded_io_async::Write;

    let mut rx_buffer = [0; 2048];
    let mut tx_buffer = [0; 2048];

    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(EmbassyDuration::from_secs(10)));

    const HTTP_PORT: u16 = 80;
    println!("Starting HTTP server on port {HTTP_PORT}");

    loop {
        println!("HTTP: Waiting for connection...");
        let r = socket
            .accept(IpListenEndpoint {
                addr: None,
                port: HTTP_PORT,
            })
            .await;

        if let Err(e) = r {
            println!("HTTP accept error: {:?}", e);
            Timer::after(EmbassyDuration::from_millis(100)).await;
            continue;
        }

        println!("HTTP: Client connected");

        // Read HTTP request
        let mut buffer = [0u8; 1024];
        let mut pos = 0;
        let mut is_android_detection = false;
        let mut is_apple_detection = false;
        let mut is_post_save = false;
        let mut content_length: usize = 0;
        let mut headers_end: usize = 0;
        let mut headers_parsed = false;

        loop {
            match socket.read(&mut buffer[pos..]).await {
                Ok(0) => break,
                Ok(len) => {
                    pos += len;
                    let received = unsafe { core::str::from_utf8_unchecked(&buffer[..pos]) };

                    if received.contains("\r\n\r\n") && !headers_parsed {
                        // Extract and analyze request line
                        if let Some(first_line) = received.lines().next() {
                            println!("HTTP Request: {}", first_line);

                            // Check for detection endpoints
                            is_android_detection = first_line.contains("GET /generate_204")
                                || first_line.contains("GET /gen_204")
                                || first_line.contains("GET /ncsi.txt")
                                || first_line.contains("GET /connecttest.txt");

                            is_apple_detection = first_line.contains("GET /hotspot-detect.html")
                                || first_line.contains("GET /library/test/success.html");

                            is_post_save = first_line.contains("POST /save");
                        }

                        // Find headers end position
                        headers_end = received.find("\r\n\r\n").unwrap_or(0) + 4;

                        // For POST requests, extract Content-Length
                        if is_post_save {
                            // Parse Content-Length header
                            for line in received.lines() {
                                if line.to_lowercase().starts_with("content-length:")
                                    && let Some(len_str) = line.split(':').nth(1)
                                {
                                    content_length = len_str.trim().parse().unwrap_or(0);
                                    println!("Content-Length: {}", content_length);
                                }
                            }
                        }

                        headers_parsed = true;
                    }

                    // For POST requests, check if we have the full body
                    if headers_parsed {
                        if is_post_save {
                            let body_received = pos.saturating_sub(headers_end);
                            println!("Body received: {}/{}", body_received, content_length);

                            if body_received >= content_length && content_length > 0 {
                                break;
                            }
                        } else {
                            // For GET requests, headers are enough
                            break;
                        }
                    }

                    if pos >= buffer.len() {
                        println!("Buffer full, breaking");
                        break;
                    }
                }
                Err(e) => {
                    println!("HTTP read error: {:?}", e);
                    break;
                }
            }
        }

        // If it's a POST to /save, parse the form data
        if is_post_save {
            let received = unsafe { core::str::from_utf8_unchecked(&buffer[..pos]) };

            // Find the body after \r\n\r\n
            if let Some(body_start) = received.find("\r\n\r\n") {
                let body = &received[body_start + 4..];
                println!("POST body: {}", body);

                // Parse form data (application/x-www-form-urlencoded)
                let mut ssid: Option<&str> = None;
                let mut password: Option<&str> = None;

                for param in body.split('&') {
                    if let Some((key, value)) = param.split_once('=') {
                        match key {
                            "ssid" => ssid = Some(value),
                            "password" => password = Some(value),
                            _ => {}
                        }
                    }
                }

                // Print the credentials
                println!("=== WiFi Credentials Received ===");
                let mut decoded_ssid_str = heapless::String::<32>::new();
                let mut decoded_password_str = heapless::String::<64>::new();

                if let Some(s) = ssid {
                    // URL decode the SSID (replace + with space, handle %XX encoding)
                    let decoded_ssid = s.replace('+', " ");
                    println!("SSID: {}", decoded_ssid);
                    let _ = decoded_ssid_str.push_str(&decoded_ssid);
                } else {
                    println!("SSID: (not found)");
                }
                if let Some(p) = password {
                    // URL decode the password (replace + with space, handle %XX encoding)
                    let decoded_password = p.replace('+', " ");
                    println!("Password: {}", decoded_password);
                    let _ = decoded_password_str.push_str(&decoded_password);
                } else {
                    println!("Password: (not found)");
                }
                println!("================================");

                // Send credentials to the connection task
                if !decoded_ssid_str.is_empty() {
                    let credentials = WifiCredentials {
                        ssid: decoded_ssid_str,
                        password: decoded_password_str,
                    };
                    println!("Sending credentials to connection task...");
                    WIFI_CREDENTIALS_CHANNEL.sender().send(credentials).await;
                    println!("Credentials sent!");
                }
            } else {
                println!("POST body not found!");
            }
        }

        // Determine response based on request path
        let response: &[u8] = if is_post_save {
            // Response for WiFi credentials submission
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: text/html; charset=utf-8\r\n\
              Connection: close\r\n\
              \r\n\
              <!DOCTYPE html>\
              <html>\
              <head>\
                  <meta charset='utf-8'>\
                  <meta name='viewport' content='width=device-width, initial-scale=1'>\
                  <title>Configuration Saved</title>\
                  <style>\
                      body { font-family: Arial, sans-serif; margin: 40px; background: #f0f0f0; }\
                      .container { max-width: 600px; margin: 0 auto; background: white; padding: 30px; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }\
                      h1 { color: #4caf50; }\
                      .success { background: #c8e6c9; padding: 15px; border-radius: 5px; margin: 20px 0; }\
                  </style>\
              </head>\
              <body>\
                  <div class='container'>\
                      <h1>Configuration Saved!</h1>\
                      <div class='success'>\
                          <p>Your WiFi credentials have been received.</p>\
                          <p>Check the serial console for details.</p>\
                      </div>\
                  </div>\
              </body>\
              </html>\r\n"
        } else if is_android_detection {
            // Android/Chrome captive portal detection - return 302 redirect
            b"HTTP/1.1 302 Found\r\n\
              Location: http://192.168.2.1/\r\n\
              Content-Length: 0\r\n\
              Connection: close\r\n\
              \r\n"
        } else if is_apple_detection {
            // Apple iOS captive portal detection - return success page
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: text/html\r\n\
              Connection: close\r\n\
              \r\n\
              <HTML><HEAD><TITLE>Success</TITLE></HEAD><BODY>Success</BODY></HTML>\r\n"
        } else {
            // Main captive portal page
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: text/html; charset=utf-8\r\n\
              Connection: close\r\n\
              \r\n\
              <!DOCTYPE html>\
              <html>\
              <head>\
                  <meta charset='utf-8'>\
                  <meta name='viewport' content='width=device-width, initial-scale=1'>\
                  <title>ESP WiFi Provisioning</title>\
                  <style>\
                      body { font-family: Arial, sans-serif; margin: 40px; background: #f0f0f0; }\
                      .container { max-width: 600px; margin: 0 auto; background: white; padding: 30px; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }\
                      h1 { color: #333; }\
                      .info { background: #e3f2fd; padding: 15px; border-radius: 5px; margin: 20px 0; }\
                      .status { color: #4caf50; font-weight: bold; }\
                      input, button { width: 100%; padding: 12px; margin: 8px 0; box-sizing: border-box; border: 1px solid #ddd; border-radius: 4px; }\
                      button { background: #2196F3; color: white; cursor: pointer; font-weight: bold; }\
                      button:hover { background: #0b7dda; }\
                  </style>\
              </head>\
              <body>\
                  <div class='container'>\
                      <h1>ESP32-C3 WiFi Provisioning</h1>\
                      <div class='info'>\
                          <p class='status'>+ Connected to ESP32-C3 Access Point</p>\
                          <p>Device: <strong>esp-radio</strong></p>\
                          <p>IP Address: <strong>192.168.2.1</strong></p>\
                      </div>\
                      <h2>Configure WiFi</h2>\
                      <form action='/save' method='post'>\
                          <input type='text' name='ssid' placeholder='WiFi SSID' required>\
                          <input type='password' name='password' placeholder='WiFi Password' required>\
                          <button type='submit'>Save Configuration</button>\
                      </form>\
                      <p style='text-align: center; color: #999; margin-top: 20px;'>Powered by Rust & Embassy</p>\
                  </div>\
              </body>\
              </html>\r\n"
        };

        // Send response
        if let Err(e) = socket.write_all(response).await {
            println!("HTTP write error: {:?}", e);
        }

        if let Err(e) = socket.flush().await {
            println!("HTTP flush error: {:?}", e);
        }

        Timer::after(EmbassyDuration::from_millis(100)).await;
        socket.close();
        Timer::after(EmbassyDuration::from_millis(100)).await;
        socket.abort();
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
                println!("=== Successfully connected to WiFi! ===");
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
async fn http_client_task(stack: Stack<'static>) {
    use embedded_io_async::Write;

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

    if let Some(config) = stack.config_v4() {
        println!("HTTP Client: Got IP address: {:?}", config.address);
        println!("HTTP Client: Gateway: {:?}", config.gateway);
        println!("HTTP Client: DNS servers: {:?}", config.dns_servers);
    }

    // Wait longer for the network to stabilize and routes to be established
    println!("HTTP Client: Waiting for network to stabilize...");
    Timer::after(EmbassyDuration::from_secs(5)).await;

    println!("HTTP Client: Starting HTTP request...");

    // Prepare buffers for TCP socket
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        println!("Making HTTP request");

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(EmbassyDuration::from_secs(20)));

        // Connect to www.mobile-j.de
        let remote_ip = Ipv4Addr::new(142, 250, 185, 115);
        let remote_port = 80;

        println!(
            "HTTP Client: Connecting to www.mobile-j.de ({}:{})...",
            remote_ip, remote_port
        );

        match socket.connect((remote_ip, remote_port)).await {
            Ok(()) => {
                println!("HTTP Client: Connected!");

                // Send HTTP/1.0 request
                let http_request = b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n";

                if let Err(e) = socket.write_all(http_request).await {
                    println!("HTTP Client: Write error: {:?}", e);
                } else if let Err(e) = socket.flush().await {
                    println!("HTTP Client: Flush error: {:?}", e);
                } else {
                    println!("HTTP Client: Request sent, reading response...");

                    // Read response
                    let mut response_buffer = [0u8; 512];
                    let mut total_read = 0;
                    let mut first_chunk = true;

                    loop {
                        match socket.read(&mut response_buffer).await {
                            Ok(0) => {
                                println!("HTTP Client: Connection closed by server");
                                break;
                            }
                            Ok(n) => {
                                total_read += n;
                                let response_chunk = unsafe {
                                    core::str::from_utf8_unchecked(&response_buffer[..n])
                                };

                                if first_chunk {
                                    println!("HTTP Client: Response received:");
                                    println!("{}", response_chunk);
                                    first_chunk = false;
                                } else {
                                    println!("{}", response_chunk);
                                }

                                if total_read > 2048 {
                                    println!("... (truncated, received {} bytes)", total_read);
                                    break;
                                }
                            }
                            Err(e) => {
                                println!("HTTP Client: Read error: {:?}", e);
                                break;
                            }
                        }
                    }

                    println!(
                        "HTTP Client: Response complete ({} bytes total)",
                        total_read
                    );
                }

                socket.close();

                // Success! Wait before next request
                println!("HTTP Client: Waiting 30 seconds before next request...");
                Timer::after(EmbassyDuration::from_secs(30)).await;
            }
            Err(e) => {
                println!("HTTP Client: Connection failed: {:?}", e);
                println!("HTTP Client: Retrying in 10 seconds...");
                Timer::after(EmbassyDuration::from_secs(10)).await;
            }
        }
    }
}

use core::fmt::{Debug, Display, Write as _};
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use core::time::Duration;
use edge_dhcp::{
    io::{self, DEFAULT_SERVER_PORT},
    server::{Server, ServerOptions},
};
use edge_http::io::server::{Connection, Handler, Server as HttpServer};
use edge_http::{Method, io::Error as HttpError};
use edge_nal::{TcpBind, UdpBind};
use edge_nal_embassy::{Tcp, TcpBuffers, Udp, UdpBuffers};
use embassy_net::Stack;
use embassy_time::Timer;
use embedded_io_async::{Read, Write};
use log::{info, warn};

use crate::network::{WIFI_CREDENTIALS, WifiCredentials};

// HTML templates embedded at compile time
const HOME_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/templates/home.html"
));
const SAVED_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/templates/saved.html"
));

const HTML_HEADERS: &[(&str, &str)] = &[("Content-Type", "text/html; charset=utf-8")];

struct HttpHandler {
    /// Absolute URL of the provisioning page, e.g. `http://192.168.2.1/`.
    portal_url: heapless::String<32>,
}

impl Handler for HttpHandler {
    type Error<E>
        = HttpError<E>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl Display + Copy,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write,
    {
        let headers = conn.headers()?;
        match (headers.method, headers.path) {
            (Method::Get, "/") => {
                conn.initiate_response(200, Some("OK"), HTML_HEADERS)
                    .await?;
                conn.write_all(HOME_HTML.as_bytes()).await?;
            }
            (Method::Get, "/saved") => {
                conn.initiate_response(200, Some("OK"), HTML_HEADERS)
                    .await?;
                conn.write_all(SAVED_HTML.as_bytes()).await?;
            }
            (Method::Post, "/save") => {
                let mut body = [0u8; 256];
                let mut len = 0;
                while len < body.len() {
                    match conn.read(&mut body[len..]).await? {
                        0 => break,
                        n => len += n,
                    }
                }

                match serde_json_core::from_slice::<WifiCredentials>(&body[..len]) {
                    // WPA2 passwords have between 8 and 64 characters. The Wi-Fi driver does not
                    // reject shorter ones, it only fails to connect.
                    Ok((credentials, _)) if credentials.password.len() >= 8 => {
                        WIFI_CREDENTIALS.signal(credentials);
                        conn.initiate_response(200, Some("OK"), &[]).await?;
                    }
                    Ok(_) => {
                        warn!("Rejected credentials: the password is shorter than 8 characters");
                        conn.initiate_response(400, Some("Bad Request"), &[])
                            .await?;
                    }
                    Err(e) => {
                        warn!("Invalid credentials request: {e:?}");
                        conn.initiate_response(400, Some("Bad Request"), &[])
                            .await?;
                    }
                }
            }
            // Operating systems probe URLs such as `/generate_204` to detect captive portals.
            // Redirecting every unknown page makes them open the provisioning page.
            (Method::Get, _) => {
                conn.initiate_response(302, Some("Found"), &[("Location", &self.portal_url)])
                    .await?;
            }
            _ => {
                conn.initiate_response(404, Some("Not Found"), &[]).await?;
            }
        }

        Ok(())
    }
}

#[embassy_executor::task]
pub async fn run_http_server(stack: Stack<'static>, gw_ip_addr: Ipv4Addr) {
    const HTTP_PORT: u16 = 80;

    let buffers = TcpBuffers::<1, 2048, 2048>::new();
    let tcp = Tcp::new(stack, &buffers);
    let mut acceptor = tcp
        .bind(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            HTTP_PORT,
        )))
        .await
        .expect("Failed to bind the HTTP server");

    let mut portal_url = heapless::String::new();
    write!(portal_url, "http://{gw_ip_addr}/").unwrap();
    let handler = HttpHandler { portal_url };

    let mut server = HttpServer::<1, 2048, 32>::new();
    info!("HTTP server listening on port {HTTP_PORT}");

    loop {
        if let Err(e) = server.run(Some(50_000), &mut acceptor, &handler).await {
            warn!("HTTP server error: {e:?}");
        }
        Timer::after_millis(100).await;
    }
}

#[embassy_executor::task]
pub async fn run_dhcp(stack: Stack<'static>, gw_ip_addr: Ipv4Addr) {
    let mut buf = [0u8; 1500];

    let buffers = UdpBuffers::<1, 1024, 1024, 10>::new();
    let udp = Udp::new(stack, &buffers);
    let mut socket = udp
        .bind(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .expect("Failed to bind the DHCP server");

    // Advertise the device as the gateway and the DNS server of its network
    let mut gateways = [Ipv4Addr::UNSPECIFIED];
    let dns_servers = [gw_ip_addr];
    let mut options = ServerOptions::new(gw_ip_addr, Some(&mut gateways));
    options.dns = &dns_servers;
    let mut server = Server::<_, 64>::new_with_et(gw_ip_addr);

    loop {
        if let Err(e) = io::server::run(&mut server, &options, &mut socket, &mut buf).await {
            warn!("DHCP server error: {e:?}");
        }
        Timer::after_millis(500).await;
    }
}

#[embassy_executor::task]
pub async fn run_captive_portal(stack: Stack<'static>, gw_ip_addr: Ipv4Addr) {
    const DNS_PORT: u16 = 53;

    let mut tx_buf = [0u8; 1500];
    let mut rx_buf = [0u8; 1500];

    let buffers = UdpBuffers::<1, 1024, 1024, 10>::new();
    let udp = Udp::new(stack, &buffers);

    // Answer every DNS query with the address of the device
    loop {
        if let Err(e) = edge_captive::io::run(
            &udp,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DNS_PORT)),
            &mut tx_buf,
            &mut rx_buf,
            gw_ip_addr,
            Duration::from_secs(60),
        )
        .await
        {
            warn!("Captive portal DNS server error: {e:?}");
        }
        Timer::after_millis(500).await;
    }
}

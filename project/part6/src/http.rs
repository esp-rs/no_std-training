use core::fmt::Debug;
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;
use edge_captive::io::run;
use edge_dhcp::{
    io::{self, DEFAULT_SERVER_PORT},
    server::{Server, ServerOptions},
};
use edge_http::io::server::{Connection, Handler, Server as HttpServer};
use edge_http::{Method, io::Error as HttpError};
use edge_nal::{TcpBind, UdpBind};
use edge_nal_embassy::{Tcp, TcpBuffers, Udp, UdpBuffers};
use embassy_net::Stack;
use embassy_sync::channel::Channel;
use embassy_time::{Duration as EmbassyDuration, Timer};
use embedded_io_async::{Read, Write};
use log::{debug, error, info};

use crate::network::WifiCredentials;

// HTML templates embedded at compile time
const HOME_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/templates/home.html"
));
const SAVED_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/templates/saved.html"
));

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
pub async fn run_http_server(
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

    let mut server = HttpServer::<1, 2048, 32>::new();

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
pub async fn run_dhcp(stack: Stack<'static>, gw_ip_addr: Ipv4Addr) {
    use core::net::{Ipv4Addr, SocketAddrV4};

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
pub async fn run_captive_portal(stack: Stack<'static>, gw_ip_addr: Ipv4Addr) {
    use core::net::{SocketAddr, SocketAddrV4};

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

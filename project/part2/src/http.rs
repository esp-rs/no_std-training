use alloc::format;
use embassy_net::{IpAddress, Stack, dns::DnsQueryType, tcp::TcpSocket};
use embassy_time::Duration;
use embedded_io_async::Write;
use log::{error, info};

// ANCHOR: http_env
const HOST_IP: Option<&str> = option_env!("HOST_IP");
const HTTP_PORT: Option<&str> = option_env!("HTTP_PORT");
const DEFAULT_HTTP_PORT: u16 = 8080;
// ANCHOR_END: http_env

/// Posts a measurement as JSON to `/sensor` on `HOST_IP`. Errors are logged.
pub async fn send_sensor_data(stack: Stack<'static>, temperature: f32, humidity: f32) {
    let Some(host) = HOST_IP.filter(|host| !host.is_empty()) else {
        error!("No HOST_IP set. Run with HOST_IP=<your-computer-ip> and optional HTTP_PORT.");
        return;
    };
    let port = HTTP_PORT
        .and_then(|port| port.parse().ok())
        .unwrap_or(DEFAULT_HTTP_PORT);

    // If the host is an IPv4 literal, bypass DNS
    let address = match host.parse() {
        Ok(ipv4) => IpAddress::Ipv4(ipv4),
        Err(_) => match stack.dns_query(host, DnsQueryType::A).await.as_deref() {
            Ok([address, ..]) => *address,
            Ok([]) => {
                error!("DNS query returned no addresses for {host}");
                return;
            }
            Err(e) => {
                error!("DNS lookup for {host} failed: {e:?}");
                return;
            }
        },
    };

    // ANCHOR: http_payload
    let body = format!(r#"{{"temperature":{temperature:.2},"humidity":{humidity:.2}}}"#);
    let request = format!(
        "POST /sensor HTTP/1.0\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    // ANCHOR_END: http_payload

    let mut rx_buffer = [0; 1024];
    let mut tx_buffer = [0; 1024];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(10)));

    if let Err(e) = socket.connect((address, port)).await {
        error!("Failed to connect to {host}:{port}: {e:?}");
        return;
    }
    // We do not wait for the response
    match socket.write_all(request.as_bytes()).await {
        Ok(()) => {
            let _ = socket.flush().await;
            info!("HTTP request sent");
        }
        Err(e) => error!("HTTP write error: {e:?}"),
    }
    socket.close();
}

use alloc::format;
use embassy_net::{Stack, dns::DnsQueryType, tcp::TcpSocket};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use log::{debug, error, info};

#[embassy_executor::task]
pub async fn http_client_task(stack: Stack<'static>) {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    stack.wait_link_up().await;

    debug!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            debug!("Got IP: {}", config.address);
            break;
        }
        embassy_time::Timer::after(Duration::from_millis(500)).await;
    }

    loop {
        // Prepare HTTP payload (JSON)
        let temperature = format!("{:.2}", 0.0); // Placeholder, will be passed from sensor
        let humidity = format!("{:.2}", 0.0); // Placeholder, will be passed from sensor
        let body = format!(
            r#"{{"temperature":{},"humidity":{}}}"#,
            temperature, humidity
        );

        // HTTP target
        let host = "www.mobile-j.de";
        let remote_port: u16 = 80;
        let path = "/sensor";

        // Resolve hostname using DNS
        debug!("Resolving {}...", host);
        let remote_ip = match stack.dns_query(host, DnsQueryType::A).await {
            Ok(addresses) => {
                if addresses.is_empty() {
                    error!("DNS query returned no addresses for {}", host);
                    debug!("Retrying in 5 seconds...");
                    embassy_time::Timer::after(Duration::from_secs(5)).await;
                    continue;
                }
                let address = addresses[0];
                debug!("Resolved {} to {}", host, address);
                address
            }
            Err(e) => {
                error!("DNS lookup failed for {}: {:?}", host, e);
                debug!("Retrying in 5 seconds...");
                embassy_time::Timer::after(Duration::from_secs(5)).await;
                continue;
            }
        };

        // Open TCP connection
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));
        debug!("connecting to {} ({}:{})...", host, remote_ip, remote_port);
        if let Err(e) = socket.connect((remote_ip, remote_port)).await {
            error!("connect error: {:?}", e);
            embassy_time::Timer::after(Duration::from_secs(2)).await;
            continue;
        }

        // Compose minimal HTTP/1.0 POST request
        let request_head = format!(
            "POST {} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            path,
            host,
            body.len()
        );

        // Send request and ignore server response
        if let Err(e) = socket.write_all(request_head.as_bytes()).await {
            error!("HTTP write (headers) error: {:?}", e);
            socket.close();
            embassy_time::Timer::after(Duration::from_secs(1)).await;
            continue;
        }
        if let Err(e) = socket.write_all(body.as_bytes()).await {
            error!("HTTP write (body) error: {:?}", e);
            socket.close();
            embassy_time::Timer::after(Duration::from_secs(1)).await;
            continue;
        }
        info!("HTTP request sent");
        // Best effort send; do not wait for any reply
        let _ = socket.flush().await;
        socket.close();

        // Small delay before next measurement
        embassy_time::Timer::after(Duration::from_secs(1)).await;
    }
}

pub async fn send_sensor_data(
    stack: Stack<'static>,
    temperature: f32,
    humidity: f32,
) -> Result<(), ()> {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    // Prepare HTTP payload (JSON)
    let temperature_str = format!("{:.2}", temperature);
    let humidity_str = format!("{:.2}", humidity);
    let body = format!(
        r#"{{"temperature":{},"humidity":{}}}"#,
        temperature_str, humidity_str
    );

    // HTTP target
    let host = "www.mobile-j.de";
    let remote_port: u16 = 80;
    let path = "/sensor";

    // Resolve hostname using DNS
    debug!("Resolving {}...", host);
    let remote_ip = match stack.dns_query(host, DnsQueryType::A).await {
        Ok(addresses) => {
            if addresses.is_empty() {
                error!("DNS query returned no addresses for {}", host);
                return Err(());
            }
            let address = addresses[0];
            debug!("Resolved {} to {}", host, address);
            address
        }
        Err(e) => {
            error!("DNS lookup failed for {}: {:?}", host, e);
            return Err(());
        }
    };

    // Open TCP connection
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));
    debug!("connecting to {} ({}:{})...", host, remote_ip, remote_port);
    if let Err(e) = socket.connect((remote_ip, remote_port)).await {
        error!("connect error: {:?}", e);
        return Err(());
    }

    // Compose minimal HTTP/1.0 POST request
    let request_head = format!(
        "POST {} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        path,
        host,
        body.len()
    );

    // Send request and ignore server response
    if let Err(e) = socket.write_all(request_head.as_bytes()).await {
        error!("HTTP write (headers) error: {:?}", e);
        socket.close();
        return Err(());
    }
    if let Err(e) = socket.write_all(body.as_bytes()).await {
        error!("HTTP write (body) error: {:?}", e);
        socket.close();
        return Err(());
    }
    info!("HTTP request sent");
    // Best effort send; do not wait for any reply
    let _ = socket.flush().await;
    socket.close();

    Ok(())
}

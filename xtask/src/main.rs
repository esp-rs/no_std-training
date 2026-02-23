use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rumqttd::{Broker, Config, ConnectionSettings, Notification, RouterConfig, ServerSettings};

const DEFAULT_PORT: u16 = 1884;
const DEFAULT_HTTP_PORT: u16 = 8080;
const DEFAULT_OTA_FIRMWARE_PATH: &str = "firmware.bin";

#[derive(Debug, Parser)]
#[command(about = "Project automation tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Starts a local MQTT v5 server and prints all received messages.
    MqttServer {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Starts a local HTTP server and prints incoming sensor payloads from `POST /sensor`.
    HttpServer {
        #[arg(long, default_value_t = DEFAULT_HTTP_PORT)]
        port: u16,
    },
    /// Starts a local OTA HTTP server and serves `GET /firmware.bin`.
    OtaServer {
        #[arg(long, default_value_t = DEFAULT_HTTP_PORT)]
        port: u16,
        #[arg(long, default_value = DEFAULT_OTA_FIRMWARE_PATH)]
        firmware: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::MqttServer { port } => run_mqtt_server(port),
        Command::HttpServer { port } => run_http_server(port),
        Command::OtaServer { port, firmware } => run_ota_server(port, firmware),
    }
}

fn run_mqtt_server(port: u16) -> ! {
    let mut broker = Broker::new(build_mqtt_broker_config(port));
    let (mut link_tx, mut link_rx) = broker
        .link("no-std-training-logger")
        .expect("failed to create mqtt logger link");

    thread::spawn(move || {
        broker.start().expect("failed to start mqtt broker");
    });

    link_tx
        .subscribe("measurement/#")
        .expect("failed to subscribe logger to `measurement/#`");

    println!("MQTT server listening on 0.0.0.0:{port}");
    print_host_ip_hint(port);
    println!("Subscribed to `measurement/#` and printing all incoming MQTT messages.");

    loop {
        let Some(notification) = link_rx.recv().expect("failed to receive mqtt notification")
        else {
            continue;
        };

        if let Notification::Forward(forward) = notification {
            let topic = String::from_utf8_lossy(forward.publish.topic.as_ref());
            let payload = String::from_utf8_lossy(forward.publish.payload.as_ref());
            println!("Topic: {topic} | Payload: {payload}");
        }
    }
}

fn build_mqtt_broker_config(port: u16) -> Config {
    let mut v5 = HashMap::new();
    v5.insert(
        "training".to_owned(),
        ServerSettings {
            name: "training-v5".to_owned(),
            listen: SocketAddr::from(([0, 0, 0, 0], port)),
            tls: None,
            next_connection_delay_ms: 1,
            connections: ConnectionSettings {
                connection_timeout_ms: 60_000,
                max_payload_size: 20 * 1024,
                max_inflight_count: 100,
                auth: None,
                external_auth: None,
                dynamic_filters: true,
            },
        },
    );

    Config {
        id: 0,
        router: RouterConfig {
            max_connections: 1_024,
            max_outgoing_packet_count: 200,
            max_segment_size: 1_024 * 1_024,
            max_segment_count: 10,
            custom_segment: None,
            initialized_filters: None,
            shared_subscriptions_strategy: Default::default(),
        },
        v4: None,
        v5: Some(v5),
        ws: None,
        cluster: None,
        console: None,
        bridge: None,
        prometheus: None,
        metrics: None,
    }
}

fn run_http_server(port: u16) -> ! {
    let listener = bind_listener("HTTP server", port);
    println!("HTTP server listening on 0.0.0.0:{port}");
    print_host_ip_hint(port);
    println!("Waiting for POST /sensor requests...");

    accept_loop(listener, handle_http_connection)
}

fn handle_http_connection(stream: &mut TcpStream) {
    let Some((request, headers_end)) = read_http_request_or_bad_request(stream) else {
        return;
    };
    let headers = &request[..headers_end];
    let Some((method, path, _version)) = parse_request_line(headers) else {
        respond(stream, 400, "Bad Request");
        return;
    };
    let body_start = headers_end + 4;
    let content_length = parse_content_length(headers);
    let body_end = content_length
        .map(|length| body_start + length)
        .unwrap_or(request.len())
        .min(request.len());
    let body = &request[body_start..body_end];

    match (method, path) {
        ("POST", "/sensor") => {
            let payload = String::from_utf8_lossy(body);
            println!("sensor payload={payload}");
            respond(stream, 200, "OK");
        }
        _ => respond(stream, 404, "Not Found"),
    }
}

fn run_ota_server(port: u16, firmware: PathBuf) -> ! {
    let firmware_bytes = fs::read(&firmware)
        .unwrap_or_else(|e| panic!("failed to read firmware file `{}`: {e}", firmware.display()));

    let listener = bind_listener("OTA server", port);
    println!("OTA server listening on 0.0.0.0:{port}");
    print_host_ip_hint(port);
    println!(
        "Serving `GET /firmware.bin` from `{}` ({} bytes).",
        firmware.display(),
        firmware_bytes.len()
    );

    accept_loop(listener, |stream| {
        handle_ota_connection(stream, &firmware_bytes)
    })
}

fn handle_ota_connection(stream: &mut TcpStream, firmware_bytes: &[u8]) {
    let Some((request, headers_end)) = read_http_request_or_bad_request(stream) else {
        return;
    };
    let headers = &request[..headers_end];
    let Some((method, path, _version)) = parse_request_line(headers) else {
        respond(stream, 400, "Bad Request");
        return;
    };

    match (method, path) {
        ("GET", "/firmware.bin") => {
            println!("Serving firmware.bin ({} bytes)", firmware_bytes.len());
            respond_binary(
                stream,
                200,
                "OK",
                "application/octet-stream",
                firmware_bytes,
            );
        }
        _ => respond(stream, 404, "Not Found"),
    }
}

fn bind_listener(server_name: &str, port: u16) -> TcpListener {
    TcpListener::bind(("0.0.0.0", port))
        .unwrap_or_else(|e| panic!("failed to bind {server_name} on port {port}: {e}"))
}

fn print_host_ip_hint(port: u16) {
    match detect_host_ipv4() {
        Some(ip) => {
            println!("Host IP (best effort): {ip}");
            println!("Example: `HOST_IP=\"{ip}\" cargo r -r`");
            println!("Remote devices should connect to {ip}:{port}");
        }
        None => {
            println!(
                "Host IP auto-detect failed; find it manually (e.g. `ipconfig getifaddr ...` or `ip addr show ...`)."
            );
        }
    }
}

fn detect_host_ipv4() -> Option<Ipv4Addr> {
    // UDP connect selects the outbound interface and lets us inspect the local socket address.
    for target in ["1.1.1.1:80", "8.8.8.8:80"] {
        let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
            continue;
        };
        if socket.connect(target).is_err() {
            continue;
        }

        let Ok(local_addr) = socket.local_addr() else {
            continue;
        };

        match local_addr.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => return Some(ip),
            _ => continue,
        }
    }

    None
}

fn accept_loop(listener: TcpListener, mut handler: impl FnMut(&mut TcpStream)) -> ! {
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => handler(&mut stream),
            Err(error) => eprintln!("failed to accept connection: {error}"),
        }
    }

    unreachable!()
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let headers = String::from_utf8_lossy(headers);
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("Content-Length") {
            return value.trim().parse::<usize>().ok();
        }
        None
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_request_line(headers: &[u8]) -> Option<(&str, &str, &str)> {
    let line = headers.split(|byte| *byte == b'\n').next()?;
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let line = std::str::from_utf8(line).ok()?;

    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    let version = parts.next()?;
    Some((method, path, version))
}

fn read_http_request(stream: &mut TcpStream) -> Option<(Vec<u8>, usize)> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("failed to set stream timeout");

    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    let mut header_end = None;
    let mut content_length = None;

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                request.extend_from_slice(&chunk[..n]);

                if header_end.is_none() {
                    header_end = find_header_end(&request);
                }

                if content_length.is_none() {
                    if let Some(headers_end) = header_end {
                        let headers = &request[..headers_end];
                        content_length = parse_content_length(headers);
                    }
                }

                if let (Some(headers_end), Some(length)) = (header_end, content_length) {
                    let body_start = headers_end + 4;
                    if request.len().saturating_sub(body_start) >= length {
                        break;
                    }
                }

                // Requests without a body (e.g. GET /firmware.bin) are complete at header end.
                if header_end.is_some() && content_length.is_none() {
                    break;
                }
            }
            Err(error) => {
                eprintln!("failed to read HTTP request: {error}");
                break;
            }
        }
    }

    let headers_end = header_end?;
    Some((request, headers_end))
}

fn read_http_request_or_bad_request(stream: &mut TcpStream) -> Option<(Vec<u8>, usize)> {
    let request = read_http_request(stream);
    if request.is_none() {
        respond(stream, 400, "Bad Request");
    }
    request
}

fn respond(stream: &mut TcpStream, status_code: u16, reason: &str) {
    let response = format!("HTTP/1.1 {status_code} {reason}\r\nContent-Length: 0\r\n\r\n");
    if let Err(error) = stream.write_all(response.as_bytes()) {
        eprintln!("failed to write HTTP response: {error}");
        return;
    }

    if let Err(error) = stream.flush() {
        eprintln!("failed to flush HTTP response: {error}");
    }
}

fn respond_binary(
    stream: &mut TcpStream,
    status_code: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) {
    let headers = format!(
        "HTTP/1.1 {status_code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );

    if let Err(error) = stream.write_all(headers.as_bytes()) {
        eprintln!("failed to write HTTP response headers: {error}");
        return;
    }

    if let Err(error) = stream.write_all(body) {
        eprintln!("failed to write HTTP response body: {error}");
        return;
    }

    if let Err(error) = stream.flush() {
        eprintln!("failed to flush HTTP response: {error}");
    }
}

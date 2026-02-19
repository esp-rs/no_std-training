use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rumqttd::{Broker, Config, ConnectionSettings, Notification, RouterConfig, ServerSettings};

const DEFAULT_PORT: u16 = 1884;
const DEFAULT_HTTP_PORT: u16 = 8080;

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
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::MqttServer { port } => run_mqtt_server(port),
        Command::HttpServer { port } => run_http_server(port),
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
    let listener = TcpListener::bind(("0.0.0.0", port))
        .unwrap_or_else(|e| panic!("failed to bind HTTP server on port {port}: {e}"));
    println!("HTTP server listening on 0.0.0.0:{port}");
    println!("Waiting for POST /sensor requests...");

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => handle_http_connection(&mut stream),
            Err(error) => eprintln!("failed to accept connection: {error}"),
        }
    }

    unreachable!()
}

fn handle_http_connection(stream: &mut TcpStream) {
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
            }
            Err(error) => {
                eprintln!("failed to read HTTP request: {error}");
                break;
            }
        }
    }

    let Some(headers_end) = header_end else {
        respond(stream, 400, "Bad Request");
        return;
    };
    let headers = &request[..headers_end];
    let body_start = headers_end + 4;
    let body_end = content_length
        .map(|length| body_start + length)
        .unwrap_or(request.len())
        .min(request.len());
    let body = &request[body_start..body_end];

    let mut lines = headers.split(|byte| *byte == b'\n');
    let request_line = lines.next().unwrap_or(&[]);
    let request_line = String::from_utf8_lossy(request_line);
    let request_line = request_line.trim_end_matches('\r');

    match request_line
        .split_whitespace()
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["POST", "/sensor", _] => {
            let payload = String::from_utf8_lossy(body);
            println!("sensor payload={payload}");
            respond(stream, 200, "OK");
        }
        _ => respond(stream, 404, "Not Found"),
    }
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

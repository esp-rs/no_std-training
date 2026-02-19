use std::collections::HashMap;
use std::net::SocketAddr;
use std::thread;

use clap::{Parser, Subcommand};
use rumqttd::{Broker, Config, ConnectionSettings, Notification, RouterConfig, ServerSettings};

const DEFAULT_PORT: u16 = 1884;

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
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::MqttServer { port } => run_mqtt_server(port),
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
        .expect("failed to subscribe logger to `#`");

    println!("MQTT server listening on 0.0.0.0:{port}");
    println!("Subscribed to `#` and printing all incoming MQTT messages.");

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

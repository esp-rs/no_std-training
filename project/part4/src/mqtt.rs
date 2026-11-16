use core::fmt::Write;
use core::num::NonZero;
use embassy_net::{IpAddress, Stack, dns::DnsQueryType, tcp::TcpSocket};
use embassy_time::{Duration, Timer};
use log::{error, info};
use rust_mqtt::{
    Bytes,
    buffer::BumpBuffer,
    client::{
        Client,
        options::{ConnectOptions, PublicationOptions, TopicReference},
    },
    config::KeepAlive,
    types::{MqttString, TopicName},
};

use crate::sensor::{Sensor, read_sensor};

const HOST_IP: Option<&str> = option_env!("HOST_IP");
const BROKER_PORT: Option<&str> = option_env!("BROKER_PORT");
// Default to the port of the local broker started with `cargo xtask mqtt-server`
const DEFAULT_BROKER_PORT: u16 = 1884;

const RETRY_DELAY: Duration = Duration::from_secs(5);
const PUBLISH_INTERVAL: Duration = Duration::from_secs(1);

#[embassy_executor::task]
pub async fn mqtt_task(stack: Stack<'static>, mut sht: Sensor) {
    let Some(host) = HOST_IP.filter(|host| !host.is_empty()) else {
        error!("No HOST_IP set. Provide e.g. HOST_IP=10.0.0.10 (or a hostname).");
        return;
    };
    let port = BROKER_PORT
        .and_then(|port| port.parse().ok())
        .unwrap_or(DEFAULT_BROKER_PORT);

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        // Wait until Wi-Fi is connected and DHCP has assigned an address
        stack.wait_config_up().await;

        // If the host is an IPv4 literal, bypass DNS
        let address = match host.parse() {
            Ok(ipv4) => IpAddress::Ipv4(ipv4),
            Err(_) => match stack.dns_query(host, DnsQueryType::A).await.as_deref() {
                Ok([address, ..]) => *address,
                Ok([]) => {
                    error!("DNS query returned no addresses for {host}");
                    Timer::after(RETRY_DELAY).await;
                    continue;
                }
                Err(e) => {
                    error!("DNS lookup for {host} failed: {e:?}");
                    Timer::after(RETRY_DELAY).await;
                    continue;
                }
            },
        };

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        info!("Connecting to MQTT broker at {host}:{port}...");
        if let Err(e) = socket.connect((address, port)).await {
            error!("TCP connect error: {e:?}");
            Timer::after(RETRY_DELAY).await;
            continue;
        }

        let mut mqtt_buffer_storage = [0; 1024];
        let mut mqtt_buffer = BumpBuffer::new(&mut mqtt_buffer_storage);
        let mut client = Client::<_, _, 1, 1, 1, 1, 0, 0, 0>::new(&mut mqtt_buffer);
        // rumqttd rejects the default keep alive of 0, which disables keep alive
        let connect_options = ConnectOptions::new()
            .clean_start()
            .keep_alive(KeepAlive::Seconds(NonZero::new(60).unwrap()));
        let client_id = MqttString::from_str("esp32c3").unwrap();

        if let Err(e) = client
            .connect(socket, &connect_options, Some(client_id))
            .await
        {
            error!("MQTT connect error: {e:?}");
            Timer::after(RETRY_DELAY).await;
            continue;
        }
        info!("Connected to MQTT broker");

        let temperature_options = publication_options("measurement/temperature");
        let humidity_options = publication_options("measurement/humidity");

        loop {
            if !stack.is_config_up() {
                info!("Network connection lost, reconnecting...");
                break;
            }

            let Some((temperature, humidity)) = read_sensor(&mut sht).await else {
                Timer::after(PUBLISH_INTERVAL).await;
                continue;
            };

            let mut temperature_payload = heapless::String::<16>::new();
            write!(temperature_payload, "{temperature:.2}").unwrap();
            let mut humidity_payload = heapless::String::<16>::new();
            write!(humidity_payload, "{humidity:.2}").unwrap();

            if let Err(e) = client
                .publish(
                    &temperature_options,
                    Bytes::from(temperature_payload.as_bytes()),
                )
                .await
            {
                error!("MQTT publish error: {e:?}");
                break;
            }
            if let Err(e) = client
                .publish(&humidity_options, Bytes::from(humidity_payload.as_bytes()))
                .await
            {
                error!("MQTT publish error: {e:?}");
                break;
            }

            Timer::after(PUBLISH_INTERVAL).await;
        }
    }
}

fn publication_options(topic: &'static str) -> PublicationOptions<'static> {
    let topic = TopicName::new(MqttString::from_str(topic).unwrap()).unwrap();
    PublicationOptions::new(TopicReference::Name(topic)).retain()
}

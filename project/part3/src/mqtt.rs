use core::fmt::Write;
use embassy_net::{IpAddress, Ipv4Address, Stack, dns::DnsQueryType, tcp::TcpSocket};
use embassy_time::{Duration, Timer};
use log::{debug, error, info};
use rust_mqtt::{
    Bytes,
    buffer::BumpBuffer,
    client::{
        Client,
        options::{ConnectOptions, PublicationOptions, TopicReference},
    },
    types::{MqttString, TopicName},
};

use crate::sensor::read_sensor;
use esp_hal::i2c::master::I2c;
use shtcx::asynchronous::AsyncShtC3 as ShtC3;

const HOST_IP: Option<&'static str> = option_env!("HOST_IP");
const BROKER_PORT: Option<&'static str> = option_env!("BROKER_PORT");

#[embassy_executor::task]
pub async fn mqtt_task(stack: Stack<'static>, mut sht: ShtC3<I2c<'static, esp_hal::Async>>) {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        // Wait for network to be ready before attempting connection
        debug!("Waiting for WiFi link to come up...");
        stack.wait_link_up().await;
        debug!("WiFi link up, waiting for network configuration...");

        // Wait for DHCP to assign an IP address
        stack.wait_config_up().await;

        debug!("Waiting to get IP address...");
        loop {
            if let Some(config) = stack.config_v4() {
                debug!("Got IP: {}", config.address);
                break;
            }
            Timer::after(Duration::from_millis(500)).await;
        }

        // Check if we still have a valid network config before proceeding
        if !stack.is_config_up() {
            debug!("Network config lost, retrying...");
            continue;
        }

        Timer::after(Duration::from_millis(1_000)).await;

        let host = match HOST_IP {
            Some(h) => h,
            None => {
                error!(
                    "No HOST_IP set. Provide e.g. HOST_IP=10.0.0.10 (or hostname) and optional BROKER_PORT."
                );
                Timer::after(Duration::from_secs(5)).await;
                continue;
            }
        };

        // Default to rumqttd's v5 listener port (1884) unless overridden
        let port: u16 = BROKER_PORT
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(1884);

        // If host is an IPv4 literal, bypass DNS
        let address = match host.parse::<Ipv4Address>() {
            Ok(ipv4) => IpAddress::Ipv4(ipv4),
            Err(_) => match stack.dns_query(host, DnsQueryType::A).await {
                Ok(addresses) if !addresses.is_empty() => addresses[0],
                Ok(_) => {
                    error!("DNS query returned no addresses for {}", host);
                    Timer::after(Duration::from_secs(5)).await;
                    continue;
                }
                Err(e) => {
                    error!("DNS lookup error: {e:?}");
                    Timer::after(Duration::from_secs(5)).await;
                    continue;
                }
            },
        };

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        let remote_endpoint = (address, port);
        info!("connecting to MQTT broker at {}:{}...", host, port);
        let connection = socket.connect(remote_endpoint).await;
        if let Err(e) = connection {
            error!("connect error: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }
        info!("connected!");

        let mut mqtt_buffer_storage = [0; 1024];
        let mut mqtt_buffer = BumpBuffer::new(&mut mqtt_buffer_storage);
        let mut client = Client::<_, _, 1, 1, 1, 1>::new(&mut mqtt_buffer);
        let connect_options = ConnectOptions::new().clean_start();
        let client_id = MqttString::from_str("esp32c3").expect("valid MQTT client id");

        if let Err(e) = client
            .connect(socket, &connect_options, Some(client_id))
            .await
        {
            error!("MQTT connect error: {:?}", e);
            continue;
        }

        // Main sensor reading and publishing loop
        loop {
            // Check network state before attempting operations
            if !stack.is_link_up() || !stack.is_config_up() {
                debug!("Network connection lost, reconnecting...");
                break;
            }

            // Read sensor
            let (temp, _) = match read_sensor(&mut sht).await {
                Some(reading) => reading,
                None => {
                    Timer::after(Duration::from_secs(1)).await;
                    continue;
                }
            };

            let mut temperature_string: heapless::String<32> = heapless::String::new();
            write!(temperature_string, "{:.2}", temp).expect("write! failed!");

            let topic = TopicName::new(
                MqttString::from_str("measurement/temperature").expect("valid MQTT topic string"),
            )
            .expect("valid MQTT topic name");
            let publish_options = PublicationOptions::new(TopicReference::Name(topic)).retain();

            if let Err(e) = client
                .publish(&publish_options, Bytes::from(temperature_string.as_bytes()))
                .await
            {
                error!("MQTT publish error: {:?}", e);
                break;
            }

            // Delay
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}

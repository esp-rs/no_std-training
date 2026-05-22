use core::fmt::Write;
use embassy_net::{IpAddress, Ipv4Address, Stack, dns::DnsQueryType, tcp::TcpSocket};
use embassy_time::{Duration as EmbassyDuration, Timer};
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
        debug!("MQTT: Waiting for WiFi link to come up...");
        stack.wait_link_up().await;
        debug!("MQTT: WiFi link up, waiting for network configuration...");

        // Wait for DHCP to assign an IP address
        stack.wait_config_up().await;

        debug!("MQTT: Waiting to get IP address...");
        loop {
            if let Some(config) = stack.config_v4() {
                debug!("MQTT: Got IP: {}", config.address);
                break;
            }
            Timer::after(EmbassyDuration::from_millis(500)).await;
        }

        // Check if we still have a valid network config before proceeding
        if !stack.is_config_up() {
            debug!("MQTT: Network config lost, retrying...");
            continue;
        }

        Timer::after(EmbassyDuration::from_millis(1_000)).await;

        let host = match HOST_IP {
            Some(h) => h,
            None => {
                error!(
                    "No HOST_IP set. Provide e.g. HOST_IP=10.0.0.10 (or hostname) and optional BROKER_PORT."
                );
                Timer::after(EmbassyDuration::from_secs(5)).await;
                continue;
            }
        };

        // Default to rumqttd's v5 listener port (1884) unless overridden
        let port: u16 = BROKER_PORT
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(1884);

        // If host is an IPv4 literal, bypass DNS
        let address = if let Ok(ipv4) = host.parse::<Ipv4Address>() {
            IpAddress::Ipv4(ipv4)
        } else {
            match stack.dns_query(host, DnsQueryType::A).await.map(|a| a[0]) {
                Ok(address) => address,
                Err(e) => {
                    error!("DNS lookup error: {e:?}");
                    Timer::after(EmbassyDuration::from_secs(5)).await;
                    continue;
                }
            }
        };

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        let remote_endpoint = (address, port);
        info!("connecting to MQTT broker at {}:{}...", host, port);
        let connection = socket.connect(remote_endpoint).await;
        if let Err(e) = connection {
            error!("connect error: {:?}", e);
            Timer::after(EmbassyDuration::from_secs(5)).await;
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
                debug!("MQTT: Network connection lost, reconnecting...");
                break;
            }

            // Read sensor
            let (temp, humidity) = match read_sensor(&mut sht).await {
                Some(reading) => reading,
                None => {
                    Timer::after(EmbassyDuration::from_secs(1)).await;
                    continue;
                }
            };

            // Format sensor values
            let mut temperature_string = heapless::String::<32>::new();
            write!(temperature_string, "{:.2}", temp).expect("write! failed!");

            let mut humidity_string = heapless::String::<32>::new();
            write!(humidity_string, "{:.2}", humidity).expect("write! failed!");

            let temperature_topic = TopicName::new(
                MqttString::from_str("measurement/temperature").expect("valid MQTT topic string"),
            )
            .expect("valid MQTT topic name");
            let temperature_options =
                PublicationOptions::new(TopicReference::Name(temperature_topic)).retain();

            if let Err(e) = client
                .publish(
                    &temperature_options,
                    Bytes::from(temperature_string.as_bytes()),
                )
                .await
            {
                error!("MQTT temperature publish error: {:?}", e);
                break;
            }

            let humidity_topic = TopicName::new(
                MqttString::from_str("measurement/humidity").expect("valid MQTT topic string"),
            )
            .expect("valid MQTT topic name");
            let humidity_options =
                PublicationOptions::new(TopicReference::Name(humidity_topic)).retain();

            if let Err(e) = client
                .publish(&humidity_options, Bytes::from(humidity_string.as_bytes()))
                .await
            {
                error!("MQTT humidity publish error: {:?}", e);
                break;
            }

            // Delay
            Timer::after(EmbassyDuration::from_secs(1)).await;
        }
    }
}

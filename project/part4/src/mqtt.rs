use core::fmt::Write;
use embassy_net::{IpAddress, Ipv4Address, Stack, dns::DnsQueryType, tcp::TcpSocket};
use embassy_time::{Duration as EmbassyDuration, Timer};
use log::{debug, error, info};
use rust_mqtt::{
    client::{client::MqttClient, client_config::ClientConfig as MqttClientConfig},
    packet::v5::{publish_packet::QualityOfService, reason_codes::ReasonCode},
    utils::rng_generator::CountingRng,
};

use crate::sensor::read_sensor;
use esp_hal::i2c::master::I2c;
use shtcx::asynchronous::ShtC3;

const BROKER_HOST: Option<&'static str> = option_env!("BROKER_HOST");
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

        let host = match BROKER_HOST {
            Some(h) => h,
            None => {
                error!(
                    "No BROKER_HOST set. Provide e.g. BROKER_HOST=10.0.0.10 (or hostname) and optional BROKER_PORT."
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

        let mut config = MqttClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(20000),
        );
        config.add_max_subscribe_qos(QualityOfService::QoS1);
        config.add_client_id("esp32c3");
        config.max_packet_size = 1024;
        let mut recv_buffer = [0; 512];
        let mut write_buffer = [0; 512];
        let write_len = write_buffer.len();
        let recv_len = recv_buffer.len();

        let mut client = MqttClient::<_, 5, _>::new(
            socket,
            &mut write_buffer,
            write_len,
            &mut recv_buffer,
            recv_len,
            config,
        );

        if let Err(mqtt_error) = client.connect_to_broker().await {
            match mqtt_error {
                ReasonCode::NetworkError => error!("MQTT Network Error"),
                _ => error!("Other MQTT Error: {:?}", mqtt_error),
            }
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

            // Helper to handle MQTT send errors
            let handle_mqtt_error = |mqtt_error: ReasonCode| match mqtt_error {
                ReasonCode::NetworkError => {
                    error!("MQTT Network Error");
                    true // Signal to break out of inner loop
                }
                _ => {
                    error!("Other MQTT Error: {:?}", mqtt_error);
                    false // Continue in inner loop
                }
            };

            // Publish temperature
            if let Err(e) = client
                .send_message(
                    "measurement/temperature",
                    temperature_string.as_bytes(),
                    QualityOfService::QoS1,
                    true,
                )
                .await
            {
                if handle_mqtt_error(e) {
                    break; // Network error, reconnect
                }
                continue;
            }

            // Publish humidity
            if let Err(e) = client
                .send_message(
                    "measurement/humidity",
                    humidity_string.as_bytes(),
                    QualityOfService::QoS1,
                    true,
                )
                .await
            {
                if handle_mqtt_error(e) {
                    break; // Network error, reconnect
                }
                continue;
            }

            // Delay
            Timer::after(EmbassyDuration::from_secs(1)).await;
        }
    }
}

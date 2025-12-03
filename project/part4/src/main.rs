// MQTT Communication (without wifi provisioning)
// 1. Install tools
// cargo install --git https://github.com/bytebeamio/rumqtt rumqttd
// brew install mosquitto
// 2. Get your IP
// ipconfig getifaddr en0
// 3. Run the broker
// rumqttd
// 4. Subscribe to the topic
// mosquitto_sub -h <IP> -p 1884 -V mqttv5 -i mac-subscriber -t 'measurement/#' -v
// 5. Run the app
// SSID="<SSID>" PASSWORD=<PASSWORD> BROKER_HOST="<IP>" BROKER_PORT="1884" cargo r -r

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::fmt::Write;

use embassy_executor::Spawner;
use embassy_net::{
    IpAddress, Ipv4Address, Runner, StackResources, dns::DnsQueryType, tcp::TcpSocket,
};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config, I2c},
    ram,
    rng::Rng,
    timer::timg::TimerGroup,
};
use esp_radio::{
    Controller,
    wifi::{ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent, WifiStaState},
};
use heapless::String;
use log::{debug, error, info};
use rust_mqtt::{
    client::{client::MqttClient, client_config::ClientConfig as MqttClientConfig},
    packet::v5::reason_codes::ReasonCode,
    utils::rng_generator::CountingRng,
};
use shtcx::{
    self,
    asynchronous::{PowerMode, max_measurement_duration, shtc3},
};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
const BROKER_HOST: Option<&'static str> = option_env!("BROKER_HOST");
const BROKER_PORT: Option<&'static str> = option_env!("BROKER_PORT");

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    let sda = peripherals.GPIO10;
    let scl = peripherals.GPIO8;
    let i2c = I2c::new(peripherals.I2C0, Config::default())
        .expect("Failed to create I2C bus")
        .with_sda(sda)
        .with_scl(scl)
        .into_async();
    let mut sht = shtc3(i2c);

    static ESP_RADIO_CTRL_CELL: static_cell::StaticCell<Controller<'static>> =
        static_cell::StaticCell::new();
    let esp_radio_ctrl = &*ESP_RADIO_CTRL_CELL
        .uninit()
        .write(esp_radio::init().expect("Failed to initialize radio controller"));

    let (controller, interfaces) =
        esp_radio::wifi::new(esp_radio_ctrl, peripherals.WIFI, Default::default())
            .expect("Failed to create WiFi controller");

    let wifi_interface = interfaces.sta;

    let config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    static STACK_RESOURCES_CELL: static_cell::StaticCell<StackResources<3>> =
        static_cell::StaticCell::new();
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        STACK_RESOURCES_CELL
            .uninit()
            .write(StackResources::<3>::new()),
        seed,
    );
    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(runner)).ok();

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        // Wait for network to be ready before attempting connection
        debug!("Waiting for WiFi link to come up...");
        stack.wait_link_up().await;
        debug!("WiFi link up, waiting for network configuration...");

        // Wait for DHCP to assign an IP address
        loop {
            if stack.is_config_up() {
                break;
            }
            Timer::after(Duration::from_millis(100)).await;
        }

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

        let host = match BROKER_HOST {
            Some(h) => h,
            None => {
                error!(
                    "No BROKER_HOST set. Provide e.g. BROKER_HOST=10.0.0.10 (or hostname) and optional BROKER_PORT."
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
        let address = if let Ok(ipv4) = host.parse::<Ipv4Address>() {
            IpAddress::Ipv4(ipv4)
        } else {
            match stack.dns_query(host, DnsQueryType::A).await.map(|a| a[0]) {
                Ok(address) => address,
                Err(e) => {
                    error!("DNS lookup error: {e:?}");
                    Timer::after(Duration::from_secs(5)).await;
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
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }
        info!("connected!");

        let mut config = MqttClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(20000),
        );
        config.add_max_subscribe_qos(rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1);
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

        match client.connect_to_broker().await {
            Ok(()) => {}
            Err(mqtt_error) => match mqtt_error {
                ReasonCode::NetworkError => {
                    error!("MQTT Network Error");
                    continue;
                }
                _ => {
                    error!("Other MQTT Error: {:?}", mqtt_error);
                    continue;
                }
            },
        }

        // Main sensor reading and publishing loop
        loop {
            // Check network state before attempting operations
            if !stack.is_link_up() || !stack.is_config_up() {
                debug!("Network connection lost, reconnecting...");
                break;
            }

            // Read sensor
            if let Err(e) = sht.start_measurement(PowerMode::NormalMode).await {
                error!("Failed to start measurement: {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
            // Wait for 12.1 ms https://github.com/Fristi/shtcx-rs/blob/feature/async-support/src/asynchronous.rs#L413-L424
            let duration = max_measurement_duration(&sht, PowerMode::NormalMode);
            Timer::after(Duration::from_micros(duration.into())).await;
            let measurement = match sht.get_measurement_result().await {
                Ok(m) => m,
                Err(e) => {
                    error!("Failed to get measurement result: {:?}", e);
                    Timer::after(Duration::from_secs(1)).await;
                    continue;
                }
            };

            info!(
                "  {:.2} °C | {:.2} %RH",
                measurement.temperature.as_degrees_celsius(),
                measurement.humidity.as_percent(),
            );

            let mut temperature_string: String<32> = String::new();
            write!(
                temperature_string,
                "{:.2}",
                measurement.temperature.as_degrees_celsius()
            )
            .expect("write! failed!");

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

            // MQTT
            match client
                .send_message(
                    "measurement/temperature",
                    temperature_string.as_bytes(),
                    rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1,
                    true,
                )
                .await
            {
                Ok(()) => {}
                Err(mqtt_error) => {
                    if handle_mqtt_error(mqtt_error) {
                        break; // Network error, reconnect
                    }
                    continue;
                }
            }

            // Delay
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    debug!("start connection task");
    debug!("Device capabilities: {:?}", controller.capabilities());
    loop {
        if esp_radio::wifi::sta_state() == WifiStaState::Connected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await;
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(SSID.into())
                    .with_password(PASSWORD.into()),
            );
            controller
                .set_config(&client_config)
                .expect("Failed to set WiFi configuration");
            debug!("Starting wifi");
            controller
                .start_async()
                .await
                .expect("Failed to start WiFi");
            debug!("Wifi started!");
        }
        debug!("About to connect...");

        match controller.connect_async().await {
            Ok(_) => info!("Wifi connected!"),
            Err(e) => {
                error!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

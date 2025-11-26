// OTA Update
// 1. Install tools
// cargo install --git https://github.com/bytebeamio/rumqtt rumqttd
// brew install mosquitto / https://mosquitto.org/download/
// 2. Get your IP
// ipconfig getifaddr en0 or ip addr show eth0
// 3. Run the broker
// rumqttd
// 4. Subscribe to the topic
// mosquitto_sub -h <IP> -p 1884 -V mqttv5 -i mac-subscriber -t 'measurement/#' -v
// 5. Install http-serve-folder
// cargo install http-serve-folder
// 6. Run the server
// http-serve-folder --ip_address <IP> -p 8080 -l debug ota
// 7. Run the app
// BROKER_HOST="<IP>" BROKER_PORT="1884" HOST_IP=<IP> cargo r -r
// 8. Join the AP network and navigate to http://<MCU_IP>/ the wifi credentials
// Once the device stops the AP mode and starts the STA mode connected to the wifi, it will start sending sensor data to the MQTT broker and wait for the button press to trigger OTA update.
// 9. Press the button to trigger OTA update. Ctrl+R to reset the device after the fimrware is downloaded.

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

mod button;
mod http;
mod mqtt;
mod network;
mod ota;
mod sensor;

use core::net::Ipv4Addr;
use core::str::FromStr;

const GW_IP_ADDR_ENV: Option<&'static str> = option_env!("GATEWAY_IP");
use embassy_executor::Spawner;
use embassy_sync::channel::Channel;
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig},
    i2c::master::{Config, I2c},
    interrupt::software::SoftwareInterruptControl,
    ram,
    timer::timg::TimerGroup,
};
use esp_radio::Controller;
use log::{debug, info};

use crate::button::{BUTTON_PRESSED, button_monitor};
use crate::http::{run_captive_portal, run_dhcp, run_http_server};
use crate::mqtt::mqtt_task;
use crate::network::{
    NetworkStacks, WifiCredentials, connection, create_network_stacks, net_task, sta_net_task,
};
use crate::ota::{FLASH_STORAGE, http_client_task};
use shtcx::asynchronous::shtc3;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(esp_hal::clock::CpuClock::max());
    let peripherals = esp_hal::init(config);

    let mut flash = esp_storage::FlashStorage::new(peripherals.FLASH);
    let mut buffer = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
    let pt = esp_bootloader_esp_idf::partitions::read_partition_table(&mut flash, &mut buffer)
        .expect("Failed to read partition table");
    info!("Currently booted partition {:?}", pt.booted_partition());

    // Store flash storage in mutex for OTA updates
    *FLASH_STORAGE.lock().await = Some(flash);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // Initialize I2C and sensor
    let sda = peripherals.GPIO10;
    let scl = peripherals.GPIO8;
    let i2c = I2c::new(peripherals.I2C0, Config::default())
        .expect("Failed to create I2C bus")
        .with_sda(sda)
        .with_scl(scl)
        .into_async();
    let mut sht = shtc3(i2c);

    debug!(
        "Raw ID register: {}",
        sht.raw_id_register()
            .await
            .expect("Failed to get raw ID register")
    );

    // Set up button on GPIO9 (BOOT button on ESP32-C3)
    let button_pin = peripherals.GPIO9;
    let config = InputConfig::default();
    let button = Input::new(button_pin, config);

    // Initialize WiFi radio
    static ESP_RADIO_CTRL_CELL: static_cell::StaticCell<Controller<'static>> =
        static_cell::StaticCell::new();
    let esp_radio_ctrl = &*ESP_RADIO_CTRL_CELL
        .uninit()
        .write(esp_radio::init().expect("Failed to initialize radio controller"));

    let (controller, interfaces) =
        esp_radio::wifi::new(esp_radio_ctrl, peripherals.WIFI, Default::default())
            .expect("Failed to create WiFi controller");

    let ap_device = interfaces.ap;
    let sta_device = interfaces.sta;

    // Setup network stacks
    let gw_ip_addr_str = GW_IP_ADDR_ENV.unwrap_or("192.168.2.1");
    let gw_ip_addr = Ipv4Addr::from_str(gw_ip_addr_str).expect("failed to parse gateway ip");

    let NetworkStacks {
        ap_stack,
        ap_runner,
        sta_stack,
        sta_runner,
    } = create_network_stacks(ap_device, sta_device, gw_ip_addr);

    // Create WiFi credentials channel
    static WIFI_CREDENTIALS_CHANNEL_CELL: static_cell::StaticCell<
        Channel<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, WifiCredentials, 1>,
    > = static_cell::StaticCell::new();
    let wifi_credentials_channel = WIFI_CREDENTIALS_CHANNEL_CELL.uninit().write(Channel::new());

    // Spawn all tasks
    spawner
        .spawn(connection(controller, wifi_credentials_channel))
        .ok();
    spawner.spawn(net_task(ap_runner)).ok();
    spawner.spawn(sta_net_task(sta_runner)).ok();
    spawner.spawn(run_dhcp(ap_stack, gw_ip_addr)).ok();
    spawner.spawn(run_captive_portal(ap_stack, gw_ip_addr)).ok();
    spawner.spawn(mqtt_task(sta_stack, sht)).ok();
    spawner.spawn(button_monitor(button, &BUTTON_PRESSED)).ok();
    spawner
        .spawn(http_client_task(sta_stack, &BUTTON_PRESSED))
        .ok();

    // Wait for AP link to come up
    loop {
        if ap_stack.is_link_up() {
            break;
        }
        Timer::after(EmbassyDuration::from_millis(500)).await;
    }
    info!("WiFi Provisioning Portal Ready");
    debug!("1. Connect to the AP: `esp-radio`");
    debug!("2. Navigate to: http://{gw_ip_addr_str}/");
    while !ap_stack.is_config_up() {
        Timer::after(EmbassyDuration::from_millis(100)).await
    }
    ap_stack
        .config_v4()
        .inspect(|c| debug!("ipv4 config: {c:?}"));

    spawner
        .spawn(run_http_server(ap_stack, wifi_credentials_channel))
        .ok();

    // Keep main task alive
    loop {
        Timer::after(EmbassyDuration::from_secs(60)).await;
    }
}

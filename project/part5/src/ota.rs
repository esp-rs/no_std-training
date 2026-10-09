use core::net::{Ipv4Addr, SocketAddr};
use edge_http::Method;
use edge_http::io::client::Connection;
use edge_nal::WithTimeout;
use edge_nal_embassy::{Tcp, TcpBuffers};
use embassy_net::Stack;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration as EmbassyDuration, Timer};
use embedded_io_async::Read;
use embedded_storage::Storage;
use esp_storage::FlashStorage;
use log::{debug, error, info};

use crate::status_led::{LED_STATUS, LedStatus};

const HOST_IP: Option<&'static str> = option_env!("HOST_IP");
const OTA_CHECK_INTERVAL_SECS: Option<&'static str> = option_env!("OTA_CHECK_INTERVAL_SECS");
const OTA_PORT: u16 = 8080;

// ANCHOR: image_layout
// ESP-IDF application image layout: a 24-byte image header and an 8-byte segment header
// precede the application descriptor created by `esp_app_desc!()`.
const IMAGE_MAGIC: u8 = 0xE9;
const APP_DESC_OFFSET: usize = 32;
const APP_DESC_MAGIC: u32 = 0xABCD_5432;
const VERSION_OFFSET: usize = APP_DESC_OFFSET + 16;
const VERSION_LEN: usize = 32;
const IMAGE_PREFIX_LEN: usize = VERSION_OFFSET + VERSION_LEN;
// ANCHOR_END: image_layout

// ANCHOR: flash_storage_static
pub static FLASH_STORAGE: Mutex<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    Option<FlashStorage<'static>>,
> = Mutex::new(None);
// ANCHOR_END: flash_storage_static

// ANCHOR: ota_interval
fn ota_check_interval() -> EmbassyDuration {
    const DEFAULT_SECS: u64 = 300;

    let secs = OTA_CHECK_INTERVAL_SECS
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|secs: &u64| *secs > 0)
        .unwrap_or(DEFAULT_SECS);

    EmbassyDuration::from_secs(secs)
}
// ANCHOR_END: ota_interval

// ANCHOR: image_version
/// Returns the version stored in the application descriptor of an image, or `None` if the
/// bytes do not start an ESP-IDF application image.
fn image_version(prefix: &[u8; IMAGE_PREFIX_LEN]) -> Option<&str> {
    let app_desc_magic = prefix[APP_DESC_OFFSET..]
        .first_chunk()
        .map(|bytes| u32::from_le_bytes(*bytes));
    if prefix[0] != IMAGE_MAGIC || app_desc_magic != Some(APP_DESC_MAGIC) {
        return None;
    }

    let version = &prefix[VERSION_OFFSET..];
    let len = version.iter().position(|&b| b == 0).unwrap_or(VERSION_LEN);
    core::str::from_utf8(&version[..len]).ok()
}
// ANCHOR_END: image_version

/// Downloads `/firmware.bin` and installs it in the next OTA slot.
///
/// Returns `Ok(true)` if a new image was installed, and `Ok(false)` if the server offers the
/// version that is already running.
async fn update_firmware(stack: Stack<'static>, host_ip: Ipv4Addr) -> Result<bool, ()> {
    // Ensure network is ready
    stack.wait_link_up().await;
    stack.wait_config_up().await;

    // Small delay before connecting
    Timer::after(EmbassyDuration::from_millis(500)).await;

    // ANCHOR: http_request
    let buffers = TcpBuffers::<1, 1024, 4096>::new();
    let tcp = WithTimeout::new(30_000, Tcp::new(stack, &buffers));
    let mut http_buffer = [0u8; 1024];
    let mut conn: Connection<'_, _> = Connection::new(
        &mut http_buffer,
        &tcp,
        SocketAddr::new(host_ip.into(), OTA_PORT),
    );

    debug!("HTTP Client: Requesting firmware from {host_ip}:{OTA_PORT}...");
    conn.initiate_request(false, Method::Get, "/firmware.bin", &[])
        .await
        .map_err(|e| error!("HTTP Client: Request error: {e:?}"))?;
    conn.initiate_response()
        .await
        .map_err(|e| error!("HTTP Client: Response error: {e:?}"))?;
    // ANCHOR_END: http_request

    // ANCHOR: check_response
    let (headers, body) = conn.split();
    if headers.code != 200 {
        error!("HTTP Client: Server responded with status {}", headers.code);
        return Err(());
    }
    let Some(content_len) = headers.headers.content_len() else {
        error!("HTTP Client: Response has no Content-Length");
        return Err(());
    };
    // ANCHOR_END: check_response

    // ANCHOR: check_version
    let mut prefix = [0u8; IMAGE_PREFIX_LEN];
    body.read_exact(&mut prefix)
        .await
        .map_err(|e| error!("HTTP Client: Read error: {e:?}"))?;

    let Some(version) = image_version(&prefix) else {
        error!("HTTP Client: Response is not an application image");
        return Err(());
    };
    let running_version = crate::ESP_APP_DESC.version();
    if version == running_version {
        info!("HTTP Client: Firmware {running_version} is up to date");
        return Ok(false);
    }
    info!("HTTP Client: Updating firmware {running_version} -> {version}");
    // ANCHOR_END: check_version

    // Get flash storage from mutex
    let mut flash_guard = FLASH_STORAGE.lock().await;
    let flash = flash_guard.as_mut().ok_or_else(|| {
        error!("HTTP Client: Flash storage not available");
    })?;

    // ANCHOR: ota_updater
    // Initialize OTA updater
    let mut ota_buffer = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
    let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(flash, &mut ota_buffer)
        .map_err(|e| {
            error!("HTTP Client: Failed to create OTA updater: {:?}", e);
        })?;

    let (mut next_app_partition, part_type) = ota.next_partition().map_err(|e| {
        error!("HTTP Client: Failed to get next partition: {:?}", e);
    })?;

    debug!("HTTP Client: Flashing image to {:?}", part_type);
    // ANCHOR_END: ota_updater

    // ANCHOR: write_firmware
    // Write the bytes already read for the version check, then stream the rest
    next_app_partition
        .write(0, &prefix)
        .map_err(|e| error!("HTTP Client: Failed to write chunk at offset 0: {e:?}"))?;

    let mut written = IMAGE_PREFIX_LEN;
    let mut chunk = [0u8; 4096];
    loop {
        let n = body
            .read(&mut chunk)
            .await
            .map_err(|e| error!("HTTP Client: Read error: {e:?}"))?;
        if n == 0 {
            break;
        }
        next_app_partition
            .write(written as u32, &chunk[..n])
            .map_err(|e| error!("HTTP Client: Failed to write chunk at offset {written}: {e:?}"))?;
        written += n;
    }

    if written as u64 != content_len {
        error!("HTTP Client: Download incomplete ({written} of {content_len} bytes)");
        return Err(());
    }
    debug!("HTTP Client: Firmware download complete ({written} bytes)");
    // ANCHOR_END: write_firmware

    // ANCHOR: activate_partition
    // Activate the next partition
    ota.activate_next_partition().map_err(|e| {
        error!("HTTP Client: Failed to activate partition: {:?}", e);
    })?;
    info!("HTTP Client: Partition activated successfully");

    // Set OTA state to NEW
    match ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::New) {
        Ok(()) => {
            debug!("HTTP Client: OTA state set to NEW");
        }
        Err(e) => {
            error!("HTTP Client: Failed to set OTA state: {:?}", e);
        }
    }
    // ANCHOR_END: activate_partition

    info!("HTTP Client: OTA update complete");
    Ok(true)
}

#[embassy_executor::task]
pub async fn http_client_task(stack: Stack<'static>) {
    debug!("HTTP Client: Task started!");
    // Wait for WiFi connection
    debug!("HTTP Client: Waiting for WiFi connection...");

    // Wait for network to be configured (which means WiFi is connected)
    stack.wait_config_up().await;
    debug!("HTTP Client: Network configured, WiFi is connected");
    LED_STATUS.signal(LedStatus::Idle);

    // Wait for network to be fully ready
    debug!("HTTP Client: Waiting for network to stabilize...");
    Timer::after(EmbassyDuration::from_secs(2)).await;

    if let Some(config) = stack.config_v4() {
        debug!("HTTP Client: Got IP address: {}", config.address);
    }

    let ota_interval = ota_check_interval();
    info!(
        "HTTP Client: Periodic OTA checks enabled (every {}s)",
        ota_interval.as_secs()
    );

    loop {
        debug!("HTTP Client: Checking for firmware update...");

        // Get host IP from environment variable
        let Some(host_ip_str) = HOST_IP else {
            debug!("HTTP Client: HOST_IP not set, skipping OTA update");
            Timer::after(ota_interval).await;
            continue;
        };
        let Ok(host_ip) = host_ip_str.parse::<Ipv4Addr>() else {
            debug!("HTTP Client: Invalid HOST_IP format: {}", host_ip_str);
            Timer::after(ota_interval).await;
            continue;
        };

        // ANCHOR: apply_update
        // Attempt firmware update - reset if a new image was installed
        LED_STATUS.signal(LedStatus::Updating);

        if update_firmware(stack, host_ip).await == Ok(true) {
            info!("HTTP Client: Rebooting into updated firmware");
            esp_hal::system::software_reset();
        }

        LED_STATUS.signal(LedStatus::Idle);
        Timer::after(ota_interval).await;
        // ANCHOR_END: apply_update
    }
}

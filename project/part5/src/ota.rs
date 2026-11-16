use core::net::{Ipv4Addr, SocketAddr};
use edge_http::Method;
use edge_http::io::client::Connection;
use edge_nal::WithTimeout;
use edge_nal_embassy::{Tcp, TcpBuffers};
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use embedded_io_async::Read;
use esp_bootloader_esp_idf::{ota::OtaImageState, ota_updater::OtaUpdater, partitions};
use esp_storage::FlashStorage;
use log::{debug, error, info};

use crate::status_led::{LED_STATUS, LedStatus};

const HOST_IP: Option<&str> = option_env!("HOST_IP");
const OTA_CHECK_INTERVAL_SECS: Option<&str> = option_env!("OTA_CHECK_INTERVAL_SECS");
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

const FLASH_SECTOR_SIZE: usize = FlashStorage::SECTOR_SIZE as usize;

// ANCHOR: ota_interval
fn ota_check_interval() -> Duration {
    const DEFAULT_SECS: u64 = 300;

    let secs = OTA_CHECK_INTERVAL_SECS
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|&secs| secs > 0)
        .unwrap_or(DEFAULT_SECS);

    Duration::from_secs(secs)
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
async fn update_firmware(
    stack: Stack<'static>,
    host_ip: Ipv4Addr,
    flash: &mut FlashStorage<'static>,
) -> Result<bool, ()> {
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
    // Flash is erased and written in whole sectors, so the download is buffered one sector at a
    // time. The first bytes contain the version of the image.
    let mut sector = [0u8; FLASH_SECTOR_SIZE];
    body.read_exact(&mut sector[..IMAGE_PREFIX_LEN])
        .await
        .map_err(|e| error!("HTTP Client: Read error: {e:?}"))?;

    let Some(version) = image_version(sector.first_chunk().unwrap()) else {
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

    // ANCHOR: ota_updater
    let mut ota_buffer = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let mut ota = OtaUpdater::new(flash, &mut ota_buffer)
        .map_err(|e| error!("HTTP Client: Failed to create OTA updater: {e:?}"))?;

    let (mut partition, part_type) = ota
        .next_partition()
        .map_err(|e| error!("HTTP Client: Failed to get next partition: {e:?}"))?;
    if content_len > partition.partition_size() as u64 {
        error!("HTTP Client: Image ({content_len} bytes) does not fit in {part_type:?}");
        return Err(());
    }
    debug!("HTTP Client: Flashing image to {part_type:?}");
    // ANCHOR_END: ota_updater

    // ANCHOR: write_firmware
    let mut filled = IMAGE_PREFIX_LEN;
    let mut written = 0;
    loop {
        let n = body
            .read(&mut sector[filled..])
            .await
            .map_err(|e| error!("HTTP Client: Read error: {e:?}"))?;
        filled += n;

        // Write each sector once it is full, and the remaining bytes at the end of the body
        if filled == sector.len() || n == 0 {
            partition
                .write(written as u32, &sector[..filled])
                .map_err(|e| error!("HTTP Client: Failed to write at offset {written}: {e:?}"))?;
            written += filled;
            filled = 0;
        }
        if n == 0 {
            break;
        }
    }

    if written as u64 != content_len {
        error!("HTTP Client: Download incomplete ({written} of {content_len} bytes)");
        return Err(());
    }
    debug!("HTTP Client: Firmware download complete ({written} bytes)");
    // ANCHOR_END: write_firmware

    // ANCHOR: activate_partition
    // Activate the next partition
    ota.activate_next_partition()
        .map_err(|e| error!("HTTP Client: Failed to activate partition: {e:?}"))?;
    info!("HTTP Client: Partition activated successfully");

    // Only relevant for bootloaders built with app rollback support
    if let Err(e) = ota.set_current_ota_state(OtaImageState::New) {
        error!("HTTP Client: Failed to set OTA state: {e:?}");
    }
    // ANCHOR_END: activate_partition

    info!("HTTP Client: OTA update complete");
    Ok(true)
}

#[embassy_executor::task]
pub async fn http_client_task(stack: Stack<'static>, mut flash: FlashStorage<'static>) {
    // ANCHOR: ota_host
    let Some(host_ip) = HOST_IP.and_then(|host| host.parse::<Ipv4Addr>().ok()) else {
        error!("HTTP Client: HOST_IP is not set to an IPv4 address, OTA updates are disabled");
        return;
    };
    // ANCHOR_END: ota_host

    let ota_interval = ota_check_interval();
    info!(
        "HTTP Client: Periodic OTA checks enabled (every {}s)",
        ota_interval.as_secs()
    );

    loop {
        // Wait until provisioning is complete and DHCP has configured the station
        stack.wait_config_up().await;

        // ANCHOR: apply_update
        LED_STATUS.signal(LedStatus::Updating);
        debug!("HTTP Client: Checking for firmware update...");
        if update_firmware(stack, host_ip, &mut flash).await == Ok(true) {
            info!("HTTP Client: Rebooting into updated firmware");
            esp_hal::system::software_reset();
        }
        LED_STATUS.signal(LedStatus::Idle);
        // ANCHOR_END: apply_update

        Timer::after(ota_interval).await;
    }
}

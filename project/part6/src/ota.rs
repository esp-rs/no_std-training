use core::fmt::Write;
use embassy_net::{IpAddress, Ipv4Address, Stack, tcp::TcpSocket};
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration as EmbassyDuration, Timer};
use embedded_io_async::Write as IoWrite;
use embedded_storage::Storage;
use esp_storage::FlashStorage;
use log::{debug, error, info};

const HOST_IP: Option<&'static str> = option_env!("HOST_IP");

pub static FLASH_STORAGE: Mutex<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    Option<FlashStorage<'static>>,
> = Mutex::new(None);

async fn download_and_flash_firmware(
    stack: Stack<'static>,
    host_ip_str: &str,
    address: IpAddress,
    flash_storage: &'static Mutex<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        Option<FlashStorage<'static>>,
    >,
) -> Result<(), ()> {
    // Ensure network is ready
    if !stack.is_link_up() {
        error!("HTTP Client: Network link is not up");
        return Err(());
    }

    if !stack.is_config_up() {
        error!("HTTP Client: Network is not configured");
        return Err(());
    }

    // Small delay before connecting
    Timer::after(EmbassyDuration::from_millis(500)).await;

    // Prepare buffers for TCP socket
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(EmbassyDuration::from_secs(30)));

    let port = 8080;
    debug!("HTTP Client: Connecting to {}:{}...", address, port);

    socket.connect((address, port)).await.map_err(|e| {
        error!("HTTP Client: Connect error: {:?}", e);
    })?;

    debug!("HTTP Client: Connected!");

    // Send HTTP GET request for firmware.bin
    let mut http_request = heapless::String::<128>::new();
    write!(
        http_request,
        "GET /firmware.bin HTTP/1.0\r\nHost: {}\r\n\r\n",
        host_ip_str
    )
    .expect("Failed to format HTTP request");

    socket
        .write_all(http_request.as_bytes())
        .await
        .map_err(|e| {
            error!("HTTP Client: Write error: {:?}", e);
        })?;

    socket.flush().await.map_err(|e| {
        error!("HTTP Client: Flush error: {:?}", e);
    })?;

    debug!("HTTP Client: Request sent, reading response...");

    // Read HTTP response headers
    let mut header_buffer = [0u8; 1024];
    let mut header_len = 0;

    // Read headers until we find the end marker
    loop {
        if header_len >= header_buffer.len() {
            error!("HTTP Client: Headers too long, aborting");
            return Err(());
        }

        match socket.read(&mut header_buffer[header_len..]).await {
            Ok(0) => {
                error!("HTTP Client: Connection closed before headers");
                return Err(());
            }
            Ok(n) => {
                header_len += n;
                // Look for double CRLF indicating end of headers
                if let Some(pos) = header_buffer[..header_len]
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                {
                    // Calculate how much data is left in the buffer after headers
                    let data_start = pos + 4;
                    let data_in_header = header_len - data_start;

                    debug!("HTTP Client: Headers received, starting firmware download...");

                    // Get flash storage from mutex
                    let mut flash_guard = flash_storage.lock().await;
                    let flash = flash_guard.as_mut().ok_or_else(|| {
                        error!("HTTP Client: Flash storage not available");
                    })?;

                    // Initialize OTA updater
                    let mut ota_buffer =
                        [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
                    let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(
                        flash,
                        &mut ota_buffer,
                    )
                    .map_err(|e| {
                        error!("HTTP Client: Failed to create OTA updater: {:?}", e);
                    })?;

                    let (mut next_app_partition, part_type) =
                        ota.next_partition().map_err(|e| {
                            error!("HTTP Client: Failed to get next partition: {:?}", e);
                        })?;

                    debug!("HTTP Client: Flashing image to {:?}", part_type);

                    // Write any data that came with headers
                    if data_in_header > 0 {
                        let chunk = &header_buffer[data_start..header_len];
                        next_app_partition.write(0, chunk).map_err(|e| {
                            error!("HTTP Client: Failed to write initial chunk: {:?}", e);
                        })?;
                        debug!("HTTP Client: Wrote initial {} bytes", data_in_header);
                    }

                    // Read and write firmware in chunks
                    let mut write_offset = data_in_header as u32;
                    let mut chunk_buffer = [0u8; 4096];
                    let mut total_written = data_in_header;

                    loop {
                        match socket.read(&mut chunk_buffer).await {
                            Ok(0) => {
                                debug!("HTTP Client: Firmware download complete");
                                break;
                            }
                            Ok(n) => {
                                let chunk = &chunk_buffer[..n];
                                next_app_partition.write(write_offset, chunk).map_err(|e| {
                                    error!(
                                        "HTTP Client: Failed to write chunk at offset {}: {:?}",
                                        write_offset, e
                                    );
                                })?;
                                write_offset += n as u32;
                                total_written += n;
                                debug!("HTTP Client: Wrote {} bytes (total: {})", n, total_written);
                            }
                            Err(e) => {
                                error!("HTTP Client: Read error: {:?}", e);
                                return Err(());
                            }
                        }
                    }

                    debug!("HTTP Client: Firmware written, activating partition...");

                    // Activate the next partition
                    ota.activate_next_partition().map_err(|e| {
                        error!("HTTP Client: Failed to activate partition: {:?}", e);
                    })?;
                    info!("HTTP Client: Partition activated successfully");

                    // Set OTA state to NEW
                    match ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::New)
                    {
                        Ok(()) => {
                            debug!("HTTP Client: OTA state set to NEW");
                        }
                        Err(e) => {
                            error!("HTTP Client: Failed to set OTA state: {:?}", e);
                        }
                    }

                    info!("HTTP Client: OTA update complete! Please reset the device.");
                    return Ok(());
                }
            }
            Err(e) => {
                error!("HTTP Client: Read error while reading headers: {:?}", e);
                return Err(());
            }
        }
    }
}

#[embassy_executor::task]
pub async fn http_client_task(
    stack: Stack<'static>,
    button_pressed: &'static embassy_sync::signal::Signal<
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        (),
    >,
) {
    debug!("HTTP Client: Task started!");
    // Wait for WiFi connection
    debug!("HTTP Client: Waiting for WiFi connection...");

    // Wait for network to be configured (which means WiFi is connected)
    loop {
        if stack.is_config_up() {
            debug!("HTTP Client: Network configured, WiFi is connected");
            break;
        }
        Timer::after(EmbassyDuration::from_millis(100)).await;
    }
    debug!("HTTP Client: WiFi connected, network configuration ready");

    // Wait for network to be fully ready
    debug!("HTTP Client: Waiting for network to stabilize...");
    Timer::after(EmbassyDuration::from_secs(2)).await;

    if let Some(config) = stack.config_v4() {
        debug!("HTTP Client: Got IP address: {}", config.address);
    }

    debug!("HTTP Client: Ready, waiting for button press...");

    loop {
        // Wait for button press signal
        debug!("HTTP Client: Waiting for BUTTON_PRESSED signal...");
        button_pressed.wait().await;
        debug!("HTTP Client: Button pressed signal received! Starting firmware download...");

        // Get host IP from environment variable
        let host_ip_str = match HOST_IP {
            Some(ip) => ip,
            None => {
                debug!("HTTP Client: HOST_IP not set, skipping OTA update");
                Timer::after(EmbassyDuration::from_millis(100)).await;
                continue;
            }
        };
        let address = match host_ip_str.parse::<Ipv4Address>() {
            Ok(ipv4) => IpAddress::Ipv4(ipv4),
            Err(_) => {
                debug!("HTTP Client: Invalid HOST_IP format: {}", host_ip_str);
                Timer::after(EmbassyDuration::from_millis(100)).await;
                continue;
            }
        };

        // Attempt firmware download - if successful, break out of loop
        if download_and_flash_firmware(stack, host_ip_str, address, &FLASH_STORAGE)
            .await
            .is_ok()
        {
            break;
        }

        // Small delay before waiting for next button press
        Timer::after(EmbassyDuration::from_millis(100)).await;
    }
}

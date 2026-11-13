# Wrapping Up

Congratulations, you have built a complete `no_std` application for the ESP32-C3. In this chapter we look back at what we built, list what a product would still need, and point to where to go next.

## What We Built

Starting from an empty project, we built a data logger that:

- Reads temperature and humidity from the SHTC3 sensor over I2C.
- Starts an access point with a captive portal to receive Wi-Fi credentials at runtime.
- Joins the provisioned network and publishes each measurement to an MQTT broker.
- Periodically checks a server for a new firmware version, installs it in a spare OTA slot, and reboots into it.
- Shows its current state on the RGB LED.

The final application is available in `project/part5/`.

## What We Learned

Along the way we covered:

- How a `no_std`, `no_main` application is structured, and how `esp_hal::init` hands out peripherals as owned singletons. See [Project Setup](./project-setup.md) and [Reading Sensor Data](./reading-sensor-data.md).
- How to use async drivers, using I2C and the SHTC3 sensor as an example. See [Reading Sensor Data](./reading-sensor-data.md).
- How `esp-rtos` runs the Embassy executor, and how to split an application into tasks with `static` resources. See [Wi-Fi Connectivity](./wifi-connectivity.md#why-we-use-tasks).
- When a heap is needed, how to provide one with `esp-alloc`, and when fixed-capacity `heapless` types avoid allocation altogether. See [Wi-Fi Connectivity](./wifi-connectivity.md#why-we-need-esp-alloc).
- How to bring up Wi-Fi with `esp-radio` and an IP network with `embassy-net`. See [Wi-Fi Connectivity](./wifi-connectivity.md).
- How to publish data with `rust-mqtt`. See [Publishing Data to an MQTT Broker](./publishing-data.md).
- How to run an access point, DHCP, DNS, and HTTP servers on the device with the `edge-*` crates, and how to pass data between tasks with `embassy-sync`. See [Wi-Fi Provisioning](./wifi-provisioning.md).
- How partition tables, `otadata`, and the ESP-IDF bootloader enable safe firmware updates with `esp-storage` and `esp-bootloader-esp-idf`. See [Over-the-Air Updates](./ota-updates.md).

## From Demo to Product

To keep the chapters focused, the application leaves out things a product needs. Each chapter lists its own limitations. Together, they are:

- **Persistent configuration.** The Wi-Fi credentials only live in RAM, so every reset and every update starts the provisioning portal again. A product stores them in flash and only starts the portal when no valid credentials are stored.
- **Security.** The provisioning access point is open and the credentials travel over plain HTTP. MQTT and the OTA download are not encrypted, and any host answering on `HOST_IP` can install its own firmware. A product protects the access point, uses TLS, and only boots signed images with [Secure Boot][idf-secure-boot].
- **Update safety.** The OTA client installs any version that differs from the running one, and the new image never confirms that it works. A product only installs newer versions and uses [app rollback][idf-app-rollback] to return to the previous image if the update does not work.
- **Robustness.** Several initialization steps panic on error, and the tasks retry at fixed intervals. A product handles errors explicitly, uses backoff when it reconnects, and enables a watchdog to recover from a stuck task.
- **Power consumption.** The radio and CPU stay on all the time. A battery-powered logger would take a measurement, publish it, and spend the rest of the time in deep sleep.
- **Observability.** Log messages are formatted on the device as text. [`defmt`][defmt] moves formatting to the host, which reduces code size and logging overhead.
- **Testing.** Nothing in the project is tested automatically. Pure logic, such as parsing the image version, can be unit-tested on the host, and code that depends on hardware can run on the device with [`embedded-test`][embedded-test].

## Exercises

If you want to keep going, here are a few ideas that build on the final application:

- ✅ Make the MQTT topic prefix configurable at build time, so that several devices can publish to the same broker.
- ✅ Store the Wi-Fi credentials in flash, so that the device reconnects after a reset or an update without starting the provisioning portal.
- ✅ Subscribe to an MQTT topic, such as `command/interval`, and use the received value to change how often the device publishes measurements.
- ✅ Use the boot button to clear the stored credentials and start the provisioning portal again.
- ✅ Port the application to another chip, such as the ESP32-C6. You will need to connect an external SHTC3, and change the `esp32c3` features and the build target.

## Where to Go Next

- The [esp-hal documentation][esp-hal-docs] describes every driver, and the [esp-hal examples][esp-hal-examples] show how to use the peripherals we did not cover.
- [`esp-generate`][esp-generate] creates new projects with the features you select, as we did in [Project Setup](./project-setup.md).
- The [esp-rs community] on Matrix is the place for questions, and the [`esp-rs/no_std-training`] repository is the place to report issues or suggest improvements to this training.

[idf-secure-boot]: https://docs.espressif.com/projects/esp-idf/en/latest/esp32c3/security/secure-boot-v2.html
[idf-app-rollback]: https://docs.espressif.com/projects/esp-idf/en/latest/esp32c3/api-reference/system/ota.html#app-rollback
[defmt]: https://defmt.ferrous-systems.com/
[embedded-test]: https://github.com/probe-rs/embedded-test
[esp-hal-docs]: https://docs.espressif.com/projects/rust/
[esp-hal-examples]: https://github.com/esp-rs/esp-hal/tree/main/examples
[esp-generate]: https://github.com/esp-rs/esp-generate/
[esp-rs community]: https://matrix.to/#/#esp-rs:matrix.org
[`esp-rs/no_std-training`]: https://github.com/esp-rs/no_std-training

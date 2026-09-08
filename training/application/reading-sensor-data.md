# Reading Sensor Data

In this chapter we will initialize the board, configure the I2C bus, and read temperature and humidity from the SHTC3 sensor on the ESP32-C3-DevKit-RUST-2. Each successful measurement is printed using the [`log`] crate, while sensor errors are reported with error-level log messages.

The completed code for this chapter is available in `project/part1/`.

## Initialization

The beginning of the application prepares logging, configures the chip, and starts the runtime support used by the rest of the application. We will look at each step separately.

### Panic Handler

In a `no_std` application, a [panic handler][panic-handler] must be provided. The minimal application in the previous chapter used a manual `#[panic_handler]`, but in this chapter we use `esp-backtrace` instead:

```rust,ignore
{{#include ../../project/part1/src/main.rs:backtrace_import}}
```

The `panic-handler` feature of `esp-backtrace` provides the panic handler for us, and also gives useful backtrace support when something goes wrong.

### Logger

First, we initialize the logger:

```rust,ignore
{{#shiftinclude auto:../../project/part1/src/main.rs:logger_init}}
```

This lets us use the standard [`log`] macros. In this chapter we use `info!` to print successful temperature and humidity readings, and `error!` to report failures while starting or reading a measurement.

`init_logger_from_env()` configures logging from the `ESP_LOG` environment variable. The value is similar to the `RUST_LOG` filtering used by [`env_logger`][env-logger] and can be set when building or running the application:

```shell
ESP_LOG=info cargo run --release
```

Common global levels are `error`, `warn`, `info`, `debug`, `trace`, and `off`. For example, use `ESP_LOG=warn` to show only warnings and errors, or `ESP_LOG=debug` to include debug messages as well.

You can also set the default log level in `.cargo/config.toml`:

```toml
{{#include ../../project/part1/.cargo/config.toml:log_env}}
```

With this configuration, `cargo run --release` will use `ESP_LOG=info` unless you override it in the shell.

You can also filter individual modules:

```shell
ESP_LOG=warn,no_std_training=info cargo run --release
```

This enables `info` logs for this application while keeping other modules at `warn`.

### `esp_hal::init`

[`esp_hal::Config`][esp-hal-config] describes the system configuration that should be applied during HAL initialization. It is a non-exhaustive type, so we create it with `Config::default()` and then adjust the parts we care about. In this application, we use `with_cpu_clock(CpuClock::max())` to request the maximum CPU clock:

```rust,ignore
{{#shiftinclude auto:../../project/part1/src/main.rs:esp_hal_init}}
```

[`esp_hal::init`][esp-hal-init] applies that configuration. It sets up the CPU clock and watchdog, then returns the peripherals and clocks needed by the HAL.

The important value for the next steps is `peripherals`. It contains one Rust value for each hardware block and pin. When we later create drivers, we pass those values into the driver constructors. That transfer of ownership is how `esp-hal` prevents accidental double-use of hardware: once `peripherals.I2C0` has been moved into the I2C driver below, no other driver can use the same I2C peripheral.

### `esp_rtos::start`

The application uses async Rust through [Embassy]. [`esp_rtos::start`][esp-rtos-start] starts the scheduler used by `esp-rtos` and Embassy. It needs a hardware timer and a software interrupt:

```rust,ignore
{{#shiftinclude auto:../../project/part1/src/main.rs:esp_rtos_start}}
```

The timer is used to drive time-based async operations, such as `Timer::after(...)`. The software interrupt is used by the runtime to wake tasks.

### Drivers

Next we create an I2C bus driver for the sensor:

```rust,ignore
{{#shiftinclude auto:../../project/part1/src/main.rs:i2c_driver}}
```

[`I2c::new`][i2c-new] takes ownership of the I2C peripheral and a configuration. The ESP32-C3-DevKit-RUST-2 connects the SHTC3 sensor to GPIO10 for SDA and GPIO8 for SCL, so we attach those pins with the `with_*` methods.

Many `esp-hal` drivers follow this pattern:

1. Create the driver with the hardware peripheral and a configuration.
2. Attach pins or optional settings with `with_*` methods, such as [`with_sda`][i2c-with-sda] and [`with_scl`][i2c-with-scl].
3. Choose the execution mode.

The final call, [`into_async`][i2c-into-async], converts the I2C driver into its async mode. This lets the sensor driver use async I2C operations and lets our task await transfers instead of busy-waiting.

This blocking/async split is common across `esp-hal` drivers. Driver types are parameterized by a mode, and async-capable drivers use the [`Async`][esp-hal-async] mode marker after conversion. The exact setup method varies by peripheral, but the pattern is the same: create the driver, configure it, then convert it to async mode when it will be used from async code.

Finally, `shtc3(i2c)` wraps the I2C bus in the sensor-specific driver from the [`shtcx2`] crate.

## Reading the Sensor

The SHTC3 is a temperature and humidity sensor controlled over I2C. The [SHTC3 datasheet][shtc3-datasheet] describes the command set, measurement modes, timing requirements, and conversion formulas. We do not need to implement those details manually; the `shtcx2` crate sends the commands and converts the raw values for us.

The application still needs to follow the sensor's measurement sequence. A reading is not returned by a single I2C transaction; instead, we:

1. Ask the sensor to start a measurement.
2. Wait until the measurement is complete.
3. Read and convert the result.

In code this looks like:

```rust,ignore
{{#shiftinclude auto:../../project/part1/src/main.rs:read_measurement}}
```

We use `PowerMode::NormalMode`, which is the higher-precision measurement mode. After `start_measurement(...)`, the SHTC3 is busy converting the temperature and humidity internally. If we try to read immediately, the sensor may not acknowledge the I2C read yet.

There are a few ways a sensor can tell us that a measurement is ready. Some sensors expose a data-ready interrupt pin; the SHTC3 used on this board is a 4-pin I2C device (`VDD`, `SCL`, `SDA`, and `VSS`), so there is no interrupt pin for this example to await. The SHTC3 also supports clock stretching, but the measurement commands used by this driver do not enable it. We therefore wait for the maximum measurement duration reported by `max_measurement_duration(...)` before trying to read the result.

This wait is separate from the async I2C transfers. Async I2C lets the task yield while I2C commands are sent or received; `Timer::after(...).await` lets the task yield while the sensor performs the measurement. In both cases, the executor can run other work instead of busy-waiting.

The returned measurement exposes typed temperature and humidity values:

```rust,ignore
{{#shiftinclude auto:../../project/part1/src/main.rs:log_measurement}}
```

The loop repeats this sequence once per second, logging the latest reading.

## Running the Application

Flash and run the application from `project/part1/`:

```shell
cargo run --release
```

This command does two things. First, Cargo builds the application in release mode for the configured ESP32-C3 target. Then Cargo runs the target-specific runner from `.cargo/config.toml`:

```toml
{{#include ../../project/part1/.cargo/config.toml:runner}}
```

The runner invokes [`espflash`] to flash the binary to the board. The `--monitor` flag keeps the serial monitor open after flashing, so the log output from the device is printed in the same terminal.

Once the application starts, it initializes the board, configures the async I2C driver, and repeatedly reads the SHTC3 sensor. Each loop prints the latest temperature and relative humidity measurement.

You should see output similar to:

```text
INFO -   24.31 °C | 42.18 %RH
INFO -   24.30 °C | 42.21 %RH
INFO -   24.32 °C | 42.20 %RH
```

At this point the application can initialize the board, configure an async I2C driver, and read sensor data. In the next chapter we will connect the device to Wi-Fi so that these measurements can be sent off the board.

[Embassy]: https://embassy.dev/
[panic-handler]: https://doc.rust-lang.org/nomicon/panic-handler.html
[`log`]: https://docs.rs/log/latest/log/
[env-logger]: https://docs.rs/env_logger/latest/env_logger/
[`espflash`]: https://github.com/esp-rs/espflash/
<!-- TODO: Use /latest/ when new docs are redeployed -->
[esp-hal-init]: https://docs.espressif.com/projects/rust/esp-hal/1.1.1/esp32c3/esp_hal/fn.init.html
[esp-hal-config]: https://docs.espressif.com/projects/rust/esp-hal/1.1.1/esp32c3/esp_hal/struct.Config.html
[esp-hal-async]: https://docs.espressif.com/projects/rust/esp-hal/1.1.1/esp32c3/esp_hal/struct.Async.html
[esp-rtos-start]: https://docs.espressif.com/projects/rust/esp-rtos/0.3.0/esp32c3/esp_rtos/fn.start.html
[i2c-new]: https://docs.espressif.com/projects/rust/esp-hal/1.1.1/esp32c3/esp_hal/i2c/master/struct.I2c.html#method.new
[i2c-with-sda]: https://docs.espressif.com/projects/rust/esp-hal/1.1.1/esp32c3/esp_hal/i2c/master/struct.I2c.html#method.with_sda
[i2c-with-scl]: https://docs.espressif.com/projects/rust/esp-hal/1.1.1/esp32c3/esp_hal/i2c/master/struct.I2c.html#method.with_scl
[i2c-into-async]: https://docs.espressif.com/projects/rust/esp-hal/1.1.1/esp32c3/esp_hal/i2c/master/struct.I2c.html#method.into_async
[`shtcx2`]: https://github.com/SergioGasquez/shtcx2
[shtc3-datasheet]: https://sensirion.com/media/documents/643F9C8E/63A5A436/Datasheet_SHTC3.pdf

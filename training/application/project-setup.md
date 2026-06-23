# Project Setup

## Generating a Project

We will begin by generating a bare-bones project as a starting point. We provide a tool, [`esp-generate`][esp-generate], specifically for such cases. It can be installed by running:

```shell
cargo install --locked esp-generate
```

With `esp-generate` installed, we can now generate an empty project:

```shell
esp-generate --headless --chip=esp32c3 -o unstable-hal -o embassy no_std-training
```

Since we are targeting the [ESP32-C3-DevKit-RUST-2] development kit, we select the ESP32-C3 for our chip. We will take advantage of asynchronous support via [Embassy], so both the `embassy` and `unstable` options are required.

You should now see `no_std-training/` in the executing directory.

[esp-generate]: https://github.com/esp-rs/esp-generate/
[esp32-c3-devkit-rust-2]: https://github.com/esp-rs/esp-rust-board
[embassy]: https://github.com/embassy-rs/embassy

## Project Structure

Your newly generated project should look as follows:

```text
no_std-training/
├─ .cargo/
│  └─ config.toml
├─ .clippy.toml
├─ build.rs
├─ Cargo.toml
├─ rust-toolchain.toml
└─ src/
   ├─ bin/
   │   └─ main.rs
   └─ lib.rs
```

Let's quickly walk through the purpose of each of these files:

- [`.cargo/config.toml`] contains configuration for Cargo.
  - The [build target] is provided, in this case `riscv32imc-unknown-none-elf`.
  - Rust compiler flags are provided to force frame pointers. This is required in order for backtraces to work correctly. This is not required if not using the `esp-backtrace` crate.
  - Additionally, we configure [espflash] to be used as the Cargo runner for the project.
- [`.clippy.toml`] contains configuration for [Clippy], a linter for Rust code.
- [`build.rs`] configures the Rust compiler to include the required linker scripts.
  - This could also be configured in `.cargo/config.toml`, but this approach provides better diagnostics in the case of build errors.
- `Cargo.toml` is the [Cargo manifest], which contains project metadata, dependencies, and more.
- [`rust-toolchain.toml`] specifies which Rust toolchain to use, in addition to specifying the required components and the build target.
- `src/lib.rs` contains any library code for our application. This is currently empty, but will be populated in later chapters.
- `src/bin/main.rs` is the entry point of our application. This is where most of our logic will live.

[`.cargo/config.toml`]: https://doc.rust-lang.org/cargo/reference/config.html
[build target]: https://doc.rust-lang.org/beta/rustc/targets/index.html
[espflash]: https://github.com/esp-rs/espflash/
[`.clippy.toml`]: https://doc.rust-lang.org/clippy/configuration.html
[clippy]: https://doc.rust-lang.org/stable/clippy/index.html
[`build.rs`]: https://doc.rust-lang.org/cargo/reference/build-scripts.html
[Cargo manifest]: https://doc.rust-lang.org/cargo/reference/manifest.html
[`rust-toolchain.toml`]: https://rust-lang.github.io/rustup/overrides.html#the-toolchain-file

## Dependencies

The generated `Cargo.toml` comes pre-populated with a number of dependencies, whose purposes are briefly described below:

- [`esp-hal`][esp-hal] - The Hardware Abstraction Layer. Provides access to on-chip peripherals using high-level APIs.
- [`esp-rtos`][esp-rtos] - Provides the runtime support necessary to run asynchronous code on top of `esp-hal`.
- [`esp-bootloader-esp-idf`][esp-bootloader-esp-idf] - Support package for using the ESP-IDF bootloader.
- [`embassy-executor`][embassy-executor] - Asynchronous executor for embedded devices. Allows us to run one or more asynchronous tasks.
- [`embassy-time`][embassy-time] - Asynchronous timekeeping, delays, and timeouts.
- [`critical-section`][critical-section] - Critical sections for embedded devices.
- [`static_cell`][static_cell] - Statically allocated, initialized at runtime cell.

Additional dependencies will be included in later chapters as well.

[esp-hal]: https://docs.espressif.com/projects/rust/esp-hal/1.0.0/esp32c3/esp_hal/index.html
[esp-rtos]: https://docs.espressif.com/projects/rust/esp-rtos/0.2.0/esp32c3/esp_rtos/index.html
[esp-bootloader-esp-idf]: https://docs.espressif.com/projects/rust/esp-bootloader-esp-idf/0.4.0/esp32c3/esp_bootloader_esp_idf/index.html
[embassy-executor]: https://docs.rs/embassy-executor/latest/embassy_executor/
[embassy-time]: https://docs.rs/embassy-time/latest/embassy_time/
[critical-section]: https://docs.rs/critical-section/latest/critical_section/
[static_cell]: https://docs.rs/static_cell/latest/static_cell/

## A Minimal Application

The `src/bin/main.rs` contains some boilerplate for a minimal application:

```rust
#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates a default app-descriptor required by the ESP-IDF bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // TODO: Spawn some tasks
    let _ = spawner;

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
```

This should mostly be straightforward, however, there are a few interesting parts which we will explain briefly.

### Application Description

The ESP-IDF bootloader expects an Application Description structure to be located at a specific address. Failure to include this description will result in confusing bootloader errors.

The Application Description is generated via the following macro invocation:

```rust
esp_bootloader_esp_idf::esp_app_desc!();
```

No further action is required. See the [ESP-IDF Documentation] for further details.

[ESP-IDF Documentation]: https://docs.espressif.com/projects/esp-idf/en/v5.5.2/esp32c3/api-reference/system/app_image_format.html#application-description

### `#[esp_rtos::main]` Macro

The [`#[esp_rtos::main]`][main] macro is used to define the application's entry point, usually the `main` function:

```rust
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
  // ...
}
```

This macro creates a new instance of an Embassy `Executor` and spawns the `main` function as an asynchronous task.

[main]: https://docs.espressif.com/projects/rust/esp-rtos/0.2.0/esp32c3/esp_rtos/attr.main.html

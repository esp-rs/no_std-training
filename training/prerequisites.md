# Prerequisites

This chapter contains information on the knowledge, hardware, and software required in order to complete this training.

## Rust

Some level of prior experience with Rust, and programming in general, is expected. This training does not aim to teach programming or Rust in general, as many high-quality resources are already available for these topics.

If you are completely new to the language, we recommend you first read [The Rust Book]. You may also wish to first read [The Rust on ESP Book], though this is not required reading.

[the rust book]: https://doc.rust-lang.org/book/
[the rust on esp book]: https://esp-rs.github.io/book/

### Toolchain

The latest stable Rust toolchain is required, and you will need to install the `riscv32imc-unknown-none-elf` target.

Rust should be installed using the official installer, found at <https://rustup.rs/>. The required toolchain and target can be installed by running:

```shell
rustup toolchain install stable --target riscv32imc-unknown-none-elf
```

## Additional Software

We will also use [espflash] for loading the firmware to the device. This can be installed using Cargo:

```shell
cargo install espflash --locked
```

[espflash]: https://github.com/esp-rs/espflash

## Hardware

We will use the [ESP32-C3-DevKit-RUST-1] development kit for this training. While it's possible to use other development kits, this board contains sensors and other devices which will be used by our application, so it is strongly recommended to use this particular kit for the best results.

The source files for this development kit are available in the [esp-rs/esp-rust-board] repository. This development kit is available on AliExpress and Mouser, see [Where to buy].

[esp32-c3-devkit-rust-1]: https://github.com/esp-rs/esp-rust-board
[esp-rs/esp-rust-board]: https://github.com/esp-rs/esp-rust-board
[where to buy]: https://github.com/esp-rs/esp-rust-board#where-to-buy

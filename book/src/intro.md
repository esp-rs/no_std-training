# Writing no_std applications

The goal of this book is to provide a getting-started guide on using the Rust programming language with Espressif SoCs and modules using [esp-hal].

> Note that there are several examples covering the use of specific peripherals under the examples folder of every SoC `esp-hal`. E.g. [`esp32c3-hal/examples`]

Examples shown here usually apply to ESP32-C3 using the [ESP32-C3-DevKit-RUST-1] board.

You can use any [SoC supported by `no_std`] but smaller code changes and configuration changes might be needed.

Also, this section of the book will only cover working locally. I.e. we will be using our host machine to develop, not [devcontainers], so make sure you have the [ecosystem properly installed].

[esp-hal]: https://github.com/esp-rs/esp-hal
[ESP32-C3-DevKit-RUST-1]: https://github.com/esp-rs/esp-rust-board
[`esp32c3-hal/examples`]: https://github.com/esp-rs/esp-hal/tree/main/esp32c3-hal/examples
[devcontainers]: https://esp-rs.github.io/book/writing-your-own-application/generate-project-from-template.html
[ecosystem properly installed]: https://esp-rs.github.io/book/installation/index.html
[SoC supported by `no_std`]: https://esp-rs.github.io/book/overview/bare-metal.html#current-support

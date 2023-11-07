<p style="text-align:center;"><img src="./assets/esp-logo-black.svg" width="50%"></p>

# Introduction

## Content of this material

The goal of this book is to provide a getting-started guide on using the Rust programming language with Espressif SoCs and modules using `no_std` (bare metal) approach. To better understand this approach, see [Developing on Bare Metal (no_std)] chapter of [The Rust on ESP Book].

The introductory trail will introduce you to the basics of embedded development and how to make the embedded board interact with the outside world by reacting to a button press, and lighting an LED.

> Note that there are several examples covering the use of specific peripherals under the examples folder of every SoC `esp-hal`. E.g. [`esp32c3-hal/examples`]

## The board

Examples shown here usually apply to ESP32-C3 using the [ESP32-C3-DevKit-RUST-1] board.

You can use any [SoC supported by `no_std`] but smaller code and configuration changes might be needed.

## Rust knowledge

- Basic Rust like [The Rust Book](https://doc.rust-lang.org/book/) Chapters 1 - 6, Chapter 4 Ownership, does not need to be fully understood.
- [The Rust on ESP Book](https://esp-rs.github.io/book/) is not required, but it is highly recommended, as it can help you understand the Rust on ESP ecosystem and many of the concepts that will be discussed during the training.


[The Rust on ESP Book]: https://esp-rs.github.io/book/overview/bare-metal.html
[Developing on Bare Metal (no_std)]: https://esp-rs.github.io/book/overview/bare-metal.html
[ESP32-C3-DevKit-RUST-1]: https://github.com/esp-rs/esp-rust-board
[`esp32c3-hal/examples`]: https://github.com/esp-rs/esp-hal/tree/main/esp32c3-hal/examples
[SoC supported by `no_std`]: https://esp-rs.github.io/book/overview/bare-metal.html#current-support

# Hello World

The `hello-world` example is basically a project generated with [esp-generate]. Templates are already covered in [The Rust on ESP Book], see [Generating Projects from Templates] chapter for more details on how to generate a project from the [esp-generate], and [Understanding esp-generate] for detail on what is inside the template project.

Since we already have the code for this example, let's use it to do a consistency check!

✅ Connect the USB-C port of the board to your computer and enter the `hello-world` directory in the workshop repository:

```console
cd intro/hello-world
```

✅ Build, flash, and monitor the project:

```console
$ cargo run
(...)
Finished release [optimized] target(s) in 1.78s
(...)
Chip type:         esp32c3 (revision v0.3)
Crystal frequency: 40MHz
Flash size:        4MB
Features:          WiFi, BLE
MAC address:       60:55:f9:c0:39:7c
App/part. size:    210,608/4,128,768 bytes, 5.10%
[00:00:00] [========================================]      13/13      0x0
[00:00:00] [========================================]       1/1       0x8000
[00:00:01] [========================================]      67/67      0x10000
[00:00:01] [========================================]      67/67      0x10000
[2023-07-07T08:16:32Z INFO ] Flashing has completed!
Commands:
    CTRL+R    Reset chip
    CTRL+C    Exit
(...)
(...)
Hello world!
```

>🔎 If `cargo run` has been successful, you can exit with `ctrl+C`.

> 🔎 `cargo run` is [configured to use `espflash`](https://github.com/esp-rs/no_std-training/blob/main/intro/hello-world/.cargo/config.toml#L2) as [custom runner](https://doc.rust-lang.org/cargo/reference/config.html#target). The same output can be achieved via:
> - Using `cargo-espflash`: `cargo espflash flash --release --monitor`
> - Building your project and flashing it with `espflash`: `cargo build --release && espflash target/riscv32imc-unknown-none-elf/release/hello_world`
> This modification is applied to all the projects in the training for convenience.

> 💡 By default espflash will use a baud-rate of 115200 which is quite conservative. An easy way to increase the baud-rate is setting the environment variable `ESPFLASH_BAUD` to e.g. 921600

[esp-generate]: https://github.com/esp-rs/esp-generate
[The Rust on ESP Book]: https://esp-rs.github.io/book/
[Generating Projects from Templates]: https://esp-rs.github.io/book/writing-your-own-application/generate-project/index.html
[Understanding esp-generate]: https://esp-rs.github.io/book/writing-your-own-application/generate-project/esp-generate.html

## Simulation

This project is available for simulation through two methods:
- [Wokwi project](https://wokwi.com/projects/382725628217620481?build-cache=disable)
- Wokwi VS Code extension:
  1. Press F1, select `Wokwi: Select Config File`, and choose `intro/hello-world/wokwi.toml`.
  2. Build your project.
  3. Press F1 again and select `Wokwi: Start Simulator`.

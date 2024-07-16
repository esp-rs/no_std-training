# `defmt`
In this chapter, we will cover [`defmt`][defmt], a highly efficient logging framework, and how to use it in the `no_std` environment.


[defmt]: https://defmt.ferrous-systems.com/

## `defmt` Ecosystem

[`esp-println`][esp-println], [`esp-backtrace`][esp-backtrace] and [`espflash`/`cargo-espflash`][espflash] provide mechanisms to use `defmt`:
- `espflash` has support for different [logging formats][espflash-logformat], one of them being `defmt`.
  - `espflash` requires framming bytes as when using `defmt` it also needs to print non-`defmt` messages, like the bootloader prints.
    - It's important to note that other `defmt`-enabled tools like `probe-rs` won't be able to parse these messages due to the extra framing bytes.
  - Uses [rzcobs encoding](https://github.com/Dirbaio/rzcobs)
- `esp-println` has a `defmt-espflash` feature, which adds framming bytes so `espflash` knows that is a `defmt` message.
- `esp-backtrace` has a `defmt` feature that uses `defmt` logging to print panic and exception handler messages.


[esp-println]: https://github.com/esp-rs/esp-hal/tree/main/esp-println
[esp-backtrace]: https://github.com/esp-rs/esp-hal/tree/main/esp-backtrace
[espflash]: https://github.com/esp-rs/espflash
[espflash-logformat]: https://github.com/esp-rs/espflash/blob/main/espflash/README.md#logging-format

# Setup

✅ Go to `intro/defmt` directory.

✅ Open the prepared project skeleton in `intro/defmt`.

`intro/defmt/examples/defmt.rs` contains the solution. You can run it with the following command:

```shell
cargo run --release --example defmt
```

## Exercise

✅ Make sure the `defmt-espflash` feature of `esp-println`  is enabled.

✅ Make sure the `defmt` feature of `esp-backtrace` is enabled.

✅ Update the [linking process](https://defmt.ferrous-systems.com/setup#linker-script) in the `.cargo/config.toml`.

✅ Make sure, the [`defmt` crate](https://crates.io/crates/defmt) is added to the dependencies.

✅ Make sure you are building `esp_println` and `esp_backtrace`
```rust,ignore
{{#include ../../intro/defmt/examples/defmt.rs:println_include}}
```

✅ Use the `defmt::println!` or any of the logging [`defmt` macros](https://docs.rs/defmt/latest/defmt/#macros) to print a message.
- If you want to use any of the logging macros like `info`, `debug`
  - Enable the `log` feature of `esp-println`
  - When building the app, [set `DEFMT_LOG`](https://defmt.ferrous-systems.com/filtering.html?highlight=DEFMT_LOG#defmt_log) level.

✅ Add a `panic!` macro to trigger a panic with a `defmt` message.

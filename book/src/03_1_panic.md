# Panic!

When something goes terribly wrong in Rust there might occur a [panic].


## Setup

✅ Go to `intro/panic` directory.

✅ Open the prepared project skeleton in `intro/panic`.

✅ Open the docs for this project with the following command:

```
cargo doc --open
```

`intro/panic/examples/panic.rs` contains the solution. You can run it with the following command:

```shell
cargo run --example panic
```

## Exercise

✅ In `main.rs` add a `panic!` somewhere, e.g. after our `println`

✅ Run the code

```shell
cargo run
```

We see where the panic occurred, and we even see a backtrace!

While in this example things are obvious, this will come in handy in more complex code.

✅ Now try running the code compiled with release profile.

```shell
cargo run --release
```

Now things are less pretty:
```text
!! A panic occured in 'examples\panic.rs', at line 15, column 5:
This is a panic

Backtrace:

0x42000100
0x42000100 - _start_rust
    at ??:??
```

We still see where the panic occurred, but the backtrace is less helpful now.

That is because the compiler omitted debug information and optimized the code,
you might have noticed the difference in the size of the flashed binary.

Generally you want to use `release` always. To get a more helpful backtrace when using the `release` profile you can add this to your `.cargo/config.toml`

```toml
[profile.release]
debug = true
```

This will include debug information in the ELF file - but that won't get flashed to the target so it's something you can and should always use.

If you are reusing this project for other exercises, be sure to remove the line causing the explicit panic.

[panic]: https://doc.rust-lang.org/book/ch09-01-unrecoverable-errors-with-panic.html

## Simulation

This project is available for simulation through two methods:
- Wokwi projects:
  - Exercise: Currently not available
  - [Solution](https://wokwi.com/projects/382726300037178369?build-cache=disable)
- Wokwi files are also present in the project folder to simulate it with Wokwi VS Code extension:
   1. Press F1, select `Wokwi: Select Config File` and choose `intro/panic/wokwi.toml`
      - Edit the `wokwi.toml` file to select between exercise and solution simulation
   2. Build you project
   3. Press F1 again and select `Wokwi: Start Simulator`

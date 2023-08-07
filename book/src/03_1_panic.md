# Panic!

When something goes terribly wrong in Rust there might occur a [panic].

intro/panic/examples/panic.rs contains the solution. You can run it with the following command:

```shell
cargo run --example panic
```


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
!! A panic occured in 'src/main.rs', at line 24, column 5

PanicInfo {
    payload: Any { .. },
    message: Some(
        This is a panic,
    ),
    location: Location {
        file: "src/main.rs",
        line: 24,
        col: 5,
    },
    can_unwind: true,
}

Backtrace:

0x4200010e
0x4200010e - _start_rust
    at ??:??
```

We still see where the panic occurred, but the backtrace is less helpful now.

That is because the compiler omitted debug information and optimized the code,
you might have noticed the difference in the size of the flashed binary.

If you are reusing this project for other exercises, be sure to remove the line causing the explicit panic.

[panic]: https://doc.rust-lang.org/book/ch09-01-unrecoverable-errors-with-panic.html

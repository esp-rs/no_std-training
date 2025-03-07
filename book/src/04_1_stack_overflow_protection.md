# Stack Overflow Detection

Rust is well known for its memory safety. Whenever possible the compiler enforces memory safety at compile.

However, the situation is different in regards to the stack memory. It's impossible to check this at compile time and even at runtime this can be difficult.

The stack is usually placed at the top of the available memory and grows from top (high addresses) to bottom (low addresses).

On desktop operating systems there are measures to prevent overflowing the stack. Also, an RTOS might include mechanisms to check for stack overflows.

In bare-metal however there is no common way to implement stack protection.

On some platforms it's done by moving the stack to the start of the RAM so that when the stack grows above its bounding an access fault will occur.
We cannot do that because on our chips there is the flash/ext-mem cache at the start of RAM which we definitely shouldn't touch.

> 🔎 On ESP32-C6/ESP32-H2 cache is not located in the start of RAM which means we can move the stack there.
> esp-hal offers the feature `flip-link` which will do that and you get stack-overflow protection "for free".

> 🔎 esp-hal also supports [stack smashing protection](https://doc.rust-lang.org/rustc/exploit-mitigations.html#stack-smashing-protection) for all targets which in our case can also double as a simple stack overflow detector. While the overhead is very small, there is some run-time cost involved.
>
> To enable it you need a nightly compiler and add `"-Z", "stack-protector=all",` to `rustflags` in `.cargo/config.toml`

Some of our chips (including ESP32-C3) include the debug-assist peripheral.

This peripheral can monitor the stack-pointer and detect read and/or write access to specified memory areas.

We could just use the stack-pointer monitoring which will work well as long as we don't use `esp-wifi`.

The reason we cannot use that with `esp-wifi` is that it runs multiple tasks by quickly switching between them which includes switching stacks. In that case the stack bounds check will trigger as soon as we switch the running task for the first time.

What we can do however is defining a protected area at the bottom of the stack and detect read and write access to it. As soon as the stack grows into this area, we will detect this.

It is important to define this area larger (ideally twice) than the largest stack allocation we expect. Otherwise, it's possible that code will start writing to memory below the stack - possibly overwriting sensitive static data or even code residing in RAM before we can detect access to the monitored memory area.

For X86 LLVM supports _probestack_ which would allow us to use a smaller safe-area. However, this feature currently isn't available for our target platforms.

We can also test for the current stack usage by temporarily increasing the safe area until we see the stack memory protection trigger.

## Setup

✅ Install the `nightly` channel of Rust:
```shell
rustup toolchain install nightly --component rust-src --target riscv32imc-unknown-none-elf
```

✅ Go to `advanced/stack-overflow-detection` directory.

✅ Open the prepared project skeleton in `advanced/stack-overflow-detection`.

✅ Open the docs for this project with the following command:

```
cargo doc --open
```

✅ Run the code

```
cargo run
```

You will see the application crash with an `Illegal instruction` exception. This is because the recursive function is placed in RAM.
If you change it to run from flash you won't see a crash but the application will just freeze after printing a weird counter number.

In this case it's easy to guess the cause of this behavior however in a real world application you probably won't know what exactly happened.

`advanced/stack-overflow-detection/examples/stack-overflow-detection.rs` contains the solution. You can run it with the following command:

```shell
cargo run --release --example stack-overflow-detection
```

## Exercise

✅ Create a function which will set up the safe memory area and enables the appropriate interrupt

The function will take the `DebugAssist` peripheral driver and the size of the safe-area.
It should move the `DebugAssist` into a static variable.

The resulting function should look like this
```rust,ignore
{{#include ../../advanced/stack-overflow-detection/examples/stack-overflow-detection.rs:debug_assists}}
```

There is quite a lot going on here but most of this is setting up the interrupt.
You should recognize most of this from the interrupt exercise in the previous chapter.

The most interesting part is probably `da.enable_region0_monitor(stack_low, stack_low + safe_area_size, true, true)`.
This actually configures the region to monitor as well as setting it up to trigger on reads and writes to that region.

Another interesting part here is how we can get the top and bottom address of the stack from symbols created by the linker script.

✅ Create the interrupt handler

As you probably remember from the introduction to interrupts we can define the interrupt handler by using the `#[interrupt]` attribute macro.
The name of the function needs to match the name of the interrupt.

```rust,ignore
{{#include ../../advanced/stack-overflow-detection/examples/stack-overflow-detection.rs:interrupt}}
...
```

Next, we need to get access to the debug assist peripheral driver which we stored in the static variable.

We need it to get the address where the access to the monitored memory region happened.
Printing this address will enable `espflash` to print the name of the function. Similar to how stack traces are printed.

We can also clear the pending interrupt and disable region monitoring here. It's not strictly needed since we won't return from the interrupt handler.

It is unfortunately not possible to generate a stack trace here since the stack is not in a correct state and we don't know the stack frame from which we can start generating the backtrace.

The whole function should look like this
```rust,ignore
{{#include ../../advanced/stack-overflow-detection/examples/stack-overflow-detection.rs:handler}}
```

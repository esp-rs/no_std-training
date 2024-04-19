# Detect a button press with interrupt

Now, instead of polling the button pin, we will use interrupts. [Interrupts] offer a mechanism by which the processor handles asynchronous events and fatal errors.


## Setup

✅ Go to `intro/button-interrupt` directory.

✅ Open the prepared project skeleton in `intro/button-interrupt`.

✅ Open the docs for this project with the following command:

```
cargo doc --open
```

`intro/button-interrupt/examples/button-interrupt.rs` contains the solution. You can run it with the following command:

```shell
cargo run --release --example button-interrupt
```

## Exercise

Inspecting the code, the first thing we notice is the `static BUTTON`. We need it since in the interrupt handler we have to clear the pending interrupt on the button and we somehow need to pass the button from main to the interrupt handler.

Since an interrupt handler can't have arguments we need a static to get the button into the interrupt handler.

We need the `Mutex` to make access to the button safe.

> Please note that this is not the Mutex you might know from `libstd` but it's the Mutex from [`critical-section`] (and that's why we need to add it as a dependency).

✅ We need to set the interrupt handler for the GPIO interrupts.

✅ Let's add a [`critical-section`], using the `with()` method and enable an interrupt for falling edges:

```rust,ignore
{{#include ../../intro/button-interrupt/examples/button-interrupt.rs:critical_section}}
```
In this line we move our button into the `static BUTTON` for the interrupt handler to get hold of it.

The code running inside the `critical_section::with` closure runs within a critical section,
`cs` is a token that you can use to "prove" that to some API.

The interrupt handler is defined via the `#[handler]` macro.
Here, the name of the function must match the interrupt.

[Interrupts]: https://docs.rust-embedded.org/book/start/interrupts.html
[`critical-section`]: https://crates.io/crates/critical-section

## Simulation

This project is available for simulation through two methods:
- Wokwi projects:
  - Exercise: Currently not available
  - [Solution](https://wokwi.com/projects/382723722184136705?build-cache=disable)
- Wokwi files are also present in the project folder to simulate it with Wokwi VS Code extension:
   1. Press F1, select `Wokwi: Select Config File` and choose `intro/button-interrupt/wokwi.toml`
      - Edit the `wokwi.toml` file to select between exercise and solution simulation
   2. Build you project
   3. Press F1 again and select `Wokwi: Start Simulator`

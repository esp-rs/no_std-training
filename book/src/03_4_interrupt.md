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
cargo run --example button-interrupt
```

## Exercise

Inspecting the code, the first thing we notice is the `static BUTTON`. We need it since in the interrupt handler we have to clear the pending interrupt on the button and we somehow need to pass the button from main to the interrupt handler.

Since an interrupt handler can't have arguments we need a static to get the button into the interrupt handler.

We need the `Mutex` to make access to the button safe.

> Please note that this is not the Mutex you might know from `libstd` but it's the Mutex from [`critical-section`] (and that's why we need to add it as a dependency).

✅ We need to call [`listen`][listen] on the button pin to configure the peripheral to raise interrupts. We can raise interrupts for [different events][events] - here we want to raise the interrupt on the falling edge.

✅ Let's add a [`critical-section`], using the `with()` method and enable an interrupt:

```rust,ignore
{{#include ../../intro/button-interrupt/examples/button-interrupt.rs:critical_section}}
```
In this line we move our button into the `static BUTTON` for the interrupt handler to get hold of it.

The code running inside the `critical_section::with` closure runs within a critical section,
`cs` is a token that you can use to "prove" that to some API.

✅ Enable the interrupt:

```rust,ignore
{{#include ../../intro/button-interrupt/examples/button-interrupt.rs:interrupt}}
```

First parameter here is the kind of interrupt we want. There are several [possible interrupts].
The second parameter, chooses the priority, in our case, we choosed `Priority3`. Priority dictates which interrupts are runned first in case of several interrupts being triggered at the same time.

✅ Enable interrupts: This can be achived by: `riscv::interrupt::enable`, but this is an unsafe
function, hence it needs to be run inside an `unsafe` block.

The interrupt handler is defined via the `#[interrupt]` macro.
Here, the name of the function must match the interrupt.

[listen]: https://docs.rs/esp32c3-hal/latest/esp32c3_hal/prelude/trait._esp_hal_gpio_Pin.html#method.listen
[Interrupts]: https://docs.rust-embedded.org/book/start/interrupts.html
[`critical-section`]: https://crates.io/crates/critical-section
[possible interrupts]: https://docs.rs/esp32c3/0.5.1/esp32c3/enum.Interrupt.html
[events]: https://docs.rs/esp32c3-hal/latest/esp32c3_hal/gpio/enum.Event.html#variants

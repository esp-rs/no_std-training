# Blinky

Let's see how to create the iconic _Blinky_.


## Setup

✅ Go to `intro/blinky` directory.

✅ Open the prepared project skeleton in `intro/blinky`.

✅ Open the docs for this project with the following command:

```
cargo doc --open
```

`intro/blinky/examples/blinky.rs` contains the solution. You can run it with the following command:

```shell
cargo run --example blinky
```

## Exercise

On [ESP32-C3-DevKit-RUST-1] there is a regular [LED connected to GPIO 7]. If you use another board consult the data-sheet.

> Note that most of the development boards from Espressif today use an addressable LED which works differently and is beyond the scope of this book. In that case, you can also connect a regular LED to some of the free pins (and don't forget to add a resistor).

✅ Initiate the IO peripheral, and create a `led` variable from GPIO connected to the LED, using the
[`into_push_pull_output` function][into-push-pull-output].

Here we see that we can drive the pin `high`, `low`, or `toggle` it.

We also see that the HAL offers a way to delay execution.

✅ Initialize a Delay instance.

✅ Using the [`toogle()`][toogle] and [`delay_ms()`][delay-ms] methods, make the LED blink every 500 ms.


[ESP32-C3-DevKit-RUST-1]:  https://github.com/esp-rs/esp-rust-board
[LED connected to GPIO 7]: https://github.com/esp-rs/esp-rust-board#pin-layout
[into-push-pull-output]: https://docs.rs/esp32c3-hal/latest/esp32c3_hal/gpio/struct.GpioPin.html#method.into_push_pull_output
[toogle]: https://docs.rs/embedded-hal/0.2.7/embedded_hal/digital/v2/trait.ToggleableOutputPin.html#tymethod.toggle
[delay-ms]: https://docs.rs/embedded-hal/0.2.7/embedded_hal/blocking/delay/trait.DelayMs.html#tymethod.delay_ms

## Simulation

This project is available for simulation through two methods:
- Wokwi projects:
  - Exercise: Currently not available
  - [Solution](https://wokwi.com/projects/382725482391094273?build-cache=disable)
- Wokwi files are also present in the project folder to simulate it with Wokwi VS Code extension:
   1. Press F1, select `Wokwi: Select Config File` and choose `intro/blinky/wokwi.toml`
      - Edit the `wokwi.toml` file to select between exercise and solution simulation
   2. Build you project
   3. Press F1 again and select `Wokwi: Start Simulator`

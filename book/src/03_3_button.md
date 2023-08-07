# Detect a button press

We are now going to make the LED ligth only when we press a button, we will create a
project that reads the state of the button GPIO and reacts to its state.

`intro/button/examples/button.rs` contains the solution. You can run it with the following command:

```shell
cargo run --example button
```

Most of the dev-boards have a button, in our case, we will use the one labeled [`BOOT` on `GPIO9`].


✅ Initiate the IO peripheral, and create variable for the LED and button, the LED can be created using the
[`into_push_pull_output` function][into-push-pull-output] as before while the button can be obtained using
[`into_pull_up_input` function][into-pull-up-input].

Similarly to turning a `GPIO` into an `output` we can turn it into an `input`. Then we can get the current state of the `input` pin with `is_high` and similar functions.

✅ In the `loop`, add some logic so if the button is not pressed, the LED is lit. If the button is pressed, the LED is off.

[`BOOT` on `GPIO9`]: https://github.com/esp-rs/esp-rust-board#ios
[into-pull-up-input]: https://docs.rs/esp32c3-hal/latest/esp32c3_hal/gpio/struct.GpioPin.html#method.into_pull_up_input
[into-push-pull-output]: https://docs.rs/esp32c3-hal/latest/esp32c3_hal/gpio/struct.GpioPin.html#method.into_push_pull_output

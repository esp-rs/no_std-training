# Detect a button press

We are now going to make the LED ligth only when we press a button, we will create a
project that reads the state of the button GPIO and reacts to its state.


## Setup

✅ Go to `intro/button` directory.

✅ Open the prepared project skeleton in `intro/button`.

✅ Open the docs for this project with the following command:

```
cargo doc --open
```

`intro/button/examples/button.rs` contains the solution. You can run it with the following command:

```shell
cargo run --example button
```

## Exercise

Most of the dev-boards have a button, in our case, we will use the one labeled [`BOOT` on `GPIO9`].


✅ Initiate the IO peripheral, and create variable for the LED and button, the LED can be created using the
[`into_push_pull_output` function][into-push-pull-output] as before while the button can be obtained using
[`into_pull_up_input` function][into-pull-up-input].

Similarly to turning a `GPIO` into an `output` we can turn it into an `input`. Then we can get the current state of the `input` pin with `is_high` and similar functions.

✅ In the `loop`, add some logic so if the button is not pressed, the LED is lit. If the button is pressed, the LED is off.

[`BOOT` on `GPIO9`]: https://github.com/esp-rs/esp-rust-board#ios
[into-pull-up-input]: https://docs.esp-rs.org/esp-hal/esp-hal/0.16.1/esp32c3/esp_hal/gpio/struct.GpioPin.html#method.into_pull_up_input
[into-push-pull-output]: https://docs.esp-rs.org/esp-hal/esp-hal/0.16.1/esp32c3/esp_hal/gpio/struct.GpioPin.html#method.into_push_pull_output

## Simulation

This project is available for simulation through two methods:
- Wokwi projects:
  - Exercise: Currently not available
  - [Solution](https://wokwi.com/projects/382725583123606529?build-cache=disable)
- Wokwi files are also present in the project folder to simulate it with Wokwi VS Code extension:
   1. Press F1, select `Wokwi: Select Config File` and choose `intro/button/wokwi.toml`
      - Edit the `wokwi.toml` file to select between exercise and solution simulation
   2. Build you project
   3. Press F1 again and select `Wokwi: Start Simulator`

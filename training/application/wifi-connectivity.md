# Wi-Fi Connectivity

In this chapter we connect the board to a local Wi-Fi network, obtain an IP address, and send each sensor reading to a computer on the same network. The completed code for this chapter is available in `project/part2/`.

The HAL initialization, I2C setup, and SHTC3 measurement sequence from [Reading Sensor Data](./reading-sensor-data.md) stay the same. What changes is the structure of the project and the new networking pieces: a heap, a network stack, background tasks, and compile-time credentials.

## From One File to Modules

The previous chapter kept everything in `main.rs`. Once we add Wi-Fi and HTTP, that file would become hard to follow, so we move related code into modules:

```rust,ignore
{{#include ../../project/part2/src/main.rs:modules}}
```

- `sensor.rs` holds the SHTC3 measurement helper that used to live in the main loop. The I2C bus is still created in `main`, but starting a measurement, waiting for it, and converting the result now live in `read_sensor`:

```rust,ignore
{{#include ../../project/part2/src/sensor.rs:read_sensor}}
```

- `network.rs` owns the two background tasks that keep the radio associated and the IP stack running.
- `http.rs` builds a JSON payload and sends it with a minimal HTTP `POST`.
- `main.rs` still initializes the hardware, starts the network stack, and runs the application loop.

This split keeps the new networking concepts easier to read, and the same modules will grow in later chapters.

## Why We Need `esp-alloc`

A `no_std` program has no global allocator unless we provide one. The previous chapter never needed a heap: the sensor driver works with stack buffers, and we never built a `String` or a `Vec`.

Wi-Fi is different. [`esp-radio`][esp-radio] wraps the precompiled radio driver, and that driver allocates at runtime. Without a heap, the application would not even start the radio. Our HTTP helper also uses the [`alloc`][alloc] crate, because `format!` builds the request in a heap-allocated `String`.

We therefore add [`esp-alloc`][esp-alloc], with the `esp32c3` feature, and enable the `alloc` crate:

```toml
{{#include ../../project/part2/Cargo.toml:esp_alloc}}
```

```rust,ignore
{{#include ../../project/part2/src/main.rs:alloc_crate}}
```

```rust,ignore
{{#include ../../project/part2/src/main.rs:alloc_import}}
```

`extern crate alloc;` makes `Vec`, `String`, and `format!` available in `no_std`. `use esp_alloc as _;` pulls in the crate so its global allocator is linked, even though we do not call it by name.

Then we create the heap before starting the scheduler or the radio:

```rust,ignore
{{#shiftinclude auto:../../project/part2/src/main.rs:heap_init}}
```

[`heap_allocator!`][heap-allocator] can be invoked more than once. Each invocation adds a region to the same global allocator. For the ESP32-C3 used in this training, the calls reserve two heap regions:

- 64 KiB of reclaimed memory, selected by `#[ram(reclaimed)]`.
- Another 36 KiB of memory.

See [Allocating Memory][alloc-book] in *The Rust on ESP Book* for more details.

> `esp-radio` requires both a heap and a running scheduler, so we call `esp_rtos::start(...)` before creating the Wi-Fi controller.

## Initializing the Network Stack

Networking is split into two layers. [`esp-radio`][esp-radio] drives the Wi-Fi hardware. [`embassy-net`][embassy-net] is the IP stack that sits on top of that interface and gives us DHCP, DNS, and TCP.

### Creating the Wi-Fi Interface

After the scheduler is running, we take ownership of the Wi-Fi peripheral and create a station interface:

```rust,ignore
{{#shiftinclude auto:../../project/part2/src/main.rs:wifi_controller}}
```

[`WifiController::new`][wifi-controller] initializes the Wi-Fi driver. The controller configures the radio and associates with an access point. [`Interface::station()`][wifi-interface] is the `embassy-net` driver: a Wi-Fi client that can send and receive Ethernet frames. There is only one station interface, so creating a second one would panic.

### `embassy-net` Configuration

`embassy-net` is configured in two places: Cargo features, and the `Config` value we pass at runtime.

The crate features select which protocols are compiled in:

```toml
{{#include ../../project/part2/Cargo.toml:embassy_net}}
```

- `medium-ethernet` is required because a Wi-Fi station presents an Ethernet-like interface.
- `dhcpv4` lets the stack request an IPv4 address from the access point.
- `udp` is used by DHCP and DNS.
- `tcp` and `dns` are used later when we open a socket and resolve a host.

We also enable the `wifi` feature on `esp-radio`, and the `esp-radio` and `esp-alloc` integrations of `esp-rtos`, which let the radio driver use the scheduler and the heap:

```toml
{{#include ../../project/part2/Cargo.toml:esp_radio}}
```

```toml
{{#include ../../project/part2/Cargo.toml:esp_rtos}}
```

At runtime we ask for a DHCP lease and seed the stack from the hardware RNG:

```rust,ignore
{{#shiftinclude auto:../../project/part2/src/main.rs:embassy_net_config}}
```

[`Config::dhcpv4`][embassy-net-config] is the `embassy-net` equivalent of "get an address from the router". A static address would also work, but DHCP is the usual setup on a development network and avoids hard-coding an IP in firmware.

The random `seed` is not a cryptographic key. `embassy-net` uses it for protocol values that must not be predictable across devices, such as TCP initial sequence numbers and DHCP transaction IDs.

### Why the Stack Resources Are Static

[`embassy_net::new`][embassy-net-new] needs a Wi-Fi driver, the config, a random seed, and a [`StackResources<N>`][stack-resources] buffer. `N` is the number of sockets the stack can keep open at once. We use `3`, which is enough for DHCP, DNS, and one TCP connection.

```rust,ignore
{{#shiftinclude auto:../../project/part2/src/main.rs:stack_init}}
```

Those resources, the `Runner`, and the spawned tasks all need a `'static` lifetime. Embassy tasks are not allowed to borrow data from `main`'s stack: once spawned, a task can run for the rest of the program, so every argument must outlive `main`.

A plain local `let resources = StackResources::new();` would be dropped when `main` returned (it never does, but the type system cannot use that fact) and could not be given to a `'static` task. A `static mut` would compile only with `unsafe`.

[`StaticCell`][static-cell] solves this. It is a one-time cell: we initialize it at runtime, then get a `&'static mut StackResources<3>`. After that, [`embassy_net::new`][embassy-net-new] returns a [`Stack`][embassy-net-stack] we can use from application code and a [`Runner`][embassy-net-runner] that must be polled continuously.

## Why We Use Tasks

The stack is not useful until two loops run in the background:

```rust,ignore
{{#shiftinclude auto:../../project/part2/src/main.rs:spawn_tasks}}
```

Both functions are Embassy tasks, created with [`#[embassy_executor::task]`][embassy-task].

`connection` owns the `WifiController`. It configures the station with the network credentials, connects, and if the connection fails or drops it waits five seconds and tries again:

```rust,ignore
{{#include ../../project/part2/src/network.rs:connection_task}}
```

`net_task` owns the `Runner`. Its only job is to keep polling the IP stack:

```rust,ignore
{{#include ../../project/part2/src/network.rs:net_task}}
```

We use tasks because these two loops never finish, and they must make progress at the same time as the application. If `main` called `runner.run().await` itself, it would never read the sensor or send HTTP. If it only ran the connection loop, DHCP and TCP would stall. Embassy's executor interleaves the tasks whenever one of them `.await`s, so the radio, the IP stack, and the application loop can all run on a single core.

The `'static` lifetimes on `WifiController` and `Runner` are the same constraint as before: a spawned task may outlive the function that spawned it.

## Environment Variables

The network name and password are not written as string literals. They are read at compile time:

```rust,ignore
{{#include ../../project/part2/src/network.rs:wifi_env}}
```

[`env!`][env-macro] expands to a `&'static str` taken from the environment when `rustc` runs. The `connection` task converts the strings into the `Ssid` and `Password` types of `esp-radio`, which reject values longer than 32 and 64 bytes, and uses WPA2 as the minimum authentication method. If `SSID` or `PASSWORD` is missing, the build fails. That keeps credentials out of the source tree, and it means you pass them when you build or flash:

```shell
SSID="your-ssid" PASSWORD="your-password" cargo run --release
```

Cargo forwards process environment variables to the compiler, so these values are baked into the firmware. Changing the network later requires rebuilding.

The HTTP helper uses optional variables for the receiver on your computer:

```rust,ignore
{{#include ../../project/part2/src/http.rs:http_env}}
```

[`option_env!`][option-env-macro] yields `None` when the variable is unset, so the firmware still builds. At runtime, `send_sensor_data` logs an error and returns if `HOST_IP` is missing. `HTTP_PORT` defaults to `8080`. Despite its name, `HOST_IP` can also contain a hostname, which is resolved with DNS.

## Waiting for an Address, Then Sending Data

After the tasks are spawned, `main` waits until DHCP has assigned an address:

```rust,ignore
{{#shiftinclude auto:../../project/part2/src/main.rs:wait_ip}}
```

`wait_config_up()` completes once the station is associated and the DHCP lease is in place. Only then do we start sending.

The application loop is now two calls: read the sensor, and if that succeeds, post the values:

```rust,ignore
{{#shiftinclude auto:../../project/part2/src/main.rs:main_loop}}
```

`send_sensor_data` formats a small JSON body, opens a TCP socket, and writes a hand-built HTTP/1.0 `POST` to `/sensor`. It logs failures itself, so the loop simply moves on to the next reading:

```rust,ignore
{{#shiftinclude auto:../../project/part2/src/http.rs:http_payload}}
```

This chapter uses HTTP only as a simple way to prove that the device can reach another host. The next chapter replaces this with MQTT.

## Running the Application

The firmware posts to a computer on the same network. From the repository root, start the receiver first:

```shell
cargo xtask http-server
```

The task prints a host IP when it can detect one. Use that value as `HOST_IP`. Then, from `project/part2/`, flash the application:

```shell
SSID="your-ssid" PASSWORD="your-password" HOST_IP="192.168.1.10" cargo run --release
```

Replace the SSID, password, and IP with your network and the address printed by `xtask`. You can also set `HTTP_PORT` if the server is not on `8080`.

Once the device associates, you should see a connection log on the serial monitor:

```text
INFO - Wi-Fi connected!
INFO -   24.31 °C | 42.18 %RH
INFO - HTTP request sent
```

The `http-server` terminal should print each payload:

```text
sensor payload={"temperature":24.31,"humidity":42.18}
```

At this point the board can join a Wi-Fi network, obtain an IP address, and send measurements off-device. In the next chapter we will publish those readings to an MQTT broker.

[esp-radio]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/index.html
[esp-alloc]: https://docs.espressif.com/projects/rust/esp-alloc/0.11.0/esp_alloc/index.html
[heap-allocator]: https://docs.espressif.com/projects/rust/esp-alloc/0.11.0/esp_alloc/macro.heap_allocator.html
[alloc-book]: https://docs.espressif.com/projects/rust/book/application-development/alloc.html
[alloc]: https://doc.rust-lang.org/alloc/
[wifi-controller]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/wifi/struct.WifiController.html#method.new
[wifi-interface]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/wifi/struct.Interface.html
[embassy-net]: https://docs.rs/embassy-net/0.9.1/embassy_net/
[embassy-net-new]: https://docs.rs/embassy-net/0.9.1/embassy_net/fn.new.html
[embassy-net-config]: https://docs.rs/embassy-net/0.9.1/embassy_net/struct.Config.html#method.dhcpv4
[embassy-net-stack]: https://docs.rs/embassy-net/0.9.1/embassy_net/struct.Stack.html
[embassy-net-runner]: https://docs.rs/embassy-net/0.9.1/embassy_net/struct.Runner.html
[stack-resources]: https://docs.rs/embassy-net/0.9.1/embassy_net/struct.StackResources.html
[static-cell]: https://docs.rs/static_cell/2.1.1/static_cell/struct.StaticCell.html
[embassy-task]: https://docs.rs/embassy-executor/0.10.0/embassy_executor/attr.task.html
[env-macro]: https://doc.rust-lang.org/core/macro.env.html
[option-env-macro]: https://doc.rust-lang.org/core/macro.option_env.html

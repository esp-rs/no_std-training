# HTTP Client
Next, we'll write a small client that retrieves data over an HTTP connection to the internet.

For demonstration purposes we implement the http client ourselves. Usually you want to use e.g. [`reqwless`](https://crates.io/crates/reqwless) or [`edge-net`](https://crates.io/crates/edge-net)

Before jumping to the exercise, let's explore how Wi-Fi works in `no_std` Rust for Espressif devices.

## Wi-Fi Ecosystem

Wi-Fi support comes in the [`esp-wifi` crate][esp-wifi]. The `esp-wifi` is home to the Wi-Fi, Bluetooth and ESP-NOW driver implementations for `no_std` Rust.
Check the repository README for current support, limitations and usage details.

There are some other relevant crates, on which `esp-wifi` depends on:
- [`smol-tcp`][smoltcp]: Event-driven TCP/IP stack implementation.
  - It does not require heap allocation (which is a requirement for some `no_std` projects)
  - For more information about the crate, see the [official documentation][smoltcp-docs]

Additionally when using async, [`embassy-net`][embassy-net] is relevant.

[esp-wifi]: https://github.com/esp-rs/esp-wifi
[embassy-net]: https://github.com/embassy-rs/embassy/tree/main/embassy-net
[smoltcp]: https://github.com/smoltcp-rs/smoltcp
[smoltcp-docs]: https://docs.rs/smoltcp/latest/smoltcp/

## Setup

✅ Go to `intro/http-client` directory.

✅ Open the prepared project skeleton in `intro/http-client`.

✅ Add your network credentials: Set the  `SSID` and `PASSWORD` environment variables.

`intro/http-client/examples/http-client.rs` contains the solution. You can run it with the following command:

```shell
cargo run --release --example http-client
```

✅ Read the [Optimization Level] section of the [`esp-wifi`] README.

[Optimization Level]: https://github.com/esp-rs/esp-wifi/tree/main/esp-wifi#optimization-level
[`esp-wifi`]: https://github.com/esp-rs/esp-wifi

## Exercise

✅ Bump the [`clock`][clock] frequency at which the target operates to its maximum. Consider using `ClockControl::configure` or `ClockControl::max`

✅ Create a [`timer`][timer] and initialize the Wi-Fi
```rust,ignore
{{#include ../../intro/http-client/examples/http-client.rs:wifi_init}}
```

✅ Configure Wi-Fi using Station Mode
```rust,ignore
{{#include ../../intro/http-client/examples/http-client.rs:wifi_config}}
```

✅ Create a Client with your Wi-Fi credentials and default configuration. Look for a suitable constructor in the documentation.
```rust,ignore
{{#include ../../intro/http-client/examples/http-client.rs:client_config_start}}
    ....
{{#include ../../intro/http-client/examples/http-client.rs:client_config_end}}
```

✅ Start the Wi-Fi controller, scan the available networks, and try to connect to the one we set.
```rust,ignore
{{#include ../../intro/http-client/examples/http-client.rs:wifi_connect}}
```

✅ Then we obtain the assigned IP
```rust,ignore
{{#include ../../intro/http-client/examples/http-client.rs:ip}}
```

If the connection succeeds, we proceed with the last part, making the HTTP request.

By default, only unencrypted HTTP is available, which limits our options of hosts to connect to. We're going to use `www.mobile-j.de/`.

To make an HTTP request, we first need to open a socket, and write to it the GET request,

✅ Open a socket with the following IPv4 address `142.250.185.115` and port `80`. See `IpAddress::Ipv4` documentation.

✅ `write` the following message to the socket and `flush` it: `b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n"`

✅ Then we wait for the response and read it out.
```rust,ignore
{{#include ../../intro/http-client/examples/http-client.rs:reponse}}
```

✅ Finally, we will close the socket and wait
```rust,ignore
{{#include ../../intro/http-client/examples/http-client.rs:socket_close}}
```

[timer]: https://docs.esp-rs.org/esp-hal/esp-hal/0.16.0/esp32c3/esp32c3/systimer/index.html
[clock]: https://docs.esp-rs.org/esp-hal/esp-hal/0.16.0/esp32c3/esp_hal/clock/index.html

## Simulation

This project is available for simulation through two methods:
- Wokwi projects:
  - Exercise: Currently not available
  - Solution: Currently not available
- Wokwi files are also present in the project folder to simulate it with Wokwi VS Code extension:
   1. Press F1, select `Wokwi: Select Config File` and choose `intro/http-client/wokwi.toml`
      - Edit the `wokwi.toml` file to select between exercise and solution simulation
   2. Build you project
   3. Press F1 again and select `Wokwi: Start Simulator`

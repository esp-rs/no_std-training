# HTTP Client
Next, we'll write a small client that retrieves data over an HTTP connection to the internet.

Before jumping to the exercise, let's explore how Wi-Fi works in `no_std` Rust for Espressif devices.

## Wi-Fi Ecosystem

Wi-Fi support comes in the [`esp-wifi` crate][esp-wifi]. The `esp-wifi` is home to the Wi-Fi, Bluetooth and ESP-NOW driver implementations for `no_std` Rust.
Check the repository README for current support, limitations and usage details.

There are some other relevant crates, on which `esp-wifi` depends on:
- [`embedded-svc`][embedded-svc]: Contains traits for features such as wifi, networking, HTTPD, and logging.
  - This allows the code to be portable from `no_std` to `std` approach since both implementations use the same set of traits.
- [`smol-tcp`][smoltcp]: Event-driven TCP/IP stack implementation.
  - It does not require heap allocation (which is a requirement for some `no_std` projects)
  - For more information about the crate, see the [official documentation][smoltcp-docs]

[esp-wifi]: https://github.com/esp-rs/esp-wifi
[embedded-svc]: https://github.com/esp-rs/embedded-svc
[smoltcp]: https://github.com/smoltcp-rs/smoltcp
[smoltcp-docs]: https://docs.rs/smoltcp/latest/smoltcp/

## Setup

✅ Go to `intro/http-client` directory.

✅ Open the prepared project skeleton in `intro/http-client`.

✅ Add your network credentials: Set the  `SSID` and `PASSWORD` environment variables.

`intro/http-client/examples/http-client.rs` contains the solution. You can run it with the following command:

```shell
cargo run --example http-client
```

## Exercise

✅ Bump the frequency at which the target operates to its maximum. Consider using `ClockControl::configure` or `ClockControl::max`

✅ Create a timer and initialize the Wi-Fi
```rust,ignore
let timer = SystemTimer::new(peripherals.SYSTIMER).alarm0;
let init = initialize(
    EspWifiInitFor::Wifi,
    timer,
    Rng::new(peripherals.RNG),
    system.radio_clock_control,
    &clocks,
)
.unwrap();
```

✅ Configure Wi-Fi using Station Mode
```rust,ignore
let (wifi, _) = peripherals.RADIO.split();
let mut socket_set_entries: [SocketStorage; 3] = Default::default();
let (iface, device, mut controller, sockets) =
    create_network_interface(&init, wifi, WifiMode::Sta, &mut socket_set_entries);
let wifi_stack = WifiStack::new(iface, device, sockets, current_millis);
```

✅ Create a Client with your Wi-Fi credentials and default configuration. Look for a suitable constructor in the documentation.
```rust,ignore
let client_config = Configuration::Client(
    ....
);
let res = controller.set_configuration(&client_config);
println!("Wi-Fi set_configuration returned {:?}", res);
```

✅ Start the Wi-Fi controller, scan the available networks, and try to connect to the one we set.
```rust,ignore
controller.start().unwrap();
println!("Is wifi started: {:?}", controller.is_started());

println!("Start Wifi Scan");
let res: Result<(heapless::Vec<AccessPointInfo, 10>, usize), WifiError> = controller.scan_n();
if let Ok((res, _count)) = res {
    for ap in res {
        println!("{:?}", ap);
    }
}

println!("{:?}", controller.get_capabilities());
println!("Wi-Fi connect: {:?}", controller.connect());

// Wait to get connected
println!("Wait to get connected");
loop {
    let res = controller.is_connected();
    match res {
        Ok(connected) => {
            if connected {
                break;
            }
        }
        Err(err) => {
            println!("{:?}", err);
            loop {}
        }
    }
}
println!("{:?}", controller.is_connected());
```

✅ Then we obtain the assigned IP
```rust,ignore
// Wait for getting an ip address
println!("Wait to get an ip address");
loop {
    wifi_stack.work();

    if wifi_stack.is_iface_up() {
        println!("got ip {:?}", wifi_stack.get_ip_info());
        break;
    }
}
```

If the connection succeeds, we proceed with the last part, making the HTTP request.

By default, only unencrypted HTTP is available, which rather limits our options of hosts to connect to. We're going to use `www.mobile-j.de/`.

To make an HTTP request, we first need to open a socket, and write to it the GET request,

✅ Open a socket with the following IPv4 address `142.250.185.115` and port `80`. See `IpAddress::Ipv4` documentation.

✅ `write` the following message to the socket and `flush` it: `b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n"`

✅ Then we wait for the response and read it out.
```rust,ignore
let wait_end = current_millis() + 20 * 1000;
loop {
    let mut buffer = [0u8; 512];
    if let Ok(len) = socket.read(&mut buffer) {
        let to_print = unsafe { core::str::from_utf8_unchecked(&buffer[..len]) };
        print!("{}", to_print);
    } else {
        break;
    }

    if current_millis() > wait_end {
        println!("Timeout");
        break;
    }
}
println!();
```

✅ Finally, we will close the socket and wait
```rust,ignore
socket.disconnect();

let wait_end = current_millis() + 5 * 1000;
while current_millis() < wait_end {
    socket.work();
}
```

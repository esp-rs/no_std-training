# Wi-Fi Provisioning

In this chapter we remove the compile-time Wi-Fi credentials. Instead, the board starts its own access point and serves a small web page where you enter the network name and password. Once the credentials arrive, the board leaves access point mode, joins the network, and publishes sensor data to the MQTT broker as before. The completed code for this chapter is available in `project/part4/`.

The I2C setup, the heap, and the MQTT task from [Publishing Data to an MQTT Broker](./publishing-data.md) stay mostly the same. The MQTT task now also publishes humidity to `measurement/humidity`. The main changes are a second network stack, a set of small servers that run on the device, and a signal that hands the credentials from the web page to the Wi-Fi connection task.

## What Is Wi-Fi Provisioning?

So far, `SSID` and `PASSWORD` were read with `env!` and baked into the firmware. That works on a development desk, but it does not scale to real devices:

- Every network requires a different firmware image.
- The person installing the device usually cannot rebuild and flash firmware.
- Changing the router or the password means reflashing the device.

**Wi-Fi provisioning** is the process of delivering network credentials to an unconfigured device at runtime. The firmware is the same for every unit, and the user supplies the credentials during setup.

There are several ways to get credentials onto a device, but the two most common are:

- **Access point (SoftAP)**: the device creates its own Wi-Fi network. The user connects to it with a phone or laptop and enters the credentials, either in a web page served by the device or in a companion app.
- **Bluetooth Low Energy (BLE)**: the device advertises over BLE, and a phone app sends the credentials over a BLE connection. The device's Wi-Fi radio never has to act as an access point, and the phone stays connected to its own network during setup.

ESP-IDF supports both transports in its [unified provisioning][idf-provisioning] framework, and Espressif provides companion apps for Android and iOS.

In this chapter we use the access point approach with a web page. It needs nothing on the user's side except a browser, and it only uses the Wi-Fi stack we already have.

The provisioning flow looks like this:

```mermaid
sequenceDiagram
    participant User as Phone or laptop
    participant AP as ESP32-C3 (access point)
    participant Router as Wi-Fi router
    participant Broker as MQTT broker

    AP->>AP: Start access point "esp-radio"
    User->>AP: Join "esp-radio"
    AP-->>User: DHCP lease (192.168.2.x, DNS server 192.168.2.1)
    User->>AP: Captive portal check, or GET http://192.168.2.1/
    AP-->>User: Provisioning page
    User->>AP: POST /save (SSID and password)
    AP-->>User: "Configuration Saved!"
    AP->>AP: Switch to station mode
    AP->>Router: Join network with received credentials
    Router-->>AP: DHCP lease
    AP->>Broker: Connect and publish measurements
```

## Station and Access Point Modes

The Wi-Fi radio can play two roles:

- In **station (STA) mode**, the device is a client that joins an existing network. This is what we used in the previous chapters.
- In **access point (AP) mode**, the device *is* the network. Other devices find its SSID, join it, and talk to it directly.

Provisioning needs both: AP mode to receive the credentials, then STA mode to use them.

### Two Interfaces, Two Network Stacks

In addition to [`Interface::station()`][wifi-interface], `esp-radio` provides [`Interface::access_point()`][wifi-interface]. Each interface is a separate network device, so each one gets its own `embassy-net` stack. `create_network_stacks` creates both interfaces and their stacks:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/main.rs:network_stacks}}
```

The two stacks are configured differently:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/network.rs:stack_configs}}
```

- The **station stack** uses DHCP, exactly as before, because the router assigns its address.
- The **access point stack** uses a static address. There is no router on the device's own network, so nobody else can hand it an address. The device is the gateway of a `/24` subnet and uses the gateway address for itself.

The gateway address defaults to `192.168.2.1` and can be overridden at compile time with `GATEWAY_IP`:

```rust,ignore
{{#include ../../project/part4/src/main.rs:gateway_ip}}
```

The access point stack also needs more sockets than the station stack, because it hosts servers for DHCP, DNS, and HTTP:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/network.rs:ap_stack}}
```

The station stack keeps the `StackResources<3>` from the previous chapters. Both stacks still need `StaticCell`, for the same `'static` lifetime reasons described in [Wi-Fi Connectivity](./wifi-connectivity.md#why-the-stack-resources-are-static). Each stack also gets its own random seed, so the two stacks do not pick the same TCP sequence numbers or DHCP transaction IDs.

### Configuring the Controller

The interfaces only carry frames. The [`WifiController`][wifi-controller] decides which role the radio plays, and the `connection` task starts it in access point mode:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/network.rs:ap_mode}}
```

[`AccessPointConfig`][ap-config] defaults to an open network on channel 1. We only change the SSID to `esp-radio`, which is the network you will join from your phone or computer.

After the credentials arrive, the same task replaces the configuration with a [`StationConfig`][sta-config]:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/network.rs:station_mode}}
```

[`set_config`][set-config] restarts Wi-Fi in the new mode, so the access point disappears and the provisioning page is no longer reachable. The station then connects and reconnects with the same loop used in the previous chapters.

> `esp-radio` also supports running both roles at once with [`Config::AccessPointStation`][wifi-config]. This would let the portal stay up while the device tries the new credentials, for example to report a wrong password back to the user. We keep the two phases separate to keep the example small.

## Tasks and Why We Need Them

The application now has seven concurrent activities. `main` initializes the hardware, creates the network stacks, and spawns the tasks:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/main.rs:spawn_tasks}}
```

| Task                 | Stack | Responsibility                                                              |
| -------------------- | ----- | --------------------------------------------------------------------------- |
| `connection`         | —     | Owns the `WifiController`, starts the access point, then switches to station mode. |
| `net_task`           | AP    | Polls the access point stack's `Runner`.                                    |
| `net_task`           | STA   | Polls the station stack's `Runner`.                                         |
| `run_dhcp`           | AP    | Hands out IP addresses to clients that join the access point.               |
| `run_captive_portal` | AP    | Answers DNS queries with the device's own address.                          |
| `run_http_server`    | AP    | Serves the provisioning page and receives the credentials.                  |
| `mqtt_task`          | STA   | Reads the sensor and publishes measurements, as in the previous chapter.    |

Each of these is a loop that never finishes. Most of them spend their time waiting for a packet, a connection, or a message. Because they are separate Embassy tasks, the executor runs whichever one has work to do, and the waiting ones cost nothing. A single sequential `main` could not wait for an HTTP request, answer DHCP, and poll two network stacks at the same time.

### Two Runner Tasks

Every `embassy-net` stack needs its `Runner` to be polled continuously, so we need one runner task per stack:

```rust,ignore
{{#include ../../project/part4/src/network.rs:runner_task}}
```

A function marked with [`#[embassy_executor::task]`][embassy-task] has a fixed-size, statically allocated task pool, and its default size is one: spawning `net_task` a second time would fail. `pool_size = 2` reserves room for two instances, one per stack.

### Servers on the Access Point

A phone that joins `esp-radio` expects the same services a normal router provides. The provisioning tasks provide the minimum set, using crates from the [`edge-net`][edge-net] family that run on top of `embassy-net` through [`edge-nal-embassy`][edge-nal-embassy].

These crates are new dependencies in `Cargo.toml`:

```toml
{{#include ../../project/part4/Cargo.toml:edge_net}}
```

`run_dhcp` uses [`edge-dhcp`][edge-dhcp] to assign addresses to clients. Without it, the user would have to configure a static IP address by hand before reaching the page. The lease also tells clients to use the device as their gateway and DNS server:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/http.rs:dhcp_server}}
```

`run_captive_portal` uses [`edge-captive`][edge-captive], a DNS server that answers every query with the gateway address. Its purpose is to make every hostname lead to the provisioning page:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/http.rs:captive_dns}}
```

`run_http_server` uses [`edge-http`][edge-http] to serve the provisioning page on port 80:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/http.rs:http_server}}
```

After spawning the tasks, `main` logs the instructions for joining the portal and returns:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/main.rs:portal_ready}}
```

The `mqtt_task` is spawned immediately, but it does nothing useful yet. Its first step is `stack.wait_config_up()` on the station stack, so it waits until provisioning is complete and the station has joined the network.

### Captive Portals

When a phone or laptop joins a new network, the operating system requests a well-known URL, such as `http://connectivitycheck.gstatic.com/generate_204` on Android or `http://www.msftconnecttest.com/connecttest.txt` on Windows, to check for Internet access. Public hotspots use this check to open a sign-in page automatically. This is called a **captive portal**.

Three pieces make the check reach the provisioning page:

1. The DHCP lease names the device as the DNS server.
2. The DNS server on port `53` answers every query, including the one for the probe host, with the address of the device.
3. The HTTP handler redirects every unknown page to the provisioning page:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/http.rs:captive_redirect}}
```

Whether and how the sign-in page opens depends on the operating system and its settings. For example, a client configured with its own DNS server, such as Android's Private DNS, never asks the device. You can always open the page yourself at `http://192.168.2.1/`.

### Passing Credentials Between Tasks

The HTTP server receives the credentials, but the `connection` task owns the `WifiController`. The two tasks communicate through an [`embassy-sync` `Signal`][embassy-signal]:

```rust,ignore
{{#include ../../project/part4/src/network.rs:credentials_signal}}
```

A `Signal` holds at most one value: sending replaces any value that has not been received yet, and never waits. That is all we need for a single set of credentials, and the HTTP handler can never block on it. It uses a `CriticalSectionRawMutex`, which is safe to share between tasks and interrupts. `Signal::new` is a `const fn`, so unlike the stack resources, the signal can be a plain `static` that both tasks use directly.

The value type is a small struct:

```rust,ignore
{{#include ../../project/part4/src/network.rs:wifi_credentials}}
```

The fields are fixed-capacity [`heapless::String`][heapless-string]s sized to the Wi-Fi limits: an SSID is at most 32 bytes, and a WPA2 passphrase is at most 63 characters, or 64 hexadecimal digits. Deriving `Deserialize` lets the HTTP handler build the struct directly from the JSON request body. The `serde` feature on `heapless` makes this possible.

The `connection` task waits on the receiving end:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/network.rs:receive_credentials}}
```

`wait().await` suspends the task until a value arrives, without polling. The two-second delay gives the browser time to receive the response and load the confirmation page before the access point shuts down.

## Serving the Provisioning Page

The HTTP handler matches on the request method and path. A `GET /` returns the provisioning page:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/http.rs:home_route}}
```

`GET /saved` returns the confirmation page in the same way. Other `GET` requests are redirected to the provisioning page, as described in [Captive Portals](#captive-portals), and any other request returns `404 Not Found`.

The form submits the credentials as JSON to `POST /save`:

```rust,ignore
{{#shiftinclude auto:../../project/part4/src/http.rs:save_route}}
```

The handler reads up to 256 bytes of the body, which can arrive in several TCP segments, and parses it with [`serde-json-core`][serde-json-core], a `no_std` JSON parser that does not allocate. If parsing succeeds, the handler signals the credentials and replies with `200 OK`. If the body is empty, too long, or not valid JSON, the handler replies with `400 Bad Request`.

The handler also rejects passwords shorter than eight characters, the minimum for WPA2. The Wi-Fi driver does not reject them: it accepts the configuration and only fails to connect, at which point the access point is already gone. Rejecting them here keeps the portal running and lets the page report the error. The form checks the same limits with the `minlength` and `maxlength` attributes, so the browser reports them before submitting.

## HTML Templates

The two pages live in `project/part4/assets/templates/`:

- `home.html` is the provisioning form with SSID and password fields.
- `saved.html` is the confirmation page shown after the credentials are submitted.

There is no file system on the device. Instead, [`include_str!`][include-str] embeds both files in the firmware at compile time:

```rust,ignore
{{#include ../../project/part4/src/http.rs:templates}}
```

`CARGO_MANIFEST_DIR` is the directory that contains `Cargo.toml`, so the paths do not depend on the directory from which you run Cargo. The pages become `&'static str` constants stored in flash, and the handler writes their bytes directly to the TCP connection.

Keep a few things in mind when editing them:

- Despite the directory name, the pages are not processed by a template engine. They are served exactly as written.
- Changes require a rebuild and reflash, because the HTML is part of the firmware image.
- `home.html` displays `esp-radio` as plain text. If you change the access point SSID, update the page as well.
- The form uses a small JavaScript `fetch` call to send JSON rather than the browser's default form encoding. This is what lets the firmware parse the body with `serde-json-core` into `WifiCredentials`. On success, the script navigates to `/saved`.
- Every byte of HTML and CSS increases the size of the firmware. Inline styles and no external resources keep the pages self-contained, which matters because the client has no Internet access while connected to the access point.

## Running the Application

Start the MQTT broker from the repository root:

```shell
cargo xtask mqtt-server
```

In another terminal, change to `project/part4/` and flash the application. `SSID` and `PASSWORD` are no longer needed:

```shell
HOST_IP="your-ip" cargo run --release
```

Replace `your-ip` with the host IP printed by `cargo xtask mqtt-server`. `BROKER_PORT` works as in the previous chapter, and `GATEWAY_IP` changes the address of the provisioning portal.

After boot, the serial monitor shows the portal instructions:

```text
INFO - Provisioning portal: join the `esp-radio` network and open http://192.168.2.1/
INFO - HTTP server listening on port 80
INFO - Wi-Fi access point started
```

Then provision the device:

1. On a phone or computer, join the open Wi-Fi network `esp-radio`.
2. If the sign-in page does not open by itself, open `http://192.168.2.1/` in a browser. Use `http://`, not `https://`.
3. Enter the SSID and password of the network on which the MQTT broker is reachable, then select **Save Configuration**.
4. Reconnect your phone or computer to its usual network. The `esp-radio` network disappears once the device switches to station mode.

The serial monitor shows the switch to station mode, followed by the MQTT connection and the sensor readings:

```text
INFO - Credentials received for SSID your-ssid
INFO - Wi-Fi connected!
INFO - Connecting to MQTT broker at 192.168.1.10:1884...
INFO - Connected to MQTT broker
INFO -   24.31 °C | 42.18 %RH
```

The broker terminal now prints both topics:

```text
Topic: measurement/temperature | Payload: 24.31
Topic: measurement/humidity | Payload: 42.18
```

If the password is wrong, the `connection` task keeps retrying every five seconds. Reset the board to start the provisioning portal again.

## Limitations

This example shows the core of Wi-Fi provisioning, but it leaves out several things a product would need:

- **The credentials are not persisted.** They only live in RAM, so every reset starts the provisioning portal again. A real device would store them in flash and only start the portal when no valid credentials are stored.
- **The access point is open.** Anyone nearby can join it and submit credentials, and the password travels in plain HTTP. Protecting the access point with a WPA2 password, or using a provisioning protocol with its own encryption, such as the security schemes in ESP-IDF's provisioning component, would address this.
- **There is no feedback on failure.** The access point is gone by the time the device tries the credentials, so the user cannot see a connection failure on the page.
- **One client at a time.** The HTTP server has a single connection handler, so a browser that keeps an idle connection open delays other requests until the connection times out.

At this point the device no longer needs credentials at compile time: the same firmware can join any network. In the next chapter we will look at updating that firmware in the field with [Over-the-Air Updates](./ota-updates.md).

[idf-provisioning]: https://docs.espressif.com/projects/esp-idf/en/latest/esp32c3/api-reference/provisioning/provisioning.html
[wifi-interface]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/wifi/struct.Interface.html
[wifi-controller]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/wifi/struct.WifiController.html
[wifi-config]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/wifi/enum.Config.html
[set-config]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/wifi/struct.WifiController.html#method.set_config
[ap-config]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/wifi/ap/struct.AccessPointConfig.html
[sta-config]: https://docs.espressif.com/projects/rust/esp-radio/1.0.0-beta.1/esp32c3/esp_radio/wifi/sta/struct.StationConfig.html
[embassy-task]: https://docs.rs/embassy-executor/0.10.0/embassy_executor/attr.task.html
[embassy-signal]: https://docs.rs/embassy-sync/0.8.0/embassy_sync/signal/struct.Signal.html
[edge-net]: https://github.com/sysgrok/edge-net
[edge-nal-embassy]: https://docs.rs/edge-nal-embassy/0.9.0/edge_nal_embassy/
[edge-dhcp]: https://docs.rs/edge-dhcp/0.8.0/edge_dhcp/
[edge-captive]: https://docs.rs/edge-captive/0.8.0/edge_captive/
[edge-http]: https://docs.rs/edge-http/0.8.0/edge_http/
[heapless-string]: https://docs.rs/heapless/0.9.3/heapless/string/type.String.html
[serde-json-core]: https://docs.rs/serde-json-core/0.7.0/serde_json_core/
[include-str]: https://doc.rust-lang.org/core/macro.include_str.html

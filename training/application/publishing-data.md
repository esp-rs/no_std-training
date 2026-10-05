# Publishing Data to an MQTT Broker

In this chapter we replace the temporary HTTP request from the previous chapter with MQTT. The board connects to an MQTT broker, reads the SHTC3 sensor, and publishes the temperature once per second. The completed code for this chapter is available in `project/part3/`.

The I2C, Wi-Fi, and network-stack initialization from [Wi-Fi Connectivity](./wifi-connectivity.md) stay the same. The main changes are the MQTT client, a dedicated task that owns the sensor, and a local broker that lets us inspect each publication.

## Wi-Fi Credentials

This chapter continues to use the compile-time `SSID` and `PASSWORD` introduced in the previous chapter. Keeping the Wi-Fi setup unchanged lets us focus on MQTT. In the next chapter, we will replace these compile-time credentials with a Wi-Fi provisioning portal.

## What MQTT Adds

MQTT is a publish/subscribe protocol. Instead of sending an HTTP request directly to a particular application endpoint, a client publishes a payload to a named topic on a broker. Other clients can subscribe to that topic and receive the payload.

This chapter uses three MQTT concepts:

- The **broker** accepts connections and routes messages.
- The ESP32-C3 is a **publisher**.
- `measurement/temperature` is the **topic** to which the device publishes.

The local broker started later in this chapter also subscribes to `measurement/#`. The `#` wildcard matches every topic below `measurement`, so the broker's logger prints messages sent to `measurement/temperature`.

## Adding the MQTT Client

The MQTT implementation lives in a new module:

```rust,ignore
{{#include ../../project/part3/src/main.rs:mqtt_module}}
```

We use the [`rust-mqtt`] crate:

```toml
{{#include ../../project/part3/Cargo.toml:rust_mqtt}}
```

Default features are disabled because this is a `no_std` application. The `v5` feature selects MQTT version 5, which matches the local broker, and `bump` enables the crate's bump-buffer storage. A bump buffer reserves a fixed block of memory up front and lets the client use it for MQTT packets without requiring a separate heap allocation for each operation.

## Starting the MQTT Task

In the previous chapter, `main` owned the sensor and performed every read and HTTP request. We now move that loop into an Embassy task:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/main.rs:spawn_mqtt}}
```

`Stack` is a lightweight, copyable handle to the network stack, so it can be passed to the task while `net_task` continues to drive the stack's runner. The SHTC3 driver is moved into `mqtt_task`, giving that task exclusive ownership of the I2C sensor.

After spawning the background tasks, `main` has no more application work to perform. It remains alive and yields to the executor:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/main.rs:idle_main}}
```

The network connection task, network runner, and MQTT task can now make progress independently whenever the other tasks are waiting.

## Initializing MQTT

The MQTT task first reserves receive and transmit buffers for its TCP socket:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:socket_buffers}}
```

These buffers live for the entire task and are reused for every connection attempt.

### Waiting for the Network

MQTT runs over TCP, so the task cannot connect to the broker until Wi-Fi is associated and DHCP has configured the network stack:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:wait_network}}
```

`wait_link_up()` waits for the Wi-Fi link. `wait_config_up()` waits for the network configuration, and `config_v4()` confirms that an IPv4 address is available. If the configuration disappears before the broker connection starts, the outer loop begins the sequence again.

### Locating the Broker

The broker address and port are read at compile time:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:broker_config}}
```

Both values are baked into the firmware when it is compiled. `HOST_IP` is required by the application logic, but it uses `option_env!` so a missing value produces a log error rather than a compiler error. `BROKER_PORT` is optional and defaults to `1884`, the port used by this training's local MQTT broker.

Despite its name, `HOST_IP` can also contain a hostname. The task first tries to parse the value as an IPv4 address. If parsing fails, it asks the network stack to resolve an IPv4 address with DNS:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:resolve_broker}}
```

Using a literal IP address avoids the DNS lookup. This is useful for the local broker, whose address is normally the IP address of the computer running the training.

### Opening TCP and Connecting MQTT

Next, the task opens a TCP socket to the resolved broker address:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:tcp_connect}}
```

The ten-second timeout prevents a connection attempt from waiting forever. A failed TCP connection pauses for five seconds and then returns to the beginning of the outer loop.

Once TCP is connected, we can initialize the MQTT client:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:mqtt_connect}}
```

The 1 KiB `BumpBuffer` supplies working storage for MQTT packets. `ConnectOptions::new().clean_start()` asks the broker to begin without a previous MQTT session, and `esp32c3` identifies this client to the broker. `client.connect(...)` sends the MQTT `CONNECT` packet over the TCP socket and waits for the broker to accept it.

TCP and MQTT are separate connection layers. A successful `socket.connect(...)` only establishes a byte stream to the broker. A successful `client.connect(...)` completes the MQTT handshake over that stream.

Finally, the task creates the topic and publication options once per MQTT connection:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:publication_options}}
```

The topic is `measurement/temperature`. Calling `retain()` asks the broker to keep the latest value, so a new subscriber can receive the most recent temperature without waiting for the next sensor reading.

## Publishing Measurements

The inner loop is the steady-state part of the task. Before reading the sensor, it verifies that both the Wi-Fi link and network configuration are still available:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:connection_check}}
```

If either has been lost, `break` leaves the publication loop. The outer loop then waits for the network and creates fresh TCP and MQTT connections.

For each publication, the task reads the sensor and formats the temperature:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:read_and_format}}
```

`read_sensor` still returns both temperature and humidity, but this chapter publishes only temperature. The `_` pattern intentionally discards the humidity value. A fixed-capacity [`heapless::String`] holds the formatted number, avoiding a heap allocation for every reading.

The string's bytes become the MQTT payload:

```rust,ignore
{{#shiftinclude auto:../../project/part3/src/mqtt.rs:publish}}
```

If publication succeeds, the task waits one second and reads again. If it fails, the task leaves the inner loop and reconnects. This gives the application one recovery path for a lost Wi-Fi link, a lost IP configuration, a closed TCP socket, or an MQTT error.

## Running the Application

Start the MQTT broker from the repository root:

```shell
cargo xtask mqtt-server
```

The command starts an MQTT version 5 broker on port `1884`, subscribes its logger to `measurement/#`, and prints the computer's IP address when it can detect one:

```text
MQTT server listening on 0.0.0.0:1884
Host IP (best effort): 192.168.1.10
Remote devices should connect to 192.168.1.10:1884
Subscribed to `measurement/#` and printing all incoming MQTT messages.
```

Keep that command running. In another terminal, change to `project/part3/` and flash the application:

```shell
SSID="your-ssid" PASSWORD="your-password" HOST_IP="your-ip" cargo run --release
```

Replace `your-ip` with the host IP printed by the previous command, and replace the credentials with values for your network. The board and the computer running the broker must be reachable from one another. To use a different broker port, pass the same port to both commands:

```shell
cargo xtask mqtt-server --port 1885
```

```shell
SSID="your-ssid" PASSWORD="your-password" HOST_IP="your-ip" BROKER_PORT="1885" cargo run --release
```

After Wi-Fi, TCP, and MQTT connect, the serial monitor should show sensor readings:

```text
INFO - Wifi connected!
INFO - connecting to MQTT broker at 192.168.1.10:1884...
INFO - connected!
INFO -   24.31 °C | 42.18 %RH
```

The broker terminal should print each retained publication:

```text
Topic: measurement/temperature | Payload: 24.31
```

At this point the board can maintain an MQTT connection and publish sensor measurements to a broker. In the next chapter we will replace the compile-time Wi-Fi credentials with a provisioning portal.

[`rust-mqtt`]: https://docs.rs/rust-mqtt/0.5.1/rust_mqtt/
[`heapless::String`]: https://docs.rs/heapless/0.9.3/heapless/struct.String.html

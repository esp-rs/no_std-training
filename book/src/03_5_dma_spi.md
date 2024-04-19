# Direct Memory Access (DMA)

The DMA peripheral is used to perform memory transfers in parallel to the work of the processor (the execution of the main program).

In this chapter you will learn how to use DMA with esp-hal. For the example we are going to use the [SPI] peripheral.

## Setup

✅ Go to `intro/dma` directory.

✅ Open the prepared project skeleton in `intro/dma`.

✅ Open the docs for this project with the following command:

```
cargo doc --open
```

`intro/dma/examples/dma.rs` contains the solution. You can run it with the following command:

```shell
cargo run --release --example dma
```

## Exercise

The project skeleton contains code to transfer a small amount of data via SPI. To make it easy to explore the example you can connect GPIO4 and GPIO2 - this way the data we send is also the data we receive.

The blocking SPI transfer looks like this
```rust,ignore
{{#include ../../intro/dma/src/main.rs:transfer}}
```

The `data` array in this case serves as the data to transmit as well as the buffer to receive data.

✅ First thing we need to use DMA is initializing the DMA peripheral driver and getting a channel.

We also need to create a buffer for data we want to send as well as a separate buffer for the data we will receive.

```rust,ignore
{{#include ../../intro/dma/examples/dma.rs:init-dma}}
```

There are also descriptors needed. That is because internally the DMA peripheral uses a [linked list] for the transfer and that is what the descriptors are needed for.
For convenience we use the `dma_buffers!` macro to create the buffers and descriptors.

> 🔎 You could use `cargo expand` or Rust Analyzer's `Expand macro recursively` command to see what code the macro expands to

✅ Next we need to configure the SPI peripheral driver to use DMA

We need to call `.with_dma` passing a _configured_ DMA channel.
To configure a DMA channel we call `configure` to enable burst-mode, pass the descriptors and set the priority.

```rust,ignore
{{#include ../../intro/dma/examples/dma.rs:configure-spi}}
```

✅ Now we are ready to start a DMA enabled SPI transfer

Now we need to pass the buffers to transmit and receive individually. Please note that we now get a `DMA transfer` from calling `transmit`.

```rust,ignore
{{#include ../../intro/dma/examples/dma.rs:transfer}}
```

What happens here is that the buffers and the SPI driver are moved into the `Transfer` we get. This way the buffers and the driver are inaccessible during the transfer.

Now we are free to let the CPU do other things while the SPI transfer is in progress.

✅ Wait for the transfer to complete and get back the buffers and the driver instance

As mentioned before the buffers and the driver are moved into the `Transfer`.

```rust,ignore
{{#include ../../intro/dma/examples/dma.rs:transfer-wait}}
```

We call `wait` on the `Transfer`. It will block until the transfer is done and we get our SPI driver and the buffers.

While using DMA needs more effort than letting the CPU do all the work it's not too complex once the general idea is understood.

> ⚠️ You might assume that it's always preferable to use DMA.
>
> That's not the case: Setting up a DMA transfer consumes more CPU cycles than setting up a blocking transfer.
> Especially if the amount of data to transfer is small this might hurt performance a lot. Also, if there is nothing to do for the CPU other than waiting for the transfer to finish
> it's preferable to use a blocking transfer.
>
> However, if the amount of data to transfer is bigger, then the time needed to setup the transfer is negligible compared to the time the CPU could use to do useful things in parallel.

[SPI]: https://en.wikipedia.org/wiki/Serial_Peripheral_Interface
[linked list]: https://www.espressif.com/sites/default/files/documentation/esp32-c3_technical_reference_manual_en.pdf#page=59

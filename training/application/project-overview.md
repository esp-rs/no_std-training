# Project Overview

In this training we will be building a simple data-logger application. The application will connect to a Wi-Fi network, read data from a sensor, and send the data to an MQTT broker.

The intent of this training is to produce an application which resembles a "real-world" project, covering what we consider to be some essential topics, without requiring the user to write thousands of lines of code before arriving at a finished product.

- First, in [Project Setup](./project-setup.md), we will guide you through the process of initializing a project. We will generate a new skeleton project and explain its components.
- In [Reading Sensor Data](./reading-sensor-data.md), we will start with device initialization, so that our board is properly configured and ready to use moving forward. Next, we read data from a connected temperature/humidity sensor over the I2C bus.
- In [Wi-Fi Connectivity](./wifi-connectivity.md), we will cover the basics of connecting to a local Wi-Fi network and establishing a connection to the Internet.
- With a sensor and Wi-Fi initialized, in [Publishing Data to an MQTT Broker](./publishing-data.md) we will connect to an MQTT broker and send our sensor data to the broker.
- In [Wi-Fi Provisioning](./wifi-provisioning.md), we will step back to the topic of Wi-Fi and demonstrate how to set up Wi-Fi provisioning, allowing the device to be connected to arbitrary networks without requiring the network credentials to be hard-coded in the firmware.
- Finally, we will explain how to perform update devices in the field using [Over-the-Air Updates](./ota-updates.md).

# HTTP Client
Next, we'll write a small client that retrieves data over an HTTP connection to the internet.

## Setup

✅ Go to `intro/http-client` directory.

✅ Open the prepared project skeleton in `intro/http-client`.

✅ Add your network credentials: Set the  `SSID` and `PASSWORD` environment variables-

## Making a connection

To use Wi-Fi, we first need to bump the frequency at which the target operates to its maximum.

Then we need to create a timer and initialize the Wi-Fi. After Wi-Fi is initialized, we will proceed with the configuration. We will be using Station Mode and use passing the Wi-Fi credentials.

Once we have configured Wi-Fi properly, we scan the available networks, and we try to connect to the one we setted. If the connection succeds, we proceed with the last part, making the HTTP request.

By default, only unencrypted HTTP is available, which rather limits our options of hosts to connect to. We're going to use `www.mobile-j.de/`.

To make an HTTP request, we first need to open a socket, and write to it the GET request, then we wait for the reponse and read it out.


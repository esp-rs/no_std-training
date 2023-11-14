#![no_std]
#![no_main]

use hal::{clock::ClockControl, peripherals::Peripherals, prelude::*, systimer::SystemTimer, Rng};

use embedded_io::blocking::*;
use embedded_svc::{
    ipv4::Interface,
    wifi::{AccessPointInfo, ClientConfiguration, Configuration, Wifi},
};

use esp_backtrace as _;
use esp_println::{print, println};
use esp_wifi::{
    current_millis, initialize,
    wifi::{utils::create_network_interface, WifiError, WifiMode},
    wifi_interface::WifiStack,
    EspWifiInitFor,
};
use smoltcp::{
    iface::SocketStorage,
    wire::{IpAddress, Ipv4Address},
};

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    // Set clocks at maximum frequency
    let clocks = ClockControl::max(system.clock_control).freeze();

    // Initialize the timers used for Wifi
    // ANCHOR: wifi_init
    let timer = SystemTimer::new(peripherals.SYSTIMER).alarm0;
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(peripherals.RNG),
        system.radio_clock_control,
        &clocks,
    )
    .unwrap();
    // ANCHOR_END: wifi_init

    // Configure Wifi
    // ANCHOR: wifi_config
    let (wifi, _) = peripherals.RADIO.split();
    let mut socket_set_entries: [SocketStorage; 3] = Default::default();
    let (iface, device, mut controller, sockets) =
        create_network_interface(&init, wifi, WifiMode::Sta, &mut socket_set_entries).unwrap();
    let wifi_stack = WifiStack::new(iface, device, sockets, current_millis);
    // ANCHOR_END: wifi_config
    // ANCHOR: client_config_start
    let client_config = Configuration::Client(ClientConfiguration {
        // ANCHOR_END: client_config_start
        ssid: SSID.into(),
        password: PASSWORD.into(),
        ..Default::default() // ANCHOR: client_config_end
    });

    let res = controller.set_configuration(&client_config);
    println!("Wi-Fi set_configuration returned {:?}", res);
    // ANCHOR_END: client_config_end

    // ANCHOR: wifi_connect
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
    // ANCHOR_END: wifi_connect

    // ANCHOR: ip
    // Wait for getting an ip address
    println!("Wait to get an ip address");
    loop {
        wifi_stack.work();

        if wifi_stack.is_iface_up() {
            println!("got ip {:?}", wifi_stack.get_ip_info());
            break;
        }
    }
    // ANCHOR_END: ip

    println!("Start busy loop on main");

    let mut rx_buffer = [0u8; 1536];
    let mut tx_buffer = [0u8; 1536];
    let mut socket = wifi_stack.get_socket(&mut rx_buffer, &mut tx_buffer);

    loop {
        println!("Making HTTP request");
        socket.work();

        socket
            .open(IpAddress::Ipv4(Ipv4Address::new(142, 250, 185, 115)), 80)
            .unwrap();

        socket
            .write(b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n")
            .unwrap();
        socket.flush().unwrap();

        // ANCHOR: reponse
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
        // ANCHOR_END: reponse

        // ANCHOR: socket_close
        socket.disconnect();

        let wait_end = current_millis() + 5 * 1000;
        while current_millis() < wait_end {
            socket.work();
        }
        // ANCHOR_END: socket_close
    }
}

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output};
use esp_hal::peripherals::Peripherals;
use esp_hal::time::{now, Instant};
use esp_hal::prelude::*;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use log::info;
use esp_wifi::{self, wifi::{WifiController,Configuration,AccessPointConfiguration, utils::create_ap_sta_network_interface}};
use smoltcp::iface::{SocketSet, SocketStorage};
use smoltcp::socket::udp::{PacketBuffer, PacketMetadata};
use smoltcp::wire::{IpCidr, Ipv4Address, Ipv4Cidr, UdpPacket};
use smoltcp_nal::embedded_nal::{UdpClientStack, UdpFullStack};
use smoltcp_nal::NetworkStack;
use embedded_time::rate::Fraction;

struct ScuffedClock {
    pub now: fn() -> Instant
}

const ONE_SECOND_IN_TICKS: u32 = esp_hal::time::Duration::secs(1).ticks() as u32;
impl embedded_time::Clock for ScuffedClock {
    type T = u32;
    
    const SCALING_FACTOR: Fraction = Fraction::new(1, ONE_SECOND_IN_TICKS); // knas

    fn try_now(&self) -> Result<embedded_time::Instant<Self>, embedded_time::clock::Error> {
        Ok(embedded_time::Instant::new((self.now)().ticks() as u32))
    }
}

#[entry]
fn main() -> ! {
    esp_alloc::heap_allocator!(72 * 1024);

    esp_println::logger::init_logger_from_env();
    let peripherals: Peripherals = esp_hal::init({
        let mut config: esp_hal::Config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let esp_wifi_controller = esp_wifi::init(timg0.timer0, Rng::new(peripherals.RNG), peripherals.RADIO_CLK).unwrap();
    
    
    let interface = create_ap_sta_network_interface(&esp_wifi_controller, peripherals.WIFI).unwrap();
    let mut controller: WifiController<'_> = interface.controller;

    let ap_conf: AccessPointConfiguration = AccessPointConfiguration {
        ssid: heapless::String::<32>::try_from("skogaholmslimpa").unwrap(),
        auth_method: esp_wifi::wifi::AuthMethod::WPA,
        password: heapless::String::<64>::try_from("billigost").unwrap(),
        ..AccessPointConfiguration::default()
    };
    
    controller.set_configuration(&Configuration::AccessPoint(ap_conf)).expect("Failed to set the wifi configuration...");
    controller.start().expect("Welp, could not start the wifi controller...");

    
    // STACK
    let device = interface.ap_device;
    let interface = interface.ap_interface;
    
    // Sockets
    let mut socket_storage: [SocketStorage;3] = Default::default();
    let mut socket_set: SocketSet = SocketSet::new(&mut socket_storage[..]);
    let mut rx_meta_storage: [PacketMetadata; 500] = [PacketMetadata::EMPTY; 500];
    let mut rx_payload_storage: [u8; 1337] = [44; 1337];
    let rx_buffer: PacketBuffer = PacketBuffer::new(&mut rx_meta_storage[..], &mut rx_payload_storage[..]);
    let mut tx_meta_storage: [PacketMetadata; 500] = [PacketMetadata::EMPTY; 500];
    let mut tx_payload_storage: [u8; 1337] = [44; 1337];
    let tx_buffer= PacketBuffer::new(&mut tx_meta_storage[..], &mut tx_payload_storage[..]);
    socket_set.add(smoltcp::socket::udp::Socket::new(rx_buffer, tx_buffer));

    let bad_clock: ScuffedClock = ScuffedClock { now };
    let mut stack = NetworkStack::new(interface, device, socket_set, bad_clock);

    let best_ipv4_address: Ipv4Cidr = Ipv4Cidr::new(Ipv4Address::new(69, 69, 69, 69), 24);
    stack.interface_mut().update_ip_addrs(|v|  v.push(IpCidr::Ipv4(best_ipv4_address)).expect("Could not add IP address."));
    let mut udp_sock = stack.socket().expect("Our puny attempt at making a UDP socket failed :/");
    stack.bind(&mut udp_sock, 67).expect("Could not bind the UDP socket to port 67.");
    

    let mut led: Output = Output::new(peripherals.GPIO21, Level::Low);
    let delay: Delay = Delay::new();
    println!("The setup didn't crash! Starting main loop...");
    loop {
        let poll_result = stack.poll().expect("Even polling the stack failed!");

        if poll_result {
            let mut buffer: [u8; 500] = [0; 500];
            let potential_datagram = stack.receive(&mut udp_sock, &mut buffer);
            match potential_datagram {
                Ok((size, remote)) => {
                    println!("Datagram of size {size} from {remote}! Data: ",);
                    println!(" {:x?}", buffer);
                    //println!(" {}", str::from_utf8(&buffer).expect("Data cannot be converted to ascii :("));
                    // let mut datagram = UdpPacket::new_checked(buffer).expect("Got a datagram that was seemingly not UDP");
                    // println!("{:?}", datagram.payload_mut());
                },
                _ => ()
            }
            
        }

        delay.delay(1000.millis());
        led.toggle();
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/v0.22.0/examples/src/bin
}

fn _print_access_points(controller: &mut WifiController<'_>) {
    let found_in_scan = controller.scan_n::<55>().expect("Scan for access points failed.");
    for ap in found_in_scan.0 {
        info!("Found: {} | Signal strength: {}", ap.ssid, ap.signal_strength);
    }
}
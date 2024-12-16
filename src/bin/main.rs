#![no_std]
#![no_main]

extern crate alloc;

use core::net::Ipv4Addr;
use alloc::boxed::Box;

use embassy_net::{tcp::TcpSocket, Config, IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{prelude::*,delay::Delay, gpio::{Level,Output}, peripherals::Peripherals, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_wifi::wifi::{WifiApDevice, WifiDevice};
use esp_wifi::{self, wifi::{Configuration,AccessPointConfiguration}};
use esp_hal_dhcp_server::{structs::DhcpServerConfig, simple_leaser::SingleDhcpLeaser};

// Get it? Because it's just running all the time :D
#[embassy_executor::task]
async fn marathon (mut runner: Runner<'static, WifiDevice<'static, WifiApDevice>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn dhcp_server(stack: Stack<'static>) {
    let config = DhcpServerConfig {
        ip: Ipv4Addr::new(192, 168, 1, 1),
        lease_time: Duration::from_secs(1800),
        gateways: &[],
        subnet: None,
        dns: &[]
    };

    let mut leaser = SingleDhcpLeaser::new(Ipv4Addr::new(192, 168, 1, 111));

    esp_hal_dhcp_server::run_dhcp_server(stack, config, &mut leaser).await;
}

#[embassy_executor::task]
async fn tcp_test(stack: Stack<'static>) {
    let mut tcp_rx_buffer = [0u8;500];
    let mut tcp_tx_buffer = [0u8;500];
    let mut tcp_socket: TcpSocket<'_> = TcpSocket::new(stack, &mut tcp_rx_buffer, &mut tcp_tx_buffer);
    tcp_socket.accept(IpListenEndpoint{addr: None, port: 80}).await.expect("Failed to start listening on tcp port.");

    let mut buf = [0; 1024];
        loop {
            let size_received = tcp_socket.read(&mut buf).await;
            match size_received {
                Ok(0) => panic!("Our tcp socket was closed while we expected to read from it!"),
                Ok(n) => println!("received {n} from! I have no clue where from!"),
                Err(e) => panic!("{:?}", e),
            }
        }
}

#[main]
async fn main(spawner: embassy_executor::Spawner) -> ! {
    // Initialize heap, logger, and peripherals.
    esp_alloc::heap_allocator!(72 * 1024);
    esp_println::logger::init_logger_from_env();
    let peripherals: Peripherals = esp_hal::init({
        let mut config: esp_hal::Config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });
    
    // Initialize embassy so async works at all.
    let timg1 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timg1.timer0);

    // Network stack setup.
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let esp_wifi_controller: Box<esp_wifi::EspWifiController<'static>> = Box::new(esp_wifi::init(
        timg0.timer0, 
        Rng::new(peripherals.RNG), 
        peripherals.RADIO_CLK
    ).unwrap());
    let esp_wifi_controller = Box::leak(esp_wifi_controller);
    let (device, mut controller) = esp_wifi::wifi::new_with_mode(esp_wifi_controller, peripherals.WIFI, WifiApDevice).unwrap();
    let stack_conf = Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Addr::new(192, 168, 1, 1), 24),
        gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
        dns_servers: Default::default()
    });
    
    let boxed_resources = Box::new(StackResources::<3>::new());
    let leaked_resources = Box::leak(boxed_resources);
    let random_seed = 687486766; // Extremely secure!
    let (stack, runner) = embassy_net::new(device, stack_conf, leaked_resources, random_seed);
    
    
    


    // Access point configuration and startup.
    let ap_conf: AccessPointConfiguration = AccessPointConfiguration {
        ssid: heapless::String::<32>::try_from("jultomten").unwrap(),
        auth_method: esp_wifi::wifi::AuthMethod::WPA,
        password: heapless::String::<64>::try_from("skorsten").unwrap(),
        ..AccessPointConfiguration::default()
    };
    println!("Starting the wifi access point!");
    controller.set_configuration(&Configuration::AccessPoint(ap_conf)).expect("Failed to set the wifi configuration...");
    controller.start().expect("Welp, could not start the wifi controller...");

    // Start running the stack concurrently.
    spawner.spawn(marathon(runner)).expect("Could not start stack runner task.");

    // Block until the stack is ready.
    loop {
        if stack.is_link_up() {
            break;
        }
        embassy_time::Timer::after(embassy_time::Duration::from_millis(500)).await;
    }
    
    println!("Starting dhcp server!");
    let s = stack.clone();
    spawner.spawn(dhcp_server(stack)).expect("Could not spawn dhcp server task.");

    println!("Starting tcp testing!");
    spawner.spawn(tcp_test(stack)).expect("Could not spawn tcp test task.");

    Timer::after(Duration::from_secs(120)).await;
    println!("Closing dhcp server after 2m...");
    esp_hal_dhcp_server::dhcp_close();

    // Blinky.
    let mut led: Output = Output::new(peripherals.GPIO21, Level::Low);
    let delay: Delay = Delay::new();
    println!("The setup didn't crash! Starting main loop...");
    loop {
        delay.delay(1000.millis());
        led.toggle();
    }
}
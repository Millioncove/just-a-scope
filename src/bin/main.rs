#![no_std]
#![no_main]

extern crate alloc;

use core::{ fmt::Debug, net::{Ipv4Addr, SocketAddrV4}};
use alloc::boxed::Box;

use edge_nal::TcpBind;
use embedded_io_async::{Read, Write};
use embassy_net::{Config, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{prelude::*, gpio::{Level,Output}, peripherals::Peripherals, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_wifi::wifi::{WifiApDevice, WifiDevice};
use esp_wifi::{self, wifi::{Configuration,AccessPointConfiguration}};
use edge_http::{io::{server::{self as http, Handler}, Error}, Method};
use edge_nal_embassy::TcpBuffers;
use embedded_websocket as ws;

const CONNECTION_TIMEOUT_MS: u32 = 30_000;

// Get it? Because it's just running all the time :D
#[embassy_executor::task]
async fn marathon (mut runner: Runner<'static, WifiDevice<'static, WifiApDevice>>) {
    runner.run().await
}

struct MyHttpHandler;

impl Handler for MyHttpHandler {
    type Error<T: Debug> = Error<T>;
    async fn handle<T, const N: usize>(
        &self,
        task_id: impl core::fmt::Display + Copy,
        conn: &mut http::Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write 
    {
        println!("Got a request! Task id: {task_id}");
        let request_headers = conn.headers()?;

        // Check if the request is a WebSocket handshake.
        let ws_headers = ws::read_http_header(
            request_headers.headers.iter().map(|h| (h.0, h.1.as_bytes()))
        );
        if let Ok(ws_headers) = ws_headers {
            if let Some(ws_context) = ws_headers {
                println!("WebSocket handshake! Subprotocols: {:?} | Secure Key: {}", ws_context.sec_websocket_protocol_list, ws_context.sec_websocket_key);

                return Ok(());
            }
        }

        // Handle non-WebSocket requests.
        if Method::Get != request_headers.method {
            conn.initiate_response(405, Some("Method Not Allowed."), &[]).await?;
        } else if "/" != request_headers.path {
            conn.initiate_response(404, Some("Only the '/' path works right now."), &[]).await?;
            conn.write_all(b"Could not find the resource.").await?;
        } else {
            conn.initiate_response(200, Some("OK"), &[("Content-Type", "text/html")]).await?;
            conn.write_all(include_bytes!("web_page.html")).await?;
        }

        Ok(())
    }
    
}

#[embassy_executor::task]
async fn http_server (stack: Stack<'static>, ip: Ipv4Addr) {
    let mut server = http::DefaultServer::new();

    let buffers: TcpBuffers<8, 500, 500> = TcpBuffers::new();
    let tcp_instance: edge_nal_embassy::Tcp<'_, 8, 500, 500> = edge_nal_embassy::Tcp::new(stack, &buffers);
    let http_socket = tcp_instance.bind(core::net::SocketAddr::V4(SocketAddrV4::new(ip, 80)))
    .await.expect("Failed to bind http socket.");
    

    server.run(Some(CONNECTION_TIMEOUT_MS), http_socket, MyHttpHandler).await.expect("Actually running the http server failed.");
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
    let boxed_resources = Box::new(StackResources::<16>::new());
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

    println!("Starting http server!");
    spawner.spawn(http_server(stack, Ipv4Addr::new(192, 168, 1, 1))).expect("Failed to spawn http server task.");

    // Blinky.
    let mut led: Output = Output::new(peripherals.GPIO21, Level::Low);
    println!("The setup didn't crash! Starting blink loop...");
    loop {
        Timer::after(Duration::from_millis(3000)).await;
        led.toggle();
    }
}
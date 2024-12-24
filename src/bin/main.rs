#![no_std]
#![no_main]

extern crate alloc;

use core::{ fmt::Debug, net::{Ipv4Addr, SocketAddr, SocketAddrV4}};
use alloc::boxed::Box;

use edge_nal::{TcpAccept, TcpBind};
use embedded_io_async::{Read, Write};
use embassy_net::{Config, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{gpio::{Level,Output}, peripherals::Peripherals, prelude::*, rng::Rng, time::now, timer::timg::TimerGroup};
use esp_println::println;
use esp_wifi::wifi::{AuthMethod, ClientConfiguration, WifiApDevice, WifiDevice, WifiStaDevice};
use esp_wifi::{self, wifi::{Configuration,AccessPointConfiguration}};
use edge_http::{io::{server::{self as http, Handler}, Error}, Method};
use edge_nal_embassy::{TcpBuffers, TcpError};
use embedded_websocket::{self as ws};
use heapless::String;
use websocket_logistics::{send_message, OscilliscopePoint};
use zerocopy::IntoBytes;

mod websocket_logistics {
    use embedded_io_async::{ErrorType, Write};
    use zerocopy::{FromBytes, Immutable, IntoBytes};

    #[derive(IntoBytes, FromBytes, Immutable)]
    #[repr(C)]
    pub struct OscilliscopePoint {
        pub voltage: f64,
        pub second: f64
    }

    pub async fn send_message<W>(to: &mut W, data: &[u8]) -> Result<(), <W as ErrorType>::Error> 
        where W: Write
    {
        let fin_rsv_opcode = 0b10000010u8; // FIN and binary data.
        let payload_length = data.len() as u8;
        if payload_length > 126 {panic!("Max payload length for a simple WebSocket message is 126 bytes.")}
        let header = [fin_rsv_opcode, payload_length];

        to.write_all(&[&header, data].concat()).await
    }
}

const CONNECTION_TIMEOUT_MS: u32 = 30_000;
const WEBSOCKET_PORT: u16 = 43822;
const AP_GATEWAY_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);
const AP_WEBSOCKET_ENDPOINT: SocketAddrV4 = SocketAddrV4::new(AP_GATEWAY_ADDRESS, WEBSOCKET_PORT);
const STA_GATEWAY_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);
const STA_STATIC_IP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 83);
const STA_WEBSOCKET_ENDPOINT: SocketAddrV4 = SocketAddrV4::new(STA_STATIC_IP_ADDRESS, WEBSOCKET_PORT);

// Get it?
#[embassy_executor::task]
async fn access_point_marathon (mut runner: Runner<'static, WifiDevice<'static, WifiApDevice>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn station_marathon (mut runner: Runner<'static, WifiDevice<'static, WifiStaDevice>>) {
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

#[embassy_executor::task(pool_size = 2)]
async fn http_server (stack: Stack<'static>, ip: Ipv4Addr) {
    let mut server = http::DefaultServer::new();

    let buffers: TcpBuffers<8, 500, 500> = TcpBuffers::new();
    let tcp_instance: edge_nal_embassy::Tcp<'_, 8, 500, 500> = edge_nal_embassy::Tcp::new(stack, &buffers);
    let http_socket = tcp_instance.bind(core::net::SocketAddr::V4(SocketAddrV4::new(ip, 80)))
    .await.expect("Failed to bind http socket.");

    server.run(Some(CONNECTION_TIMEOUT_MS), http_socket, MyHttpHandler).await.expect("Actually running the http server failed.");
}

#[embassy_executor::task(pool_size = 2)]
async fn web_socket_server(stack: Stack<'static>, address_and_port: SocketAddr) {
    'single_web_socket: loop {
        // Set up the underlying TCP socket and listen for a WebSocket handshake request.
        let buffers: TcpBuffers<2, 500, 500> = TcpBuffers::new();
        let tcp_instance: edge_nal_embassy::Tcp<'_, 2, 500, 500> = edge_nal_embassy::Tcp::new(stack, &buffers);
        let web_socket = tcp_instance.bind(address_and_port)
        .await.expect("Failed to bind WebSocket.");
        let (endpoint, mut web_socket) = web_socket.accept()
        .await.expect("Something went wrong when expecting a connection to the websocket.");

        // Something connected. Make sure it sent a http message. (WebSocket handshakes are http messages)
        let mut header_buffer = [0u8; 500];
        let mut bytes_read: usize = 0;
        loop {
            match web_socket.read(&mut header_buffer).await {
                Ok(0) => break,
                Ok(s) => bytes_read += s,
                Err(e) => {
                    println!("Received a garbled WebSocket handshake request. Error: {:?}", e);
                    continue 'single_web_socket
                }
            }
            if &header_buffer[bytes_read-4..bytes_read] == b"\r\n\r\n" {
                // Data is definitely a http message at least. 
                break;
            } else {
                println!("Received data, but it was not http (so also not a WebSocket handshake).");
            }
        }
        
        // Definitely received a http message at this point. Extract WebSocket handshake data from the headers.
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut request = httparse::Request::new(&mut headers);
        request.parse(&header_buffer).unwrap();
        let headers = request.headers.iter().map(|f| (f.name, f.value));
        let ws_context = ws::read_http_header(headers).unwrap().unwrap();

        // At this point we have a definite handshake request. Send a handshake approval response.
        let mut ws_server = ws::WebSocketServer::new_server();
        let mut handshake_approval = [0u8; 500];
        let len = ws_server.server_accept(
            &ws_context.sec_websocket_key, ws_context.sec_websocket_protocol_list.first(),
            &mut handshake_approval
        ).expect("Creating WebSocket handshake response failed.");
        web_socket.write_all(&handshake_approval[..len]).await.
            expect("Could not write the WebSocket handshake response to the socket.");

        println!("WebSocket connection should be successfully opened on {}.", endpoint);

        loop {
            // Send data to the WebSocket.
            let point = OscilliscopePoint {
                voltage: (now().ticks() as f64 / 1_000_000f64) % 5f64,
                second: (now().duration_since_epoch().to_micros() as f64) * 0.000001f64
            };
            
            match send_message(&mut web_socket, point.as_bytes()).await {
                Ok(_) => (),
                Err(TcpError::General(embassy_net::tcp::Error::ConnectionReset)) => {
                    println!("WebSocket connection was closed.");
                    break;
                },
                Err(e) => panic!("Sending WebSocket message failed: {e:?}")
            }

            Timer::after(Duration::from_millis(100)).await;
        }
    }
}

fn station_auth_method() -> AuthMethod {
    const AUTH_STR: &str = env!("station_auth_method");

    match env!("station_auth_method") {
        "none" => return AuthMethod::None,
        "wep" => { return AuthMethod::WEP; }
        "wpa" => { return AuthMethod::WPA; }
        "wapipersonal" => { return AuthMethod::WAPIPersonal; }
        "wpa2enterprise" => { return AuthMethod::WPA2Enterprise; }
        "wpa2personal" => { return AuthMethod::WPA2Personal; }
        "wpa2wpa3personal" => { return AuthMethod::WPA2WPA3Personal; }
        "wpa3personal" => { return AuthMethod::WPA3Personal; }
        "wpawpa2personal" => { return AuthMethod::WPAWPA2Personal; }
        _ => panic!("Configuration value for station auth method '{AUTH_STR}' is invalid.")
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

    // Set up wifi peripheral.
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let esp_wifi_controller: Box<esp_wifi::EspWifiController<'static>> = Box::new(esp_wifi::init(
        timg0.timer0, 
        Rng::new(peripherals.RNG), 
        peripherals.RADIO_CLK
    ).unwrap());
    let esp_wifi_controller = Box::leak(esp_wifi_controller);
    
    let (ap_device, sta_device, mut controller) = 
    esp_wifi::wifi::new_ap_sta(esp_wifi_controller, peripherals.WIFI).unwrap();
    //esp_wifi::wifi::new_with_mode(esp_wifi_controller, peripherals.WIFI, WifiApDevice).unwrap();

    // Station stack setup.
    let sta_stack_conf = Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(STA_STATIC_IP_ADDRESS, 24),
        gateway: Some(STA_GATEWAY_ADDRESS),
        dns_servers: Default::default()
    });
    let sta_resources = Box::leak(Box::new(StackResources::<16>::new()));
    let random_seed = 35181354; // Extremely secure!
    let (sta_stack, sta_runner) = embassy_net::new(sta_device, sta_stack_conf, sta_resources, random_seed);

    // Access point network stack setup.
    let ap_stack_conf = Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(AP_GATEWAY_ADDRESS, 24),
        gateway: Some(AP_GATEWAY_ADDRESS),
        dns_servers: Default::default()
    });
    let ap_resources = Box::leak(Box::new(StackResources::<16>::new()));
    let random_seed = 687486766; // Extremely secure!
    let (ap_stack, ap_runner) = embassy_net::new(ap_device, ap_stack_conf, ap_resources, random_seed);

    // Station configuration.
    let sta_conf: ClientConfiguration = ClientConfiguration { 
        ssid: String::<32>::try_from(env!("station_ssid")).expect("Station ssid has wrong format."),
        auth_method: station_auth_method(), 
        password: String::<64>::try_from(env!("station_password")).expect("Station password has wrong format."),
        ..ClientConfiguration::default()
    };

    // Access point configuration.
    let ap_conf: AccessPointConfiguration = AccessPointConfiguration {
        ssid: String::<32>::try_from("jultomten").unwrap(),
        auth_method: esp_wifi::wifi::AuthMethod::WPA,
        password: String::<64>::try_from("skorsten").unwrap(),
        ..AccessPointConfiguration::default()
    };
    println!("Starting the wifi access point, and trying to connect to '{}'", sta_conf.ssid);
    controller.set_configuration(&Configuration::Mixed(sta_conf, ap_conf)).expect("Failed to set the wifi configurations...");
    controller.start().expect("Welp, could not start the wifi controller...");

    // Start running the stacks concurrently.
    spawner.spawn(access_point_marathon(ap_runner)).expect("Could not start access point stack runner task.");
    spawner.spawn(station_marathon(sta_runner)).expect("Could not start station stack runner task.");

    // Block until the stack is ready.
    loop {
        if ap_stack.is_link_up() {
            break;
        }
        embassy_time::Timer::after(embassy_time::Duration::from_millis(500)).await;
    }

    // Start the servers!
    println!("Starting http servers!");
    spawner.spawn(http_server(ap_stack, AP_GATEWAY_ADDRESS)).expect("Failed to spawn access point http server task.");
    spawner.spawn(http_server(sta_stack, STA_STATIC_IP_ADDRESS)).expect("Failed to spawn station http server task.");
    println!("Starting WebSocket servers!");
    spawner.spawn(web_socket_server(ap_stack, SocketAddr::V4(AP_WEBSOCKET_ENDPOINT))).expect("Failed to spawn access point WebSocket server task.");
    spawner.spawn(web_socket_server(sta_stack, SocketAddr::V4(STA_WEBSOCKET_ENDPOINT))).expect("Failed to spawn station WebSocket server task.");

    // Blinky.
    let mut led: Output = Output::new(peripherals.GPIO21, Level::Low);
    println!("The setup didn't crash! Starting blink loop...");
    let mut mac =  [0u8; 6];
    esp_wifi::wifi::sta_mac(&mut mac);
    println!("My wifi MAC is {:x?}", mac);
    loop {
        // let scan = controller.scan_n::<15>().unwrap();
        // println!("I can find {} available access points.", scan.1);
        // for ap_info in scan.0 {
        //     println!(" - {}", ap_info.ssid);
        // }

        if !controller.is_connected().unwrap() {
            match controller.connect() {
                Ok(_) => {
                    match controller.is_connected() {
                        Ok(true) => println!("Connection to access point network established!"),
                        Ok(false) => println!("Unexpected error while trying to connect to access point."),
                    Err(_) => println!("Failed to connect to access point."),
                    }
                },
                Err(e) => println!("Failed when trying to connect: '{e:?}'")
            }
        }
        Timer::after(Duration::from_millis(7000)).await;
        led.toggle();
    }
}
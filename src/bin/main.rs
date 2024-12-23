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
use esp_wifi::wifi::{WifiApDevice, WifiDevice};
use esp_wifi::{self, wifi::{Configuration,AccessPointConfiguration}};
use edge_http::{io::{server::{self as http, Handler}, Error}, Method};
use edge_nal_embassy::{TcpBuffers, TcpError};
use embedded_websocket::{self as ws};
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
const GATEWAY_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);
const WEBSOCKET_PORT: u16 = 43822;
const WEBSOCKET_ENDPOINT: SocketAddrV4 = SocketAddrV4::new(GATEWAY_ADDRESS, WEBSOCKET_PORT);

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

#[embassy_executor::task]
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
        //web_socket.flush().await.expect("Flushing the WebSocket failed.");
        
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

    // Start the servers!
    println!("Starting http server!");
    spawner.spawn(http_server(stack, GATEWAY_ADDRESS)).expect("Failed to spawn http server task.");
    println!("Starting WebSocket server!");
    spawner.spawn(web_socket_server(stack, SocketAddr::V4(WEBSOCKET_ENDPOINT))).expect("Failed to spawn http server task.");

    // Blinky.
    let mut led: Output = Output::new(peripherals.GPIO21, Level::Low);
    println!("The setup didn't crash! Starting blink loop...");
    loop {
        Timer::after(Duration::from_millis(3000)).await;
        led.toggle();
    }
}
#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use embassy_net::IpEndpoint;
use embassy_net::Runner;
use embassy_net::udp::PacketMetadata;
use embassy_net::udp::UdpSocket;
use embassy_time::{Duration, Timer};
use esp_wifi::wifi::{WifiController, WifiDevice, WifiEvent, WifiState};
use j2::get_receiver;
use j2::sense_task;

// use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;

use esp_hal::clock::CpuClock;
use esp_hal::timer::{systimer::SystemTimer, timg::TimerGroup};
use esp_hal::{
    Async,
    spi::master::{Config as SpiConfig, Spi},
};
use esp_wifi::{
    EspWifiController,
    wifi::{ClientConfiguration, Configuration},
};

// use esp_wifi::ble::controller::BleConnector;
use rtt_target::rprintln;
use static_cell::StaticCell;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

const RX_BUFFER_SIZE: usize = 1024;
const TX_BUFFER_SIZE: usize = 1024;

static SSID: &str = env!("SSID");
static PASSWORD: &str = env!("PASSWORD");

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    // generator version: 0.5.0

    rtt_target::rtt_init_print!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 64 * 1024);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(#[unsafe(link_section = ".dram2_uninit")] size: 64 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);

    rprintln!("Embassy initialized!");

    #[allow(dead_code)]
    static QUAD_SPI: StaticCell<Mutex<NoopRawMutex, Spi<'static, Async>>> = StaticCell::new();
    #[allow(unused_variables)]
    let quad_spi = Spi::new(peripherals.SPI2, SpiConfig::default())
        .expect("Failed to initialize QuadSPI bus")
        .with_mosi(peripherals.GPIO14)
        .with_miso(peripherals.GPIO10)
        .with_sck(peripherals.GPIO15)
        .with_cs(peripherals.GPIO11)
        .with_sio2(peripherals.GPIO16)
        .with_sio3(peripherals.GPIO12)
        .with_dma(peripherals.DMA_CH0);

    let mut rng = esp_hal::rng::Rng::new(peripherals.RNG);
    let timer1 = TimerGroup::new(peripherals.TIMG0);
    let wifi_init = &*mk_static!(
        EspWifiController<'static>,
        esp_wifi::init(timer1.timer0, rng).expect("Failed to initialize WIFI/BLE controller")
    );
    let (mut wifi_controller, interfaces) = esp_wifi::wifi::new(&wifi_init, peripherals.WIFI)
        .expect("Failed to initialize WIFI controller");

    // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    // let transport = BleConnector::new(&wifi_init, peripherals.BT);
    // let _ble_controller = ExternalController::<_, 20>::new(transport);

    wifi_controller
        .set_power_saving(esp_wifi::config::PowerSaveMode::None)
        .unwrap();

    let wifi_interface = interfaces.sta;

    let config = embassy_net::Config::dhcpv4(Default::default());
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    let client_config = ClientConfiguration {
        ssid: SSID.into(),
        password: PASSWORD.into(),
        bssid: None,
        auth_method: esp_wifi::wifi::AuthMethod::WPA2Personal,
        channel: None,
    };

    let config = Configuration::Client(client_config);

    wifi_controller.set_configuration(&config).unwrap();

    spawner.spawn(connection(wifi_controller)).ok();
    spawner.spawn(net_task(runner)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    rprintln!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            rprintln!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    let _ = spawner.must_spawn(sense_task(
        peripherals.I2C0,
        peripherals.GPIO6,
        peripherals.GPIO7,
    ));

    let config = stack.config_v4().unwrap();
    let local_ip = config.address.address();
    let broadcast_ip = config.address.broadcast().unwrap();

    rprintln!("UDP broadcast task: Local IP: {:?}", local_ip);
    rprintln!("UDP broadcast task: Broadcast IP: {:?}", broadcast_ip);

    const BROADCAST_PORT: u16 = 8080;

    // RX buffers
    let mut rx_buffer: [u8; RX_BUFFER_SIZE] = [0; RX_BUFFER_SIZE];
    let mut rx_meta = [PacketMetadata::EMPTY; 16]; // Enough metadata slots for 16 incoming packets

    // TX buffers
    let mut tx_meta = [PacketMetadata::EMPTY; 16]; // Enough metadata slots for 16 incoming packets
    let mut tx_buffer: [u8; TX_BUFFER_SIZE] = [0; TX_BUFFER_SIZE];

    // Create a UDP socket with the correctly sized metadata and data buffers
    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    // let mut tx_buffer = [0; 128]; // Max UDP payload size can be larger, adjust as needed
    // let mut rx_buffer = [0; 128]; // Not strictly needed for sending, but good practice for socket

    // // Create a UDP socket. It takes mutable references to the send and receive buffers.
    // // Ensure these buffers are `static` or live long enough for the socket.
    // let mut socket = UdpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

    // Optional: Bind the socket to a local port.
    // If you don't bind, a random ephemeral port will be used.
    // This is the source port of your outgoing packets.
    let local_udp_port = 50000; // Choose any available local port
    socket.bind(local_udp_port).unwrap();
    rprintln!(
        "UDP broadcast task: UDP Socket bound to local port {}",
        local_udp_port
    );

    let mut data_rec = get_receiver().unwrap();
    loop {
        // Prepare your sensor data
        // For demonstration, let's just send a simple message with a timestamp
        let sensor_data = data_rec.get().await;
        let json_data = match serde_json::to_string(&sensor_data) {
            Ok(s) => s,
            Err(e) => {
                rprintln!("Failed to serialze data: {:?}", e);
                continue;
            }
        };
        let data_to_send = json_data.as_bytes();

        if data_to_send.len() > TX_BUFFER_SIZE {
            rprintln!("Data too large for transmit buffer!");
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        // Send the data to the broadcast address and target port
        let remote_endpoint = IpEndpoint::new(broadcast_ip.into(), BROADCAST_PORT);

        match socket.send_to(data_to_send, remote_endpoint).await {
            Ok(_) => {
                rprintln!(
                    "UDP broadcast task: Sent {} bytes to {:?}",
                    data_to_send.len(),
                    remote_endpoint
                );
            }
            Err(e) => {
                rprintln!("UDP broadcast task: Failed to send UDP packet: {:?}", e);
            }
        }

        // Wait for a bit before sending the next packet
        Timer::after(Duration::from_secs(5)).await; // Send every 5 seconds
    }
}

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    rprintln!("start connection task");
    rprintln!("Device capabilities: {:?}", controller.capabilities());

    loop {
        match esp_wifi::wifi::wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.into(),
                password: PASSWORD.into(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            rprintln!("Starting wifi");
            controller.start_async().await.unwrap();
            rprintln!("Wifi started!");
        }
        rprintln!("About to connect...");

        match controller.connect_async().await {
            Ok(_) => rprintln!("Wifi connected!"),
            Err(e) => {
                rprintln!("Failed to connect to wifi: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task(pool_size = 1)]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

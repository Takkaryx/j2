use crate::get_receiver;
use embassy_net::{
    udp::{PacketMetadata, UdpSocket},
    IpEndpoint, Runner, Stack, StackResources,
};
use embassy_time::{Duration, Timer};
use esp_hal::{
    peripherals::{RNG, TIMG0, WIFI},
    timer::timg::TimerGroup,
};
use esp_wifi::{
    wifi::{ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiState},
    EspWifiController,
};
use rtt_target::rprintln;

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const RX_BUFFER_SIZE: usize = 1024;
const TX_BUFFER_SIZE: usize = 1024;

static SSID: &str = env!("SSID");
static PASSWORD: &str = env!("PASSWORD");

pub fn config_wifi(
    r: RNG<'static>,
    timer: TIMG0<'static>,
    wifi: WIFI<'static>,
) -> (
    Stack<'static>,
    WifiController<'static>,
    Runner<'static, WifiDevice<'static>>,
) {
    let mut rng = esp_hal::rng::Rng::new(r);
    let timer1 = TimerGroup::new(timer);
    let wifi_init = &*mk_static!(
        EspWifiController<'static>,
        esp_wifi::init(timer1.timer0, rng).expect("Failed to initialize WIFI/BLE controller")
    );
    let (mut wifi_controller, interfaces) =
        esp_wifi::wifi::new(wifi_init, wifi).expect("Failed to initialize WIFI controller");

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
    (stack, wifi_controller, runner)
}

#[embassy_executor::task]
pub async fn udp_broadcast(stack: Stack<'static>) {
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
    let config = stack.config_v4().unwrap();
    let local_ip = config.address.address();
    let broadcast_ip = config.address.broadcast().unwrap();

    rprintln!("UDP broadcast task: Local IP: {:?}", local_ip);
    rprintln!("UDP broadcast task: Broadcast IP: {:?}", broadcast_ip);

    const BROADCAST_PORT: u16 = 8080;

    // RX buffers
    let mut rx_buffer: [u8; RX_BUFFER_SIZE] = [0; RX_BUFFER_SIZE];
    let mut rx_meta = [PacketMetadata::EMPTY; 16];

    // TX buffers
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buffer: [u8; TX_BUFFER_SIZE] = [0; TX_BUFFER_SIZE];

    // Create a UDP socket with the correctly sized metadata and data buffers
    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );

    let local_udp_port = 50000; // Choose any available local port
    socket.bind(local_udp_port).unwrap();
    rprintln!(
        "UDP broadcast task: UDP Socket bound to local port {}",
        local_udp_port
    );

    let mut data_rec = get_receiver().unwrap();
    loop {
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

        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    rprintln!("start connection task");
    rprintln!("Device capabilities: {:?}", controller.capabilities());

    loop {
        if esp_wifi::wifi::wifi_state() == WifiState::StaConnected {
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await;
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

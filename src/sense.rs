use embassy_sync::watch::DynReceiver;
use embassy_time::{Delay, Duration, Timer};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::peripherals::{GPIO6, GPIO7, I2C0};
use libscd::asynchronous::scd4x::Scd4x;
use rtt_target::rprintln;
use serde::Serialize;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Watch};

#[derive(Clone, Default, Serialize)]
pub struct SensorData {
    pub co2: u16,
    pub humidity: u16,
    pub temperature: u16,
    pub time: u64,
}

const C02_CONSUMERS: usize = 1;
static C02: Watch<CriticalSectionRawMutex, SensorData, C02_CONSUMERS> = Watch::new();

pub fn get_receiver() -> Option<DynReceiver<'static, SensorData>> {
    C02.dyn_receiver()
}

#[embassy_executor::task]
pub async fn sense_task(twi: I2C0<'static>, sda: GPIO6<'static>, scl: GPIO7<'static>) -> ! {
    let i2c = I2c::new(twi, I2cConfig::default())
        .expect("failed to begin I2C")
        .with_sda(sda)
        .with_scl(scl)
        .into_async();

    let mut c02_sensor = Scd4x::new(i2c, Delay);
    Timer::after_millis(50).await;

    _ = c02_sensor.stop_periodic_measurement();

    Timer::after_millis(100).await;

    rprintln!(
        "Sensor Serial number: {:?}",
        c02_sensor.serial_number().await
    );

    if let Err(e) = c02_sensor.start_periodic_measurement().await {
        rprintln!("Failed to start periodic measurement: {:?}", e);
    }

    let tx = C02.sender();
    loop {
        if c02_sensor.data_ready().await.unwrap() {
            let m = c02_sensor.read_measurement().await.unwrap();
            let current_time_ms = embassy_time::Instant::now().as_millis();
            rprintln!(
                "C02: {}, Humidity: {}, Temperature: {}",
                m.co2 as u16,
                m.humidity as u16,
                m.temperature as u16
            );
            tx.send(SensorData {
                co2: m.co2 as u16,
                humidity: m.humidity as u16,
                temperature: m.temperature as u16,
                time: current_time_ms,
            });
        };
        Timer::after(Duration::from_secs(1)).await;
    }
}

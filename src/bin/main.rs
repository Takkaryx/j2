#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use j2::config_wifi;
use j2::connection;
use j2::display_config;
use j2::net_task;
use j2::sense_task;
use j2::udp_broadcast;

use embassy_executor::Spawner;

use esp_hal::{clock::CpuClock, timer::systimer::SystemTimer};

use rtt_target::rprintln;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

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
    esp_alloc::heap_allocator!(#[unsafe(link_section = ".dram2_uninit")] size: 64 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);

    rprintln!("Embassy initialized!");

    let (stack, wifi_controller, runner) =
        config_wifi(peripherals.RNG, peripherals.TIMG0, peripherals.WIFI);

    spawner.spawn(connection(wifi_controller)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner.spawn(udp_broadcast(stack)).ok();

    spawner.must_spawn(sense_task(
        peripherals.I2C0,
        peripherals.GPIO6,
        peripherals.GPIO7,
    ));

    let disp = display_config(
        peripherals.GPIO14,
        peripherals.GPIO10,
        peripherals.GPIO15,
        peripherals.GPIO11,
        peripherals.GPIO16,
        peripherals.GPIO12,
        peripherals.DMA_CH0,
        peripherals.SPI2,
    );
}

use crate::rm690b0::*;

use esp_hal::peripherals::{DMA_CH0, GPIO10, GPIO11, GPIO12, GPIO14, GPIO15, GPIO16, SPI2};
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;

pub fn display_config(
    mosi: GPIO14,
    miso: GPIO10,
    sck: GPIO15,
    cs: GPIO11,
    sio2: GPIO16,
    sio3: GPIO12,
    dma: DMA_CH0,
    spi: SPI2,
) -> RM690B0 {
    let quad_spi = Spi::new(spi, SpiConfig::default().with_frequency(Rate::from_mhz(40)))
        .expect("Failed to initialize QuadSPI bus")
        .with_mosi(mosi)
        .with_miso(miso)
        .with_sck(sck)
        .with_cs(cs)
        .with_sio2(sio2)
        .with_sio3(sio3)
        .with_dma(dma);
}

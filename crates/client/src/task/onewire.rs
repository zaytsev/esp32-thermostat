use defmt::{debug, expect, info};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async,
    gpio::Pin,
    peripherals::RMT,
    rmt::{self, Rmt},
    time::Rate,
};
use esp_hal_rmt_onewire::{OneWire, OneWireConfigZST, Search};

use crate::{AppEvent, AppEventsSender};

pub type OneWireBus<'a> = OneWire<
    'a,
    OneWireConfigZST<
        rmt::Channel<Async, rmt::ConstChannelAccess<rmt::Rx, 2>>,
        rmt::Channel<Async, rmt::ConstChannelAccess<rmt::Tx, 0>>,
        rmt::ConstChannelAccess<rmt::Tx, 0>,
    >,
>;

pub fn init_bus<'d, P>(rmt: RMT<'d>, pin: P) -> OneWireBus<'d>
where
    P: Pin + 'd,
{
    let rmt = expect!(
        Rmt::new(rmt, Rate::from_mhz(80u32)),
        "Failed to initialize RMT"
    )
    .into_async();

    let one_wire_bus = expect!(
        OneWire::new(rmt.channel0, rmt.channel2, pin),
        "OneWire instance initialized"
    );

    one_wire_bus
}

#[embassy_executor::task]
pub async fn read_temperature(
    mut bus: OneWireBus<'static>,
    read_interval: Duration,
    app_events: AppEventsSender<'static>,
) {
    info!("Initializing temperature sensor...");
    loop {
        debug!("Resetting the bus");
        bus.reset().await.unwrap();

        debug!("Broadcasting a measure temperature command to all attached sensors");
        for a in [0xCC, 0x44] {
            bus.send_byte(a).await.unwrap();
        }

        let mut search = Search::new();
        debug!("Starting bus search");
        while let Ok(address) = search.next(&mut bus).await {
            // debug!("Reading device {:?}", address);
            bus.reset().await.unwrap();
            bus.send_byte(0x55).await.unwrap();
            bus.send_address(address).await.unwrap();
            bus.send_byte(0xBE).await.unwrap();
            let temp_low = bus
                .exchange_byte(0xFF)
                .await
                .expect("failed to get low byte of temperature");
            let temp_high = bus
                .exchange_byte(0xFF)
                .await
                .expect("failed to get high byte of temperature");
            let temp = fixed::types::I12F4::from_le_bytes([temp_low, temp_high]);
            let temp = f32::from(temp);
            debug!("Temp is: {=f32}", temp);
            app_events
                .send(AppEvent::TempSensorRead {
                    address: address.0,
                    value: temp,
                })
                .await;
        }
        debug!("Bus search finished");

        Timer::after(read_interval).await
    }
}

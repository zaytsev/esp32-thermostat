use defmt::{debug, expect, info, warn};
use embassy_time::{Duration, Timer};
use esp_hal::{gpio::Pin, peripherals::RMT, rmt::Rmt, time::Rate};
use esp_hal_rmt_onewire::{Address, OneWire, Search};
// use hectl_common::proto::{BatchSensorReading, SensorReading};

use crate::{AppEvent, AppEventsSender};

pub fn init_bus<'d, P>(rmt: RMT<'d>, pin: P) -> OneWire<'d>
where
    P: Pin + 'd,
{
    // let c0 = rmt.channel0;
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

#[repr(u8)]
enum Command {
    SearchRom = 0xf0,
    SkipRom = 0xcc,
    MatchRom = 0x55,

    ConvertTemp = 0x44,
    ReadScratchpad = 0xbe,
}

struct SensorReading {
    sensor_id: u64,
    value: f32,
}

const BATCH_SIZE: usize = 4;

#[embassy_executor::task]
pub async fn read_temperature(
    mut bus: OneWire<'static>,
    read_interval: Duration,
    app_events: AppEventsSender<'static>,
) {
    info!("Initializing temperature sensor...");
    loop {
        let mut readings: heapless::Vec<SensorReading, BATCH_SIZE> = heapless::Vec::new();
        match read_ds18b20(&mut bus, &mut readings).await {
            Ok(_) => {
                for r in readings {
                    app_events
                        .send(AppEvent::TempSensorRead {
                            address: r.sensor_id,
                            value: r.value,
                        })
                        .await;
                }
            }
            Err(esp_hal_rmt_onewire::Error::InputNotHigh) => {
                warn!("Error reading onewire sensors: input not high");
            }
            Err(esp_hal_rmt_onewire::Error::ReceiveTimedOut) => {
                warn!("Error reading onewire sensors: receive timed out");
            }
            Err(esp_hal_rmt_onewire::Error::ReceiveError(e)) => {
                warn!("Error reading onewire sensors: receive error: {:?}", e);
            }
            Err(esp_hal_rmt_onewire::Error::SendError(e)) => {
                warn!("Error reading onewire sensors: send error: {:?}", e);
            }
        }

        Timer::after(read_interval).await
    }
}

async fn read_ds18b20(
    bus: &mut OneWire<'static>,
    readings: &mut heapless::Vec<SensorReading, BATCH_SIZE>,
) -> Result<(), esp_hal_rmt_onewire::Error> {
    bus.reset().await?;

    // SEARCH FIRST
    let mut search = Search::new();
    debug!("Searching for connected temperature sensors");
    readings.truncate(0);

    // Collect addresses first
    while let Ok(address) = search.next(bus).await {
        debug!("Found sensor {:?}", address.0);
        if readings
            .push(SensorReading {
                sensor_id: address.0,
                value: 0f32,
            })
            .is_err()
        {
            warn!("Number of attached sensors exceed batch size");
        }
    }
    debug!("Bus search finished, found {} sensors", readings.len());

    if !readings.is_empty() {
        // NOW trigger conversion for all sensors at once
        bus.reset().await?;
        bus.send_byte(0xCC).await?; // SKIP ROM (all devices)
        bus.send_byte(0x44).await?; // CONVERT_T

        // Wait for conversion to complete
        Timer::after(Duration::from_millis(750)).await; // Or poll the bus

        // Read temperatures from each sensor
        for sensor_reading in readings.iter_mut() {
            debug!("Reading sensor {:?}", sensor_reading.sensor_id);
            bus.reset().await?;
            bus.send_byte(0x55).await?; // MATCH ROM
            bus.send_address(Address(sensor_reading.sensor_id)).await?;
            bus.send_byte(0xBE).await?; // READ SCRATCHPAD

            let temp_low = bus.exchange_byte(0xFF).await?;
            let temp_high = bus.exchange_byte(0xFF).await?;
            let temp = fixed::types::I12F4::from_le_bytes([temp_low, temp_high]);
            let temp = f32::from(temp);
            debug!("Temp is: {=f32}", temp);

            sensor_reading.value = temp;
        }
    }

    Ok(())
}

// use defmt::{debug, expect, info};
// use embassy_time::{Duration, Timer};
// use esp_hal::{
//     Async,
//     gpio::Pin,
//     peripherals::RMT,
//     rmt::{self, Rmt},
//     time::Rate,
// };
// use esp_hal_rmt_onewire::{OneWire, OneWireConfigZST, Search};

// use crate::{AppEvent, AppEventsSender};

// pub type OneWireBus<'a> = OneWire<
//     'a,
//     OneWireConfigZST<
//         rmt::Channel<Async, rmt::ConstChannelAccess<rmt::Rx, 2>>,
//         rmt::Channel<Async, rmt::ConstChannelAccess<rmt::Tx, 0>>,
//         rmt::ConstChannelAccess<rmt::Tx, 0>,
//     >,
// >;

// pub fn init_bus<'d, P>(rmt: RMT<'d>, pin: P) -> OneWireBus<'d>
// where
//     P: Pin + 'd,
// {
//     let rmt = expect!(
//         Rmt::new(rmt, Rate::from_mhz(80u32)),
//         "Failed to initialize RMT"
//     )
//     .into_async();

//     let one_wire_bus = expect!(
//         OneWire::new(rmt.channel0, rmt.channel2, pin),
//         "OneWire instance initialized"
//     );

//     one_wire_bus
// }

// #[embassy_executor::task]
// pub async fn read_temperature(
//     mut bus: OneWireBus<'static>,
//     read_interval: Duration,
//     app_events: AppEventsSender<'static>,
// ) {
//     info!("Initializing temperature sensor...");
//     loop {
//         debug!("Resetting the bus");
//         bus.reset().await.unwrap();

//         debug!("Broadcasting a measure temperature command to all attached sensors");
//         for a in [0xCC, 0x44] {
//             bus.send_byte(a).await.unwrap();
//         }

//         let mut search = Search::new();
//         debug!("Starting bus search");
//         while let Ok(address) = search.next(&mut bus).await {
//             // debug!("Reading device {:?}", address);
//             bus.reset().await.unwrap();
//             bus.send_byte(0x55).await.unwrap();
//             bus.send_address(address).await.unwrap();
//             bus.send_byte(0xBE).await.unwrap();
//             let temp_low = bus
//                 .exchange_byte(0xFF)
//                 .await
//                 .expect("failed to get low byte of temperature");
//             let temp_high = bus
//                 .exchange_byte(0xFF)
//                 .await
//                 .expect("failed to get high byte of temperature");
//             let temp = fixed::types::I12F4::from_le_bytes([temp_low, temp_high]);
//             let temp = f32::from(temp);
//             debug!("Temp is: {=f32}", temp);
//             app_events
//                 .send(AppEvent::TempSensorRead {
//                     address: address.0,
//                     value: temp,
//                 })
//                 .await;
//         }
//         debug!("Bus search finished");

//         Timer::after(read_interval).await
//     }
// }

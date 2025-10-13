#![no_std]
#![no_main]

use defmt::{debug, error, info};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net::{HardwareAddress, Stack, StackResources};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Duration;
use esp_backtrace as _;
use esp_bootloader_esp_idf::partitions;
use esp_hal::{
    clock::CpuClock,
    config::WatchdogConfig,
    gpio::{Level, Output, OutputConfig},
    rng::Trng,
    timer::timg::TimerGroup,
};
use esp_storage::FlashStorage;
use esp_wifi::{EspWifiController, init};
use esp32_thermostat_common::proto::HeaterParams;

mod config;
mod task;

const WIFI_RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
const TEMPERATURE_READING_INTERVAL: Duration = Duration::from_secs(30);

use crate::task::{heater, telemetry};

esp_bootloader_esp_idf::esp_app_desc!();

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
const SERVER_PORT: u16 = esp_config::esp_config_int!(u16, "SERVER_PORT");
const SHARED_SECRET: &str = env!("NOISE_SECRET");

#[derive(Clone, defmt::Format)]
enum AppEvent {
    WifiConnected,
    WifiDisconnected,
    TempSensorRead { address: u64, value: f32 },
    HeaterEnabled(bool),
    UpdateHeaterParams(HeaterParams),
}

const APP_EVENT_CHANNEL_CAPACITY: usize = 4;
type AppEventsChannel =
    embassy_sync::channel::Channel<CriticalSectionRawMutex, AppEvent, APP_EVENT_CHANNEL_CAPACITY>;
type AppEventsSender<'a> = embassy_sync::channel::Sender<
    'a,
    CriticalSectionRawMutex,
    AppEvent,
    APP_EVENT_CHANNEL_CAPACITY,
>;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default()
        .with_cpu_clock(CpuClock::max())
        .with_watchdog(WatchdogConfig::default().with_timg0(
            esp_hal::config::WatchdogStatus::Enabled(
                esp_hal::time::Duration::from_millis(TEMPERATURE_READING_INTERVAL.as_millis()) * 3,
            ),
        ));
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let mut wdt = timg0.wdt;
    let trng = Trng::new(peripherals.RNG, peripherals.ADC1);
    let mut rng = trng.rng;

    let esp_wifi_ctrl = &*mk_static!(
        EspWifiController<'static>,
        defmt::unwrap!(init(timg0.timer0, rng.clone()))
    );
    let (controller, interfaces) = esp_wifi::wifi::new(esp_wifi_ctrl, peripherals.WIFI).unwrap();

    let wifi_interface = interfaces.sta;

    cfg_if::cfg_if! {
        if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.TIMG1);
            esp_hal_embassy::init(timg1.timer0);
        } else {
            use esp_hal::timer::systimer::SystemTimer;
            let systimer = SystemTimer::new(peripherals.SYSTIMER);
            esp_hal_embassy::init(systimer.alarm0);
        }
    }

    let config = embassy_net::Config::dhcpv4(Default::default());

    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        mk_static!(StackResources<2>, StackResources::<2>::new()),
        seed,
    );

    let mut flash = FlashStorage::new();
    let mut pt_mem = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let partition_table = partitions::read_partition_table(&mut flash, &mut pt_mem).unwrap();

    let config_storage = partition_table
        .find_partition(partitions::PartitionType::Data(
            partitions::DataPartitionSubType::Nvs,
        ))
        .unwrap()
        .unwrap();
    let mut config_storage = config_storage.as_embedded_storage(&mut flash);
    let mut config = config::Config::load(&mut config_storage);

    let app_events = mk_static!(AppEventsChannel, AppEventsChannel::new());
    let heater_control = mk_static!(heater::ControlChannel, heater::ControlChannel::new());
    let telemetry_control = mk_static!(telemetry::ControlChannel, telemetry::ControlChannel::new());

    spawner.must_spawn(task::network::run(runner));
    spawner.must_spawn(task::wifi::connect(
        controller,
        SSID,
        PASSWORD,
        WIFI_RECONNECT_INTERVAL,
        app_events.sender(),
    ));
    spawner.must_spawn(task::onewire::read_temperature(
        task::onewire::init_bus(peripherals.RMT, peripherals.GPIO0), // onewire_bus,
        TEMPERATURE_READING_INTERVAL,
        app_events.sender(),
    ));
    spawner.must_spawn(heater::run(
        config.heater.clone(),
        Output::new(peripherals.GPIO1, Level::High, OutputConfig::default()),
        heater_control.receiver(),
        app_events.sender(),
    ));

    let sender_id = get_node_id(stack);

    spawner.must_spawn(task::telemetry::run(
        sender_id,
        telemetry::PresharedKey::from(SHARED_SECRET),
        telemetry::ConnectionOptions {
            server_port: SERVER_PORT,
            ..Default::default()
        },
        stack,
        trng,
        telemetry_control.receiver(),
        app_events.sender(),
    ));

    let mut latest_temp = 0.0f32;
    let mut latest_heater_status = false;
    let heater_control = heater_control.sender();
    let telemetry_control = telemetry_control.sender();

    loop {
        match app_events.receive().await {
            AppEvent::WifiConnected => {}
            AppEvent::WifiDisconnected => {}
            AppEvent::TempSensorRead { value, .. } => {
                debug!("Received temperature sensor data: {=f32}", value);
                latest_temp = value;
                let _ = embassy_futures::join::join(
                    heater_control.send(heater::ControlEvent::CurrentTempUpdated(value)),
                    telemetry_control.send(telemetry::ControlMessage::SendMetrics {
                        inside_temp: latest_temp,
                        heater_enabled: latest_heater_status,
                    }),
                )
                .await;
                wdt.feed();
            }
            AppEvent::HeaterEnabled(enabled) => {
                latest_heater_status = enabled;
                telemetry_control
                    .send(telemetry::ControlMessage::SendMetrics {
                        inside_temp: latest_temp,
                        heater_enabled: latest_heater_status,
                    })
                    .await;
            }
            AppEvent::UpdateHeaterParams(params) => {
                debug!("Updating heater params {:?}", params);

                config.heater.target_temp = params.target_temp;
                config.heater.temp_tolerance = params.temp_tolerance;
                match config.save(&mut config_storage) {
                    Ok(_) => {
                        debug!("Config updated with heater params {:?}", params);
                    }
                    Err(e) => {
                        error!("Failed to persist config: {:?}", e);
                    }
                }

                heater_control
                    .send(heater::ControlEvent::ParamsUpdated(params))
                    .await;
            }
        }
    }
}

pub fn get_node_id(stack: Stack<'static>) -> u64 {
    match stack.hardware_address() {
        HardwareAddress::Ethernet(mac) => {
            let bytes = &mac.0;
            (bytes[0] as u64) << 40
                | (bytes[1] as u64) << 32
                | (bytes[2] as u64) << 24
                | (bytes[3] as u64) << 16
                | (bytes[4] as u64) << 8
                | bytes[5] as u64
        }
    }
}

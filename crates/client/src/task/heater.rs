use defmt::info;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use esp_hal::gpio::{Level, Output};
use esp32_thermostat_common::proto::HeaterParams;

use crate::{AppEvent, AppEventsSender, config::HeaterConfig};

const HEATER_ON: Level = Level::Low;
const HEATER_OFF: Level = Level::High;

pub enum ControlEvent {
    CurrentTempUpdated(f32),
    ParamsUpdated(HeaterParams),
}

pub type ControlChannel = embassy_sync::channel::Channel<CriticalSectionRawMutex, ControlEvent, 4>;
// pub type ControlChannelSender<'a> =
//     embassy_sync::channel::Sender<'a, CriticalSectionRawMutex, ControlEvent, 4>;
pub type ControlChannelReceiver<'a> =
    embassy_sync::channel::Receiver<'a, CriticalSectionRawMutex, ControlEvent, 4>;

#[embassy_executor::task]
pub async fn run(
    config: HeaterConfig,
    mut control_pin: Output<'static>,
    event_source: ControlChannelReceiver<'static>,
    app_events: AppEventsSender<'static>,
) {
    info!("Starting heater controller");

    control_pin.set_level(HEATER_OFF);
    app_events.send(AppEvent::HeaterEnabled(false)).await;

    let mut lo_temp = config.target_temp - config.temp_tolerance;
    let mut hi_temp = config.target_temp + config.temp_tolerance;
    let mut current_temp = None;

    loop {
        match event_source.receive().await {
            ControlEvent::CurrentTempUpdated(temp) => {
                current_temp = Some(temp);
            }
            ControlEvent::ParamsUpdated(HeaterParams {
                target_temp,
                temp_tolerance,
            }) => {
                lo_temp = target_temp - temp_tolerance;
                hi_temp = target_temp + temp_tolerance;
            }
        };

        if let Some(current_temp) = current_temp {
            let is_on = control_pin.output_level() == HEATER_ON;
            if current_temp < lo_temp && !is_on {
                info!(
                    "Current temp {=f32} is below the threshold {=f32}. Enabling heater",
                    current_temp, lo_temp
                );
                control_pin.set_level(HEATER_ON);
                app_events.send(AppEvent::HeaterEnabled(true)).await;
            } else if current_temp > hi_temp && is_on {
                info!(
                    "Current temp {=f32} is above the threshold {=f32}. Disabling heater",
                    current_temp, hi_temp
                );
                control_pin.set_level(HEATER_OFF);
                app_events.send(AppEvent::HeaterEnabled(false)).await;
            } else {
                info!(
                    "Current heater status: lo={=f32}; cur={=f32}; hi={=f32}; on={=bool}",
                    lo_temp, current_temp, hi_temp, is_on,
                );
            }
        }
    }
}

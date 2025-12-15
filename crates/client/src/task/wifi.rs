use defmt::{error, expect, info};
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{
    ClientConfig, ModeConfig, WifiController, WifiEvent, WifiStaState, sta_state,
};

use crate::{AppEvent, AppEventsSender};

#[embassy_executor::task]
pub async fn connect(
    mut controller: WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
    reconnect_interval: Duration,
    app_events: AppEventsSender<'static>,
) {
    info!("Starting WiFi connection task");

    expect!(
        controller.set_power_saving(esp_radio::wifi::PowerSaveMode::None),
        "disabled wifi power saving"
    );

    expect!(
        controller.set_mode(esp_radio::wifi::WifiMode::Sta),
        "set wifi mode to sta"
    );

    loop {
        match sta_state() {
            WifiStaState::Connected => {
                app_events.send(AppEvent::WifiConnected).await;
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                app_events.send(AppEvent::WifiDisconnected).await;
                Timer::after(reconnect_interval).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            info!("Configuring wifi connection with SSID={=str}", ssid);
            let mode_config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(ssid.into())
                    .with_password(password.into()),
            );
            match controller.set_config(&mode_config) {
                Ok(_) => {
                    info!("Wifi conifguration applied");
                }
                Err(e) => {
                    error!("Error configuring wifi controller: {:?}", e);
                    continue;
                }
            }
            match controller.start_async().await {
                Ok(_) => {
                    info!("Wifi controller started!");
                }
                Err(e) => {
                    error!("Error starting wifi controller: {:?}", e);
                    continue;
                }
            }
        }

        info!("Wifi connecting...");

        match controller.connect_async().await {
            Ok(_) => {
                info!("Wifi connected!");
            }
            Err(e) => {
                error!("Failed to connect to wifi: {:?}", e);
                Timer::after(reconnect_interval).await
            }
        }
    }
}

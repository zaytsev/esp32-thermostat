use defmt::{error, expect, info};
use embassy_time::{Duration, Timer};
use esp_wifi::wifi::{
    ClientConfiguration, Configuration, WifiController, WifiEvent, WifiState, wifi_state,
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
        controller.set_power_saving(esp_wifi::config::PowerSaveMode::None),
        "disabled wifi power saving"
    );

    expect!(
        controller.set_mode(esp_wifi::wifi::WifiMode::Sta),
        "set wifi mode to sta"
    );

    loop {
        match wifi_state() {
            WifiState::StaConnected => {
                app_events.send(AppEvent::WifiConnected).await;
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                app_events.send(AppEvent::WifiDisconnected).await;
                Timer::after(reconnect_interval).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            info!("Configuring wifi connection with SSID={=str}", ssid);
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: ssid.into(),
                password: password.into(),
                // TODO: configure channel?
                ..Default::default()
            });
            match controller.set_configuration(&client_config) {
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

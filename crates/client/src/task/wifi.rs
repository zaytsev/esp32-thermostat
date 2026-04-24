use defmt::{error, info};
use embassy_time::{Duration, Timer};
use esp_radio::wifi::WifiController;

use crate::{AppEvent, AppEventsSender};

#[embassy_executor::task]
pub async fn connect(
    mut controller: WifiController<'static>,
    reconnect_interval: Duration,
    app_events: AppEventsSender<'static>,
) {
    info!("Starting WiFi connection task");

    loop {
        info!("Wifi connecting...");

        match controller.connect_async().await {
            Ok(_) => {
                info!("Wifi connected!");
                app_events.send(AppEvent::WifiConnected).await;
                match controller.wait_for_disconnect_async().await {
                    Ok(info) => {
                        info!("Wifi disconnected: {:?}", info.reason);
                    }
                    Err(e) => {
                        error!("Failed to read Wifi controller status: {:?}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to connect to wifi: {:?}", e);
            }
        }

        app_events.send(AppEvent::WifiDisconnected).await;
        Timer::after(reconnect_interval).await
    }
}

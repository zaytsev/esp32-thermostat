use defmt::info;
use embassy_net::Runner;
use esp_wifi::wifi::WifiDevice;

#[embassy_executor::task]
pub async fn run(mut runner: Runner<'static, WifiDevice<'static>>) {
    info!("Initializing network");
    runner.run().await
}

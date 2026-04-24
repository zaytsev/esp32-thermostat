use defmt::info;
use embassy_net::Runner;
use esp_radio::wifi::Interface;

#[embassy_executor::task]
pub async fn run(mut runner: Runner<'static, Interface<'static>>) {
    info!("Initializing network");
    runner.run().await
}

use std::net::SocketAddr;

use anyhow::{Context, anyhow};
use clap::Parser;
use tokio::net::{TcpListener, lookup_host};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod db;
mod telemetry;
mod web;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// HMAC key (hex or plain string). If not 32 bytes it will be hashed with SHA-256.
    #[arg(long, env = "SECRET", env = "NOISE_SECRET")]
    secret: String,

    /// Address to bind to (default: "0.0.0.0")
    #[arg(long, env, default_value = "0.0.0.0")]
    telemetry_host: String,

    /// Port to listen on
    #[arg(long, env, default_value_t = 40333)]
    telemetry_port: u16,

    #[arg(long, env, default_value = "0.0.0.0")]
    web_host: String,

    #[arg(long, env, default_value_t = 8080)]
    web_port: u16,

    #[arg(long, env, default_value = "metrics.db")]
    db_path: String,
}

async fn get_addr(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    let formatted_addr = format!("{}:{}", host, port);
    let addr = lookup_host(&formatted_addr)
        .await
        .with_context(|| format!("Error resolving address {}", { &formatted_addr }))?
        .next()
        .ok_or_else(|| anyhow!("Empty result when resolving '{}'", &formatted_addr))?;
    Ok(addr)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let args = Args::parse();
    let metrics_store = db::Store::open(&args.db_path).await?;
    let telemetry_server = telemetry::Server::new(&args.secret, metrics_store.clone());
    let node_manager = telemetry_server.node_manager();

    let telemetry_addr = get_addr(&args.telemetry_host, args.telemetry_port).await?;
    let telemetry_service = telemetry_server.serve(telemetry_addr);

    let web_addr = get_addr(&args.web_host, args.web_port).await?;
    let web_listener = TcpListener::bind(web_addr).await?;
    let web_service = axum::serve(web_listener, web::app(node_manager, metrics_store));

    let _r = tokio::try_join!(
        tokio::task::spawn(async move { telemetry_service.await }),
        tokio::task::spawn(async move { web_service.await })
    )?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
        .with_level(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}

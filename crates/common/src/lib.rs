#![cfg_attr(not(any(feature = "alloc", feature = "std")), no_std)]
pub mod proto {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub enum ClientMessage {
        Telemetry(TelemetryMessage),
        Ping(u64),
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub enum ServerMessage {
        StatusQuery,
        HeaterParamsUpdate(HeaterParams),
        Pong(u64),
    }

    #[derive(Debug, Serialize, Deserialize, Default)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct DiscoveryRequest {
        pub sender_id: u64,
    }

    #[derive(Debug, Serialize, Deserialize, Default)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct DiscoveryResponse {}

    #[derive(Debug, Serialize, Deserialize, Default)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct TelemetryMessage {
        pub sender_id: u64,
        pub inside_temp: f32,
        pub heater_enabled: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct HeaterParams {
        pub target_temp: f32,
        pub temp_tolerance: f32,
    }
}

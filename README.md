# ESP32 Thermostat

Educational project demonstrating embedded Rust on ESP32-C3 with a full-stack telemetry system.

## Features

**Firmware (ESP32-C3)**
- No-std Rust using [esp-hal](https://github.com/esp-rs/esp-hal) and [embassy](https://github.com/embassy-rs/embassy) for async runtime
- DS18B20 temperature sensor via OneWire ([RMT](https://docs.espressif.com/projects/esp-idf/en/latest/esp32c3/api-reference/peripherals/rmt.html) peripheral)
- GPIO relay control for heater switching
- WiFi connectivity with secure UDP telemetry
- Persistent configuration storage
- Custom protocol using [Noise](https://github.com/jmlepisto/clatter) protocol for encryption + [postcard](https://github.com/jamesmunns/postcard) serialization

**Server**
- Tokio-based async backend
- DuckDB for metrics storage and aggregation
- Axum web server with admin dashboard
- Server-side [rendered HTML](https://maud.lambda.xyz) + [unpoly](https://unpoly.com) + [chart.js](https://www.chartjs.org)
- Remote thermostat configuration (target temp, tolerance)

![Dashboard](docs/dashboard.jpg)

## Project Structure

```
crates/
├── common/ # Shared protocol definitions and types
├── client/ # ESP32-C3 firmware
└── server/ # Backend + web dashboard
```

## Quick Start

### Development Environment

Using [devenv.sh](https://devenv.sh):
```bash
devenv shell
export NOISE_SECRET="sup3r-s3cr3t"
```

### Build & Flash Firmware

```bash
cd crates/client
export SSID="my-wifi-ssid"
export PASSWORD="my-wifi-pass"
cargo esp32c3
``` 

### Build & Run Server

```bash
cargo run --bin esp32-thermostat-server
```

## License

MIT

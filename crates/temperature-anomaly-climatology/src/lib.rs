//! Library facade for the climatology pipeline.
//!
//! Exposing modules via `lib.rs` lets integration tests under `tests/`
//! reach internal helpers (the binary in `main.rs` keeps using the same
//! modules through `temperature_anomaly_climatology::…`).

pub mod build;
pub mod netcdf;

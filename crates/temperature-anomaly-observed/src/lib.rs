//! Library facade for the observed-anomaly pipeline.
//!
//! Exposing modules via `lib.rs` lets integration tests under `tests/`
//! reach internal helpers; the `main.rs` binary uses the same paths.

pub mod cds;

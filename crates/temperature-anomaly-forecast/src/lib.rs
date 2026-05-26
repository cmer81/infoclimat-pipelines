//! Crate `temperature-anomaly-forecast` : calcule les anomalies journalières
//! prévisionnelles (ARPEGE France via Open-Meteo) sur l'horizon J..J+N.
//!
//! - [`openmeteo`] : client HTTP pour fetcher les OMfiles horaires ARPEGE.
//! - Le binaire (`main.rs`) orchestre fetch → moyenne journalière → soustraction
//!   climato → écriture/upload R2.

pub mod openmeteo;

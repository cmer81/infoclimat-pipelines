//! Crate `arome-om-forecast` — pipeline batch qui télécharge la prévision
//! AROME-OM Réunion depuis l'API Météo-France, décode le GRIB2 (via cfgrib),
//! et publie des OMfiles spatiaux sur R2 pour consommation par `maps/`.
//!
//! Voir `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`.
//!
//! Modules :
//! - [`grib_decoder`] — wrapper Rust autour du script Python de décodage GRIB2.
//! - [`planning`] — construction du plan de download (Package × TimeWindow).
//! - [`variables`] — registry statique des variables AROME-OM exposées.

pub mod grib_decoder;
pub mod planning;
pub mod variables;

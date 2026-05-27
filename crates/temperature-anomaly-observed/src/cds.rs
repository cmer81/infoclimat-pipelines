//! Wrapper qui appelle le script Python `download_era5.py` pour télécharger
//! ERA5/ERA5T pour une journée donnée.
//!
//! L'API CDS est asynchrone (submit → poll → download). Réimplémenter ce
//! protocole en Rust pur représente beaucoup de plomberie pour peu de gain ;
//! on délègue au script Python existant qui utilise `cdsapi`. Pour un jour
//! de données (24 valeurs horaires sur la bbox France), le payload est de
//! quelques MB — la latence du sous-processus est négligeable.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};

use pipeline_core::grid::EUROPE_DOWNLOAD_BBOX;

/// Résultat d'une tentative de téléchargement d'un jour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadOutcome {
    /// NetCDF téléchargé dans `output`.
    Downloaded,
    /// Date pas encore publiée côté ERA5/ERA5T (délai ~5 j) — cas attendu,
    /// à retenter au prochain run. Pas une erreur.
    NotAvailableYet,
}

/// Lance `python3 <script_path>` pour télécharger un fichier NetCDF couvrant
/// `date` (24 heures UTC) sur la bbox Europe. Le script écrit dans `output`.
///
/// Retourne `NotAvailableYet` (code de sortie 3 du script) quand la date n'est
/// pas encore publiée — distinct d'une vraie erreur (`Err`).
pub fn download_day(date: NaiveDate, output: &Path, script_path: &Path) -> Result<DownloadOutcome> {
    let bbox = EUROPE_DOWNLOAD_BBOX;
    let status = Command::new("python3")
        .arg(script_path)
        .arg("--year")
        .arg(date.year().to_string())
        .arg("--month")
        .arg(date.month().to_string())
        .arg("--day")
        .arg(date.day().to_string())
        .arg("--bbox-north")
        .arg(format!("{}", bbox.lat_max))
        .arg("--bbox-west")
        .arg(format!("{}", bbox.lon_min))
        .arg("--bbox-south")
        .arg(format!("{}", bbox.lat_min))
        .arg("--bbox-east")
        .arg(format!("{}", bbox.lon_max))
        .arg("--output")
        .arg(output)
        .status()
        .with_context(|| format!("spawning python3 {script_path:?}"))?;

    match status.code() {
        Some(0) => Ok(DownloadOutcome::Downloaded),
        Some(3) => Ok(DownloadOutcome::NotAvailableYet),
        other => anyhow::bail!("python download failed for {date} (exit {other:?})"),
    }
}

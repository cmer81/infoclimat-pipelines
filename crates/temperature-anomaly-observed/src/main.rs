//! CLI `temperature-anomaly-observed` — calcule les anomalies journalières
//! observées (ERA5/ERA5T) pour les `refresh_days` derniers jours, écrit les
//! OMfiles spatiaux, les pousse sur R2, et GC les fichiers plus vieux que
//! `days_back` (rétention). Les jours entre `refresh_days` et `days_back`
//! persistent en R2 sans re-téléchargement.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use clap::Parser;
use pipeline_core::anomaly::subtract_with_nan;
use pipeline_core::climatology::{ClimatologyCache, day_of_year_index};
use pipeline_core::grid::{ArpegeEuropeGrid, Bbox};
use pipeline_core::omfile_io::{ANOMALY_VARIABLE, OmfileMetadata, write_spatial_omfile};
use pipeline_core::r2::{R2Client, R2Config};
use pipeline_core::regrid::bilinear_regrid;

use temperature_anomaly_observed::cds;

#[derive(Debug, Parser)]
#[command(
    about = "Compute daily observed temperature anomalies (ERA5/ERA5T) and upload to R2"
)]
struct Args {
    /// Fenêtre de rétention en jours (seuil GC + horizon des métadonnées).
    /// Les fichiers plus anciens sont supprimés du bucket.
    #[arg(long, default_value_t = 30)]
    days_back: i64,
    /// Nombre de jours récents à (re)télécharger à chaque run (J-1 .. J-refresh_days).
    /// Plus petit que `days_back` : les jours plus anciens persistent déjà en R2
    /// (téléchargés quand ils étaient récents) et ne sont pas re-téléchargés.
    /// Couvre les nouveaux jours + les révisions ERA5T. Mettre `>= days_back`
    /// pour un backfill initial complet.
    #[arg(long, default_value_t = 7)]
    refresh_days: i64,
    /// Dossier local contenant les 366 OMfiles climatologiques.
    #[arg(long)]
    climato_dir: PathBuf,
    /// Dossier de travail (NetCDF temporaires + OMfiles produits).
    #[arg(long)]
    work_dir: PathBuf,
    /// Préfixe R2 sous lequel publier les anomalies (`<prefix>/YYYY-MM-DD.om`).
    #[arg(long)]
    r2_anomaly_prefix: String,
    /// Chemin vers `scripts/download_era5.py`.
    #[arg(long)]
    download_script: PathBuf,
    /// Si présent, n'uploade pas vers R2 et saute le GC (test local).
    #[arg(long)]
    skip_upload: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    pipeline_core::logging::init();
    let args = Args::parse();

    tracing::info!(?args, "starting observed run");
    std::fs::create_dir_all(&args.work_dir)?;

    let climato = ClimatologyCache::load_from_dir(&args.climato_dir)
        .with_context(|| format!("loading climato from {:?}", args.climato_dir))?;
    let dst_grid = ArpegeEuropeGrid;
    let r2 = if !args.skip_upload {
        Some(R2Client::new(R2Config::from_env()?).await?)
    } else {
        None
    };

    let today = Utc::now().date_naive();
    let mut written = 0u32;
    let mut skipped = 0u32;
    let mut failures = 0u32;

    for offset in 1..=args.refresh_days {
        let day = today - Duration::days(offset);
        match process_day(day, &args, &climato, &dst_grid, r2.as_ref()).await {
            Ok(ProcessOutcome::Done) => written += 1,
            Ok(ProcessOutcome::SkippedNotAvailable) => {
                tracing::info!(%day, "skipped — pas encore publié côté ERA5/ERA5T (retenté demain)");
                skipped += 1;
            }
            Err(e) => {
                tracing::error!(?day, error = %e, "day failed");
                failures += 1;
            }
        }
    }

    // GC : supprimer les fichiers dont la date est antérieure à `today - days_back`.
    if let Some(r2) = &r2 {
        let cutoff = today - Duration::days(args.days_back);
        match r2.list_prefix(&args.r2_anomaly_prefix).await {
            Ok(keys) => {
                for key in keys {
                    if let Some(d) = parse_date_from_key(&key)
                        && d < cutoff
                    {
                        if let Err(e) = r2.delete(&key).await {
                            tracing::warn!(%key, error = %e, "GC delete failed");
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "GC list_prefix failed"),
        }
    }

    if let Some(r2) = &r2 {
        if let Err(e) = pipeline_core::anomaly_metadata::update_anomaly_metadata(r2).await {
            tracing::error!(error = %e, "failed to update anomaly metadata");
        }
    }

    tracing::info!(written, skipped, failures, "observed run done");
    Ok(())
}

/// Issue du traitement d'un jour.
enum ProcessOutcome {
    /// Anomalie calculée et (éventuellement) uploadée.
    Done,
    /// Jour pas encore publié côté ERA5/ERA5T — à retenter au prochain run.
    SkippedNotAvailable,
}

async fn process_day(
    day: NaiveDate,
    args: &Args,
    climato: &ClimatologyCache,
    dst_grid: &ArpegeEuropeGrid,
    r2: Option<&R2Client>,
) -> Result<ProcessOutcome> {
    let nc_path = args.work_dir.join(format!("era5_{day}.nc"));
    match cds::download_day(day, &nc_path, &args.download_script)
        .with_context(|| format!("download {day}"))?
    {
        cds::DownloadOutcome::Downloaded => {}
        cds::DownloadOutcome::NotAvailableYet => return Ok(ProcessOutcome::SkippedNotAvailable),
    }

    let era5 = temperature_anomaly_climatology::netcdf::read_era5_hourly(&nc_path)?;
    let daily =
        temperature_anomaly_climatology::netcdf::aggregate_daily_mean(&era5.data, day);
    let daily_arr = daily
        .get(&day)
        .context("daily mean missing for target day")?;

    let bbox = Bbox {
        lon_min: *era5.lons.first().context("empty lons")?,
        lon_max: *era5.lons.last().context("empty lons")?,
        lat_min: *era5.lats.first().context("empty lats")?,
        lat_max: *era5.lats.last().context("empty lats")?,
    };
    anyhow::ensure!(era5.lons.len() >= 2, "need at least 2 lons");
    anyhow::ensure!(era5.lats.len() >= 2, "need at least 2 lats");
    let src_dx = (bbox.lon_max - bbox.lon_min) / (era5.lons.len() - 1) as f64;
    let src_dy = (bbox.lat_max - bbox.lat_min) / (era5.lats.len() - 1) as f64;

    let regridded = bilinear_regrid(daily_arr, bbox, src_dx, src_dy, dst_grid)?;
    let celsius = regridded.mapv(|v| v - 273.15);

    let doy = day_of_year_index(day);
    let climato_arr = climato
        .get(doy)
        .with_context(|| format!("missing climato for DOY {doy}"))?;
    let anomaly = subtract_with_nan(&celsius, climato_arr);

    let filename = format!("{day}.om");
    let local_path = args.work_dir.join(&filename);
    let meta = OmfileMetadata {
        source: "era5_or_era5t".to_string(),
        generated_at: Utc::now(),
        extra: serde_json::json!({ "day": day.to_string(), "doy": doy }),
    };
    write_spatial_omfile(&local_path, ANOMALY_VARIABLE, &anomaly, dst_grid, &meta)?;

    if let Some(r2) = r2 {
        let key = format!("{}/{}", args.r2_anomaly_prefix.trim_end_matches('/'), filename);
        r2.upload_file(&key, &local_path, pipeline_core::r2::CACHE_ROLLING)
            .await?;
    }

    // Nettoyage du NetCDF volumineux (l'OMfile reste si on veut le ré-inspecter).
    if let Err(e) = std::fs::remove_file(&nc_path) {
        tracing::warn!(?nc_path, error = %e, "could not remove NetCDF");
    }
    Ok(ProcessOutcome::Done)
}

/// Extrait `YYYY-MM-DD` de la queue d'une clé R2 type `…/YYYY-MM-DD.om`.
fn parse_date_from_key(key: &str) -> Option<NaiveDate> {
    let filename = Path::new(key).file_name()?.to_str()?;
    let stem = filename.strip_suffix(".om")?;
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_date_from_key_extracts_date() {
        assert_eq!(
            parse_date_from_key("anomaly/observed/2026-05-26.om"),
            Some(NaiveDate::from_ymd_opt(2026, 5, 26).unwrap())
        );
    }

    #[test]
    fn parse_date_from_key_rejects_non_date_stem() {
        assert_eq!(parse_date_from_key("anomaly/observed/latest.om"), None);
    }

    #[test]
    fn parse_date_from_key_rejects_wrong_extension() {
        assert_eq!(parse_date_from_key("anomaly/observed/2026-05-26.txt"), None);
    }
}

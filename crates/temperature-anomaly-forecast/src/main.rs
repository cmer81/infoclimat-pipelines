//! CLI `temperature-anomaly-forecast` — pipeline complet :
//! 1. Charge la climatologie 366 jours (lecture des OMfiles de `--climato-dir`).
//! 2. Détermine le run ARPEGE le plus récent (`OpenMeteoClient::latest_model_run`).
//! 3. Pour chaque jour `J ∈ [today, today + days_ahead]` :
//!    a. Récupère les 24 OMfiles horaires disponibles.
//!    b. Décode chacun (déjà en °C chez Open-Meteo), accumule (somme + count).
//!    c. Moyenne journalière, NaN si count==0.
//!    d. Soustrait la climato du DOY.
//!    e. Écrit l'OMfile spatial local + upload R2 (clé `{prefix}/YYYY-MM-DD.om`).
//! 4. Pas de GC : les fichiers prévision sont écrasés en place à chaque run.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::Bytes;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use clap::Parser;
use ndarray::Array2;
use omfiles::{
    InMemoryBackend,
    reader::OmFileReader,
    traits::{OmArrayVariable, OmFileReadable},
};
use pipeline_core::anomaly::subtract_with_nan;
use pipeline_core::climatology::{ClimatologyCache, day_of_year_index};
use pipeline_core::grid::{ArpegeEuropeGrid, Grid};
use pipeline_core::omfile_io::{ANOMALY_VARIABLE, OmfileMetadata, write_spatial_omfile};
use pipeline_core::r2::{R2Client, R2Config};

use temperature_anomaly_forecast::openmeteo::OpenMeteoClient;

#[derive(Debug, Parser)]
#[command(
    about = "Compute daily forecast temperature anomalies (ARPEGE Europe via Open-Meteo) and upload to R2"
)]
struct Args {
    /// Nombre de jours en avant à recalculer (J..J+days_ahead).
    #[arg(long, default_value_t = 4)]
    days_ahead: i64,
    /// Dossier local contenant les 366 OMfiles climatologiques.
    #[arg(long)]
    climato_dir: PathBuf,
    /// Dossier de travail (OMfiles produits localement).
    #[arg(long)]
    work_dir: PathBuf,
    /// Préfixe R2 sous lequel publier les anomalies prévision (`<prefix>/YYYY-MM-DD.om`).
    #[arg(long)]
    r2_anomaly_prefix: String,
    /// Nombre de jours récents (J-1..J-provisional_days) à combler avec ARPEGE
    /// en attendant ERA5 (estimation provisoire). 0 pour désactiver.
    #[arg(long, default_value_t = 5)]
    provisional_days: i64,
    /// Préfixe R2 pour les anomalies provisoires (ARPEGE sur jours récents).
    /// Requis si `provisional_days > 0`.
    #[arg(long, default_value = "anomaly/temperature_2m/provisional")]
    r2_provisional_prefix: String,
    /// Si présent, n'uploade pas vers R2 (test local).
    #[arg(long)]
    skip_upload: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    pipeline_core::logging::init();
    let args = Args::parse();

    tracing::info!(?args, "starting forecast run");
    std::fs::create_dir_all(&args.work_dir)?;

    let climato = ClimatologyCache::load_from_dir(&args.climato_dir)
        .with_context(|| format!("loading climato from {:?}", args.climato_dir))?;
    let dst_grid = ArpegeEuropeGrid;

    let r2 = if !args.skip_upload {
        Some(R2Client::new(R2Config::from_env()?).await?)
    } else {
        None
    };

    let om_client = OpenMeteoClient::new();
    let now = Utc::now();
    let model_run = OpenMeteoClient::latest_model_run(now);
    tracing::info!(%model_run, "using model run");

    let today = now.date_naive();
    let mut written = 0u32;
    let mut skipped = 0u32;
    let mut failures = 0u32;
    // Prévision : J+0 → J+days_ahead, depuis le run 00Z du jour.
    for offset in 0..=args.days_ahead {
        let day = today + Duration::days(offset);
        match process_day(
            day,
            model_run,
            &args.r2_anomaly_prefix,
            &om_client,
            &climato,
            &dst_grid,
            &args,
            r2.as_ref(),
        )
        .await
        {
            Ok(ProcessOutcome::Written) => written += 1,
            Ok(ProcessOutcome::SkippedPartial) => {
                tracing::info!(%day, "skipped — moins de 24h dispo (au-delà de l'horizon du run)");
                skipped += 1;
                // Purge un éventuel fichier périmé pour ce jour (écrit par un
                // run antérieur, potentiellement partiel) : sinon il reste dans
                // valid_times avec une valeur biaisée.
                if let Some(r2) = r2.as_ref() {
                    let key = format!(
                        "{}/{day}.om",
                        args.r2_anomaly_prefix.trim_end_matches('/')
                    );
                    let _ = r2.delete(&key).await;
                }
            }
            Err(e) => {
                tracing::error!(?day, error = %e, "forecast day failed");
                failures += 1;
            }
        }
    }

    // Provisoire : J-1 → J-provisional_days, depuis le run 00Z de CHAQUE jour
    // (qui couvre ce jour de 00h à 23h). Comble le trou ERA5T en attendant la
    // réanalyse définitive. Écrit dans un préfixe séparé `provisional/`.
    let mut provisional = 0u32;
    for offset in 1..=args.provisional_days {
        let day = today - Duration::days(offset);
        let day_00z = day.and_hms_opt(0, 0, 0).expect("valid hms").and_utc();
        match process_day(
            day,
            day_00z,
            &args.r2_provisional_prefix,
            &om_client,
            &climato,
            &dst_grid,
            &args,
            r2.as_ref(),
        )
        .await
        {
            Ok(ProcessOutcome::Written) => provisional += 1,
            Ok(ProcessOutcome::SkippedPartial) => {
                tracing::info!(%day, "provisional skipped — <24h (run pas/plus dispo)");
            }
            Err(e) => tracing::warn!(?day, error = %e, "provisional day failed"),
        }
    }

    if let Some(r2) = &r2 {
        // GC : supprimer les fichiers forecast dont la date est passée. Sinon le
        // J+0 d'hier reste dans `forecast/` et, devenu une date passée, il est
        // annoncé dans `valid_times` puis routé par le client vers `observed/`
        // (où il n'existe pas) → 404. Les jours passés doivent venir de
        // l'observed (ERA5/ERA5T), pas d'une vieille prévision.
        gc_before(r2, &args.r2_anomaly_prefix, today, "forecast").await;
        // Provisoire : on ne garde que la fenêtre J-1..J-provisional_days ;
        // au-delà, ERA5 a (normalement) pris le relais côté observed.
        let provisional_cutoff = today - Duration::days(args.provisional_days);
        gc_before(r2, &args.r2_provisional_prefix, provisional_cutoff, "provisional").await;

        if let Err(e) = pipeline_core::anomaly_metadata::update_anomaly_metadata(r2).await {
            tracing::error!(error = %e, "failed to update anomaly metadata");
        }
    }

    tracing::info!(written, skipped, provisional, failures, "forecast run done");
    Ok(())
}

/// Supprime les OMfiles d'un préfixe dont la date est antérieure à `cutoff`.
async fn gc_before(r2: &R2Client, prefix: &str, cutoff: NaiveDate, label: &str) {
    let prefix = format!("{}/", prefix.trim_end_matches('/'));
    let keys = match r2.list_prefix(&prefix).await {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(error = %e, %label, "GC: list failed");
            return;
        }
    };
    for key in keys {
        let Some(date) = pipeline_core::anomaly_metadata::parse_date_from_key(&key) else {
            continue;
        };
        if date < cutoff {
            if let Err(e) = r2.delete(&key).await {
                tracing::warn!(key, error = %e, %label, "GC: delete failed");
            } else {
                tracing::info!(key, %label, "GC: deleted out-of-window file");
            }
        }
    }
}

/// Issue du traitement d'un jour de prévision.
enum ProcessOutcome {
    /// Anomalie calculée sur 24h pleines et (éventuellement) uploadée.
    Written,
    /// Jour incomplet (< 24h dispo, au-delà de l'horizon du run) — sauté pour
    /// ne pas publier une moyenne journalière partielle (biaisée).
    SkippedPartial,
}

#[expect(clippy::too_many_arguments, reason = "pipeline context struct not yet introduced")]
async fn process_day(
    day: NaiveDate,
    model_run: DateTime<Utc>,
    target_prefix: &str,
    om: &OpenMeteoClient,
    climato: &ClimatologyCache,
    dst_grid: &ArpegeEuropeGrid,
    args: &Args,
    r2: Option<&R2Client>,
) -> Result<ProcessOutcome> {
    let hours = om.fetch_day(day, model_run).await?;
    // On exige les 24 heures : une moyenne journalière sur un sous-ensemble
    // (ex. J+0 démarrant à 06Z, ou J+4 au-delà de l'horizon) est biaisée.
    if hours.len() < 24 {
        return Ok(ProcessOutcome::SkippedPartial);
    }

    let mut acc = Array2::<f32>::zeros((dst_grid.ny(), dst_grid.nx()));
    let mut counts = Array2::<u32>::zeros((dst_grid.ny(), dst_grid.nx()));

    for (_h, bytes) in &hours {
        let arr = read_omfile_bytes(bytes, dst_grid)?;
        // Les OMfiles `temperature_2m` data_spatial d'Open-Meteo sont DÉJÀ en
        // °C (le client maps les rend directement contre une échelle °C). Pas
        // de conversion K→°C ici, contrairement à la climato/observed qui lisent
        // ERA5 NetCDF en Kelvin.
        for ((j, i), &v) in arr.indexed_iter() {
            if v.is_nan() {
                continue;
            }
            acc[[j, i]] += v;
            counts[[j, i]] += 1;
        }
    }

    let daily_mean = Array2::from_shape_fn((dst_grid.ny(), dst_grid.nx()), |(j, i)| {
        let n = counts[[j, i]];
        if n == 0 {
            f32::NAN
        } else {
            acc[[j, i]] / n as f32
        }
    });

    let doy = day_of_year_index(day);
    let climato_arr = climato
        .get(doy)
        .with_context(|| format!("missing climato for DOY {doy}"))?;
    let anomaly = subtract_with_nan(&daily_mean, climato_arr);

    let filename = format!("{day}.om");
    let local_path = args.work_dir.join(&filename);
    let meta = OmfileMetadata {
        source: format!("arpege_europe_{}", model_run.format("%Y%m%dT%HZ")),
        generated_at: Utc::now(),
        extra: serde_json::json!({
            "day": day.to_string(),
            "doy": doy,
            "model_run": model_run.to_rfc3339(),
            "hours_available": hours.len(),
        }),
    };
    write_spatial_omfile(&local_path, ANOMALY_VARIABLE, &anomaly, dst_grid, &meta)?;

    if let Some(r2) = r2 {
        let key = format!("{}/{}", target_prefix.trim_end_matches('/'), filename);
        r2.upload_file(&key, &local_path, pipeline_core::r2::CACHE_ROLLING)
            .await?;
    }
    Ok(ProcessOutcome::Written)
}

/// Décode un OMfile spatial Open-Meteo (variable `temperature_2m`) fourni
/// sous forme de `Bytes` en mémoire. Vérifie que les dimensions correspondent
/// à la grille ARPEGE Europe (741×521).
fn read_omfile_bytes(bytes: &Bytes, dst_grid: &ArpegeEuropeGrid) -> Result<Array2<f32>> {
    let backend = Arc::new(InMemoryBackend::new(bytes.to_vec()));
    let root = OmFileReader::new(backend)
        .map_err(|e| anyhow::anyhow!("open omfile from memory: {e}"))?;

    // Open-Meteo publie les OMfiles ARPEGE avec un enfant nommé d'après la
    // variable (`temperature_2m`). Cf. `infoclimat-om-worker/src/aggregate.rs`
    // qui fait la même lecture.
    let var_node = root
        .get_child_by_name("temperature_2m")
        .context("variable 'temperature_2m' absente du OMfile")?;
    let array_var = var_node
        .expect_array()
        .map_err(|e| anyhow::anyhow!("'temperature_2m' is not an array: {e}"))?;

    let dims: Vec<u64> = array_var.get_dimensions().to_vec();
    anyhow::ensure!(dims.len() == 2, "expected 2D OMfile, got {}D", dims.len());
    let ny = dims[0] as usize;
    let nx = dims[1] as usize;
    anyhow::ensure!(
        ny == dst_grid.ny() && nx == dst_grid.nx(),
        "OMfile dims ({ny}, {nx}) != ARPEGE Europe ({}, {})",
        dst_grid.ny(),
        dst_grid.nx()
    );

    let full_range: Vec<std::ops::Range<u64>> = dims.iter().map(|&d| 0..d).collect();
    let dynd = array_var
        .read::<f32>(&full_range)
        .map_err(|e| anyhow::anyhow!("read array: {e}"))?;
    let arr2: Array2<f32> = dynd
        .into_dimensionality::<ndarray::Ix2>()
        .map_err(|e| anyhow::anyhow!("dim cast to 2D: {e}"))?;
    Ok(arr2)
}

#[cfg(test)]
mod tests {
    // Le calcul d'anomalie utilise subtract_with_nan (testé dans pipeline-core).
    // Les fonctions HTTP sont testées indirectement via les types — on ne fait
    // pas de network mock ici (pas de mockito en deps).
    use super::*;

    #[test]
    fn arpege_europe_grid_is_741_by_521() {
        let g = ArpegeEuropeGrid;
        assert_eq!(g.nx(), 741);
        assert_eq!(g.ny(), 521);
    }
}

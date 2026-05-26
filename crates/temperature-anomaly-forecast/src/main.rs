//! CLI `temperature-anomaly-forecast` — pipeline complet :
//! 1. Charge la climatologie 366 jours (lecture des OMfiles de `--climato-dir`).
//! 2. Détermine le run ARPEGE le plus récent (`OpenMeteoClient::latest_model_run`).
//! 3. Pour chaque jour `J ∈ [today, today + days_ahead]` :
//!    a. Récupère les 24 OMfiles horaires disponibles.
//!    b. Décode chacun, convertit K → °C, accumule (somme + count par pixel).
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
use pipeline_core::omfile_io::{OmfileMetadata, write_spatial_omfile};
use pipeline_core::r2::{R2Client, R2Config};

use temperature_anomaly_forecast::openmeteo::OpenMeteoClient;

#[derive(Debug, Parser)]
#[command(
    about = "Compute daily forecast temperature anomalies (ARPEGE France via Open-Meteo) and upload to R2"
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
    /// Préfixe R2 sous lequel publier les anomalies (`<prefix>/YYYY-MM-DD.om`).
    #[arg(long)]
    r2_anomaly_prefix: String,
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
    let dst_grid = ArpegeEuropeGrid::default();

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
    let mut failures = 0u32;
    for offset in 0..=args.days_ahead {
        let day = today + Duration::days(offset);
        match process_day(day, model_run, &om_client, &climato, &dst_grid, &args, r2.as_ref())
            .await
        {
            Ok(_) => written += 1,
            Err(e) => {
                tracing::error!(?day, error = %e, "forecast day failed");
                failures += 1;
            }
        }
    }

    if let Some(r2) = &r2 {
        if let Err(e) = pipeline_core::anomaly_metadata::update_anomaly_metadata(r2).await {
            tracing::error!(error = %e, "failed to update anomaly metadata");
        }
    }

    tracing::info!(written, failures, "forecast run done");
    Ok(())
}

async fn process_day(
    day: NaiveDate,
    model_run: DateTime<Utc>,
    om: &OpenMeteoClient,
    climato: &ClimatologyCache,
    dst_grid: &ArpegeEuropeGrid,
    args: &Args,
    r2: Option<&R2Client>,
) -> Result<()> {
    let hours = om.fetch_day(day, model_run).await?;
    anyhow::ensure!(!hours.is_empty(), "no hours available for {day}");

    let mut acc = Array2::<f32>::zeros((dst_grid.ny(), dst_grid.nx()));
    let mut counts = Array2::<u32>::zeros((dst_grid.ny(), dst_grid.nx()));

    for (_h, bytes) in &hours {
        let arr = read_omfile_bytes(bytes, dst_grid)?;
        // K → °C, NaN-skip.
        for ((j, i), &v) in arr.indexed_iter() {
            if v.is_nan() {
                continue;
            }
            acc[[j, i]] += v - 273.15;
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
        source: format!("arpege_france_{}", model_run.format("%Y%m%dT%HZ")),
        generated_at: Utc::now(),
        extra: serde_json::json!({
            "day": day.to_string(),
            "doy": doy,
            "model_run": model_run.to_rfc3339(),
            "hours_available": hours.len(),
        }),
    };
    write_spatial_omfile(&local_path, &anomaly, dst_grid, &meta)?;

    if let Some(r2) = r2 {
        let key = format!(
            "{}/{}",
            args.r2_anomaly_prefix.trim_end_matches('/'),
            filename
        );
        r2.upload_file(&key, &local_path).await?;
    }
    Ok(())
}

/// Décode un OMfile spatial Open-Meteo (variable `temperature_2m`) fourni
/// sous forme de `Bytes` en mémoire. Vérifie que les dimensions correspondent
/// à la grille ARPEGE France (180×105).
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
        "OMfile dims ({ny}, {nx}) != ARPEGE France ({}, {})",
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
        let g = ArpegeEuropeGrid::default();
        assert_eq!(g.nx(), 741);
        assert_eq!(g.ny(), 521);
    }
}

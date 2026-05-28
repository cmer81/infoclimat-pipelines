use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use pipeline_core::grid::{ArpegeEuropeGrid, Bbox};
use pipeline_core::omfile_io::{ANOMALY_VARIABLE, OmfileMetadata, write_spatial_omfile};
use pipeline_core::r2::{R2Client, R2Config};
use pipeline_core::regrid::bilinear_regrid;

use temperature_anomaly_climatology::build::{DoyAccumulator, smooth_climatology_15d};
use temperature_anomaly_climatology::netcdf::{aggregate_daily_mean, read_era5_hourly};

#[derive(Debug, Parser)]
#[command(about = "Build climatology files from ERA5 NetCDF inputs")]
struct Args {
    /// Dossier contenant les NetCDF annuels (un par année, format
    /// `era5_2m_temperature_{year}.nc`).
    #[arg(long)]
    input_dir: PathBuf,
    /// Dossier de sortie local pour les 366 OMfiles.
    #[arg(long)]
    output_dir: PathBuf,
    /// Préfixe R2 où uploader (ex:
    /// `climatology/temperature_2m/era5_1991-2020/arpege_france`).
    #[arg(long)]
    r2_prefix: String,
    /// Si présent, n'uploade pas vers R2 (test local).
    #[arg(long)]
    skip_upload: bool,
    #[arg(long, default_value_t = 1991)]
    year_start: i32,
    #[arg(long, default_value_t = 2020)]
    year_end: i32,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    pipeline_core::logging::init();
    let args = Args::parse();

    tracing::info!(?args, "starting climatology build");
    std::fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("creating {:?}", args.output_dir))?;

    // 1. Lire chaque NetCDF annuel, agréger en moyennes journalières,
    //    regridder sur la grille ARPEGE Europe, convertir K → °C, et
    //    accumuler dans une somme glissante par DOY. On ne garde JAMAIS
    //    plus d'une année en mémoire (cf. DoyAccumulator) — indispensable
    //    sur la grille Europe (521×741) où 30 ans = ~17 GB sinon.
    let dst_grid = ArpegeEuropeGrid;
    let mut acc = DoyAccumulator::new();

    for year in args.year_start..=args.year_end {
        let path = args
            .input_dir
            .join(format!("era5_2m_temperature_{year}.nc"));
        tracing::info!(year, ?path, "reading ERA5 annual file");
        let era5 = read_era5_hourly(&path)
            .with_context(|| format!("reading ERA5 for {year}"))?;

        let bbox = Bbox {
            lon_min: *era5.lons.first().expect("non-empty lons"),
            lon_max: *era5.lons.last().expect("non-empty lons"),
            lat_min: *era5.lats.first().expect("non-empty lats"),
            lat_max: *era5.lats.last().expect("non-empty lats"),
        };
        let src_dx = (bbox.lon_max - bbox.lon_min) / (era5.lons.len() - 1) as f64;
        let src_dy = (bbox.lat_max - bbox.lat_min) / (era5.lats.len() - 1) as f64;

        let start_day = NaiveDate::from_ymd_opt(year, 1, 1)
            .ok_or_else(|| anyhow::anyhow!("invalid year {year}"))?;
        let daily = aggregate_daily_mean(&era5.data, start_day);

        let mut by_doy = HashMap::new();
        for (day, arr) in daily {
            let regridded = bilinear_regrid(&arr, bbox, src_dx, src_dy, &dst_grid)
                .with_context(|| format!("regrid {day}"))?;
            // Kelvin → Celsius pour stockage.
            let celsius = regridded.mapv(|v| v - 273.15);
            by_doy.insert(pipeline_core::climatology::day_of_year_index(day), celsius);
        }
        acc.add_year(by_doy);
        // `era5` (le gros Array3 horaire) est droppé ici à la fin de l'itération.
    }

    // 2. Moyenne DOY-par-DOY à travers les années.
    tracing::info!("computing DOY mean across years");
    let raw_climato = acc.finalize();

    // 3. Lissage 15 jours centré.
    tracing::info!("applying 15-day smoothing");
    let smoothed = smooth_climatology_15d(&raw_climato);

    // 4. Écriture des OMfiles + upload R2 (optionnel).
    let r2 = if !args.skip_upload {
        Some(R2Client::new(R2Config::from_env()?).await?)
    } else {
        None
    };

    for (doy, arr) in &smoothed {
        let filename = format!("{doy:03}.om");
        let local_path = args.output_dir.join(&filename);
        let meta = OmfileMetadata {
            source: format!("era5_{}-{}", args.year_start, args.year_end),
            generated_at: chrono::Utc::now(),
            extra: serde_json::json!({
                "doy": doy,
                "smoothing_days": 15,
                "unit": "celsius",
                "grid": "arpege_europe",
            }),
        };
        write_spatial_omfile(&local_path, ANOMALY_VARIABLE, arr, &dst_grid, &meta)
            .with_context(|| format!("write DOY {doy}"))?;
        if let Some(r2) = &r2 {
            let key = format!("{}/{}", args.r2_prefix.trim_end_matches('/'), filename);
            r2.upload_file(&key, &local_path, pipeline_core::r2::CACHE_IMMUTABLE)
                .await
                .with_context(|| format!("upload DOY {doy}"))?;
        }
    }

    tracing::info!(count = smoothed.len(), "climatology build complete");
    Ok(())
}

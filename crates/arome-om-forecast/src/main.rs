//! CLI `arome-om-forecast` — pipeline AROME-OM Réunion (prévision brute).
//!
//! Étapes (cf. `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`):
//!  1. Détermine le run target (`floor_6h(now - publication_delay)` ou `--run`).
//!  2. Build le plan (packages × leadtimes 0..=horizon).
//!  3. Pour chaque (pkg, leadtime), parallel buffer_unordered :
//!     a. Download GRIB2 (1 fichier par leadtime, `time=001H`…).
//!     b. Décode (script python) → N slices (var, leadtime, Array2).
//!     c. Écrit OMfile local + upload R2.
//!  4. Update metadata.
//!  5. GC des runs trop vieux.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Timelike, Utc};
use clap::Parser;
use futures::stream::{self, StreamExt};

use pipeline_core::arome_om_metadata::update_metadata;
use pipeline_core::grid::{Grid, ReunionGrid};
use pipeline_core::meteofrance_api::{AromeOmClient, AromeOmTerritory, MeteoFranceAuth, MeteoFranceError};
use pipeline_core::omfile_io::{OmfileMetadata, write_spatial_omfile};
use pipeline_core::r2::{CACHE_ROLLING, R2Client, R2Config};

use arome_om_forecast::grib_decoder::{self, DecodedSlice};
use arome_om_forecast::planning::{Package, build_plan};
use arome_om_forecast::variables::{VARIABLES, VariableEntry, variables_for_package};

const PUBLICATION_DELAY_H: i64 = 6;

#[derive(Debug, Parser)]
#[command(about = "Compute AROME-OM forecast OMfiles (raw values) and upload to R2")]
struct Args {
    /// Territoire AROME-OM (pour l'instant: reunion).
    #[arg(long, default_value = "reunion")]
    territory: String,
    /// Run cible (ISO 8601). Si omis : floor_6h(now - PUBLICATION_DELAY_H).
    #[arg(long)]
    run: Option<DateTime<Utc>>,
    /// Horizon max en heures (capped par l'horizon du modèle, 48h).
    #[arg(long, default_value_t = 48)]
    horizon_h: u32,
    /// Packages à télécharger (CSV).
    #[arg(long, default_value = "SP1,SP2")]
    packages: String,
    /// Concurrence (downloads parallèles).
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
    /// Dossier de travail (GRIB téléchargés + OMfiles produits).
    #[arg(long)]
    work_dir: PathBuf,
    /// Préfixe R2 cible.
    #[arg(long, default_value = "data_spatial/arome_om_reunion")]
    r2_prefix: String,
    /// Combien de runs garder en R2 (GC).
    #[arg(long, default_value_t = 4)]
    keep_runs_back: u32,
    /// Si présent, n'uploade pas vers R2 (test local).
    #[arg(long)]
    skip_upload: bool,
}

/// `floor_6h(now - publication_delay)`. Renvoie l'heure 00/06/12/18
/// la plus récente disponible : runs AROME-OM à 00/06/12/18 UTC, publication ~6h après.
fn latest_run(now: DateTime<Utc>) -> DateTime<Utc> {
    let candidate = now - Duration::hours(PUBLICATION_DELAY_H);
    let h = candidate.hour();
    let floor_h = (h / 6) * 6; // -> 0, 6, 12, or 18
    candidate
        .date_naive()
        .and_hms_opt(floor_h, 0, 0)
        .expect("valid hms")
        .and_utc()
}

fn parse_territory(s: &str) -> Result<AromeOmTerritory> {
    match s.to_lowercase().as_str() {
        "reunion" | "réunion" => Ok(AromeOmTerritory::Reunion),
        other => anyhow::bail!("unsupported territory: {other:?} (only 'reunion' for now)"),
    }
}

fn parse_packages(s: &str) -> Result<Vec<&'static str>> {
    let mut out = Vec::new();
    for item in s.split(',') {
        let p = item.trim();
        match p {
            "SP1" => out.push("SP1"),
            "SP2" => out.push("SP2"),
            "SP3" => out.push("SP3"),
            other => anyhow::bail!("unsupported package: {other:?}"),
        }
    }
    Ok(out)
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    pipeline_core::logging::init();
    let args = Args::parse();
    tracing::info!(?args, "starting arome-om-forecast");
    std::fs::create_dir_all(&args.work_dir).context("creating work_dir")?;

    let territory = parse_territory(&args.territory)?;
    let packages: Vec<Package> = parse_packages(&args.packages)?
        .into_iter()
        .map(|p| match p {
            "SP1" => Package::Sp1,
            "SP2" => Package::Sp2,
            "SP3" => Package::Sp3,
            _ => unreachable!("parse_packages guarantees set"),
        })
        .collect();

    let run = args.run.unwrap_or_else(|| latest_run(Utc::now()));
    let plan = build_plan(args.horizon_h, &packages);
    tracing::info!(%run, items = plan.len(), concurrency = args.concurrency, "plan built");

    let auth = Arc::new(MeteoFranceAuth::from_env().context("init auth")?);
    let mf = Arc::new(AromeOmClient::new(auth));
    let r2 = if !args.skip_upload {
        Some(Arc::new(R2Client::new(R2Config::from_env().context("R2 cfg")?).await?))
    } else {
        None
    };
    let grid = ReunionGrid;

    let work_dir = args.work_dir.clone();
    let r2_prefix = args.r2_prefix.clone();

    // Counters partagés.
    let counters = Arc::new(tokio::sync::Mutex::new((0u32, 0u32))); // (written, failures)

    stream::iter(plan.into_iter().map(|(pkg, leadtime)| {
        let mf = mf.clone();
        let r2 = r2.clone();
        let work_dir = work_dir.clone();
        let r2_prefix = r2_prefix.clone();
        let counters = counters.clone();
        async move {
            match process_item(&mf, r2.as_deref(), territory, pkg, leadtime, run, &grid, &work_dir, &r2_prefix).await {
                Ok(n) => {
                    let mut c = counters.lock().await;
                    c.0 += n;
                    tracing::info!(%pkg, leadtime, n, "item OK");
                }
                Err(e) => {
                    let mut c = counters.lock().await;
                    c.1 += 1;
                    tracing::error!(%pkg, leadtime, error = %e, "item FAILED");
                    if matches!(e.downcast_ref::<MeteoFranceError>(), Some(MeteoFranceError::Auth(_))) {
                        tracing::error!("auth error — aborting");
                        std::process::exit(2);
                    }
                }
            }
        }
    }))
    .buffer_unordered(args.concurrency)
    .collect::<Vec<_>>()
    .await;

    let (written, failures) = *counters.lock().await;
    tracing::info!(written, failures, "all items done");

    // Metadata + GC seulement si au moins un fichier a été écrit et qu'on uploade.
    if let Some(r2) = r2.as_deref() {
        if written > 0 {
            let var_names: Vec<&'static str> = VARIABLES.iter().map(|v| v.om_name).collect();
            if let Err(e) = update_metadata(r2, run, &var_names).await {
                tracing::error!(error = %e, "metadata update failed");
            }
            if let Err(e) = gc_old_runs(r2, &r2_prefix, run, args.keep_runs_back).await {
                tracing::error!(error = %e, "GC failed");
            }
        }
    }

    if failures > 0 || written == 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[expect(clippy::too_many_arguments, reason = "pipeline context struct not yet introduced")]
async fn process_item(
    mf: &AromeOmClient,
    r2: Option<&R2Client>,
    territory: AromeOmTerritory,
    pkg: Package,
    leadtime: u32,
    run: DateTime<Utc>,
    grid: &ReunionGrid,
    work_dir: &std::path::Path,
    r2_prefix: &str,
) -> Result<u32> {
    let run_dir = work_dir.join(format!("{}Z", run.format("%Y%m%dT%H%M")));
    std::fs::create_dir_all(&run_dir)?;
    let grib_path = run_dir.join(format!("{pkg}_{leadtime:03}h.grib2"));

    let bytes = mf
        .fetch_package(territory, pkg.as_api_id(), run, leadtime)
        .await
        .with_context(|| format!("fetch {pkg} leadtime={leadtime}"))?;
    std::fs::write(&grib_path, &bytes).with_context(|| format!("write {grib_path:?}"))?;

    let nc_dir = run_dir.join(format!("nc_{pkg}_{leadtime:03}h"));
    let pkg_id = pkg.as_api_id();
    let vars_of_interest: Vec<&VariableEntry> = variables_for_package(pkg_id).collect();
    let slices = grib_decoder::decode(&grib_path, &nc_dir, &vars_of_interest, (grid.ny(), grid.nx()))
        .await
        .with_context(|| format!("decode {pkg} leadtime={leadtime}"))?;

    let mut written = 0u32;
    for slice in slices {
        match write_and_upload_slice(slice, run, &run_dir, r2, r2_prefix, grid).await {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!(error = %e, "slice failed"),
        }
    }
    Ok(written)
}

async fn write_and_upload_slice(
    slice: DecodedSlice,
    run: DateTime<Utc>,
    run_local_dir: &std::path::Path,
    r2: Option<&R2Client>,
    r2_prefix: &str,
    grid: &ReunionGrid,
) -> Result<()> {
    let filename = format!("{}_{:03}h.om", slice.om_name, slice.leadtime_h);
    let local = run_local_dir.join(&filename);
    let meta = OmfileMetadata {
        source: format!("arome_om_reunion_{}", run.format("%Y%m%dT%HZ")),
        generated_at: Utc::now(),
        extra: serde_json::json!({
            "variable": slice.om_name,
            "leadtime_h": slice.leadtime_h,
            "run": run.to_rfc3339(),
        }),
    };
    write_spatial_omfile(&local, &slice.data, grid, &meta)
        .context("write OMfile")?;
    if let Some(r2) = r2 {
        let key = format!(
            "{}/{}/{}/{}/{}Z/{}",
            r2_prefix.trim_end_matches('/'),
            run.format("%Y"), run.format("%m"), run.format("%d"),
            run.format("%H%M"),
            filename,
        );
        r2.upload_file(&key, &local, CACHE_ROLLING).await?;
    }
    Ok(())
}

/// Supprime les préfixes de run plus vieux que `run - keep_runs_back × 3h`.
async fn gc_old_runs(
    r2: &R2Client,
    r2_prefix: &str,
    current_run: DateTime<Utc>,
    keep_runs_back: u32,
) -> Result<()> {
    let cutoff = current_run - Duration::hours(3 * i64::from(keep_runs_back));
    let all = r2.list_prefix(&format!("{}/", r2_prefix.trim_end_matches('/'))).await?;
    for k in all {
        // On parse `r2_prefix/Y/M/D/HHMMZ/...` et on garde tout >= cutoff.
        let Some(rest) = k.strip_prefix(&format!("{}/", r2_prefix.trim_end_matches('/'))) else {
            continue;
        };
        let mut parts = rest.split('/');
        let Some(y) = parts.next().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let Some(m) = parts.next().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(d) = parts.next().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(hhmmz) = parts.next() else { continue };
        let Some(hhmm) = hhmmz.strip_suffix('Z') else { continue };
        let Ok(hh) = hhmm.get(..2).unwrap_or("").parse::<u32>() else { continue };
        let Some(date) = chrono::NaiveDate::from_ymd_opt(y, m, d) else { continue };
        let Some(run_dt) = date.and_hms_opt(hh, 0, 0).map(|t| t.and_utc()) else { continue };
        if run_dt < cutoff {
            if let Err(e) = r2.delete(&k).await {
                tracing::warn!(key=%k, error=%e, "GC delete failed");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn latest_run_floors_to_6h_with_publication_delay() {
        // 2026-05-28 14:23Z → candidate = 08:23Z → floor_6h = 06:00Z.
        let now = Utc.with_ymd_and_hms(2026, 5, 28, 14, 23, 0).unwrap();
        let run = latest_run(now);
        assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 28, 6, 0, 0).unwrap());
    }

    #[test]
    fn latest_run_handles_day_boundary() {
        // 2026-05-28 01:00Z → candidate = 2026-05-27 19:00Z → floor_6h = 18:00Z J-1.
        let now = Utc.with_ymd_and_hms(2026, 5, 28, 1, 0, 0).unwrap();
        let run = latest_run(now);
        assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 27, 18, 0, 0).unwrap());
    }

    #[test]
    fn parse_territory_accepts_reunion_case_insensitive() {
        assert!(matches!(
            parse_territory("reunion").unwrap(),
            AromeOmTerritory::Reunion
        ));
        assert!(matches!(
            parse_territory("REUNION").unwrap(),
            AromeOmTerritory::Reunion
        ));
    }

    #[test]
    fn parse_packages_csv() {
        assert_eq!(parse_packages("SP1,SP3").unwrap(), vec!["SP1", "SP3"]);
        assert!(parse_packages("SP1,FOO").is_err());
    }
}

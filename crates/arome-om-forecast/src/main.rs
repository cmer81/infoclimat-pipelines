//! CLI `arome-om-forecast` — pipeline AROME-OM Réunion (prévision brute).
//!
//! Étapes (cf. `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`):
//!  1. Détermine le run target (`floor_3h(now - publication_delay)` ou `--run`).
//!  2. Build le plan (packages × windows).
//!  3. Pour chaque (pkg, window), parallel buffer_unordered :
//!     a. Download GRIB2.
//!     b. Décode (script python) → N slices (var, leadtime, Array2).
//!     c. Écrit OMfile local + upload R2.
//!  4. Update metadata.
//!  5. GC des runs trop vieux.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Timelike, Utc};
use clap::Parser;

use pipeline_core::meteofrance_api::{AromeOmTerritory, MeteoFranceAuth};

const PUBLICATION_DELAY_H: i64 = 4;

#[derive(Debug, Parser)]
#[command(about = "Compute AROME-OM forecast OMfiles (raw values) and upload to R2")]
struct Args {
    /// Territoire AROME-OM (pour l'instant: reunion).
    #[arg(long, default_value = "reunion")]
    territory: String,
    /// Run cible (ISO 8601). Si omis : floor_3h(now - PUBLICATION_DELAY_H).
    #[arg(long)]
    run: Option<DateTime<Utc>>,
    /// Horizon max en heures (multiple de 6, capped par l'horizon du modèle).
    #[arg(long, default_value_t = 42)]
    horizon_h: u32,
    /// Packages à télécharger (CSV).
    #[arg(long, default_value = "SP1,SP2,SP3")]
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

/// `floor_3h(now - publication_delay)`. Renvoie l'heure 00/03/06/09/12/15/18/21
/// la plus récente >= maintenant - PUBLICATION_DELAY_H.
fn latest_run(now: DateTime<Utc>) -> DateTime<Utc> {
    let candidate = now - Duration::hours(PUBLICATION_DELAY_H);
    let h = candidate.hour();
    let floor_h = (h / 3) * 3;
    // SAFETY: floor_h is in 0..=21 (since (h/3)*3 where h<24),
    // so and_hms_opt can never return None for valid minutes/seconds (both 0).
    candidate
        .date_naive()
        .and_hms_opt(floor_h, 0, 0)
        .expect("floor_h in 0..=21 with zero minutes/seconds: infallible")
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
    let packages = parse_packages(&args.packages)?;
    let run = args.run.unwrap_or_else(|| latest_run(Utc::now()));
    tracing::info!(%run, ?packages, horizon_h = args.horizon_h, "plan parameters");

    let auth = Arc::new(MeteoFranceAuth::from_env().context("init auth")?);
    let _ = (auth, territory, run); // util'd at Task 14

    tracing::info!("plan-only mode — orchestration in Task 14");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn latest_run_floors_to_3h_with_publication_delay() {
        // 2026-05-28 14:23Z → candidate = 10:23Z → floor 09:00Z.
        let now = Utc.with_ymd_and_hms(2026, 5, 28, 14, 23, 0).unwrap();
        let run = latest_run(now);
        assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 28, 9, 0, 0).unwrap());
    }

    #[test]
    fn latest_run_handles_day_boundary() {
        // 2026-05-28 01:00Z → candidate = 21:00 the day before → 21:00Z J-1.
        let now = Utc.with_ymd_and_hms(2026, 5, 28, 1, 0, 0).unwrap();
        let run = latest_run(now);
        assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 27, 21, 0, 0).unwrap());
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

//! Génération des métadonnées JSON du domaine `arome_om_reunion`.
//!
//! Le client `maps/` lit `data_spatial/arome_om_reunion/latest.json` (+
//! `in-progress.json` + `{run}/meta.json`) pour piloter son sélecteur de temps.
//! Schema simplifié vs `anomaly_metadata` (pas de `provisional_times`, pas
//! d'union observé/forecast — produit de pure prévision).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::r2::R2Client;

/// Shape compatible `DomainMetaDataJson` côté client.
#[derive(Debug, Clone, Serialize)]
pub struct ForecastDomainMetadata {
    pub reference_time: String,
    pub valid_times: Vec<String>,
    pub variables: Vec<String>,
}

const META_DOMAIN_PREFIX: &str = "data_spatial/arome_om_reunion";

/// Extrait `(variable, leadtime_h)` d'une clé R2 du type
/// `data_spatial/arome_om_reunion/Y/M/D/HHMMZ/{var}_{HHHh}.om`. Retourne `None`
/// pour les autres clés (meta.json, latest.json, etc.).
pub fn parse_run_key(key: &str) -> Option<(String, u32)> {
    let stem = key.rsplit('/').next()?.strip_suffix(".om")?;
    let (var, lead) = stem.rsplit_once('_')?;
    let lead = lead.strip_suffix('h')?;
    let lead_h = lead.parse::<u32>().ok()?;
    Some((var.to_string(), lead_h))
}

/// `data_spatial/arome_om_reunion/2026/05/28/0000Z/2t_006h.om` → `"2026-05-28T06:00:00Z"`.
///
/// Le `reference_time` est lu depuis la position fixe `Y/M/D/HHMMZ` dans la clé.
pub fn key_to_valid_time(key: &str) -> Option<String> {
    // Repère `data_spatial/arome_om_reunion/Y/M/D/HHMMZ/...`
    let trimmed = key.strip_prefix(META_DOMAIN_PREFIX)?.trim_start_matches('/');
    let mut parts = trimmed.split('/');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    let run_seg = parts.next()?; // ex "0000Z"
    let run_h: u32 = run_seg.strip_suffix('Z')?.get(..2)?.parse().ok()?;
    let (_, lead_h) = parse_run_key(key)?;
    let total_h = i64::from(run_h) + i64::from(lead_h);
    let date = chrono::NaiveDate::from_ymd_opt(y, m, d)?;
    let dt = date.and_hms_opt(0, 0, 0)?.and_utc() + chrono::Duration::hours(total_h);
    Some(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

pub async fn update_metadata(
    r2: &R2Client,
    run: DateTime<Utc>,
    variables: &[&'static str],
) -> Result<()> {
    let run_prefix = format!(
        "{META_DOMAIN_PREFIX}/{}/{}/{}/{}Z/",
        run.format("%Y"),
        run.format("%m"),
        run.format("%d"),
        run.format("%H%M"),
    );
    let keys = r2.list_prefix(&run_prefix).await.context("listing run keys")?;

    let mut times: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for k in &keys {
        if let Some(vt) = key_to_valid_time(k) {
            times.insert(vt);
        }
    }
    if times.is_empty() {
        tracing::warn!("no AROME-OM OMfiles found for run — skipping metadata write");
        return Ok(());
    }

    let meta = ForecastDomainMetadata {
        reference_time: run.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        valid_times: times.into_iter().collect(),
        variables: variables.iter().map(|s| s.to_string()).collect(),
    };
    let body = serde_json::to_vec(&meta).context("serializing metadata")?;
    let cc = "public, max-age=300";
    let ct = "application/json";
    let run_meta_key = format!("{run_prefix}meta.json");

    for key in [
        format!("{META_DOMAIN_PREFIX}/latest.json"),
        format!("{META_DOMAIN_PREFIX}/in-progress.json"),
        run_meta_key,
    ] {
        r2.put_bytes(&key, body.clone(), ct, cc)
            .await
            .with_context(|| format!("writing {key}"))?;
    }
    tracing::info!(reference_time = %meta.reference_time, vars = meta.variables.len(), "arome-om metadata written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_run_key_extracts_var_and_lead() {
        let k = "data_spatial/arome_om_reunion/2026/05/28/0000Z/temperature_2m_006h.om";
        assert_eq!(parse_run_key(k), Some(("temperature_2m".to_string(), 6)));
    }

    #[test]
    fn parse_run_key_rejects_non_om() {
        assert!(parse_run_key("foo/bar/meta.json").is_none());
        // "no_lead.om" → stem "no_lead" → rsplit_once('_') = ("no","lead")
        // → strip_suffix('h') fails on "lead" → None.
        assert!(parse_run_key("foo/bar/no_lead.om").is_none());
    }

    #[test]
    fn key_to_valid_time_adds_lead_to_run() {
        let k = "data_spatial/arome_om_reunion/2026/05/28/0000Z/temperature_2m_006h.om";
        assert_eq!(
            key_to_valid_time(k),
            Some("2026-05-28T06:00:00Z".to_string())
        );
    }

    #[test]
    fn key_to_valid_time_crosses_day_boundary() {
        let k = "data_spatial/arome_om_reunion/2026/05/28/1800Z/temperature_2m_012h.om";
        assert_eq!(
            key_to_valid_time(k),
            Some("2026-05-29T06:00:00Z".to_string())
        );
    }

    #[test]
    fn metadata_json_shape() {
        let meta = ForecastDomainMetadata {
            reference_time: "2026-05-28T00:00:00Z".into(),
            valid_times: vec!["2026-05-28T01:00:00Z".into()],
            variables: vec!["temperature_2m".into()],
        };
        let j = serde_json::to_value(&meta).unwrap();
        assert_eq!(j["reference_time"], "2026-05-28T00:00:00Z");
        assert_eq!(j["valid_times"][0], "2026-05-28T01:00:00Z");
        assert_eq!(j["variables"][0], "temperature_2m");
    }
}

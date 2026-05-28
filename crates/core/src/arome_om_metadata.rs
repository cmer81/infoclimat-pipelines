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

/// Extrait le valid_time ISO d'une clé R2 `…/{YYYY-MM-DDTHHMM}.om`.
///
/// Correspond au nouveau layout `data_spatial` : un seul OMfile multi-variable
/// par leadtime, nommé d'après son `valid_time` (ex. `2026-05-29T0000.om`).
/// Retourne `None` pour les autres clés (meta.json, latest.json, etc.).
pub fn key_to_valid_time(key: &str) -> Option<String> {
    let stem = key.rsplit('/').next()?.strip_suffix(".om")?;
    // Format attendu : YYYY-MM-DDTHHMM (13 chars : 4+1+2+1+2+1+4 = 15 avec tirets/T).
    // On valide en parsant strictement — rejette les noms hors-format.
    let dt = chrono::NaiveDateTime::parse_from_str(stem, "%Y-%m-%dT%H%M").ok()?;
    Some(dt.and_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string())
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
    fn key_to_valid_time_extracts_iso_from_filename() {
        let k = "data_spatial/arome_om_reunion/2026/05/28/0600Z/2026-05-29T0000.om";
        assert_eq!(
            key_to_valid_time(k),
            Some("2026-05-29T00:00:00Z".to_string())
        );
    }

    #[test]
    fn key_to_valid_time_preserves_intraday_time() {
        let k = "data_spatial/arome_om_reunion/2026/05/28/0000Z/2026-05-28T1800.om";
        assert_eq!(
            key_to_valid_time(k),
            Some("2026-05-28T18:00:00Z".to_string())
        );
    }

    #[test]
    fn key_to_valid_time_rejects_non_om() {
        assert!(key_to_valid_time("foo/bar/meta.json").is_none());
        assert!(key_to_valid_time("foo/bar/not_a_timestamp.om").is_none());
        // Ancien format (variable_leadtime) doit être rejeté.
        assert!(key_to_valid_time("data_spatial/arome_om_reunion/2026/05/28/0000Z/temperature_2m_006h.om").is_none());
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

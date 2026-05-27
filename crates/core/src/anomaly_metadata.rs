//! Génération des métadonnées JSON du pseudo-domaine `anomaly_europe`.
//!
//! Le client `maps/` lit `data_spatial/anomaly_europe/latest.json` (+
//! `in-progress.json` + `{run}/meta.json`) pour piloter son sélecteur de
//! temps. On y publie un `reference_time` synthétique (aujourd'hui 00Z) et la
//! liste réelle des `valid_times` (dates pour lesquelles un OMfile existe).

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::Serialize;

use crate::r2::R2Client;

/// Shape compatible `DomainMetaDataJson` côté client (champs minimaux requis).
#[derive(Debug, Clone, Serialize)]
pub struct DomainMetadataJson {
    pub reference_time: String,
    pub valid_times: Vec<String>,
    pub variables: Vec<String>,
    /// Dates servies depuis le préfixe `provisional/` (estimation ARPEGE en
    /// attendant ERA5), PAS encore couvertes par l'observé définitif. Le client
    /// les affiche en opacité réduite et les fetch depuis `provisional/`.
    #[serde(default)]
    pub provisional_times: Vec<String>,
}

/// Extrait la date d'une clé R2 du type
/// `anomaly/temperature_2m/{observed|forecast}/YYYY-MM-DD.om`.
pub fn parse_date_from_key(key: &str) -> Option<NaiveDate> {
    let stem = key.rsplit('/').next()?.strip_suffix(".om")?;
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()
}

/// Union triée (croissante) des dates observed + forecast, au format ISO Z.
pub fn build_valid_times(observed_keys: &[String], forecast_keys: &[String]) -> Vec<String> {
    let dates: Vec<NaiveDate> = observed_keys
        .iter()
        .chain(forecast_keys.iter())
        .filter_map(|k| parse_date_from_key(k))
        .collect();
    dates_to_iso(dates)
}

/// Préfixes R2 sous lesquels vivent les OMfiles d'anomalie.
const OBSERVED_PREFIX: &str = "anomaly/temperature_2m/observed/";
const FORECAST_PREFIX: &str = "anomaly/temperature_2m/forecast/";
const PROVISIONAL_PREFIX: &str = "anomaly/temperature_2m/provisional/";

/// Convertit des dates en `valid_times` ISO Z, triées croissantes et dédupliquées.
fn dates_to_iso(mut dates: Vec<NaiveDate>) -> Vec<String> {
    dates.sort_unstable();
    dates.dedup();
    dates
        .into_iter()
        .map(|d| format!("{}T00:00:00Z", d.format("%Y-%m-%d")))
        .collect()
}
/// Préfixe de métadonnées (layout `data_spatial` attendu par le client maps).
const META_DOMAIN_PREFIX: &str = "data_spatial/anomaly_europe";

/// Liste les OMfiles d'anomalie dans R2, construit les métadonnées, et écrit
/// `latest.json`, `in-progress.json` et `{run}/meta.json`.
///
/// Idempotent : appelable depuis observed et forecast indifféremment, les deux
/// régénèrent la même vue à jour.
pub async fn update_anomaly_metadata(r2: &R2Client) -> Result<()> {
    let observed_keys = r2
        .list_prefix(OBSERVED_PREFIX)
        .await
        .context("listing observed keys")?;
    let forecast_keys = r2
        .list_prefix(FORECAST_PREFIX)
        .await
        .context("listing forecast keys")?;
    let provisional_keys = r2
        .list_prefix(PROVISIONAL_PREFIX)
        .await
        .context("listing provisional keys")?;

    let observed_dates: std::collections::BTreeSet<NaiveDate> = observed_keys
        .iter()
        .filter_map(|k| parse_date_from_key(k))
        .collect();
    let provisional_dates: Vec<NaiveDate> = provisional_keys
        .iter()
        .filter_map(|k| parse_date_from_key(k))
        .collect();

    // valid_times = union observed ∪ forecast ∪ provisional.
    let all_dates: Vec<NaiveDate> = observed_keys
        .iter()
        .chain(forecast_keys.iter())
        .chain(provisional_keys.iter())
        .filter_map(|k| parse_date_from_key(k))
        .collect();
    let valid_times = dates_to_iso(all_dates);
    if valid_times.is_empty() {
        tracing::warn!("no anomaly OMfiles found — skipping metadata write");
        return Ok(());
    }

    // provisional_times = dates provisoires NON encore couvertes par l'observé
    // définitif (l'observé gagne toujours).
    let provisional_times = dates_to_iso(
        provisional_dates
            .into_iter()
            .filter(|d| !observed_dates.contains(d))
            .collect(),
    );

    // reference_time synthétique = aujourd'hui 00:00Z.
    let today = chrono::Utc::now().date_naive();
    let reference_time = format!("{}T00:00:00Z", today.format("%Y-%m-%d"));

    // Run paths `{YYYY}/{MM}/{DD}/0000Z` pour lesquels publier un meta.json.
    // Le client (domaine `model_interval: daily`) dérive un "model run" = la
    // date sélectionnée et va chercher `{run}/meta.json`. Il faut donc un
    // meta.json par date sélectionnable (chaque valid_time), + aujourd'hui
    // (reference_time). Tous ont le même contenu.
    let mut run_paths: std::collections::BTreeSet<String> = valid_times
        .iter()
        .filter_map(|vt| vt.get(0..10)) // "YYYY-MM-DD"
        .map(|d| format!("{}/0000Z", d.replace('-', "/")))
        .collect();
    run_paths.insert(format!("{}/0000Z", today.format("%Y/%m/%d")));

    let meta = DomainMetadataJson {
        reference_time: reference_time.clone(),
        valid_times,
        variables: vec!["temperature_2m_anomaly".to_string()],
        provisional_times,
    };
    let json = serde_json::to_vec(&meta).context("serializing metadata")?;

    // Cache court : les métadonnées changent à chaque run.
    let cc = "public, max-age=300";
    let ct = "application/json";

    let mut keys = vec![
        format!("{META_DOMAIN_PREFIX}/latest.json"),
        format!("{META_DOMAIN_PREFIX}/in-progress.json"),
    ];
    keys.extend(
        run_paths
            .iter()
            .map(|rp| format!("{META_DOMAIN_PREFIX}/{rp}/meta.json")),
    );

    for key in keys {
        r2.put_bytes(&key, json.clone(), ct, cc)
            .await
            .with_context(|| format!("writing {key}"))?;
    }

    tracing::info!(reference_time, runs = run_paths.len(), "anomaly metadata written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn parse_date_from_key_extracts_date() {
        let key = "anomaly/temperature_2m/observed/2026-05-21.om";
        assert_eq!(
            parse_date_from_key(key),
            Some(NaiveDate::from_ymd_opt(2026, 5, 21).unwrap())
        );
    }

    #[test]
    fn parse_date_from_key_rejects_non_om() {
        assert_eq!(parse_date_from_key("anomaly/temperature_2m/observed/"), None);
        assert_eq!(parse_date_from_key("foo/bar/notadate.om"), None);
    }

    #[test]
    fn valid_times_unions_and_sorts() {
        let observed = vec![
            "anomaly/temperature_2m/observed/2026-05-20.om".to_string(),
            "anomaly/temperature_2m/observed/2026-05-19.om".to_string(),
        ];
        let forecast = vec![
            "anomaly/temperature_2m/forecast/2026-05-26.om".to_string(),
            "anomaly/temperature_2m/forecast/2026-05-25.om".to_string(),
        ];
        let vt = build_valid_times(&observed, &forecast);
        assert_eq!(
            vt,
            vec![
                "2026-05-19T00:00:00Z",
                "2026-05-20T00:00:00Z",
                "2026-05-25T00:00:00Z",
                "2026-05-26T00:00:00Z",
            ]
        );
    }

    #[test]
    fn valid_times_dedupes_overlap() {
        // Une même date présente dans observed ET forecast n'apparaît qu'une fois.
        let observed = vec!["anomaly/temperature_2m/observed/2026-05-26.om".to_string()];
        let forecast = vec!["anomaly/temperature_2m/forecast/2026-05-26.om".to_string()];
        let vt = build_valid_times(&observed, &forecast);
        assert_eq!(vt, vec!["2026-05-26T00:00:00Z"]);
    }

    #[test]
    fn metadata_json_shape() {
        let meta = DomainMetadataJson {
            reference_time: "2026-05-26T00:00:00Z".to_string(),
            valid_times: vec!["2026-05-26T00:00:00Z".to_string()],
            variables: vec!["temperature_2m_anomaly".to_string()],
            provisional_times: vec!["2026-05-24T00:00:00Z".to_string()],
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["reference_time"], "2026-05-26T00:00:00Z");
        assert_eq!(json["valid_times"][0], "2026-05-26T00:00:00Z");
        assert_eq!(json["variables"][0], "temperature_2m_anomaly");
        assert_eq!(json["provisional_times"][0], "2026-05-24T00:00:00Z");
    }
}

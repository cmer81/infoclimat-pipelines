//! Fetch d'OMfiles spatiaux ARPEGE France depuis `map-tiles.open-meteo.com`.
//!
//! Le service publie pour chaque run (00/06/12/18Z) un OMfile par heure
//! d'horizon de prévision, sur la grille ARPEGE France native (180×105).
//! On expose ici :
//! - [`OpenMeteoClient::latest_model_run`] : sélection du run le plus récent
//!   qui devrait être totalement publié (les fichiers arrivent ~6h après
//!   l'heure du run, donc on prend `floor_6h(now - 6h)`).
//! - [`OpenMeteoClient::url_for_hour`] : construction de l'URL pour une heure.
//! - [`OpenMeteoClient::fetch_om`] / [`OpenMeteoClient::fetch_day`] : récupération
//!   HTTP. Les erreurs par-heure sont loggées et silencées.

use anyhow::{Context, Result};
use bytes::Bytes;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};
use reqwest::Client;

const BASE_URL_ENV: &str = "OPENMETEO_BASE_URL";
const DEFAULT_BASE: &str = "https://map-tiles.open-meteo.com";
const DOMAIN: &str = "meteofrance_arpege_europe";
/// Délai (heures) après lequel le run 00Z est supposé publié sur Open-Meteo.
const RUN_PUBLISH_DELAY_H: i64 = 6;

pub struct OpenMeteoClient {
    http: Client,
    base: String,
}

impl OpenMeteoClient {
    /// Construit un client. Le `base_url` provient de `OPENMETEO_BASE_URL`
    /// (fallback : `https://map-tiles.open-meteo.com`).
    pub fn new() -> Self {
        let base = std::env::var(BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_BASE.to_string());
        let http = Client::builder()
            .user_agent("infoclimat-pipelines/0.1")
            .build()
            .expect("reqwest client build");
        Self { http, base }
    }

    /// Sélectionne le run le plus récent supposément publié.
    ///
    /// On utilise impérativement un run **00Z** : il couvre le jour J+0 de 00h
    /// à 23h, donc la moyenne journalière est calculée sur les 24 heures. Un
    /// run 06/12/18Z raterait les premières heures du jour (les plus froides)
    /// → moyenne biaisée chaud → anomalie faussement positive.
    ///
    /// Le run 00Z du jour est publié ~6 h après 00Z. Avant ça, on retombe sur
    /// le run 00Z de la veille (qui couvre quand même tout J+0).
    ///
    /// Exemples :
    /// - 07:30 UTC → aujourd'hui 00Z (publié).
    /// - 03:00 UTC → hier 00Z (le 00Z d'aujourd'hui pas encore publié).
    pub fn latest_model_run(now: DateTime<Utc>) -> DateTime<Utc> {
        let today_00z = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("valid hms")
            .and_utc();
        if now >= today_00z + Duration::hours(RUN_PUBLISH_DELAY_H) {
            today_00z
        } else {
            today_00z - Duration::days(1)
        }
    }

    /// URL d'un OMfile horaire ARPEGE Europe pour `time` à partir du `model_run`.
    ///
    /// Schéma (calqué sur `infoclimat-om-worker/src/aggregate.rs::source_url`) :
    /// `{base}/data_spatial/{domain}/{Y}/{M}/{D}/{HH}{mm}Z/{Y}-{M}-{D}T{HH}{mm}.om`
    ///
    /// La variable n'apparaît PAS dans le path. Les OMfiles `data_spatial` ne
    /// contiennent qu'une seule variable nommée comme enfant du root, le path
    /// HTTP n'a donc pas besoin de la nommer.
    pub fn url_for_hour(&self, model_run: DateTime<Utc>, time: DateTime<Utc>) -> String {
        format!(
            "{base}/data_spatial/{domain}/{y:04}/{m:02}/{d:02}/{hh:02}{mm:02}Z/{t}.om",
            base = self.base.trim_end_matches('/'),
            domain = DOMAIN,
            y = model_run.year(),
            m = model_run.month(),
            d = model_run.day(),
            hh = model_run.hour(),
            mm = model_run.minute(),
            t = time.format("%Y-%m-%dT%H%M"),
        )
    }

    /// GET HTTP, renvoie le body en bytes. Erreur sur status non-2xx.
    pub async fn fetch_om(&self, url: &str) -> Result<Bytes> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("HTTP {status} for {url}");
        }
        let bytes = resp
            .bytes()
            .await
            .with_context(|| format!("read body {url}"))?;
        Ok(bytes)
    }

    /// Récupère les heures disponibles du jour `day` pour le `model_run` donné.
    ///
    /// Heures avant `model_run` ignorées (pas dans l'horizon de prévision).
    /// Les erreurs par-heure (404, timeout) sont **loggées et silencées** : on
    /// renvoie ce qui a pu être récupéré, à charge du caller de décider si
    /// c'est suffisant.
    pub async fn fetch_day(
        &self,
        day: NaiveDate,
        model_run: DateTime<Utc>,
    ) -> Result<Vec<(u32, Bytes)>> {
        let mut hours = Vec::new();
        for h in 0..24u32 {
            let t = day
                .and_hms_opt(h, 0, 0)
                .expect("valid hms")
                .and_utc();
            if t < model_run {
                continue;
            }
            let url = self.url_for_hour(model_run, t);
            match self.fetch_om(&url).await {
                Ok(b) => hours.push((h, b)),
                Err(e) => tracing::warn!(?day, hour = h, error = %e, "hourly fetch failed"),
            }
        }
        Ok(hours)
    }
}

impl Default for OpenMeteoClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn latest_run_at_midnight_returns_previous_day_00z() {
        // 00:00 UTC : le 00Z du jour n'est pas encore publié (< 06Z) → 00Z J-1.
        let now = Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap();
        let run = OpenMeteoClient::latest_model_run(now);
        assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap());
    }
}

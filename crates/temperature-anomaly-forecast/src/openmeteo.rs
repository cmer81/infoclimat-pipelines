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
const DEFAULT_VARIABLE: &str = "temperature_2m";
const DOMAIN: &str = "meteofrance_arpege_europe";

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
    /// Les fichiers ARPEGE arrivent sur Open-Meteo plusieurs heures après
    /// l'heure du run. On applique donc une marge de sécurité : on prend
    /// `now - 6h`, puis on l'arrondit à la borne 6h inférieure.
    ///
    /// Exemples (cf. plan & tests) :
    /// - 07:30 UTC → 00Z (6:30 − 6h = 01:30 → floor=00).
    /// - 13:05 UTC → 06Z (12:05 + 1 − 6h = 07:05 → floor=06).
    pub fn latest_model_run(now: DateTime<Utc>) -> DateTime<Utc> {
        let shifted = now - Duration::hours(6);
        let floored_hour = (shifted.hour() / 6) * 6;
        shifted
            .date_naive()
            .and_hms_opt(floored_hour, 0, 0)
            .expect("valid hms")
            .and_utc()
    }

    /// URL d'un OMfile horaire ARPEGE France pour `time` à partir du `model_run`.
    ///
    /// Schéma : `{base}/data_spatial/meteofrance_arpege_europe/{Y}/{M}/{D}/{HH}{mm}Z/{var}/{Y}-{M}-{D}T{HH}{mm}.om`
    pub fn url_for_hour(
        &self,
        model_run: DateTime<Utc>,
        time: DateTime<Utc>,
        variable: &str,
    ) -> String {
        format!(
            "{base}/data_spatial/{domain}/{y:04}/{m:02}/{d:02}/{hh:02}{mm:02}Z/{var}/{t}.om",
            base = self.base.trim_end_matches('/'),
            domain = DOMAIN,
            y = model_run.year(),
            m = model_run.month(),
            d = model_run.day(),
            hh = model_run.hour(),
            mm = model_run.minute(),
            var = variable,
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
            let url = self.url_for_hour(model_run, t, DEFAULT_VARIABLE);
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
    fn latest_run_at_midnight_local_returns_previous_day_18z() {
        // 00:00 UTC : shifted = J-1 18:00 → 18Z J-1.
        let now = Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap();
        let run = OpenMeteoClient::latest_model_run(now);
        assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 25, 18, 0, 0).unwrap());
    }
}

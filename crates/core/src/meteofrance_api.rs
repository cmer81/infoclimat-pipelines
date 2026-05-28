//! Client HTTP minimal pour le portail Météo-France (auth OAuth2 + download
//! GRIB2). Scope MVP : AROME-OM. Pattern réutilisable pour radar plus tard.

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error)]
pub enum MeteoFranceError {
    #[error("auth failed: {0}")]
    Auth(String),
    #[error("rate limited (Retry-After: {retry_after_s:?})")]
    RateLimited { retry_after_s: Option<u64> },
    #[error("http {status}: {body}")]
    Http { status: u16, body: String },
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("incomplete response: expected {expected} bytes, got {got}")]
    Incomplete { expected: u64, got: u64 },
}

pub const PUBLIC_API_BASE: &str = "https://public-api.meteofrance.fr";
pub const TOKEN_ENDPOINT: &str = "https://portail-api.meteofrance.fr/token";

/// Construit l'URL d'un fichier GRIB2 sur l'API DPPaquetAROME-OM.
///
/// Le path exact (`DPPaquetAROME-OM` ou alternative) doit être confirmé sur
/// un appel réel (Task 0 du plan). Cette fonction prend les composants comme
/// paramètres pour rester testable et trivialement ajustable.
pub fn build_product_url(
    base: &str,
    api_namespace: &str,           // ex. "DPPaquetAROME-OM"
    model: &str,                   // ex. "AROME-INDIEN" (résolu à Task 0)
    grid: &str,                    // ex. "0.025"
    package: &str,                 // ex. "SP1"
    reference_time: DateTime<Utc>,
    time_window: &str,             // ex. "00H06H"
) -> String {
    format!(
        "{base}/previnum/{ns}/v1/models/{model}/grids/{grid}/packages/{package}/productARO?referencetime={rt}&time={tw}&format=grib2",
        base = base.trim_end_matches('/'),
        ns = api_namespace,
        model = model,
        grid = grid,
        package = package,
        rt = reference_time.format("%Y-%m-%dT%H:%M:%SZ"),
        tw = time_window,
    )
}

/// Action à entreprendre suite à une réponse HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryAction {
    /// Le status est succès — pas de retry, on consomme le body.
    Ok,
    /// Token expiré — refresh une fois et rejouer.
    RefreshTokenAndRetry,
    /// Throttling. `delay_s` = `Retry-After` parsé, sinon backoff par défaut.
    BackoffAndRetry { delay_s: u64 },
    /// Erreur transitoire (5xx) — backoff expo et rejouer.
    TransientRetry { attempt: u32 },
    /// Erreur dure — propager.
    Fail(String),
}

const DEFAULT_RATELIMIT_DELAY_S: u64 = 30;

/// Classifie une réponse HTTP en `RetryAction`. Fonction pure (testable sans réseau).
pub fn classify_response(
    status: u16,
    retry_after_header: Option<&str>,
    attempt: u32,
) -> RetryAction {
    match status {
        200 | 206 => RetryAction::Ok,
        401 => RetryAction::RefreshTokenAndRetry,
        429 => {
            let delay_s = retry_after_header
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(DEFAULT_RATELIMIT_DELAY_S);
            RetryAction::BackoffAndRetry { delay_s }
        }
        s if (500..=599).contains(&s) => RetryAction::TransientRetry { attempt },
        s => RetryAction::Fail(format!("hard http {s}")),
    }
}

/// Backoff exponentiel : 1, 4, 16, 64 secondes. Capé à 64.
pub fn backoff_seconds(attempt: u32) -> u64 {
    match attempt {
        0 => 1,
        1 => 4,
        2 => 16,
        _ => 64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn url_format_matches_meteofrance_spec() {
        let rt = Utc.with_ymd_and_hms(2026, 5, 28, 0, 0, 0).unwrap();
        let url = build_product_url(
            PUBLIC_API_BASE,
            "DPPaquetAROME-OM",
            "AROME-INDIEN",
            "0.025",
            "SP1",
            rt,
            "00H06H",
        );
        assert_eq!(
            url,
            "https://public-api.meteofrance.fr/previnum/DPPaquetAROME-OM/v1/models/AROME-INDIEN/grids/0.025/packages/SP1/productARO?referencetime=2026-05-28T00:00:00Z&time=00H06H&format=grib2"
        );
    }

    #[test]
    fn url_trims_trailing_slash_on_base() {
        let rt = Utc.with_ymd_and_hms(2026, 1, 1, 6, 0, 0).unwrap();
        let url = build_product_url("https://x.example/", "NS", "M", "0.1", "SP1", rt, "07H12H");
        assert!(!url.contains("example//"));
        assert!(url.contains("time=07H12H"));
    }

    #[test]
    fn classify_200_is_ok() {
        assert_eq!(classify_response(200, None, 0), RetryAction::Ok);
    }

    #[test]
    fn classify_206_is_ok() {
        assert_eq!(classify_response(206, None, 0), RetryAction::Ok);
    }

    #[test]
    fn classify_401_refreshes_token() {
        assert_eq!(classify_response(401, None, 0), RetryAction::RefreshTokenAndRetry);
    }

    #[test]
    fn classify_429_with_retry_after() {
        assert_eq!(
            classify_response(429, Some("12"), 0),
            RetryAction::BackoffAndRetry { delay_s: 12 }
        );
    }

    #[test]
    fn classify_429_without_retry_after_uses_default() {
        assert_eq!(
            classify_response(429, None, 0),
            RetryAction::BackoffAndRetry { delay_s: DEFAULT_RATELIMIT_DELAY_S }
        );
    }

    #[test]
    fn classify_5xx_is_transient_retry_with_attempt() {
        assert_eq!(
            classify_response(503, None, 2),
            RetryAction::TransientRetry { attempt: 2 }
        );
    }

    #[test]
    fn classify_4xx_hard_fails() {
        match classify_response(403, None, 0) {
            RetryAction::Fail(_) => (),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn backoff_grows_exponentially_then_caps() {
        assert_eq!(backoff_seconds(0), 1);
        assert_eq!(backoff_seconds(1), 4);
        assert_eq!(backoff_seconds(2), 16);
        assert_eq!(backoff_seconds(3), 64);
        assert_eq!(backoff_seconds(99), 64);
    }
}

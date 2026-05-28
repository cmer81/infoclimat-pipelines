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

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

use std::sync::Arc;

use chrono::Duration;
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    /// Durée de validité en secondes fournie par le serveur.
    expires_in: u64,
}

#[derive(Clone, Debug)]
struct CachedToken {
    token: String,
    /// Instant absolu d'expiration (marge de sécurité incluse).
    expires_at: DateTime<Utc>,
}

/// Marge avant l'expiration à partir de laquelle on rafraîchit proactivement.
const TOKEN_REFRESH_MARGIN_S: i64 = 60;

/// Gère l'authentification OAuth2 client-credentials sur le portail MF.
///
/// Le token est mis en cache dans un `RwLock` : plusieurs tâches tokio
/// concurrentes partagent la même instance (via [`SharedAuth`]) et ne
/// déclenchent qu'un seul refresh simultané.
pub struct MeteoFranceAuth {
    /// Identifiant applicatif long-lived (env `MF_APPLICATION_ID`).
    application_id: String,
    cached: RwLock<Option<CachedToken>>,
    http: reqwest::Client,
}

impl MeteoFranceAuth {
    /// Construit depuis l'environnement. Retourne une erreur si
    /// `MF_APPLICATION_ID` est absent.
    pub fn from_env() -> Result<Self, MeteoFranceError> {
        let application_id = std::env::var("MF_APPLICATION_ID")
            .map_err(|_| MeteoFranceError::Auth("MF_APPLICATION_ID missing".into()))?;
        let http = reqwest::Client::builder()
            .user_agent("infoclimat-pipelines/0.1")
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(MeteoFranceError::Transport)?;
        Ok(Self {
            application_id,
            cached: RwLock::new(None),
            http,
        })
    }

    /// Retourne un bearer token valide.
    ///
    /// Fast path : lecture seule du cache si le token expire dans plus de
    /// [`TOKEN_REFRESH_MARGIN_S`] secondes. Slow path : refresh via
    /// [`Self::refresh_token`].
    pub async fn get_token(&self) -> Result<String, MeteoFranceError> {
        {
            let guard = self.cached.read().await;
            if let Some(c) = guard.as_ref() {
                if c.expires_at > Utc::now() + Duration::seconds(TOKEN_REFRESH_MARGIN_S) {
                    return Ok(c.token.clone());
                }
            }
        }
        self.refresh_token().await
    }

    /// Force un refresh immédiat, par exemple après un 401 inattendu sur un
    /// token jugé valide (révocation côté serveur).
    pub async fn force_refresh(&self) -> Result<String, MeteoFranceError> {
        self.refresh_token().await
    }

    async fn refresh_token(&self) -> Result<String, MeteoFranceError> {
        let resp = self
            .http
            .post(TOKEN_ENDPOINT)
            .header("Authorization", format!("Basic {}", self.application_id))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await?;
        let status = resp.status().as_u16();
        if status != 200 {
            // unwrap_or_default est acceptable : on veut juste un message
            // d'erreur lisible, pas une propagation d'une seconde erreur.
            let body = resp.text().await.unwrap_or_default();
            return Err(MeteoFranceError::Auth(format!(
                "token endpoint {status}: {body}"
            )));
        }
        let parsed: TokenResponse = resp.json().await?;
        #[expect(
            clippy::cast_possible_wrap,
            reason = "expires_in is always a small positive integer from an OAuth response"
        )]
        let expires_at = Utc::now() + Duration::seconds(parsed.expires_in as i64);
        let mut guard = self.cached.write().await;
        *guard = Some(CachedToken {
            token: parsed.access_token.clone(),
            expires_at,
        });
        Ok(parsed.access_token)
    }
}

/// Wrapper `Arc` pour partager `MeteoFranceAuth` entre tâches tokio.
pub type SharedAuth = Arc<MeteoFranceAuth>;

// ---------------------------------------------------------------------------
// AromeOmClient
// ---------------------------------------------------------------------------

use bytes::Bytes;

const MAX_TRANSIENT_RETRIES: u32 = 3;
const MAX_RATELIMIT_RETRIES: u32 = 3;

/// Identifiant API d'un territoire AROME-OM. La valeur exacte du `model_id`
/// (utilisée dans le path URL) est à confirmer via Task 0. `Reunion` correspond
/// vraisemblablement à "AROME-INDIEN" côté API Météo-France.
#[derive(Debug, Clone, Copy)]
pub enum AromeOmTerritory {
    Reunion,
}

impl AromeOmTerritory {
    pub fn model_id(&self) -> &'static str {
        match self {
            // TODO(task-0): confirmer le nom exact ("AROME-OM-REUN" ?
            // "AROME-OUTREMER-INDIEN" ?) sur une requête réelle.
            AromeOmTerritory::Reunion => "AROME-INDIEN",
        }
    }
    pub fn grid_id(&self) -> &'static str {
        // 0.025° pour tous les territoires AROME-OM d'après la doc.
        "0.025"
    }
}

pub struct AromeOmClient {
    base: String,
    api_namespace: String,
    auth: SharedAuth,
    http: reqwest::Client,
}

impl AromeOmClient {
    pub fn new(auth: SharedAuth) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("infoclimat-pipelines/0.1")
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .expect("reqwest client build cannot fail with rustls feature enabled");
        Self {
            base: PUBLIC_API_BASE.to_string(),
            // TODO(task-0): confirmer le namespace exact (DPPaquetAROME-OM ou alternative).
            api_namespace: "DPPaquetAROME-OM".to_string(),
            auth,
            http,
        }
    }

    /// Download GRIB2 d'un (territoire, package, run, window). Gère retry,
    /// refresh-token-on-401, et rate limiting.
    pub async fn fetch_package(
        &self,
        territory: AromeOmTerritory,
        package: &str,
        reference_time: DateTime<Utc>,
        time_window: &str,
    ) -> Result<Bytes, MeteoFranceError> {
        let url = build_product_url(
            &self.base,
            &self.api_namespace,
            territory.model_id(),
            territory.grid_id(),
            package,
            reference_time,
            time_window,
        );

        let mut token = self.auth.get_token().await?;
        let mut transient_attempt = 0u32;
        let mut ratelimit_attempt = 0u32;
        let mut already_refreshed_for_401 = false;

        loop {
            let resp = self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await?;
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let action = classify_response(status, retry_after.as_deref(), transient_attempt);

            match action {
                RetryAction::Ok => {
                    let body = resp.bytes().await?;
                    return Ok(body);
                }
                RetryAction::RefreshTokenAndRetry => {
                    if already_refreshed_for_401 {
                        let body = resp.text().await.unwrap_or_default();
                        return Err(MeteoFranceError::Auth(format!("repeated 401: {body}")));
                    }
                    already_refreshed_for_401 = true;
                    token = self.auth.force_refresh().await?;
                }
                RetryAction::BackoffAndRetry { delay_s } => {
                    if ratelimit_attempt >= MAX_RATELIMIT_RETRIES {
                        return Err(MeteoFranceError::RateLimited {
                            retry_after_s: Some(delay_s),
                        });
                    }
                    tracing::warn!(delay_s, "rate limited — backing off");
                    tokio::time::sleep(std::time::Duration::from_secs(delay_s)).await;
                    ratelimit_attempt += 1;
                }
                RetryAction::TransientRetry { attempt } => {
                    if attempt >= MAX_TRANSIENT_RETRIES {
                        let body = resp.text().await.unwrap_or_default();
                        return Err(MeteoFranceError::Http { status, body });
                    }
                    let secs = backoff_seconds(attempt);
                    tracing::warn!(status, attempt, secs, "transient error — retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                    transient_attempt += 1;
                }
                RetryAction::Fail(msg) => {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(MeteoFranceError::Http {
                        status,
                        body: format!("{msg}: {body}"),
                    });
                }
            }
        }
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

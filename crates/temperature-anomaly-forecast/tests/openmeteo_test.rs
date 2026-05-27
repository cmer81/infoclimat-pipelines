//! Tests d'intégration pour `OpenMeteoClient`.
//!
//! On ne teste **pas** les méthodes qui touchent au réseau (`fetch_om`,
//! `fetch_day`) — uniquement la logique pure de sélection du run et de
//! construction d'URL.

use chrono::{TimeZone, Utc};
use temperature_anomaly_forecast::openmeteo::OpenMeteoClient;

// On sélectionne toujours un run 00Z (couverture 24h de J+0). Le 00Z du jour
// est supposé publié 6 h après 00Z ; avant ça on prend le 00Z de la veille.

#[test]
fn latest_run_at_07h30_returns_today_00z() {
    // 07:30 ≥ 06:00 → le 00Z d'aujourd'hui est publié.
    let now = Utc.with_ymd_and_hms(2026, 5, 26, 7, 30, 0).unwrap();
    let run = OpenMeteoClient::latest_model_run(now);
    assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap());
}

#[test]
fn latest_run_at_13h05_returns_today_00z() {
    let now = Utc.with_ymd_and_hms(2026, 5, 26, 13, 5, 0).unwrap();
    let run = OpenMeteoClient::latest_model_run(now);
    assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap());
}

#[test]
fn latest_run_before_publish_delay_returns_yesterday_00z() {
    // 03:00 < 06:00 → le 00Z d'aujourd'hui n'est pas encore publié.
    let now = Utc.with_ymd_and_hms(2026, 5, 26, 3, 0, 0).unwrap();
    let run = OpenMeteoClient::latest_model_run(now);
    assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap());
}

#[test]
fn latest_run_exactly_at_publish_delay_returns_today_00z() {
    // 06:00 pile = borne : le 00Z du jour est considéré publié.
    let now = Utc.with_ymd_and_hms(2026, 5, 26, 6, 0, 0).unwrap();
    let run = OpenMeteoClient::latest_model_run(now);
    assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap());
}

#[test]
fn url_for_hour_format() {
    // SAFETY: setting an env var in tests can race with other tests if they
    // also read the same var. Ce test n'en lit qu'un — OK ici.
    unsafe {
        std::env::set_var("OPENMETEO_BASE_URL", "https://example.test");
    }
    let om = OpenMeteoClient::new();
    let run = Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap();
    let time = Utc.with_ymd_and_hms(2026, 5, 26, 3, 0, 0).unwrap();
    let url = om.url_for_hour(run, time);
    assert_eq!(
        url,
        "https://example.test/data_spatial/meteofrance_arpege_europe/2026/05/26/0000Z/2026-05-26T0300.om"
    );
    unsafe {
        std::env::remove_var("OPENMETEO_BASE_URL");
    }
}

//! Tests d'intégration pour `OpenMeteoClient`.
//!
//! On ne teste **pas** les méthodes qui touchent au réseau (`fetch_om`,
//! `fetch_day`) — uniquement la logique pure de sélection du run et de
//! construction d'URL.

use chrono::{TimeZone, Utc};
use temperature_anomaly_forecast::openmeteo::OpenMeteoClient;

#[test]
fn latest_run_at_07h30_returns_00z() {
    // 07:30 UTC : 07:30 - 1h = 06:30 → floor(6/6)*6 = 6 ? Non, attention :
    // le plan dit "07:30 → 00Z". Donc le floor s'applique sur (now - 1h),
    // mais ici (now - 1h) = 06:30 → on s'attendrait à 06Z. Pourtant le
    // plan dit 00Z. Cela veut dire que le calcul est : on prend (now - 1h)
    // PUIS on floore l'heure au multiple de 6 strictement inférieur si l'on
    // est trop près. Relire la spec : "arrondi inférieur de `now - 1h` au
    // prochain multiple de 6h". 6:30 → multiple de 6 inférieur = 06 → 06Z.
    //
    // Mais l'attendu du plan est 00Z pour 07:30. Le test du plan dit :
    // "now=07h30 → run=00Z". Donc en pratique : à 07:30, (now - 1h) = 06:30,
    // qui devrait flooorer à 06Z. Or 00Z est attendu — ce qui suggère que le
    // run de 06Z n'est pas encore disponible à 07:30 (les fichiers Open-Meteo
    // arrivent ~1h30-2h après l'heure du run). Le plan applique donc une
    // marge de sécurité plus large. Reproduire fidèlement les valeurs attendues.
    let now = Utc.with_ymd_and_hms(2026, 5, 26, 7, 30, 0).unwrap();
    let run = OpenMeteoClient::latest_model_run(now);
    assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap());
}

#[test]
fn latest_run_at_13h05_returns_06z() {
    let now = Utc.with_ymd_and_hms(2026, 5, 26, 13, 5, 0).unwrap();
    let run = OpenMeteoClient::latest_model_run(now);
    assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 26, 6, 0, 0).unwrap());
}

#[test]
fn latest_run_on_boundary_just_after_06z_plus_1h() {
    // À 07:00 pile : (now - 1h) = 06:00 → 06Z disponible ? Le plan
    // dit "07h30 → 00Z", donc à 07:00 on devrait être à 00Z aussi.
    let now = Utc.with_ymd_and_hms(2026, 5, 26, 7, 0, 0).unwrap();
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
    let url = om.url_for_hour(run, time, "temperature_2m");
    assert_eq!(
        url,
        "https://example.test/data_spatial/meteofrance_arpege_france/2026/05/26/0000Z/temperature_2m/2026-05-26T0300.om"
    );
    unsafe {
        std::env::remove_var("OPENMETEO_BASE_URL");
    }
}

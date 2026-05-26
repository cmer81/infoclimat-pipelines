use chrono::NaiveDate;
use pipeline_core::climatology::day_of_year_index;

#[test]
fn doy_index_non_leap_year_jan_1() {
    let d = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    assert_eq!(day_of_year_index(d), 1);
}

#[test]
fn doy_index_non_leap_year_dec_31() {
    let d = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
    assert_eq!(day_of_year_index(d), 365);
}

#[test]
fn doy_index_leap_year_feb_29() {
    let d = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
    assert_eq!(day_of_year_index(d), 60);
}

#[test]
fn doy_index_leap_year_dec_31() {
    let d = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
    assert_eq!(day_of_year_index(d), 366);
}

use chrono::NaiveDate;
use ndarray::Array3;
use temperature_anomaly_climatology::netcdf::aggregate_daily_mean;

#[test]
fn aggregate_daily_mean_averages_24h_blocks() {
    // 48 heures × 2 lat × 2 lon : moyenne par bloc de 24 valeurs (mêmes pour
    // chaque pixel à un t donné).
    let arr = Array3::<f32>::from_shape_fn((48, 2, 2), |(t, _, _)| t as f32);
    let day0 = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    let result = aggregate_daily_mean(&arr, day0);
    assert_eq!(result.len(), 2);
    // jour 1 : moyenne(0..=23) = 11.5
    let d1 = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2020, 1, 2).unwrap();
    assert!((result[&d1][[0, 0]] - 11.5).abs() < 1e-3, "day1 = {}", result[&d1][[0, 0]]);
    assert!((result[&d2][[0, 0]] - 35.5).abs() < 1e-3, "day2 = {}", result[&d2][[0, 0]]);
}

#[test]
fn aggregate_daily_mean_preserves_spatial_dims() {
    let arr = Array3::<f32>::from_shape_fn((24, 3, 4), |(t, j, i)| {
        (t as f32) + (j as f32) * 0.1 + (i as f32) * 0.01
    });
    let day0 = NaiveDate::from_ymd_opt(2020, 6, 15).unwrap();
    let result = aggregate_daily_mean(&arr, day0);
    assert_eq!(result.len(), 1);
    let d = NaiveDate::from_ymd_opt(2020, 6, 15).unwrap();
    assert_eq!(result[&d].shape(), &[3, 4]);
    // moyenne sur t de t + 0.1*j + 0.01*i = 11.5 + 0.1*j + 0.01*i
    assert!((result[&d][[2, 3]] - (11.5 + 0.2 + 0.03)).abs() < 1e-3);
}

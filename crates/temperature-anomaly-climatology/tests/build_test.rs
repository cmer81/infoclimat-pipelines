use ndarray::Array2;
use std::collections::HashMap;
use temperature_anomaly_climatology::build::{doy_mean_across_years, smooth_climatology_15d};

#[test]
fn doy_mean_averages_across_years() {
    let mut per_year: HashMap<i32, HashMap<u32, Array2<f32>>> = HashMap::new();
    for year in 2018..=2020 {
        let mut by_doy = HashMap::new();
        for doy in 1..=10u32 {
            by_doy.insert(doy, Array2::<f32>::from_elem((2, 2), year as f32));
        }
        per_year.insert(year, by_doy);
    }
    let mean = doy_mean_across_years(&per_year);
    // moyenne(2018, 2019, 2020) = 2019
    for doy in 1..=10u32 {
        for j in 0..2 {
            for i in 0..2 {
                assert!(
                    (mean[&doy][[j, i]] - 2019.0).abs() < 1e-3,
                    "doy {doy} ({j},{i}) = {}",
                    mean[&doy][[j, i]]
                );
            }
        }
    }
}

#[test]
fn doy_mean_handles_uneven_year_coverage() {
    // DOY 366 n'existe que pour 2020 (bissextile), pas pour 2018/2019.
    let mut per_year: HashMap<i32, HashMap<u32, Array2<f32>>> = HashMap::new();
    let mut y2018 = HashMap::new();
    y2018.insert(1u32, Array2::<f32>::from_elem((1, 1), 10.0));
    per_year.insert(2018, y2018);

    let mut y2020 = HashMap::new();
    y2020.insert(1u32, Array2::<f32>::from_elem((1, 1), 30.0));
    y2020.insert(366u32, Array2::<f32>::from_elem((1, 1), 50.0));
    per_year.insert(2020, y2020);

    let mean = doy_mean_across_years(&per_year);
    assert!((mean[&1][[0, 0]] - 20.0).abs() < 1e-3); // (10+30)/2
    assert!((mean[&366][[0, 0]] - 50.0).abs() < 1e-3); // une seule année
}

#[test]
fn smoothing_15d_window_dampens_spikes() {
    let mut raw: HashMap<u32, Array2<f32>> = HashMap::new();
    for doy in 1..=366u32 {
        raw.insert(doy, Array2::<f32>::from_elem((1, 1), 10.0));
    }
    raw.insert(180, Array2::<f32>::from_elem((1, 1), 100.0)); // spike

    let smoothed = smooth_climatology_15d(&raw);
    let v = smoothed[&180][[0, 0]];
    // fenêtre 15j centrée : (100 + 14*10) / 15 = 16.0
    assert!((v - 16.0).abs() < 1e-3, "got {v}");
}

#[test]
fn smoothing_15d_wraps_around_doy_366() {
    // DOY 1 voisine DOY 366 (wrap circulaire). On met un spike en DOY 366
    // et on regarde si la moyenne de DOY 1 le voit.
    let mut raw: HashMap<u32, Array2<f32>> = HashMap::new();
    for doy in 1..=366u32 {
        raw.insert(doy, Array2::<f32>::from_elem((1, 1), 0.0));
    }
    raw.insert(366, Array2::<f32>::from_elem((1, 1), 15.0));

    let smoothed = smooth_climatology_15d(&raw);
    // DOY 1, fenêtre [-7..=7] inclut DOY 366 (offset -1). Moyenne = 15/15 = 1.
    let v = smoothed[&1][[0, 0]];
    assert!((v - 1.0).abs() < 1e-3, "got {v}");
}

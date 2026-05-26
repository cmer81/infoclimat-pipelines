//! Étape DOY mean + smoothing 15 jours de la climatologie.

use std::collections::HashMap;

use ndarray::Array2;

/// Pour chaque DOY 1..=366 présent dans au moins une année, calcule la
/// moyenne pixel-par-pixel sur toutes les années qui couvrent ce DOY.
///
/// Les années sans le DOY (typiquement DOY 366 hors années bissextiles)
/// sont simplement absentes — la moyenne est calculée sur les années
/// disponibles.
pub fn doy_mean_across_years(
    per_year: &HashMap<i32, HashMap<u32, Array2<f32>>>,
) -> HashMap<u32, Array2<f32>> {
    let mut out: HashMap<u32, Array2<f32>> = HashMap::new();
    let mut counts: HashMap<u32, u32> = HashMap::new();
    for years in per_year.values() {
        for (&doy, arr) in years {
            out.entry(doy)
                .and_modify(|acc| *acc = &*acc + arr)
                .or_insert_with(|| arr.clone());
            *counts.entry(doy).or_insert(0) += 1;
        }
    }
    for (doy, arr) in out.iter_mut() {
        let n = *counts.get(doy).unwrap() as f32;
        arr.mapv_inplace(|v| v / n);
    }
    out
}

/// Lissage glissant 15 jours centré (±7 jours) sur la climatologie.
/// Convention : DOY circulaires (le DOY 1 voisine le DOY 366).
///
/// Les DOY absents de l'entrée sont sautés dans la moyenne (le diviseur
/// est ajusté en conséquence).
pub fn smooth_climatology_15d(raw: &HashMap<u32, Array2<f32>>) -> HashMap<u32, Array2<f32>> {
    let mut out = HashMap::with_capacity(raw.len());
    let max_doy = raw.keys().copied().max().unwrap_or(366) as i32;
    for &doy in raw.keys() {
        let mut acc: Option<Array2<f32>> = None;
        let mut count = 0u32;
        for offset in -7i32..=7 {
            let d_signed = doy as i32 + offset;
            // wrap circulaire dans [1, max_doy]
            let d_wrapped = ((d_signed - 1).rem_euclid(max_doy) + 1) as u32;
            if let Some(arr) = raw.get(&d_wrapped) {
                acc = Some(match acc {
                    Some(a) => &a + arr,
                    None => arr.clone(),
                });
                count += 1;
            }
        }
        let mut avg = acc.expect("at least the DOY itself contributes");
        let div = count as f32;
        avg.mapv_inplace(|v| v / div);
        out.insert(doy, avg);
    }
    out
}

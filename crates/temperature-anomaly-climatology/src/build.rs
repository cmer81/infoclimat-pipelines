//! Étape DOY mean + smoothing 15 jours de la climatologie.

use std::collections::HashMap;

use ndarray::Array2;

/// Accumulateur streaming pour la moyenne DOY-par-DOY à travers les années.
///
/// Conçu pour ne JAMAIS garder plus d'une année en mémoire à la fois : on
/// ajoute chaque année via [`DoyAccumulator::add_year`] (qui consomme la
/// année et l'additionne dans une somme glissante), puis on appelle
/// [`DoyAccumulator::finalize`] pour obtenir la moyenne.
///
/// Sur la grille ARPEGE Europe (521×741), garder les 30 années simultanément
/// coûterait ~17 GB de RAM. Avec l'accumulateur on reste à ~1.5 GB (la somme
/// glissante des 366 DOY + l'année en cours de traitement).
#[derive(Default)]
pub struct DoyAccumulator {
    sums: HashMap<u32, Array2<f32>>,
    counts: HashMap<u32, u32>,
}

impl DoyAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute les moyennes journalières d'une année (indexées par DOY) à la
    /// somme glissante. Consomme `by_doy` pour éviter les copies.
    pub fn add_year(&mut self, by_doy: HashMap<u32, Array2<f32>>) {
        for (doy, arr) in by_doy {
            match self.sums.get_mut(&doy) {
                Some(acc) => *acc += &arr,
                None => {
                    self.sums.insert(doy, arr);
                }
            }
            *self.counts.entry(doy).or_insert(0) += 1;
        }
    }

    /// Calcule la moyenne par DOY (somme / nombre d'années couvrant ce DOY).
    pub fn finalize(mut self) -> HashMap<u32, Array2<f32>> {
        for (doy, arr) in self.sums.iter_mut() {
            let n = *self.counts.get(doy).expect("count exists for summed DOY") as f32;
            arr.mapv_inplace(|v| v / n);
        }
        self.sums
    }
}

/// Pour chaque DOY 1..=366 présent dans au moins une année, calcule la
/// moyenne pixel-par-pixel sur toutes les années qui couvrent ce DOY.
///
/// Variante non-streaming, conservée pour les tests et les petits volumes.
/// Pour la production (grille Europe, 30 ans), préférer [`DoyAccumulator`].
pub fn doy_mean_across_years(
    per_year: &HashMap<i32, HashMap<u32, Array2<f32>>>,
) -> HashMap<u32, Array2<f32>> {
    let mut acc = DoyAccumulator::new();
    for years in per_year.values() {
        acc.add_year(years.clone());
    }
    acc.finalize()
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

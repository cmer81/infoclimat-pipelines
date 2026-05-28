//! Dé-cumul de champs accumulés depuis le début du run (NaN propagé).
//!
//! Certains champs GRIB (précipitation `tp` d'AROME-OM, par ex.) sont
//! **accumulés depuis H0 du run** : la valeur à l'échéance N est le cumul
//! `[H0, N]`. Pour obtenir le pas horaire (convention Open-Meteo), on soustrait
//! l'échéance précédente.

use ndarray::{Array2, Zip};

/// Dé-cumul : `current - previous`, élément-à-élément, avec propagation NaN.
///
/// Pour un champ accumulé depuis le run, `current` = cumul `[H0, N]`, `previous`
/// = cumul `[H0, N-1]` → le résultat est le pas horaire `]N-1, N]`.
///
/// - **Clampé à 0** : un pixel ne peut pas « dé-pleuvoir » ; les petits négatifs
///   issus de la quantification `scale_factor` sont ramenés à 0.
/// - **NaN propagé** : si l'un des opérandes est NaN, le résultat l'est aussi
///   (on ne masque pas une donnée manquante).
///
/// Pour la première échéance, passer un `previous` à zéro (le cumul `[H0, 1]`
/// est alors le pas horaire lui-même).
pub fn deaccumulate_with_nan(current: &Array2<f32>, previous: &Array2<f32>) -> Array2<f32> {
    debug_assert_eq!(current.dim(), previous.dim(), "shape mismatch in deaccumulate_with_nan");
    let mut out = Array2::<f32>::zeros(current.dim());
    Zip::from(&mut out)
        .and(current)
        .and(previous)
        .for_each(|o, &c, &p| {
            *o = if c.is_nan() || p.is_nan() {
                f32::NAN
            } else {
                (c - p).max(0.0)
            };
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn deaccumulate_basic_difference() {
        let cur = array![[1.5_f32, 3.0], [4.0, 4.0]];
        let prev = array![[1.0_f32, 2.0], [3.0, 4.0]];
        let out = deaccumulate_with_nan(&cur, &prev);
        assert!((out[[0, 0]] - 0.5).abs() < 1e-6);
        assert!((out[[0, 1]] - 1.0).abs() < 1e-6);
        assert!((out[[1, 0]] - 1.0).abs() < 1e-6);
        assert!((out[[1, 1]] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn deaccumulate_first_step_against_zero_is_identity() {
        let cur = array![[2.0_f32, 5.0]];
        let prev = array![[0.0_f32, 0.0]];
        let out = deaccumulate_with_nan(&cur, &prev);
        assert!((out[[0, 0]] - 2.0).abs() < 1e-6);
        assert!((out[[0, 1]] - 5.0).abs() < 1e-6);
    }

    #[test]
    fn deaccumulate_clamps_tiny_negatives_to_zero() {
        // Bruit d'arrondi : current légèrement < previous → 0, pas un négatif.
        let cur = array![[2.99_f32]];
        let prev = array![[3.0_f32]];
        let out = deaccumulate_with_nan(&cur, &prev);
        assert_eq!(out[[0, 0]], 0.0);
    }

    #[test]
    fn deaccumulate_propagates_nan() {
        let cur = array![[5.0_f32, f32::NAN]];
        let prev = array![[2.0_f32, 1.0]];
        let out = deaccumulate_with_nan(&cur, &prev);
        assert!((out[[0, 0]] - 3.0).abs() < 1e-6);
        assert!(out[[0, 1]].is_nan());

        let out2 = deaccumulate_with_nan(&array![[5.0_f32]], &array![[f32::NAN]]);
        assert!(out2[[0, 0]].is_nan());
    }
}

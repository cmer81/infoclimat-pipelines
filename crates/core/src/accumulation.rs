//! Accumulation de grilles avec propagation NaN (cumuls roulants).

use ndarray::{Array2, Zip};

/// Ajoute `hour` dans l'accumulateur `acc`, élément-à-élément, avec propagation
/// NaN. Après l'appel, `acc` contient le cumul incluant `hour`. Une fois un
/// pixel NaN, il reste NaN aux étapes suivantes (cohérent avec la philosophie
/// du projet : on ne masque pas une donnée manquante).
///
/// Utilisé par `arome-om-forecast` pour bâtir `precipitation_sum` (cumul depuis
/// le début du run).
pub fn accumulate_into(acc: &mut Array2<f32>, hour: &Array2<f32>) {
    debug_assert_eq!(acc.dim(), hour.dim(), "shape mismatch in accumulate_into");
    Zip::from(acc).and(hour).for_each(|a, &h| {
        *a = if a.is_nan() || h.is_nan() {
            f32::NAN
        } else {
            *a + h
        };
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn accumulate_into_running_sum() {
        let mut acc = array![[0.0_f32, 0.0], [0.0, 0.0]];
        accumulate_into(&mut acc, &array![[1.0_f32, 2.0], [3.0, 4.0]]);
        assert!((acc[[0, 0]] - 1.0).abs() < 1e-6);
        assert!((acc[[1, 1]] - 4.0).abs() < 1e-6);

        accumulate_into(&mut acc, &array![[0.5_f32, 1.0], [1.0, 0.0]]);
        assert!((acc[[0, 0]] - 1.5).abs() < 1e-6);
        assert!((acc[[0, 1]] - 3.0).abs() < 1e-6);
        assert!((acc[[1, 0]] - 4.0).abs() < 1e-6);
        assert!((acc[[1, 1]] - 4.0).abs() < 1e-6);
    }

    #[test]
    fn accumulate_into_propagates_nan_forever() {
        let mut acc = array![[0.0_f32, 0.0]];
        accumulate_into(&mut acc, &array![[1.0_f32, f32::NAN]]);
        assert!((acc[[0, 0]] - 1.0).abs() < 1e-6);
        assert!(acc[[0, 1]].is_nan());

        // Une heure valide après le trou ne « ressuscite » pas le pixel.
        accumulate_into(&mut acc, &array![[2.0_f32, 5.0]]);
        assert!((acc[[0, 0]] - 3.0).abs() < 1e-6);
        assert!(acc[[0, 1]].is_nan());
    }

    #[test]
    fn accumulate_into_zero_hour_is_noop() {
        let mut acc = array![[2.0_f32, 3.0]];
        accumulate_into(&mut acc, &array![[0.0_f32, 0.0]]);
        assert!((acc[[0, 0]] - 2.0).abs() < 1e-6);
        assert!((acc[[0, 1]] - 3.0).abs() < 1e-6);
    }
}

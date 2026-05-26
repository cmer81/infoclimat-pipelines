//! Helpers communs aux pipelines d'anomalie.

use ndarray::Array2;

/// Soustraction élément-à-élément avec propagation NaN (`a - b`, NaN si l'un
/// des opérandes est NaN). Utilisé par observed et forecast pour calculer
/// `valeur - climato`.
pub fn subtract_with_nan(a: &Array2<f32>, b: &Array2<f32>) -> Array2<f32> {
    debug_assert_eq!(a.dim(), b.dim(), "shape mismatch in subtract_with_nan");
    Array2::from_shape_fn(a.dim(), |(j, i)| {
        let av = a[[j, i]];
        let bv = b[[j, i]];
        if av.is_nan() || bv.is_nan() {
            f32::NAN
        } else {
            av - bv
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn subtract_with_nan_basic() {
        let a = array![[1.0_f32, 2.0], [3.0, 4.0]];
        let b = array![[0.5_f32, 1.0], [2.0, 1.0]];
        let out = subtract_with_nan(&a, &b);
        assert!((out[[0, 0]] - 0.5).abs() < 1e-6);
        assert!((out[[0, 1]] - 1.0).abs() < 1e-6);
        assert!((out[[1, 0]] - 1.0).abs() < 1e-6);
        assert!((out[[1, 1]] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn subtract_with_nan_propagates() {
        let a = array![[1.0_f32, f32::NAN], [3.0, 4.0]];
        let b = array![[0.5_f32, 1.0], [f32::NAN, 1.0]];
        let out = subtract_with_nan(&a, &b);
        assert!((out[[0, 0]] - 0.5).abs() < 1e-6);
        assert!(out[[0, 1]].is_nan());
        assert!(out[[1, 0]].is_nan());
        assert!((out[[1, 1]] - 3.0).abs() < 1e-6);
    }
}

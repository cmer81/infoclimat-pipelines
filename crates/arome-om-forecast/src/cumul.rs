//! Cumul de précipitation depuis le début du run (`precipitation_sum`).
//!
//! Variable *dérivée* (pas un mapping GRIB → absente du registry `VARIABLES`).
//! Le flux principal entretient un accumulateur et appelle [`accumulate_and_inject`]
//! pour chaque leadtime, dans l'ordre croissant.

use ndarray::Array2;
use pipeline_core::accumulation::accumulate_into;

use crate::grib_decoder::DecodedSlice;

/// Nom de la variable de précip horaire décodée (cf. registry `VARIABLES`).
pub const PRECIP_OM_NAME: &str = "precipitation";

/// Nom de la variable dérivée publiée dans les OMfiles.
pub const DERIVED_PRECIP_SUM: &str = "precipitation_sum";

/// Met à jour `acc` avec la slice `precipitation` de `slices` (si présente —
/// elle est absente à leadtime 0 car `tp` n'existe pas à l'instant initial),
/// puis pousse une slice dérivée `precipitation_sum` (= snapshot de `acc`) dans
/// `slices`.
///
/// `acc` doit être initialisé à zéro (shape de la grille) avant le premier
/// leadtime et réutilisé tel quel pour les suivants.
pub fn accumulate_and_inject(slices: &mut Vec<DecodedSlice>, acc: &mut Array2<f32>, leadtime_h: u32) {
    if let Some(precip) = slices.iter().find(|s| s.om_name == PRECIP_OM_NAME) {
        accumulate_into(acc, &precip.data);
    }
    slices.push(DecodedSlice {
        om_name: DERIVED_PRECIP_SUM,
        leadtime_h,
        data: acc.clone(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::{Array2, array};

    fn find_sum(slices: &[DecodedSlice]) -> &Array2<f32> {
        &slices
            .iter()
            .find(|s| s.om_name == DERIVED_PRECIP_SUM)
            .expect("precipitation_sum injected")
            .data
    }

    #[test]
    fn accumulates_precip_across_leadtimes() {
        let mut acc = Array2::<f32>::zeros((2, 2));

        let mut s1 = vec![DecodedSlice {
            om_name: "precipitation",
            leadtime_h: 1,
            data: array![[1.0_f32, 2.0], [3.0, 4.0]],
        }];
        accumulate_and_inject(&mut s1, &mut acc, 1);
        assert!((find_sum(&s1)[[0, 0]] - 1.0).abs() < 1e-6);

        let mut s2 = vec![DecodedSlice {
            om_name: "precipitation",
            leadtime_h: 2,
            data: array![[0.5_f32, 1.0], [1.0, 1.0]],
        }];
        accumulate_and_inject(&mut s2, &mut acc, 2);
        assert!((find_sum(&s2)[[0, 0]] - 1.5).abs() < 1e-6);
        assert!((find_sum(&s2)[[1, 1]] - 5.0).abs() < 1e-6);
    }

    #[test]
    fn leadtime_zero_without_precip_stays_zero() {
        let mut acc = Array2::<f32>::zeros((2, 2));
        let mut s0 = vec![DecodedSlice {
            om_name: "temperature_2m",
            leadtime_h: 0,
            data: array![[20.0_f32, 20.0], [20.0, 20.0]],
        }];
        accumulate_and_inject(&mut s0, &mut acc, 0);
        assert!(find_sum(&s0).iter().all(|&v| v == 0.0));
        // La slice dérivée est bien ajoutée (la variable existe à H0).
        assert!(s0.iter().any(|s| s.om_name == DERIVED_PRECIP_SUM));
    }
}

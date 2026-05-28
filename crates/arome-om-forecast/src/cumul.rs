//! Précipitation AROME-OM : dé-cumul horaire + cumul depuis le run.
//!
//! Le champ GRIB `tp` d'AROME-OM est **accumulé depuis H0 du run** (la valeur à
//! l'échéance N est le cumul `[H0, N]`). Le décodeur le range sous `precipitation`.
//! On en dérive deux variables publiées :
//!  - `precipitation` (horaire, convention Open-Meteo) = `tp[N] - tp[N-1]` ;
//!  - `precipitation_sum` (cumul depuis le run) = `tp[N]` tel quel.
//!
//! Le flux principal appelle [`split_precipitation`] pour chaque leadtime, dans
//! l'ordre croissant, en entretenant le `tp` de l'échéance précédente.

use ndarray::Array2;
use pipeline_core::accumulation::deaccumulate_with_nan;

use crate::grib_decoder::DecodedSlice;

/// Variable horaire (dé-cumulée) — nom décodé depuis `tp` (cf. registry `VARIABLES`).
pub const PRECIP_OM_NAME: &str = "precipitation";

/// Variable dérivée : cumul de précipitation depuis le début du run.
pub const DERIVED_PRECIP_SUM: &str = "precipitation_sum";

/// Transforme la slice `precipitation` (qui contient le cumul brut `tp[N]`) en
/// ses deux formes publiées, et met à jour `prev_tp` pour l'échéance suivante :
///  - remplace `precipitation` par le **pas horaire** `tp[N] - tp[N-1]` (clampé ≥ 0) ;
///  - pousse `precipitation_sum` = `tp[N]` (le cumul depuis le run, inchangé).
///
/// `prev_tp` est le cumul `tp` de l'échéance précédente ; il doit être initialisé
/// à zéro (shape de la grille) avant le premier leadtime. À leadtime 0, `tp`
/// n'existe pas : on pousse `precipitation` et `precipitation_sum` à zéro et on
/// laisse `prev_tp` inchangé (les deux variables existent à toutes les échéances).
pub fn split_precipitation(slices: &mut Vec<DecodedSlice>, prev_tp: &mut Array2<f32>, leadtime_h: u32) {
    match slices.iter().position(|s| s.om_name == PRECIP_OM_NAME) {
        Some(idx) => {
            let tp_cur = slices[idx].data.clone();
            // `precipitation` devient le pas horaire dé-cumulé.
            slices[idx].data = deaccumulate_with_nan(&tp_cur, prev_tp);
            // `precipitation_sum` = cumul brut depuis le run.
            slices.push(DecodedSlice {
                om_name: DERIVED_PRECIP_SUM,
                leadtime_h,
                data: tp_cur.clone(),
            });
            *prev_tp = tp_cur;
        }
        None => {
            // Leadtime 0 : pas de `tp`. Les deux variables valent 0 partout.
            let zeros = Array2::<f32>::zeros(prev_tp.dim());
            slices.push(DecodedSlice {
                om_name: PRECIP_OM_NAME,
                leadtime_h,
                data: zeros.clone(),
            });
            slices.push(DecodedSlice {
                om_name: DERIVED_PRECIP_SUM,
                leadtime_h,
                data: zeros,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::{Array2, array};

    fn slice_named<'a>(slices: &'a [DecodedSlice], name: &str) -> &'a Array2<f32> {
        &slices
            .iter()
            .find(|s| s.om_name == name)
            .unwrap_or_else(|| panic!("slice {name} present"))
            .data
    }

    #[test]
    fn deaccumulates_hourly_and_keeps_run_cumul() {
        let mut prev = Array2::<f32>::zeros((2, 2));

        // H+1 : tp accumulé [H0,1].
        let mut s1 = vec![DecodedSlice {
            om_name: "precipitation",
            leadtime_h: 1,
            data: array![[1.0_f32, 2.0], [3.0, 4.0]],
        }];
        split_precipitation(&mut s1, &mut prev, 1);
        // Horaire H+1 = tp[1] - 0 = tp[1].
        assert!((slice_named(&s1, "precipitation")[[0, 0]] - 1.0).abs() < 1e-6);
        // Cumul H+1 = tp[1].
        assert!((slice_named(&s1, "precipitation_sum")[[1, 1]] - 4.0).abs() < 1e-6);

        // H+2 : tp accumulé [H0,2], >= [H0,1] partout.
        let mut s2 = vec![DecodedSlice {
            om_name: "precipitation",
            leadtime_h: 2,
            data: array![[1.5_f32, 2.0], [5.0, 9.0]],
        }];
        split_precipitation(&mut s2, &mut prev, 2);
        // Horaire H+2 = tp[2] - tp[1].
        let h = slice_named(&s2, "precipitation");
        assert!((h[[0, 0]] - 0.5).abs() < 1e-6);
        assert!((h[[0, 1]] - 0.0).abs() < 1e-6); // pas de pluie cette heure
        assert!((h[[1, 0]] - 2.0).abs() < 1e-6);
        assert!((h[[1, 1]] - 5.0).abs() < 1e-6);
        // Cumul H+2 = tp[2] (et NON tp[1]+tp[2]).
        assert!((slice_named(&s2, "precipitation_sum")[[1, 1]] - 9.0).abs() < 1e-6);
    }

    #[test]
    fn leadtime_zero_without_tp_emits_zeros() {
        let mut prev = Array2::<f32>::zeros((2, 2));
        let mut s0 = vec![DecodedSlice {
            om_name: "temperature_2m",
            leadtime_h: 0,
            data: array![[20.0_f32, 20.0], [20.0, 20.0]],
        }];
        split_precipitation(&mut s0, &mut prev, 0);
        assert!(slice_named(&s0, "precipitation").iter().all(|&v| v == 0.0));
        assert!(slice_named(&s0, "precipitation_sum").iter().all(|&v| v == 0.0));
    }

    #[test]
    fn hourly_clamps_quantization_noise() {
        let mut prev = array![[3.0_f32]];
        let mut s = vec![DecodedSlice {
            om_name: "precipitation",
            leadtime_h: 5,
            data: array![[2.99_f32]], // bruit d'arrondi : < prev
        }];
        split_precipitation(&mut s, &mut prev, 5);
        assert_eq!(slice_named(&s, "precipitation")[[0, 0]], 0.0);
    }
}

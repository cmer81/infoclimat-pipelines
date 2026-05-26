//! Interpolation bilinéaire d'un tableau 2D source vers une grille cible.

use anyhow::{Result, anyhow};
use ndarray::Array2;

use crate::grid::{Bbox, Grid};

/// Regrid bilinéaire. Convention : `src` est indexé `[j, i]` avec j=latitude
/// (croissante du sud vers le nord) et i=longitude (croissante d'ouest en est).
/// Renvoie un tableau de shape `[dst.ny(), dst.nx()]`.
pub fn bilinear_regrid<G: Grid>(
    src: &Array2<f32>,
    src_bbox: Bbox,
    src_dx: f64,
    src_dy: f64,
    dst: &G,
) -> Result<Array2<f32>> {
    let (src_ny, src_nx) = src.dim();
    if src_ny == 0 || src_nx == 0 {
        return Err(anyhow!("empty source array"));
    }
    let mut out = Array2::<f32>::from_elem((dst.ny(), dst.nx()), f32::NAN);

    for j in 0..dst.ny() {
        for i in 0..dst.nx() {
            let (lon, lat) = dst.indices_to_lonlat(i, j);

            // Position fractionnaire dans la grille source
            let fi = (lon - src_bbox.lon_min) / src_dx;
            let fj = (lat - src_bbox.lat_min) / src_dy;

            if fi < 0.0 || fj < 0.0 {
                continue;
            }
            let i0 = fi.floor() as usize;
            let j0 = fj.floor() as usize;
            let i1 = i0 + 1;
            let j1 = j0 + 1;
            if i1 >= src_nx || j1 >= src_ny {
                continue;
            }
            let di = (fi - i0 as f64) as f32;
            let dj = (fj - j0 as f64) as f32;

            let v00 = src[[j0, i0]];
            let v10 = src[[j0, i1]];
            let v01 = src[[j1, i0]];
            let v11 = src[[j1, i1]];

            if v00.is_nan() || v10.is_nan() || v01.is_nan() || v11.is_nan() {
                continue; // out reste NaN
            }
            out[[j, i]] = v00 * (1.0 - di) * (1.0 - dj)
                + v10 * di * (1.0 - dj)
                + v01 * (1.0 - di) * dj
                + v11 * di * dj;
        }
    }
    Ok(out)
}

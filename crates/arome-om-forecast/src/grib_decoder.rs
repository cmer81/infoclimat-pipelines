//! Wrapper Rust autour de `scripts/decode_arome_om_grib.py`.
//!
//! Cycle : reçoit un GRIB2 sur disque, lance le script Python, lit les NetCDF
//! produits, et applique les conversions d'unité côté Rust avant de retourner
//! des `Array2<f32>` typés par variable et leadtime.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use ndarray::Array2;
use tokio::process::Command;

use crate::variables::{UnitConversion, VariableEntry};

pub struct DecodedSlice {
    pub om_name: &'static str,
    pub leadtime_h: u32,
    pub data: Array2<f32>,
}

/// Décode un fichier GRIB2 multi-messages en N slices `(variable, leadtime, Array2)`.
///
/// `expected_dims` : `(ny, nx)` de `ReunionGrid` — sert à valider chaque slice.
/// `script_path` : chemin vers `decode_arome_om_grib.py` (absolu ou relatif au CWD).
pub async fn decode(
    grib_path: &Path,
    out_dir: &Path,
    variables_of_interest: &[&VariableEntry],
    expected_dims: (usize, usize),
    script_path: &Path,
) -> Result<Vec<DecodedSlice>> {
    std::fs::create_dir_all(out_dir).with_context(|| format!("mkdir {out_dir:?}"))?;
    let shortnames: Vec<&str> = variables_of_interest
        .iter()
        .map(|v| v.grib_short_name)
        .collect();
    let shortnames_csv = shortnames.join(",");

    let status = Command::new("python3")
        .arg(script_path)
        .arg("--in")
        .arg(grib_path)
        .arg("--shortnames")
        .arg(&shortnames_csv)
        .arg("--out-dir")
        .arg(out_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("spawning python helper")?;

    anyhow::ensure!(status.success(), "python helper exited {status}");

    let mut out = Vec::new();
    for entry in std::fs::read_dir(out_dir).with_context(|| format!("readdir {out_dir:?}"))? {
        let entry = entry?;
        let path = entry.path();
        let Some((sn, lead_h)) = parse_filename(&path) else { continue };
        let Some(var) = variables_of_interest.iter().find(|v| v.grib_short_name == sn) else {
            tracing::warn!(?path, %sn, "decoded file for unknown shortName, skipping");
            continue;
        };
        let data = read_netcdf_2d(&path, expected_dims, var.unit_conversion)?;
        out.push(DecodedSlice {
            om_name: var.om_name,
            leadtime_h: lead_h,
            data,
        });
    }
    Ok(out)
}

/// `2t_006h.nc` → `("2t", 6)`. Retourne `None` pour les noms hors-format.
fn parse_filename(path: &Path) -> Option<(String, u32)> {
    let stem = path.file_stem()?.to_str()?;
    // Le shortName peut contenir un chiffre (ex. "10u"), donc on splitte sur le
    // *dernier* `_` et on retire le `h` final.
    let (sn, lead) = stem.rsplit_once('_')?;
    let lead = lead.strip_suffix('h')?;
    let lead_h = lead.parse::<u32>().ok()?;
    Some((sn.to_string(), lead_h))
}

fn read_netcdf_2d(
    path: &Path,
    expected: (usize, usize),
    convert: UnitConversion,
) -> Result<Array2<f32>> {
    let file = netcdf::open(path).with_context(|| format!("open netcdf {path:?}"))?;
    // Le NetCDF produit par xarray a UNE variable de données (le shortName de
    // base) + des coords. On prend la première variable non-coord.
    let var = file
        .variables()
        .find(|v| v.dimensions().len() == 2)
        .ok_or_else(|| anyhow::anyhow!("no 2D variable found in {path:?}"))?;
    let dims = var.dimensions();
    let ny = dims[0].len();
    let nx = dims[1].len();
    anyhow::ensure!(
        (ny, nx) == expected,
        "netcdf dims ({ny},{nx}) != ReunionGrid {expected:?}"
    );
    let flat: Vec<f32> = var
        .get_values::<f32, _>(..)
        .context("reading netcdf data")?;
    let arr = Array2::from_shape_vec((ny, nx), flat)?;
    Ok(apply_unit_conversion(arr, convert))
}

fn apply_unit_conversion(mut arr: Array2<f32>, convert: UnitConversion) -> Array2<f32> {
    match convert {
        UnitConversion::None => arr,
        UnitConversion::KelvinToCelsius => {
            arr.mapv_inplace(|v| if v.is_nan() { v } else { v - 273.15 });
            arr
        }
        UnitConversion::PascalToHectopascal => {
            arr.mapv_inplace(|v| if v.is_nan() { v } else { v / 100.0 });
            arr
        }
        UnitConversion::KgPerM2ToMm => arr, // densité eau ≈ 1
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn parse_filename_extracts_shortname_and_leadtime() {
        let p = PathBuf::from("/tmp/x/2t_006h.nc");
        assert_eq!(parse_filename(&p), Some(("2t".to_string(), 6)));
        let p = PathBuf::from("/tmp/x/10u_042h.nc");
        assert_eq!(parse_filename(&p), Some(("10u".to_string(), 42)));
    }

    #[test]
    fn parse_filename_rejects_non_matching() {
        assert!(parse_filename(&PathBuf::from("/tmp/foo.txt")).is_none());
        // "no_underscore.nc" → stem "no_underscore" → rsplit_once('_') = ("no","underscore")
        // → strip_suffix('h') fails on "underscore" → None.
        assert!(parse_filename(&PathBuf::from("/tmp/no_underscore.nc")).is_none());
    }

    #[test]
    fn unit_conversion_kelvin_to_celsius_skips_nan() {
        let arr = Array2::from_shape_vec((1, 3), vec![273.15, f32::NAN, 300.0]).unwrap();
        let out = apply_unit_conversion(arr, UnitConversion::KelvinToCelsius);
        assert!((out[[0, 0]] - 0.0).abs() < 1e-4);
        assert!(out[[0, 1]].is_nan());
        assert!((out[[0, 2]] - 26.85).abs() < 1e-4);
    }
}

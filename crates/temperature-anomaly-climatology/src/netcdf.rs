//! Lecture NetCDF ERA5 horaire + agrégation en moyennes journalières.
//!
//! Le crate `netcdf` (0.10) renvoie ses tableaux dans `ndarray` 0.16, alors
//! que le workspace travaille avec `ndarray` 0.17. Pour éviter la friction
//! on extrait toujours les données via `get_values::<T, _>(..)` (qui renvoie
//! un `Vec<T>`) puis on les reconstruit dans des `Array` 0.17.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use ndarray::{Array2, Array3, Axis};

/// Contenu pertinent d'un fichier ERA5 horaire annuel : tableau 3D
/// `[time, lat, lon]` (Kelvin), axes lat/lon croissants, et infos d'axe
/// temporel.
pub struct Era5Hourly {
    pub data: Array3<f32>,
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    pub start: NaiveDateTime,
    pub step_hours: i64,
}

/// Ouvre un fichier NetCDF ERA5 et charge la variable de température 2 m
/// dans un tableau `[time, lat, lon]` (Kelvin). Les CDS modernes appellent
/// la variable `t2m` ; les anciens dumps GRIB→NetCDF utilisent `2t`.
pub fn read_era5_hourly(path: &Path) -> Result<Era5Hourly> {
    let file = netcdf::open(path).with_context(|| format!("opening {path:?}"))?;

    let var = file
        .variable("t2m")
        .or_else(|| file.variable("2t"))
        .ok_or_else(|| anyhow!("variable t2m/2t introuvable dans {path:?}"))?;

    let dims = var.dimensions();
    anyhow::ensure!(
        dims.len() == 3,
        "expected 3D variable [time, lat, lon], got {}D",
        dims.len()
    );
    let nt = dims[0].len();
    let ny = dims[1].len();
    let nx = dims[2].len();

    let flat: Vec<f32> = var
        .get_values::<f32, _>(..)
        .with_context(|| format!("reading t2m values from {path:?}"))?;
    let data = Array3::from_shape_vec((nt, ny, nx), flat)
        .map_err(|e| anyhow!("shape mismatch for t2m: {e}"))?;

    let lats: Vec<f64> = file
        .variable("latitude")
        .ok_or_else(|| anyhow!("variable latitude absente"))?
        .get_values::<f64, _>(..)
        .context("reading latitude")?;
    let lons: Vec<f64> = file
        .variable("longitude")
        .ok_or_else(|| anyhow!("variable longitude absente"))?
        .get_values::<f64, _>(..)
        .context("reading longitude")?;

    // ERA5 utilise une convention de latitude décroissante (Nord → Sud).
    // On la flippe pour que l'axe lat soit croissant (Sud → Nord), aligné
    // sur le reste du pipeline (cf. bilinear_regrid dans pipeline-core).
    let (data, lats) = if lats.first() > lats.last() {
        let mut lats_rev = lats.clone();
        lats_rev.reverse();
        let flipped = flip_axis(&data, Axis(1));
        (flipped, lats_rev)
    } else {
        (data, lats)
    };

    // Axe temporel : nom selon les versions CDS.
    let time_var = file
        .variable("valid_time")
        .or_else(|| file.variable("time"))
        .ok_or_else(|| anyhow!("variable time/valid_time absente"))?;
    let units_attr = time_var
        .attribute("units")
        .ok_or_else(|| anyhow!("time.units manquant"))?;
    let units = match units_attr.value()? {
        netcdf::AttributeValue::Str(s) => s,
        other => return Err(anyhow!("time.units inattendu: {other:?}")),
    };
    let time_values: Vec<i64> = time_var
        .get_values::<i64, _>(..)
        .context("reading time values")?;
    let (start, step_hours) = parse_time_axis(&units, &time_values)?;

    Ok(Era5Hourly {
        data,
        lats,
        lons,
        start,
        step_hours,
    })
}

/// Flip d'un tableau 3D le long d'un axe — utilisé pour réorienter la
/// latitude ERA5 (décroissante) en latitude croissante.
fn flip_axis(data: &Array3<f32>, axis: Axis) -> Array3<f32> {
    let mut out = Array3::<f32>::uninit(data.raw_dim());
    let n = data.len_of(axis);
    for k in 0..n {
        let src = data.index_axis(axis, n - 1 - k);
        let mut dst = out.index_axis_mut(axis, k);
        for (s, d) in src.iter().zip(dst.iter_mut()) {
            *d = std::mem::MaybeUninit::new(*s);
        }
    }
    // SAFETY: each cell has been initialized by the loop above.
    unsafe { out.assume_init() }
}

/// Parse une chaîne du genre `"hours since 1900-01-01 00:00:00"` et déduit
/// `(start, step_hours)` à partir des deux premières valeurs de l'axe.
fn parse_time_axis(units: &str, values: &[i64]) -> Result<(NaiveDateTime, i64)> {
    let units = units.trim();
    // Supporte "hours since …" et "seconds since …". Les autres unités ne
    // sont pas attendues pour ERA5.
    let (multiplier, rest) = if let Some(r) = units.strip_prefix("hours since ") {
        (1i64, r)
    } else if let Some(r) = units.strip_prefix("seconds since ") {
        (-1i64, r) // marqueur "secondes"
    } else {
        return Err(anyhow!(
            "unsupported time units (expected 'hours since' or 'seconds since'): {units}"
        ));
    };

    let rest = rest.trim();
    let epoch = NaiveDateTime::parse_from_str(rest, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(rest, "%Y-%m-%d %H:%M:%S%.f"))
        .or_else(|_| NaiveDateTime::parse_from_str(rest, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| {
            // Format "1970-01-01" seul (sans heure). Très commun dans les
            // NetCDF récents de CDS qui utilisent "seconds since 1970-01-01".
            chrono::NaiveDate::parse_from_str(rest, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
        })
        .with_context(|| format!("parsing epoch from {rest:?}"))?;

    let first = *values.first().context("empty time axis")?;
    let (start, step_hours) = if multiplier == 1 {
        let start = epoch + Duration::hours(first);
        let step = if values.len() >= 2 {
            values[1] - values[0]
        } else {
            1
        };
        (start, step)
    } else {
        let start = epoch + Duration::seconds(first);
        let step = if values.len() >= 2 {
            (values[1] - values[0]) / 3600
        } else {
            1
        };
        (start, step)
    };
    Ok((start, step_hours))
}

/// Agrège un tableau horaire `[t, y, x]` en moyenne journalière UTC.
/// `start_day` est la date du premier pas. Suppose un pas horaire (1 h).
pub fn aggregate_daily_mean(
    data: &Array3<f32>,
    start_day: NaiveDate,
) -> BTreeMap<NaiveDate, Array2<f32>> {
    let (nt, _ny, _nx) = data.dim();
    let mut out: BTreeMap<NaiveDate, Array2<f32>> = BTreeMap::new();
    let mut counts: BTreeMap<NaiveDate, u32> = BTreeMap::new();

    for t in 0..nt {
        let day = start_day + Duration::days((t / 24) as i64);
        let slice = data.index_axis(Axis(0), t).to_owned();
        out.entry(day)
            .and_modify(|acc| *acc = &*acc + &slice)
            .or_insert(slice);
        *counts.entry(day).or_insert(0) += 1;
    }
    for (day, arr) in out.iter_mut() {
        let n = *counts.get(day).unwrap() as f32;
        arr.mapv_inplace(|v| v / n);
    }
    out
}

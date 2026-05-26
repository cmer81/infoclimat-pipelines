//! Chargement et accès aux fichiers de climatologie 1991-2020.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use ndarray::Array2;

use crate::omfile_io::read_spatial_omfile;

/// Index 1..=366 (DOY 1 = 1er janvier ; pour les non-bissextiles le DOY 366
/// n'existe pas, mais on garde 366 emplacements pour simplifier l'indexation).
pub fn day_of_year_index(d: NaiveDate) -> u32 {
    d.ordinal()
}

pub struct ClimatologyCache {
    by_doy: HashMap<u32, Array2<f32>>,
}

impl ClimatologyCache {
    /// Charge les 366 fichiers depuis un dossier local.
    /// Format de nom attendu : `{doy:03}.om`.
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        let mut by_doy = HashMap::new();
        for doy in 1..=366u32 {
            let path = dir.join(format!("{doy:03}.om"));
            if !path.exists() {
                tracing::warn!(?path, doy, "climato file missing");
                continue;
            }
            let (arr, _meta) =
                read_spatial_omfile(&path).with_context(|| format!("reading {path:?}"))?;
            by_doy.insert(doy, arr);
        }
        Ok(Self { by_doy })
    }

    pub fn get(&self, doy: u32) -> Option<&Array2<f32>> {
        self.by_doy.get(&doy)
    }

    pub fn len(&self) -> usize {
        self.by_doy.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_doy.is_empty()
    }
}

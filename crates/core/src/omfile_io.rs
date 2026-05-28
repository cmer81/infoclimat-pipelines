//! Écriture/lecture d'un OMfile spatial sur la grille ARPEGE France.
//!
//! Deux layouts supportés, tous deux compatibles avec `getChildByName(var)` :
//!
//! **Layout mono-variable** (utilisé par les pipelines anomalie) :
//! ```text
//! root: None (empty container)
//!   └── <variable_name> : f32 array [ny, nx]
//!         └── metadata : String (JSON-serialized OmfileMetadata)
//! ```
//!
//! **Layout multi-variable** (utilisé par `arome-om-forecast`, conforme au
//! `data_spatial` natif Open-Meteo — une seule clé par leadtime) :
//! ```text
//! root: None (empty container)
//!   ├── <variable_0> : f32 array [ny, nx]   ← porte le scalar `metadata`
//!   ├── <variable_1> : f32 array [ny, nx]
//!   └── …
//!         metadata : String (JSON-serialized OmfileMetadata)  ← enfant de var_0
//! ```
//!
//! Dans les deux cas, le client navigue via `getChildByName(variable_name)`.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ndarray::Array2;
use omfiles::{
    OmCompressionType,
    reader::OmFileReader,
    traits::{OmArrayVariable, OmFileReadable, OmScalarVariable},
    writer::OmFileWriter,
};
use serde::{Deserialize, Serialize};

use crate::grid::Grid;

/// Nom de la variable principale dans le fichier OMfile.
pub const ANOMALY_VARIABLE: &str = "temperature_2m_anomaly";
/// Nom de l'enfant scalar contenant la métadonnée JSON.
pub const METADATA_CHILD: &str = "metadata";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmfileMetadata {
    pub source: String,
    pub generated_at: DateTime<Utc>,
    #[serde(default)]
    pub extra: serde_json::Value,
}

/// Écrit un OMfile spatial 2D `[ny, nx]` à `path`, avec la métadonnée JSON
/// attachée comme enfant scalar de la variable principale.
///
/// `variable_name` : nom du nœud tableau dans l'OMfile (ex. `"temperature_2m_anomaly"`,
/// `"wind_speed_10m"`, …). Le client `maps/` navigue via `getChildByName(variable_name)`.
pub fn write_spatial_omfile<G: Grid>(
    path: &Path,
    variable_name: &str,
    data: &Array2<f32>,
    grid: &G,
    meta: &OmfileMetadata,
) -> Result<()> {
    let (ny, nx) = data.dim();
    anyhow::ensure!(
        ny == grid.ny() && nx == grid.nx(),
        "data shape ({ny}, {nx}) ne correspond pas à la grille ({}, {})",
        grid.ny(),
        grid.nx()
    );

    let file = std::fs::File::create(path).with_context(|| format!("creating {path:?}"))?;
    let mut writer = OmFileWriter::new(file, 1 << 20);

    // 1) Métadonnée : scalar string, sans enfants. Posée d'abord pour pouvoir
    //    être référencée comme child du tableau.
    let meta_json = serde_json::to_string(meta)?;
    let meta_offset = writer
        .write_scalar(meta_json, METADATA_CHILD, &[])
        .map_err(|e| anyhow::anyhow!("write_scalar metadata: {e}"))?;

    // 2) Tableau principal. Chunk 64x64 est généreux pour 180x105 mais cohérent
    //    avec ce que produit le worker.
    let chunk_y = (ny as u64).min(64);
    let chunk_x = (nx as u64).min(64);
    let finalized = {
        let mut arr_writer = writer
            .prepare_array::<f32>(
                vec![ny as u64, nx as u64],
                vec![chunk_y, chunk_x],
                OmCompressionType::PforDelta2dInt16,
                100.0, // scale_factor : 0.01 K de résolution
                0.0,
            )
            .map_err(|e| anyhow::anyhow!("prepare_array: {e}"))?;
        arr_writer
            .write_data(data.view().into_dyn(), None, None)
            .map_err(|e| anyhow::anyhow!("write_data: {e}"))?;
        arr_writer.finalize()
    };

    let arr_offset = writer
        .write_array(finalized, variable_name, &[meta_offset])
        .map_err(|e| anyhow::anyhow!("write_array: {e}"))?;
    let root_offset = writer
        .write_none("", &[arr_offset])
        .map_err(|e| anyhow::anyhow!("write_none root: {e}"))?;

    writer
        .write_trailer(root_offset)
        .map_err(|e| anyhow::anyhow!("write_trailer: {e}"))?;
    Ok(())
}

/// Écrit un OMfile spatial contenant plusieurs variables comme enfants du root,
/// chacune un tableau 2D `[ny, nx]` sur la même grille. Correspond au layout
/// `data_spatial` natif Open-Meteo : le client fait `reader.getChildByName(var)`
/// pour récupérer le tableau de la variable sélectionnée.
///
/// Toutes les variables partagent la même métadonnée JSON (un seul scalar
/// `metadata` attaché comme enfant de la première variable).
///
/// Chaque variable porte son propre `scale_factor` (3e élément du tuple) : le
/// codec `PforDelta2dInt16` stocke `round(valeur × scale_factor)` en `i16`, donc
/// la valeur physique max représentable est `32767 / scale_factor`. Un facteur
/// par variable évite l'overflow des grandeurs élevées (pression ~1013 hPa,
/// cumuls de précip > 327 mm) tout en gardant une bonne résolution sur les
/// petites (température, etc.). Le facteur est écrit dans le fichier → le lecteur
/// décode automatiquement.
pub fn write_multi_variable_omfile<G: Grid>(
    path: &Path,
    variables: &[(&str, &Array2<f32>, f32)],
    grid: &G,
    meta: &OmfileMetadata,
) -> Result<()> {
    anyhow::ensure!(!variables.is_empty(), "no variables to write");
    let (ny, nx) = (grid.ny(), grid.nx());
    for (name, arr, _) in variables {
        let (any, anx) = arr.dim();
        anyhow::ensure!(
            any == ny && anx == nx,
            "variable {name:?} shape ({any}, {anx}) ne correspond pas à la grille ({ny}, {nx})",
        );
    }

    let file = std::fs::File::create(path).with_context(|| format!("creating {path:?}"))?;
    let mut writer = OmFileWriter::new(file, 1 << 20);

    // 1) Métadonnée d'abord (sera référencée comme child du 1er tableau).
    let meta_json = serde_json::to_string(meta)?;
    let meta_offset = writer
        .write_scalar(meta_json, METADATA_CHILD, &[])
        .map_err(|e| anyhow::anyhow!("write_scalar metadata: {e}"))?;

    // 2) Chaque variable comme tableau ; la première porte le scalar metadata.
    //    `meta_offset` est consommé (non-Copy) lors du premier appel write_array,
    //    donc on l'enveloppe dans une Option et on le prend exactement une fois.
    let chunk_y = (ny as u64).min(64);
    let chunk_x = (nx as u64).min(64);
    let mut array_offsets = Vec::with_capacity(variables.len());
    let mut pending_meta = Some(meta_offset);
    for (name, arr, scale_factor) in variables {
        let children: Vec<_> = pending_meta.take().into_iter().collect();
        let finalized = {
            let mut aw = writer
                .prepare_array::<f32>(
                    vec![ny as u64, nx as u64],
                    vec![chunk_y, chunk_x],
                    OmCompressionType::PforDelta2dInt16,
                    *scale_factor,
                    0.0,
                )
                .map_err(|e| anyhow::anyhow!("prepare_array {name}: {e}"))?;
            aw.write_data(arr.view().into_dyn(), None, None)
                .map_err(|e| anyhow::anyhow!("write_data {name}: {e}"))?;
            aw.finalize()
        };
        let offset = writer
            .write_array(finalized, name, &children)
            .map_err(|e| anyhow::anyhow!("write_array {name}: {e}"))?;
        array_offsets.push(offset);
    }

    // 3) Root : conteneur vide pointant sur toutes les variables.
    let root_offset = writer
        .write_none("", &array_offsets)
        .map_err(|e| anyhow::anyhow!("write_none root: {e}"))?;
    writer
        .write_trailer(root_offset)
        .map_err(|e| anyhow::anyhow!("write_trailer: {e}"))?;
    Ok(())
}

/// Lit un OMfile spatial 2D écrit par [`write_spatial_omfile`] et retourne
/// le tableau `[ny, nx]` accompagné de sa métadonnée.
///
/// `variable_name` doit correspondre au nom passé lors de l'écriture.
pub fn read_spatial_omfile(path: &Path, variable_name: &str) -> Result<(Array2<f32>, OmfileMetadata)> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {path:?}"))?;
    let root = OmFileReader::from_file(path_str)
        .map_err(|e| anyhow::anyhow!("open {path:?}: {e}"))?;

    let var = root
        .get_child_by_name(variable_name)
        .with_context(|| format!("variable {variable_name:?} absente dans {path:?}"))?;
    let arr = var
        .expect_array()
        .map_err(|e| anyhow::anyhow!("variable {variable_name:?} n'est pas un array: {e}"))?;
    let dims: Vec<u64> = arr.get_dimensions().to_vec();
    anyhow::ensure!(dims.len() == 2, "expected 2D variable, got {}D", dims.len());
    let dynd = arr
        .read::<f32>(&[0..dims[0], 0..dims[1]])
        .map_err(|e| anyhow::anyhow!("read array: {e}"))?;
    let arr_data: Array2<f32> = dynd
        .into_dimensionality::<ndarray::Ix2>()
        .map_err(|e| anyhow::anyhow!("dim cast to 2D: {e}"))?;

    let meta_var = var
        .get_child_by_name(METADATA_CHILD)
        .with_context(|| format!("scalar {METADATA_CHILD} absent"))?;
    let meta_scalar = meta_var
        .expect_scalar()
        .map_err(|e| anyhow::anyhow!("metadata is not a scalar: {e}"))?;
    let meta_json: String = meta_scalar
        .read_scalar::<String>()
        .ok_or_else(|| anyhow::anyhow!("metadata scalar n'est pas une String"))?;
    let meta: OmfileMetadata = serde_json::from_str(&meta_json)
        .with_context(|| "deserializing OmfileMetadata JSON")?;

    Ok((arr_data, meta))
}

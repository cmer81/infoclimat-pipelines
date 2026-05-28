use ndarray::Array2;
use omfiles::{reader::OmFileReader, traits::{OmArrayVariable, OmFileReadable}};
use pipeline_core::grid::{ArpegeEuropeGrid, Grid};
use pipeline_core::omfile_io::{
    OmfileMetadata, read_spatial_omfile, write_multi_variable_omfile, write_spatial_omfile,
};
use tempfile::NamedTempFile;

#[test]
fn omfile_roundtrip_preserves_data() {
    let dst = ArpegeEuropeGrid;
    let arr = Array2::<f32>::from_shape_fn((dst.ny(), dst.nx()), |(j, i)| {
        (j as f32) * 0.01 + (i as f32) * 0.001
    });

    let tmp = NamedTempFile::new().unwrap();
    let meta = OmfileMetadata {
        source: "test".to_string(),
        generated_at: chrono::Utc::now(),
        extra: serde_json::json!({"key": "value"}),
    };

    write_spatial_omfile(tmp.path(), "temperature_2m_anomaly", &arr, &dst, &meta).unwrap();
    let (read_arr, read_meta) = read_spatial_omfile(tmp.path(), "temperature_2m_anomaly").unwrap();

    assert_eq!(read_arr.shape(), arr.shape());
    for ((j, i), &v) in arr.indexed_iter() {
        assert!(
            (read_arr[[j, i]] - v).abs() < 1e-2,
            "mismatch at ({j},{i}): expected {v}, got {}",
            read_arr[[j, i]]
        );
    }
    assert_eq!(read_meta.source, "test");
    assert_eq!(read_meta.extra["key"], "value");
}

/// Vérifie que [`write_multi_variable_omfile`] écrit correctement plusieurs
/// variables et que chacune est relisible via `getChildByName`.
#[test]
fn multi_variable_omfile_roundtrip_preserves_all_variables() {
    // Utilise une petite grille synthétique pour garder le test rapide
    // (ReunionGrid = 1395×899 serait trop lourd pour un test unitaire).
    // On passe une grille fictive via ArpegeEuropeGrid qui est déjà petite
    // mais suffisante pour valider le layout.
    let grid = ArpegeEuropeGrid;
    let ny = grid.ny();
    let nx = grid.nx();

    // Valeurs petites pour rester dans la plage i16 avec scale_factor=100
    // (i16::MAX ≈ 327 en unité physique). On utilise des gradients modestes.
    let arr_a = Array2::<f32>::from_shape_fn((ny, nx), |(j, i)| j as f32 * 0.01 + i as f32 * 0.001);
    let arr_b = Array2::<f32>::from_shape_fn((ny, nx), |(j, i)| -(j as f32 * 0.01 + i as f32 * 0.001));
    let arr_c = Array2::<f32>::from_shape_fn((ny, nx), |(j, i)| (j + i) as f32 * 0.005);

    let meta = OmfileMetadata {
        source: "multi_test".to_string(),
        generated_at: chrono::Utc::now(),
        extra: serde_json::json!({"run": "2026-05-28T06Z"}),
    };

    let tmp = NamedTempFile::new().unwrap();
    write_multi_variable_omfile(
        tmp.path(),
        &[
            ("temperature_2m", &arr_a),
            ("relative_humidity_2m", &arr_b),
            ("precipitation", &arr_c),
        ],
        &grid,
        &meta,
    )
    .unwrap();

    // Lit chaque variable via getChildByName sur le root.
    let path_str = tmp.path().to_str().unwrap();
    let root = OmFileReader::from_file(path_str).unwrap();

    for (name, expected) in [
        ("temperature_2m", &arr_a),
        ("relative_humidity_2m", &arr_b),
        ("precipitation", &arr_c),
    ] {
        let var = root.get_child_by_name(name).unwrap_or_else(|| {
            panic!("variable {name:?} not found in multi-variable OMfile")
        });
        let arr_node = var.expect_array().unwrap();
        let dims: Vec<u64> = arr_node.get_dimensions().to_vec();
        assert_eq!(dims.len(), 2, "{name}: expected 2D");
        assert_eq!(dims[0] as usize, ny, "{name}: ny mismatch");
        assert_eq!(dims[1] as usize, nx, "{name}: nx mismatch");

        let read_dyn = arr_node
            .read::<f32>(&[0..dims[0], 0..dims[1]])
            .unwrap();
        let read_arr: Array2<f32> = read_dyn
            .into_dimensionality::<ndarray::Ix2>()
            .unwrap();

        for ((j, i), &v) in expected.indexed_iter() {
            assert!(
                (read_arr[[j, i]] - v).abs() < 1e-2,
                "{name}: mismatch at ({j},{i}): expected {v}, got {}",
                read_arr[[j, i]]
            );
        }
    }

    // La métadonnée est lisible depuis la première variable.
    let (_, read_meta) = read_spatial_omfile(tmp.path(), "temperature_2m").unwrap();
    assert_eq!(read_meta.source, "multi_test");
    assert_eq!(read_meta.extra["run"], "2026-05-28T06Z");
}

/// Vérifie que [`write_multi_variable_omfile`] rejette une liste vide.
#[test]
fn multi_variable_omfile_rejects_empty_variable_list() {
    let grid = ArpegeEuropeGrid;
    let meta = OmfileMetadata {
        source: "x".to_string(),
        generated_at: chrono::Utc::now(),
        extra: serde_json::Value::Null,
    };
    let tmp = NamedTempFile::new().unwrap();
    let result = write_multi_variable_omfile(tmp.path(), &[], &grid, &meta);
    assert!(result.is_err(), "should reject empty variable list");
}

/// Vérifie que [`write_multi_variable_omfile`] rejette un tableau dont la
/// forme ne correspond pas à la grille.
#[test]
fn multi_variable_omfile_rejects_mismatched_shape() {
    let grid = ArpegeEuropeGrid;
    let wrong_shape = Array2::<f32>::zeros((3, 3)); // ne correspond pas à la grille
    let meta = OmfileMetadata {
        source: "x".to_string(),
        generated_at: chrono::Utc::now(),
        extra: serde_json::Value::Null,
    };
    let tmp = NamedTempFile::new().unwrap();
    let result = write_multi_variable_omfile(
        tmp.path(),
        &[("bad_var", &wrong_shape)],
        &grid,
        &meta,
    );
    assert!(result.is_err(), "should reject wrong shape");
}

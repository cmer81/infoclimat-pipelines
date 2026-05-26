use ndarray::Array2;
use pipeline_core::grid::{ArpegeFranceGrid, Grid};
use pipeline_core::omfile_io::{OmfileMetadata, read_spatial_omfile, write_spatial_omfile};
use tempfile::NamedTempFile;

#[test]
fn omfile_roundtrip_preserves_data() {
    let dst = ArpegeFranceGrid::default();
    let arr = Array2::<f32>::from_shape_fn((dst.ny(), dst.nx()), |(j, i)| {
        (j as f32) * 0.01 + (i as f32) * 0.001
    });

    let tmp = NamedTempFile::new().unwrap();
    let meta = OmfileMetadata {
        source: "test".to_string(),
        generated_at: chrono::Utc::now(),
        extra: serde_json::json!({"key": "value"}),
    };

    write_spatial_omfile(tmp.path(), &arr, &dst, &meta).unwrap();
    let (read_arr, read_meta) = read_spatial_omfile(tmp.path()).unwrap();

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

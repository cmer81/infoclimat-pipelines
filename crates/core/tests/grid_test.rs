use pipeline_core::grid::{ArpegeFranceGrid, Grid};

#[test]
fn arpege_france_grid_dimensions() {
    let g = ArpegeFranceGrid::default();
    assert_eq!(g.nx(), 180);
    assert_eq!(g.ny(), 105);
}

#[test]
fn arpege_france_grid_bounds() {
    let g = ArpegeFranceGrid::default();
    assert!((g.lon_min() - (-5.95)).abs() < 1e-6);
    assert!((g.lon_max() - 11.95).abs() < 1e-6);
    assert!((g.lat_min() - 41.0).abs() < 1e-6);
    assert!((g.lat_max() - 51.5).abs() < 1e-6);
}

#[test]
fn arpege_france_grid_indexing() {
    let g = ArpegeFranceGrid::default();
    let (i, j) = g.lonlat_to_indices(2.35, 48.85).unwrap(); // Paris
    let (lon, lat) = g.indices_to_lonlat(i, j);
    assert!((lon - 2.35).abs() < 0.1);
    assert!((lat - 48.85).abs() < 0.1);
}

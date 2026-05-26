use pipeline_core::grid::{ArpegeEuropeGrid, Grid};

#[test]
fn arpege_europe_grid_dimensions() {
    let g = ArpegeEuropeGrid::default();
    assert_eq!(g.nx(), 741);
    assert_eq!(g.ny(), 521);
}

#[test]
fn arpege_europe_grid_bounds() {
    let g = ArpegeEuropeGrid::default();
    assert!((g.lon_min() - (-32.0)).abs() < 1e-6);
    assert!((g.lon_max() - 42.0).abs() < 1e-6);
    assert!((g.lat_min() - 20.0).abs() < 1e-6);
    assert!((g.lat_max() - 72.0).abs() < 1e-6);
}

#[test]
fn arpege_europe_grid_indexing_paris() {
    let g = ArpegeEuropeGrid::default();
    let (i, j) = g.lonlat_to_indices(2.35, 48.85).unwrap();
    let (lon, lat) = g.indices_to_lonlat(i, j);
    assert!((lon - 2.4).abs() < 0.1);
    assert!((lat - 48.9).abs() < 0.1);
}

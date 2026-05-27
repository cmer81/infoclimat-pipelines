use ndarray::Array2;
use pipeline_core::grid::{ArpegeEuropeGrid, Bbox, Grid};
use pipeline_core::regrid::bilinear_regrid;

fn era5_source() -> (Array2<f32>, Bbox, f64) {
    // grille 0.25°, bbox -7°E à 13°E, 40°N à 53°N → 81 x 53 pixels
    let bbox = Bbox {
        lon_min: -7.0,
        lon_max: 13.0,
        lat_min: 40.0,
        lat_max: 53.0,
    };
    let dx = 0.25;
    let nx = ((bbox.lon_max - bbox.lon_min) / dx) as usize + 1;
    let ny = ((bbox.lat_max - bbox.lat_min) / dx) as usize + 1;
    // T = lat * 0.5 + lon (champ linéaire → l'interpolation bilinéaire est exacte)
    let arr = Array2::from_shape_fn((ny, nx), |(j, i)| {
        let lat = bbox.lat_min + (j as f64) * dx;
        let lon = bbox.lon_min + (i as f64) * dx;
        (lat * 0.5 + lon) as f32
    });
    (arr, bbox, dx)
}

#[test]
fn bilinear_is_exact_for_linear_field() {
    let (src, src_bbox, src_dx) = era5_source();
    let dst_grid = ArpegeEuropeGrid;
    let out = bilinear_regrid(&src, src_bbox, src_dx, src_dx, &dst_grid).unwrap();

    // Spot check : Paris. lonlat_to_indices rounds to the nearest grid node,
    // so the value at out[[j,i]] is the bilinear interpolation at the *grid
    // node* — not at the query (lon, lat). Recompute the expectation from
    // the actual node coordinates returned by indices_to_lonlat.
    let (i, j) = dst_grid.lonlat_to_indices(2.35, 48.85).unwrap();
    let (lon_node, lat_node) = dst_grid.indices_to_lonlat(i, j);
    let expected = lat_node * 0.5 + lon_node;
    let got = out[[j, i]] as f64;
    assert!((got - expected).abs() < 1e-3, "got {got} expected {expected}");
}

#[test]
fn bilinear_propagates_nan() {
    let (mut src, src_bbox, src_dx) = era5_source();
    src[[10, 10]] = f32::NAN; // Met un NaN quelque part dans la source
    let dst_grid = ArpegeEuropeGrid;
    let out = bilinear_regrid(&src, src_bbox, src_dx, src_dx, &dst_grid).unwrap();

    // Au moins un pixel d'output doit être NaN (ceux qui interpolent depuis le NaN)
    let nan_count = out.iter().filter(|v| v.is_nan()).count();
    assert!(nan_count >= 1, "expected at least 1 NaN, got {nan_count}");
}

#[test]
fn bilinear_output_dimensions_match_target_grid() {
    let (src, src_bbox, src_dx) = era5_source();
    let dst_grid = ArpegeEuropeGrid;
    let out = bilinear_regrid(&src, src_bbox, src_dx, src_dx, &dst_grid).unwrap();
    assert_eq!(out.shape(), &[dst_grid.ny(), dst_grid.nx()]);
}

//! Définition des grilles de prévision utilisées dans les pipelines.
//!
//! Grilles disponibles :
//! - [`ArpegeEuropeGrid`] — ARPEGE Europe (Open-Meteo, 521×741, 0.1°), origine
//!   SW (lat 20°, lon -32°), coin NE (lat 72°, lon 42°).
//! - [`ReunionGrid`] — AROME-OM Océan Indien (Météo-France, 1395×899, 0.025°),
//!   couvre Réunion, Mayotte, Madagascar et l'océan Indien environnant (côte
//!   est-africaine, sud de l'Inde). Nom conservé pour la continuité côté client.

pub trait Grid {
    fn nx(&self) -> usize;
    fn ny(&self) -> usize;
    fn lon_min(&self) -> f64;
    fn lon_max(&self) -> f64;
    fn lat_min(&self) -> f64;
    fn lat_max(&self) -> f64;
    fn dx(&self) -> f64;
    fn dy(&self) -> f64;

    fn lonlat_to_indices(&self, lon: f64, lat: f64) -> Option<(usize, usize)> {
        if lon < self.lon_min() || lon > self.lon_max() {
            return None;
        }
        if lat < self.lat_min() || lat > self.lat_max() {
            return None;
        }
        let i = ((lon - self.lon_min()) / self.dx()).round() as usize;
        let j = ((lat - self.lat_min()) / self.dy()).round() as usize;
        Some((i.min(self.nx() - 1), j.min(self.ny() - 1)))
    }

    fn indices_to_lonlat(&self, i: usize, j: usize) -> (f64, f64) {
        let lon = self.lon_min() + (i as f64) * self.dx();
        let lat = self.lat_min() + (j as f64) * self.dy();
        (lon, lat)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ArpegeEuropeGrid;

impl Default for ArpegeEuropeGrid {
    fn default() -> Self {
        Self
    }
}

impl Grid for ArpegeEuropeGrid {
    fn nx(&self) -> usize {
        741
    }
    fn ny(&self) -> usize {
        521
    }
    fn lon_min(&self) -> f64 {
        -32.0
    }
    fn lon_max(&self) -> f64 {
        42.0
    }
    fn lat_min(&self) -> f64 {
        20.0
    }
    fn lat_max(&self) -> f64 {
        72.0
    }
    fn dx(&self) -> f64 {
        0.1
    }
    fn dy(&self) -> f64 {
        0.1
    }
}

/// Grille AROME-OM Réunion (modèle Météo-France, Outre-Mer Océan Indien).
///
/// Lat/lon régulière à 0.025°. Le domaine couvre l'ensemble de l'océan Indien
/// occidental : Réunion, Mayotte, Madagascar, côte est-africaine et sud de
/// l'Inde. Dimensions et extent lus sur le header GRIB2 d'un fichier réel
/// (Task 0 — probe API réalisée le 2026-05-28). Le nom `ReunionGrid` est
/// conservé pour la continuité côté client `maps/`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReunionGrid;

impl Grid for ReunionGrid {
    fn nx(&self) -> usize {
        1395
    }
    fn ny(&self) -> usize {
        899
    }
    fn lon_min(&self) -> f64 {
        32.75
    }
    fn lon_max(&self) -> f64 {
        67.6
    }
    fn lat_min(&self) -> f64 {
        -25.9
    }
    fn lat_max(&self) -> f64 {
        -3.45
    }
    fn dx(&self) -> f64 {
        0.025
    }
    fn dy(&self) -> f64 {
        0.025
    }
}

/// Bbox utilisée pour télécharger ERA5. Couvre la grille ARPEGE Europe
/// avec ~1° de marge pour permettre une interpolation bilinéaire correcte
/// jusqu'aux bords.
#[derive(Debug, Clone, Copy)]
pub struct Bbox {
    pub lon_min: f64,
    pub lon_max: f64,
    pub lat_min: f64,
    pub lat_max: f64,
}

pub const EUROPE_DOWNLOAD_BBOX: Bbox = Bbox {
    lon_min: -33.0,
    lon_max: 43.0,
    lat_min: 19.0,
    lat_max: 73.0,
};

#[cfg(test)]
mod reunion_tests {
    use super::*;

    #[test]
    fn reunion_grid_has_expected_dimensions() {
        let g = ReunionGrid;
        // Valeurs lues sur le header GRIB2 réel (Task 0, 2026-05-28).
        // Tester chaque constante explicitement : un search-replace partiel
        // doit échouer ici, pas silencieusement.
        assert_eq!(g.nx(), 1395);
        assert_eq!(g.ny(), 899);
        assert!((g.dx() - 0.025).abs() < 1e-9);
        assert!((g.dy() - 0.025).abs() < 1e-9);
        assert!((g.lon_min() - 32.75).abs() < 1e-9);
        assert!((g.lon_max() - 67.6).abs() < 1e-9);
        assert!((g.lat_min() - (-25.9)).abs() < 1e-9);
        assert!((g.lat_max() - (-3.45)).abs() < 1e-9);
    }

    #[test]
    fn reunion_grid_corner_roundtrip() {
        let g = ReunionGrid;
        let (i, j) = g.lonlat_to_indices(g.lon_min(), g.lat_min()).unwrap();
        assert_eq!((i, j), (0, 0));
        let (lon, lat) = g.indices_to_lonlat(0, 0);
        assert!((lon - g.lon_min()).abs() < 1e-9);
        assert!((lat - g.lat_min()).abs() < 1e-9);
    }

    #[test]
    fn reunion_grid_rejects_outside_bbox() {
        let g = ReunionGrid;
        // Greenwich (lon 0°) — way west of the bbox, outside.
        assert!(g.lonlat_to_indices(0.0, 0.0).is_none());
        // Saint-Denis, Réunion (~55.5°E, -21.1°N) — inside the domain.
        assert!(g.lonlat_to_indices(55.5, -21.1).is_some());
        // Mayotte (~45.2°E, -12.8°N) — also covered by the wider Indian Ocean domain.
        assert!(g.lonlat_to_indices(45.2, -12.8).is_some());
    }
}

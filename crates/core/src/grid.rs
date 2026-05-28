//! Définition de la grille ARPEGE Europe (la grille native publiée par
//! Open-Meteo pour le domaine `meteofrance_arpege_europe`).
//!
//! Dimensions : 521 lignes (latitude) × 741 colonnes (longitude) à 0.1°.
//! Origine au coin sud-ouest : (lat 20.0°, lon -32.0°). Coin nord-est :
//! (lat 72.0°, lon 42.0°).

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
/// Lat/lon régulière. Les valeurs définitives proviennent du header GRIB2 d'un
/// fichier réel (cf. `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`,
/// section "Inconnues à lever dès la première PR"). Pour l'instant ce sont des
/// PLACEHOLDERS estimés — à remplacer dès que Task 0 (probe API) sera exécutée.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReunionGrid;

impl Grid for ReunionGrid {
    // TODO(task-0): remplacer ces 8 valeurs par celles lues sur grib_dump.
    fn nx(&self) -> usize {
        201
    }
    fn ny(&self) -> usize {
        161
    }
    fn lon_min(&self) -> f64 {
        53.0
    }
    fn lon_max(&self) -> f64 {
        58.0
    }
    fn lat_min(&self) -> f64 {
        -23.0
    }
    fn lat_max(&self) -> f64 {
        -19.0
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
        // Placeholder values — TODO(task-0): replace after probing the real GRIB header.
        assert_eq!(g.nx(), 201);
        assert_eq!(g.ny(), 161);
        assert!((g.dx() - 0.025).abs() < 1e-9);
        assert!((g.dy() - 0.025).abs() < 1e-9);
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
        // Méditerranée — way outside.
        assert!(g.lonlat_to_indices(0.0, 0.0).is_none());
        // Around Saint-Denis (Réunion). Should be inside.
        assert!(g.lonlat_to_indices(55.5, -21.1).is_some());
    }
}

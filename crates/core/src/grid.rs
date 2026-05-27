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

#[derive(Debug, Clone, Copy, Default)]
pub struct ArpegeEuropeGrid;

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

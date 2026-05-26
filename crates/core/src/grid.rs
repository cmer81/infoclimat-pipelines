//! Définition de la grille ARPEGE France.

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
pub struct ArpegeFranceGrid;

impl Default for ArpegeFranceGrid {
    fn default() -> Self {
        Self
    }
}

impl Grid for ArpegeFranceGrid {
    fn nx(&self) -> usize {
        180
    }
    fn ny(&self) -> usize {
        105
    }
    fn lon_min(&self) -> f64 {
        -5.95
    }
    fn lon_max(&self) -> f64 {
        11.95
    }
    fn lat_min(&self) -> f64 {
        41.0
    }
    fn lat_max(&self) -> f64 {
        51.5
    }
    fn dx(&self) -> f64 {
        0.1
    }
    fn dy(&self) -> f64 {
        0.1
    }
}

/// Bbox élargie pour télécharger ERA5 (couvre la grille ARPEGE France
/// avec ~0.5° de marge pour le regridding bilinéaire).
#[derive(Debug, Clone, Copy)]
pub struct Bbox {
    pub lon_min: f64,
    pub lon_max: f64,
    pub lat_min: f64,
    pub lat_max: f64,
}

pub const FRANCE_DOWNLOAD_BBOX: Bbox = Bbox {
    lon_min: -6.5,
    lon_max: 12.5,
    lat_min: 40.5,
    lat_max: 52.0,
};

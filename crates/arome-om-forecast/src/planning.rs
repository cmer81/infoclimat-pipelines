//! Build le plan de download (cartésien Package × TimeWindow) pour un run AROME-OM.

use std::fmt;

/// Packages AROME-OM supportés pour le MVP (toutes variables surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Package { Sp1, Sp2, Sp3 }

impl Package {
    /// Identifiant API tel qu'attendu dans le path URL (`packages/<id>`).
    pub fn as_api_id(&self) -> &'static str {
        match self {
            Package::Sp1 => "SP1",
            Package::Sp2 => "SP2",
            Package::Sp3 => "SP3",
        }
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_api_id())
    }
}

/// Plage temporelle de 6 heures côté API Météo-France (ex. `00H06H`, `07H12H`).
///
/// Invariants : `end_h > start_h`, `end_h - start_h == 6`, `start_h % 6 == 0`
/// (sauf la 1re qui commence à 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindow {
    pub start_h: u32,
    pub end_h: u32,
}

impl TimeWindow {
    pub fn new(start_h: u32, end_h: u32) -> Self {
        assert!(end_h > start_h, "TimeWindow end_h must be > start_h");
        Self { start_h, end_h }
    }
    /// Format API : `00H06H`, `07H12H`, `13H18H`, etc.
    pub fn as_api_param(&self) -> String {
        format!("{:02}H{:02}H", self.start_h, self.end_h)
    }
}

impl fmt::Display for TimeWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_api_param())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_api_ids_match_meteofrance_spec() {
        assert_eq!(Package::Sp1.as_api_id(), "SP1");
        assert_eq!(Package::Sp2.as_api_id(), "SP2");
        assert_eq!(Package::Sp3.as_api_id(), "SP3");
    }

    #[test]
    fn time_window_api_param_zero_padded() {
        assert_eq!(TimeWindow::new(0, 6).as_api_param(), "00H06H");
        assert_eq!(TimeWindow::new(7, 12).as_api_param(), "07H12H");
        assert_eq!(TimeWindow::new(37, 42).as_api_param(), "37H42H");
    }

    #[test]
    #[should_panic(expected = "end_h must be > start_h")]
    fn time_window_rejects_inverted_bounds() {
        TimeWindow::new(6, 6);
    }
}

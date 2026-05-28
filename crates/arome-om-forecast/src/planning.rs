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

/// Construit le plan de download pour un run donné : produit cartésien
/// `packages × windows`, où les windows couvrent `[0, horizon_h)` par
/// tranches de 6 h pleines.
///
/// Si `horizon_h` n'est pas multiple de 6, la fraction restante est ignorée :
/// l'API attend des fenêtres de 6h exactes (ex. `00H06H`, `07H12H`, ...). Le
/// caller peut bumper l'horizon pour obtenir une couverture plus large.
pub fn build_plan(horizon_h: u32, packages: &[Package]) -> Vec<(Package, TimeWindow)> {
    let mut windows = Vec::new();
    let mut start = 0u32;
    while start + 6 <= horizon_h {
        windows.push(TimeWindow::new(start, start + 6));
        start += 6;
    }
    let mut plan = Vec::with_capacity(windows.len() * packages.len());
    for pkg in packages {
        for w in &windows {
            plan.push((*pkg, *w));
        }
    }
    plan
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

    #[test]
    fn build_plan_42h_three_packages_yields_21_items() {
        let plan = build_plan(42, &[Package::Sp1, Package::Sp2, Package::Sp3]);
        assert_eq!(plan.len(), 7 * 3);
    }

    #[test]
    fn build_plan_windows_are_contiguous_and_six_hours() {
        let plan = build_plan(42, &[Package::Sp1]);
        let windows: Vec<TimeWindow> = plan.iter().map(|(_, w)| *w).collect();
        assert_eq!(windows[0], TimeWindow::new(0, 6));
        assert_eq!(windows[1], TimeWindow::new(6, 12));
        assert_eq!(windows.last().copied(), Some(TimeWindow::new(36, 42)));
        for pair in windows.windows(2) {
            assert_eq!(pair[0].end_h, pair[1].start_h);
        }
    }

    #[test]
    fn build_plan_zero_packages_returns_empty() {
        assert!(build_plan(42, &[]).is_empty());
    }

    #[test]
    fn build_plan_partial_horizon_rounds_to_full_six_h() {
        // horizon=10 => on émet une seule window 0-6 (pas de window 6-12 partielle).
        let plan = build_plan(10, &[Package::Sp1]);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].1, TimeWindow::new(0, 6));
    }
}

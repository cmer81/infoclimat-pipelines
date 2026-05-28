//! Build le plan de download (cartésien Package × leadtime) pour un run AROME-OM.

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

/// Construit le plan de download pour un run donné : produit cartésien
/// `packages × leadtimes`, où les leadtimes sont les heures `0..=horizon_h`
/// (incluses des deux côtés). L'API AROME-OM expose un fichier par leadtime
/// horaire (`time=000H`, `001H`, …), pas des fenêtres groupées.
pub fn build_plan(horizon_h: u32, packages: &[Package]) -> Vec<(Package, u32)> {
    let mut plan = Vec::with_capacity(packages.len() * (horizon_h as usize + 1));
    for pkg in packages {
        for lead in 0..=horizon_h {
            plan.push((*pkg, lead));
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
    fn build_plan_48h_three_packages_yields_147_items() {
        let plan = build_plan(48, &[Package::Sp1, Package::Sp2, Package::Sp3]);
        // 49 leadtimes (0..=48) × 3 packages
        assert_eq!(plan.len(), 49 * 3);
    }

    #[test]
    fn build_plan_leadtimes_are_zero_to_horizon_inclusive() {
        let plan = build_plan(48, &[Package::Sp1]);
        let leads: Vec<u32> = plan.iter().map(|(_, l)| *l).collect();
        assert_eq!(leads.first().copied(), Some(0));
        assert_eq!(leads.last().copied(), Some(48));
        assert_eq!(leads.len(), 49);
        for win in leads.windows(2) {
            assert_eq!(win[1], win[0] + 1);
        }
    }

    #[test]
    fn build_plan_zero_packages_returns_empty() {
        assert!(build_plan(48, &[]).is_empty());
    }

    #[test]
    fn build_plan_zero_horizon_yields_only_lead_0() {
        let plan = build_plan(0, &[Package::Sp1]);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].1, 0);
    }
}

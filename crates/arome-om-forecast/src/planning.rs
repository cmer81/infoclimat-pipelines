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

/// Nombre maximum d'échéances **de queue** manquantes toléré avant de
/// considérer le run comme un échec dur.
///
/// Météo-France publie les échéances d'AROME-OM progressivement — les plus
/// lointaines en dernier, avec un délai de mise à disposition supérieur à
/// `PUBLICATION_DELAY_H` et variable d'un run à l'autre. Quelques échéances de
/// queue encore absentes au moment du fetch sont donc normales et ne doivent
/// pas faire échouer le pipeline tant que le reste de l'horizon est livré.
/// Au-delà de ce seuil, le run est vraisemblablement cassé/incomplet (ex. la
/// publication n'a quasiment pas commencé) → on échoue pour alerter.
pub const MAX_TAIL_FAILURES_TOLERATED: u32 = 6;

/// Décide si un run partiellement échoué doit être traité comme un succès.
///
/// `max_written_leadtime` = plus grand leadtime écrit avec succès (`None` si
/// aucun fichier n'a été produit). `failed_leadtimes` = leadtimes ayant échoué
/// (fetch/decode/write). `max_tail` = nombre d'échecs de queue toléré
/// (typiquement [`MAX_TAIL_FAILURES_TOLERATED`]).
///
/// Règle : on tolère uniquement des échecs **de queue contiguë** — chaque
/// leadtime échoué est strictement supérieur au plus grand leadtime écrit — et
/// en nombre ≤ `max_tail`. Un trou « intérieur » (échéance manquante avant la
/// dernière écrite) ou un nombre d'échecs trop élevé reste un échec dur.
pub fn tail_failures_are_tolerable(
    max_written_leadtime: Option<u32>,
    failed_leadtimes: &[u32],
    max_tail: u32,
) -> bool {
    let Some(max_ok) = max_written_leadtime else {
        return false; // rien écrit → échec dur
    };
    if failed_leadtimes.is_empty() {
        return true;
    }
    if failed_leadtimes.len() as u32 > max_tail {
        return false;
    }
    failed_leadtimes.iter().all(|&lead| lead > max_ok)
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

    #[test]
    fn tail_tolerable_when_no_failures() {
        assert!(tail_failures_are_tolerable(Some(48), &[], 6));
    }

    #[test]
    fn tail_tolerable_when_only_last_leadtime_missing() {
        // Écrit 0..=47, échéance 48 absente (cas réel : publication MF en retard).
        assert!(tail_failures_are_tolerable(Some(47), &[48], 6));
    }

    #[test]
    fn tail_tolerable_for_a_few_contiguous_tail_leadtimes() {
        // Écrit 0..=45, échéances 46/47/48 absentes.
        assert!(tail_failures_are_tolerable(Some(45), &[46, 47, 48], 6));
    }

    #[test]
    fn tail_not_tolerable_for_interior_hole() {
        // Écrit jusqu'à 48 mais 47 manque → trou intérieur, pas une queue.
        assert!(!tail_failures_are_tolerable(Some(48), &[47], 6));
    }

    #[test]
    fn tail_not_tolerable_when_too_many_missing() {
        // Run quasi vide (seul H0 écrit, 1..=48 absents) → dépasse le seuil.
        let failed: Vec<u32> = (1..=48).collect();
        assert!(!tail_failures_are_tolerable(Some(0), &failed, 6));
    }

    #[test]
    fn tail_not_tolerable_when_nothing_written() {
        assert!(!tail_failures_are_tolerable(None, &[1, 2, 3], 6));
    }
}

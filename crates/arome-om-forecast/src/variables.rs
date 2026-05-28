//! Registry statique des variables AROME-OM exposées dans les OMfiles produits.
//!
//! Mapping `grib_short_name` ↔ `om_name` ↔ unit conversion. Les noms `om_name`
//! suivent la convention Open-Meteo (`temperature_2m`, `precipitation`, etc.)
//! pour être consommés tels quels par le client `maps/`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitConversion {
    None,
    KelvinToCelsius,
    /// `kg/m²` → `mm` (pluies) : facteur 1.0 (densité eau ≈ 1) ; on garde l'enum
    /// pour rendre explicite l'unité de sortie.
    KgPerM2ToMm,
    PascalToHectopascal,
}

#[derive(Debug, Clone, Copy)]
pub struct VariableEntry {
    /// Nom du `shortName` GRIB tel qu'exposé par eccodes / cfgrib.
    pub grib_short_name: &'static str,
    /// Nom de sortie utilisé dans l'OMfile et dans le path R2.
    pub om_name: &'static str,
    pub unit_conversion: UnitConversion,
    /// Package AROME-OM dans lequel la variable se trouve.
    pub package: &'static str,
}

/// Inventaire MVP : SP1 (8 vars) + SP2 (4 vars). ShortNames et répartition par
/// package issus de l'inventaire réel de l'API AROME-OM (Task 0, 2026-05-28).
/// SP3 exclu du MVP. Total 11 vars (SP1: 7, SP2: 4) — `wind_speed_10m`
/// retiré parce que le client `maps/` le dérive depuis u/v.
pub const VARIABLES: &[VariableEntry] = &[
    // SP1 (8 vars) — paramètres courants surface
    VariableEntry { grib_short_name: "2t",        om_name: "temperature_2m",       unit_conversion: UnitConversion::KelvinToCelsius,     package: "SP1" },
    VariableEntry { grib_short_name: "2r",        om_name: "relative_humidity_2m", unit_conversion: UnitConversion::None,                package: "SP1" },
    // Naming aligné sur la convention Open-Meteo (variableOptions du package
    // @openmeteo/weather-map-layer) : `wind_u_component_10m`, pas `wind_u_10m`.
    // Le client maps/ dérive la vitesse depuis u/v → pas besoin de la publier.
    VariableEntry { grib_short_name: "10u",       om_name: "wind_u_component_10m", unit_conversion: UnitConversion::None,                package: "SP1" },
    VariableEntry { grib_short_name: "10v",       om_name: "wind_v_component_10m", unit_conversion: UnitConversion::None,                package: "SP1" },
    VariableEntry { grib_short_name: "max_i10fg", om_name: "wind_gusts_10m",       unit_conversion: UnitConversion::None,                package: "SP1" },
    VariableEntry { grib_short_name: "prmsl",     om_name: "pressure_msl",         unit_conversion: UnitConversion::PascalToHectopascal, package: "SP1" },
    VariableEntry { grib_short_name: "tp",        om_name: "precipitation",        unit_conversion: UnitConversion::KgPerM2ToMm,         package: "SP1" },
    // SP2 (4 vars) — additionnels surface
    VariableEntry { grib_short_name: "2d",        om_name: "dew_point_2m",         unit_conversion: UnitConversion::KelvinToCelsius,     package: "SP2" },
    VariableEntry { grib_short_name: "lcc",       om_name: "cloud_cover_low",      unit_conversion: UnitConversion::None,                package: "SP2" },
    VariableEntry { grib_short_name: "mcc",       om_name: "cloud_cover_mid",      unit_conversion: UnitConversion::None,                package: "SP2" },
    VariableEntry { grib_short_name: "hcc",       om_name: "cloud_cover_high",     unit_conversion: UnitConversion::None,                package: "SP2" },
];

pub fn variables_for_package(pkg: &str) -> impl Iterator<Item = &'static VariableEntry> {
    VARIABLES.iter().filter(move |v| v.package == pkg)
}

pub fn lookup_by_grib(short: &str) -> Option<&'static VariableEntry> {
    VARIABLES.iter().find(|v| v.grib_short_name == short)
}

pub fn lookup_by_om(name: &str) -> Option<&'static VariableEntry> {
    VARIABLES.iter().find(|v| v.om_name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_round_trip_grib_to_om_to_grib() {
        for v in VARIABLES {
            let by_grib = lookup_by_grib(v.grib_short_name).expect("grib not found");
            let by_om = lookup_by_om(by_grib.om_name).expect("om not found");
            assert_eq!(by_om.grib_short_name, v.grib_short_name);
        }
    }

    #[test]
    fn registry_om_names_are_unique() {
        let mut names: Vec<&str> = VARIABLES.iter().map(|v| v.om_name).collect();
        names.sort_unstable();
        let len = names.len();
        names.dedup();
        assert_eq!(names.len(), len, "duplicate om_name in VARIABLES");
    }

    #[test]
    fn registry_grib_short_names_are_unique() {
        let mut names: Vec<&str> = VARIABLES.iter().map(|v| v.grib_short_name).collect();
        names.sort_unstable();
        let len = names.len();
        names.dedup();
        assert_eq!(names.len(), len, "duplicate grib_short_name");
    }

    #[test]
    fn variables_for_each_package_non_empty() {
        for pkg in ["SP1", "SP2"] {
            assert!(variables_for_package(pkg).next().is_some(), "no vars in {pkg}");
        }
    }
}

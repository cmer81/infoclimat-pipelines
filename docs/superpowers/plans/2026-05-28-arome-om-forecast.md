# AROME-OM Réunion Forecast Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Livrer un nouveau pipeline batch `arome-om-forecast` qui télécharge la prévision AROME-OM Réunion depuis l'API Météo-France, décode le GRIB2 via cfgrib, et publie ~12 variables × 7 fenêtres temporelles d'OMfiles sur R2 sous `data_spatial/arome_om_reunion/`, directement consommable par le client `maps/`.

**Architecture:** Nouveau crate binaire `arome-om-forecast` côté `crates/`, ré-utilisant `pipeline-core` étendu avec (a) `ReunionGrid` (2e impl du trait `Grid`), (b) `meteofrance_api` (auth OAuth2 + download GRIB2 — réutilisable pour radar plus tard), (c) `arome_om_metadata` (JSON pour le client `maps/`). Le décodage GRIB2 est délégué à un script Python (`scripts/decode_arome_om_grib.py`) utilisant `cfgrib` (eccodes), aligné avec le pattern CDS existant.

**Tech Stack:** Rust 1.85 (édition 2024), `tokio`, `reqwest`, `clap`, `serde`, `anyhow` / `thiserror`, `ndarray`, `omfiles`, `chrono`, `tracing`. Python 3 + `cfgrib` + `xarray` + `netCDF4`. `libeccodes` système. GitHub Actions pour la prod.

**Spec source:** `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`

---

## Phase 0 — Pre-flight discovery

Les 5 inconnues techniques du spec (URL exacte AROME-OM, dimensions grille Réunion, cadence runs, horizon, latence publication) sont délibérément non bloquantes pour la structure mais **doivent être levées avant d'écrire `ReunionGrid` (Task 2)** parce que leurs valeurs deviennent des constantes dans le code.

### Task 0: Probe Météo-France API to resolve TBDs

**Files:**
- Create (temporary, non-commité): `/tmp/arome-om-probe.sh`
- Modify: `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md` (annote les 5 inconnues avec leurs vraies valeurs)

**Pré-requis :** Le user doit avoir un compte sur https://portail-api.meteofrance.fr/ et avoir généré un `application_id` (long-lived). Cette valeur va dans `MF_APPLICATION_ID` (env var). Sans ça, on ne peut pas avancer.

- [ ] **Step 1: Vérifier que `MF_APPLICATION_ID` est dispo dans l'env (sinon arrêter et demander au user)**

Run:
```bash
test -n "$MF_APPLICATION_ID" && echo "OK" || echo "MISSING — demande au user de configurer son .env"
```
Expected: `OK`. Si `MISSING`, créer un `.env` à la racine avec `MF_APPLICATION_ID=…` et le sourcer (`source .env`).

- [ ] **Step 2: Obtenir un bearer token**

Run:
```bash
curl -s -X POST "https://portail-api.meteofrance.fr/token" \
  -H "Authorization: Basic $MF_APPLICATION_ID" \
  -d "grant_type=client_credentials" | tee /tmp/mf-token.json
```
Expected: JSON contenant `access_token`, `expires_in`. Si erreur 401, l'`application_id` n'est pas valide.

Extraire le token :
```bash
export MF_TOKEN=$(jq -r .access_token /tmp/mf-token.json)
echo "Token (len): ${#MF_TOKEN}"
```

- [ ] **Step 3: Lister les grids/packages disponibles pour AROME-OM (essais successifs)**

Hypothèse 1 — endpoint dédié `DPPaquetAROME-OM` :
```bash
curl -s -H "Authorization: Bearer $MF_TOKEN" \
  "https://public-api.meteofrance.fr/previnum/DPPaquetAROME-OM/v1/models" | tee /tmp/aom-models.json
```

Hypothèse 2 — endpoint commun avec un suffix de modèle :
```bash
curl -s -H "Authorization: Bearer $MF_TOKEN" \
  "https://public-api.meteofrance.fr/previnum/DPPaquetAROME/v1/models" | tee /tmp/aom-models-v2.json
```

Expected: Une des deux retourne un JSON listant des modèles. Identifier celui qui contient « AROME-OM », « AROME-INDIEN », « AROME-REUN » ou similaire pour la Réunion.

- [ ] **Step 4: Télécharger 1 fichier GRIB SP1 sur la fenêtre 00H06H pour le run le plus récent**

```bash
# Adapter MODEL, GRID, et la base URL selon ce qui a marché au Step 3.
# Exemple si MODEL=AROME-INDIEN, GRID=0.025 :
MODEL=AROME-INDIEN
GRID=0.025
RUN=$(date -u -d "$(date -u +%Y-%m-%dT%H:00:00) -6 hours" +%Y-%m-%dT%H:00:00Z | sed 's/T[0-9][0-9]:/T00:/')
curl -s -H "Authorization: Bearer $MF_TOKEN" \
  -o /tmp/aom-sp1-00H06H.grib2 \
  "https://public-api.meteofrance.fr/previnum/DPPaquetAROME-OM/v1/models/$MODEL/grids/$GRID/packages/SP1/productARO?referencetime=$RUN&time=00H06H&format=grib2"
ls -l /tmp/aom-sp1-00H06H.grib2
```
Expected: Fichier non vide (typiquement quelques MB). Si 404, ajuster l'URL.

- [ ] **Step 5: Lire le header GRIB pour extraire les dimensions de la grille**

Pré-requis : `eccodes` installé (`apt install libeccodes-tools` ou équivalent).

```bash
grib_dump -O /tmp/aom-sp1-00H06H.grib2 | grep -E "Ni |Nj |latitudeOfFirstGridPoint|latitudeOfLastGridPoint|longitudeOfFirstGridPoint|longitudeOfLastGridPoint|iDirectionIncrement|jDirectionIncrement" | head -20
```
Expected: Affiche `Ni` (nx), `Nj` (ny), les 4 coordonnées et `iDirectionIncrement` (dx en millionièmes de degré), `jDirectionIncrement` (dy).

- [ ] **Step 6: Inventorier les variables présentes dans SP1, SP2, SP3 (3 downloads)**

```bash
for P in SP1 SP2 SP3; do
  curl -s -H "Authorization: Bearer $MF_TOKEN" \
    -o /tmp/aom-$P.grib2 \
    "https://public-api.meteofrance.fr/previnum/DPPaquetAROME-OM/v1/models/$MODEL/grids/$GRID/packages/$P/productARO?referencetime=$RUN&time=00H06H&format=grib2"
  echo "=== $P ==="
  grib_ls -p shortName,units,typeOfLevel,level,stepRange /tmp/aom-$P.grib2 | head -30
done
```
Expected: Listing des `shortName` (ex. `2t`, `2d`, `tp`, `10u`, `10v`, `prmsl`, `lcc`, `mcc`, `hcc`, `ssrd`…), leurs unités, et les `stepRange` qui couvrent la fenêtre.

- [ ] **Step 7: Mettre à jour la spec avec les valeurs trouvées**

Édite `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`, section "Inconnues à lever dès la première PR" : remplace les 5 TBD par les valeurs réelles. Garder le fichier comme journal de bord ; ces valeurs seront ensuite hardcodées dans le code (Task 2, 5, etc).

Run pour le commit :
```bash
git add docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md
git commit -m "docs(arome-om): resolve 5 pre-impl unknowns from real API probe"
```

- [ ] **Step 8: Garder le GRIB SP1 comme fixture pour les tests d'intégration**

```bash
mkdir -p crates/arome-om-forecast/tests/fixtures
# On copie un seul GRIB compact pour servir de fixture. Le fichier complet
# peut être trop gros — on le réduira avec grib_filter à Task 18 si besoin.
cp /tmp/aom-sp1-00H06H.grib2 crates/arome-om-forecast/tests/fixtures/sp1_00H06H.grib2
ls -lh crates/arome-om-forecast/tests/fixtures/
```
**Ne PAS commit cette fixture maintenant** (le crate n'existe pas encore) — Task 1 créera la structure, Task 18 ajoutera la fixture proprement avec un filtrage taille.

---

## Phase 1 — Crate scaffold

### Task 1: Create `arome-om-forecast` crate skeleton

**Files:**
- Create: `crates/arome-om-forecast/Cargo.toml`
- Create: `crates/arome-om-forecast/src/main.rs`
- Create: `crates/arome-om-forecast/src/lib.rs`
- Modify: `Cargo.toml` (workspace members + workspace deps)

- [ ] **Step 1: Add `netcdf` to workspace dependencies if not already shared**

Lire `Cargo.toml` racine — si `netcdf` n'est qu'utilisé par `temperature-anomaly-climatology`, le promouvoir en `[workspace.dependencies]`. Si déjà dans `[workspace.dependencies]`, rien à faire.

Le ligne attendue dans `[workspace.dependencies]` (déjà présente d'après le fichier actuel) :
```toml
netcdf = { version = "0.10", features = ["static"] }
```

- [ ] **Step 2: Add `tempfile` to workspace dependencies (utilisé pour les fichiers intermédiaires GRIB→NetCDF)**

Modifier `Cargo.toml` racine, dans `[workspace.dependencies]` :
```toml
tempfile = "3"
```

- [ ] **Step 3: Add the new crate as workspace member**

Modifier `Cargo.toml` racine, dans `[workspace]` :
```toml
members = [
    "crates/core",
    "crates/temperature-anomaly-climatology",
    "crates/temperature-anomaly-observed",
    "crates/temperature-anomaly-forecast",
    "crates/arome-om-forecast",
]
```

- [ ] **Step 4: Create the new crate's `Cargo.toml`**

Create `crates/arome-om-forecast/Cargo.toml` with content:
```toml
[package]
name = "arome-om-forecast"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[lib]
name = "arome_om_forecast"
path = "src/lib.rs"

[[bin]]
name = "arome-om-forecast"
path = "src/main.rs"

[lints]
workspace = true

[dependencies]
pipeline-core.workspace = true
anyhow.workspace = true
clap.workspace = true
chrono.workspace = true
ndarray.workspace = true
reqwest.workspace = true
bytes.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
futures.workspace = true
tracing.workspace = true
dotenvy.workspace = true
omfiles.workspace = true
netcdf.workspace = true
tempfile.workspace = true
thiserror.workspace = true
```

- [ ] **Step 5: Create minimal `lib.rs` (modules vides pour l'instant)**

Create `crates/arome-om-forecast/src/lib.rs`:
```rust
pub mod grib_decoder;
pub mod planning;
pub mod variables;
```

- [ ] **Step 6: Create minimal `main.rs` (compile mais ne fait rien)**

Create `crates/arome-om-forecast/src/main.rs`:
```rust
//! CLI `arome-om-forecast` — pipeline AROME-OM Réunion (prévision brute).
//! Voir `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`.

fn main() {
    eprintln!("arome-om-forecast: not yet implemented");
    std::process::exit(0);
}
```

- [ ] **Step 7: Create stub module files (empty but compilable)**

Create `crates/arome-om-forecast/src/grib_decoder.rs`:
```rust
//! Wrapper Rust autour de `scripts/decode_arome_om_grib.py`.
```

Create `crates/arome-om-forecast/src/planning.rs`:
```rust
//! Build le plan de download (cartésien Package × TimeWindow) pour un run AROME-OM.
```

Create `crates/arome-om-forecast/src/variables.rs`:
```rust
//! Registry statique des variables AROME-OM exposées dans les OMfiles produits.
```

- [ ] **Step 8: Verify the workspace builds and lints pass**

Run:
```bash
cargo build -p arome-om-forecast
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```
Expected: Build OK, clippy clean.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml crates/arome-om-forecast/
git commit -m "feat(arome-om): scaffold arome-om-forecast crate"
```

### Task 2: `ReunionGrid` (2nd impl of `Grid` trait)

**Files:**
- Modify: `crates/core/src/grid.rs`

**Pré-requis :** Les valeurs `nx`, `ny`, `lon_min`, `lon_max`, `lat_min`, `lat_max`, `dx`, `dy` doivent provenir du Step 5 de la Task 0. Ci-dessous on utilise des **valeurs placeholder** que tu DOIS remplacer par les vraies. Ces placeholders correspondent à une estimation raisonnable AROME-OM Réunion ~0.025° couvrant l'île.

- [ ] **Step 1: Write the failing test**

Append to `crates/core/src/grid.rs` (avant la fermeture du module si applicable, ou en bas du fichier) :
```rust
#[cfg(test)]
mod reunion_tests {
    use super::*;

    #[test]
    fn reunion_grid_has_expected_dimensions() {
        let g = ReunionGrid::default();
        // ⚠️ Remplacer ces valeurs par celles de la Task 0 Step 5.
        assert_eq!(g.nx(), 201);
        assert_eq!(g.ny(), 161);
        assert!((g.dx() - 0.025).abs() < 1e-9);
        assert!((g.dy() - 0.025).abs() < 1e-9);
    }

    #[test]
    fn reunion_grid_corner_roundtrip() {
        let g = ReunionGrid::default();
        let (i, j) = g.lonlat_to_indices(g.lon_min(), g.lat_min()).unwrap();
        assert_eq!((i, j), (0, 0));
        let (lon, lat) = g.indices_to_lonlat(0, 0);
        assert!((lon - g.lon_min()).abs() < 1e-9);
        assert!((lat - g.lat_min()).abs() < 1e-9);
    }

    #[test]
    fn reunion_grid_rejects_outside_bbox() {
        let g = ReunionGrid::default();
        assert!(g.lonlat_to_indices(0.0, 0.0).is_none());     // Méditerranée
        assert!(g.lonlat_to_indices(55.5, -21.1).is_some());   // Saint-Denis approx
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:
```bash
cargo test -p pipeline-core grid::reunion_tests
```
Expected: FAIL — `cannot find type ReunionGrid in this scope`.

- [ ] **Step 3: Implement `ReunionGrid` after `ArpegeEuropeGrid`**

Append to `crates/core/src/grid.rs` (avant le bloc `#[cfg(test)]`) :
```rust
/// Grille AROME-OM Réunion (modèle Météo-France, Outre-Mer Océan Indien).
///
/// Lat/lon régulière. Valeurs extraites du header GRIB2 d'un fichier réel
/// (cf. `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`,
/// section "Inconnues à lever dès la première PR" — résolu Task 0).
#[derive(Debug, Clone, Copy, Default)]
pub struct ReunionGrid;

impl Grid for ReunionGrid {
    // ⚠️ Remplacer ces 8 valeurs par celles lues sur grib_dump (Task 0 Step 5).
    fn nx(&self) -> usize { 201 }
    fn ny(&self) -> usize { 161 }
    fn lon_min(&self) -> f64 { 53.0 }
    fn lon_max(&self) -> f64 { 58.0 }
    fn lat_min(&self) -> f64 { -23.0 }
    fn lat_max(&self) -> f64 { -19.0 }
    fn dx(&self) -> f64 { 0.025 }
    fn dy(&self) -> f64 { 0.025 }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:
```bash
cargo test -p pipeline-core grid::reunion_tests
```
Expected: 3 tests PASS.

- [ ] **Step 5: Run clippy on the workspace**

Run:
```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```
Expected: Clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/grid.rs
git commit -m "feat(core): add ReunionGrid as 2nd Grid impl (AROME-OM)"
```

---

## Phase 2 — Pure modules (TDD)

### Task 3: `Package` enum + `TimeWindow` type

**Files:**
- Modify: `crates/arome-om-forecast/src/planning.rs`

- [ ] **Step 1: Write failing tests**

Replace content of `crates/arome-om-forecast/src/planning.rs`:
```rust
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
```

- [ ] **Step 2: Run test to verify it passes**

Run:
```bash
cargo test -p arome-om-forecast planning::tests
```
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/arome-om-forecast/src/planning.rs
git commit -m "feat(arome-om): Package and TimeWindow types"
```

### Task 4: `build_plan()` pure function

**Files:**
- Modify: `crates/arome-om-forecast/src/planning.rs`

- [ ] **Step 1: Write failing tests**

Append to `crates/arome-om-forecast/src/planning.rs` (avant `#[cfg(test)]`):

```rust
/// Construit le plan de download pour un run donné : produit cartésien
/// `packages × windows`, où les windows couvrent `[0, horizon_h)` par
/// tranches de 6 h. Les windows à cheval (ex. horizon=42, dernière 36-42) sont
/// inclusives sur leur fin.
pub fn build_plan(horizon_h: u32, packages: &[Package]) -> Vec<(Package, TimeWindow)> {
    // On émet uniquement des fenêtres 6 h pleines qui tiennent dans `horizon_h`.
    // Si horizon_h n'est pas multiple de 6, la fraction restante est ignorée
    // (le user peut demander plus en passant un horizon multiple de 6).
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
```

Et dans `#[cfg(test)] mod tests`, append :
```rust
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
```

- [ ] **Step 2: Run tests**

Run:
```bash
cargo test -p arome-om-forecast planning
```
Expected: 7 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/arome-om-forecast/src/planning.rs
git commit -m "feat(arome-om): build_plan cartesian product of packages and windows"
```

### Task 5: Variable registry

**Files:**
- Modify: `crates/arome-om-forecast/src/variables.rs`

**Pré-requis :** Le mapping GRIB shortName ↔ nom variable OMfile doit refléter l'inventaire réel découvert à Task 0 Step 6. Les valeurs ci-dessous sont **réalistes pour AROME-OM** mais à confirmer.

- [ ] **Step 1: Write failing tests**

Replace content of `crates/arome-om-forecast/src/variables.rs`:
```rust
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

/// Inventaire MVP : pack surface (SP1+SP2+SP3). À aligner sur Task 0 Step 6.
pub const VARIABLES: &[VariableEntry] = &[
    // SP1 — variables atmosphériques surface essentielles
    VariableEntry { grib_short_name: "2t",    om_name: "temperature_2m",        unit_conversion: UnitConversion::KelvinToCelsius,       package: "SP1" },
    VariableEntry { grib_short_name: "2d",    om_name: "dew_point_2m",          unit_conversion: UnitConversion::KelvinToCelsius,       package: "SP1" },
    VariableEntry { grib_short_name: "2r",    om_name: "relative_humidity_2m",  unit_conversion: UnitConversion::None,                  package: "SP1" },
    VariableEntry { grib_short_name: "10u",   om_name: "wind_u_10m",            unit_conversion: UnitConversion::None,                  package: "SP1" },
    VariableEntry { grib_short_name: "10v",   om_name: "wind_v_10m",            unit_conversion: UnitConversion::None,                  package: "SP1" },
    VariableEntry { grib_short_name: "fg10",  om_name: "wind_gusts_10m",        unit_conversion: UnitConversion::None,                  package: "SP1" },
    VariableEntry { grib_short_name: "prmsl", om_name: "pressure_msl",          unit_conversion: UnitConversion::PascalToHectopascal,   package: "SP1" },
    // SP2 — précipitations + radiations
    VariableEntry { grib_short_name: "tp",    om_name: "precipitation",         unit_conversion: UnitConversion::KgPerM2ToMm,           package: "SP2" },
    VariableEntry { grib_short_name: "ssrd",  om_name: "shortwave_radiation",   unit_conversion: UnitConversion::None,                  package: "SP2" },
    // SP3 — nuages
    VariableEntry { grib_short_name: "lcc",   om_name: "cloud_cover_low",       unit_conversion: UnitConversion::None,                  package: "SP3" },
    VariableEntry { grib_short_name: "mcc",   om_name: "cloud_cover_mid",       unit_conversion: UnitConversion::None,                  package: "SP3" },
    VariableEntry { grib_short_name: "hcc",   om_name: "cloud_cover_high",      unit_conversion: UnitConversion::None,                  package: "SP3" },
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
        for pkg in ["SP1", "SP2", "SP3"] {
            assert!(variables_for_package(pkg).next().is_some(), "no vars in {pkg}");
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run:
```bash
cargo test -p arome-om-forecast variables
```
Expected: 4 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/arome-om-forecast/src/variables.rs
git commit -m "feat(arome-om): variable registry (grib shortName <-> om name)"
```

---

## Phase 3 — `meteofrance_api` in `pipeline-core`

### Task 6: `MeteoFranceError` + URL builders

**Files:**
- Create: `crates/core/src/meteofrance_api.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/Cargo.toml` (ajouter `thiserror` si absent)

- [ ] **Step 1: Ensure `thiserror` is in pipeline-core deps**

Lire `crates/core/Cargo.toml` ; si `thiserror` est absent des `[dependencies]`, l'ajouter :
```toml
thiserror.workspace = true
```

- [ ] **Step 2: Add module to `lib.rs`**

Modify `crates/core/src/lib.rs`, append:
```rust
pub mod meteofrance_api;
```

- [ ] **Step 3: Write failing tests + impl for URL builders**

Create `crates/core/src/meteofrance_api.rs`:
```rust
//! Client HTTP minimal pour le portail Météo-France (auth OAuth2 + download
//! GRIB2). Scope MVP : AROME-OM. Pattern réutilisable pour radar plus tard.

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error)]
pub enum MeteoFranceError {
    #[error("auth failed: {0}")]
    Auth(String),
    #[error("rate limited (Retry-After: {retry_after_s:?})")]
    RateLimited { retry_after_s: Option<u64> },
    #[error("http {status}: {body}")]
    Http { status: u16, body: String },
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("incomplete response: expected {expected} bytes, got {got}")]
    Incomplete { expected: u64, got: u64 },
}

pub const PUBLIC_API_BASE: &str = "https://public-api.meteofrance.fr";
pub const TOKEN_ENDPOINT: &str = "https://portail-api.meteofrance.fr/token";

/// Construit l'URL d'un fichier GRIB2 sur l'API DPPaquetAROME-OM.
///
/// ⚠️ Le path exact (`DPPaquetAROME-OM` vs alternative) est confirmé à Task 0
/// Step 3. Ajuster ici si nécessaire.
pub fn build_product_url(
    base: &str,
    api_namespace: &str,           // ex. "DPPaquetAROME-OM"
    model: &str,                   // ex. "AROME-INDIEN" (résolu à Task 0)
    grid: &str,                    // ex. "0.025"
    package: &str,                 // ex. "SP1"
    reference_time: DateTime<Utc>,
    time_window: &str,             // ex. "00H06H"
) -> String {
    format!(
        "{base}/previnum/{ns}/v1/models/{model}/grids/{grid}/packages/{package}/productARO?referencetime={rt}&time={tw}&format=grib2",
        base = base.trim_end_matches('/'),
        ns = api_namespace,
        model = model,
        grid = grid,
        package = package,
        rt = reference_time.format("%Y-%m-%dT%H:%M:%SZ"),
        tw = time_window,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn url_format_matches_meteofrance_spec() {
        let rt = Utc.with_ymd_and_hms(2026, 5, 28, 0, 0, 0).unwrap();
        let url = build_product_url(
            PUBLIC_API_BASE,
            "DPPaquetAROME-OM",
            "AROME-INDIEN",
            "0.025",
            "SP1",
            rt,
            "00H06H",
        );
        assert_eq!(
            url,
            "https://public-api.meteofrance.fr/previnum/DPPaquetAROME-OM/v1/models/AROME-INDIEN/grids/0.025/packages/SP1/productARO?referencetime=2026-05-28T00:00:00Z&time=00H06H&format=grib2"
        );
    }

    #[test]
    fn url_trims_trailing_slash_on_base() {
        let rt = Utc.with_ymd_and_hms(2026, 1, 1, 6, 0, 0).unwrap();
        let url = build_product_url("https://x.example/", "NS", "M", "0.1", "SP1", rt, "07H12H");
        assert!(!url.contains("example//"));
        assert!(url.contains("time=07H12H"));
    }
}
```

- [ ] **Step 4: Run tests**

Run:
```bash
cargo test -p pipeline-core meteofrance_api
```
Expected: 2 tests PASS.

- [ ] **Step 5: Run clippy**

Run:
```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```
Expected: Clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/lib.rs crates/core/src/meteofrance_api.rs crates/core/Cargo.toml
git commit -m "feat(core): meteofrance_api skeleton + URL builder"
```

### Task 7: Retry classification (pure function)

**Files:**
- Modify: `crates/core/src/meteofrance_api.rs`

- [ ] **Step 1: Write failing tests**

Append to `crates/core/src/meteofrance_api.rs` (avant `#[cfg(test)]`):
```rust
/// Action à entreprendre suite à une réponse HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryAction {
    /// Le status est succès — pas de retry, on consomme le body.
    Ok,
    /// Token expiré — refresh une fois et rejouer.
    RefreshTokenAndRetry,
    /// Throttling. `delay` = `Retry-After` parsé, sinon backoff par défaut.
    BackoffAndRetry { delay_s: u64 },
    /// Erreur transitoire (5xx) — backoff expo et rejouer.
    TransientRetry { attempt: u32 },
    /// Erreur dure — propager.
    Fail(String),
}

const DEFAULT_RATELIMIT_DELAY_S: u64 = 30;

/// Classifie une réponse HTTP en `RetryAction`. Fonction pure (testable sans réseau).
pub fn classify_response(
    status: u16,
    retry_after_header: Option<&str>,
    attempt: u32,
) -> RetryAction {
    match status {
        200 | 206 => RetryAction::Ok,
        401 => RetryAction::RefreshTokenAndRetry,
        429 => {
            let delay_s = retry_after_header
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(DEFAULT_RATELIMIT_DELAY_S);
            RetryAction::BackoffAndRetry { delay_s }
        }
        s if (500..=599).contains(&s) => RetryAction::TransientRetry { attempt },
        s => RetryAction::Fail(format!("hard http {s}")),
    }
}

/// Backoff exponentiel : 1, 4, 16, 64 secondes. Capé à 64.
pub fn backoff_seconds(attempt: u32) -> u64 {
    match attempt {
        0 => 1,
        1 => 4,
        2 => 16,
        _ => 64,
    }
}
```

Append tests in `#[cfg(test)] mod tests`:
```rust
    #[test]
    fn classify_200_is_ok() {
        assert_eq!(classify_response(200, None, 0), RetryAction::Ok);
    }

    #[test]
    fn classify_206_is_ok() {
        assert_eq!(classify_response(206, None, 0), RetryAction::Ok);
    }

    #[test]
    fn classify_401_refreshes_token() {
        assert_eq!(classify_response(401, None, 0), RetryAction::RefreshTokenAndRetry);
    }

    #[test]
    fn classify_429_with_retry_after() {
        assert_eq!(
            classify_response(429, Some("12"), 0),
            RetryAction::BackoffAndRetry { delay_s: 12 }
        );
    }

    #[test]
    fn classify_429_without_retry_after_uses_default() {
        assert_eq!(
            classify_response(429, None, 0),
            RetryAction::BackoffAndRetry { delay_s: DEFAULT_RATELIMIT_DELAY_S }
        );
    }

    #[test]
    fn classify_5xx_is_transient_retry_with_attempt() {
        assert_eq!(
            classify_response(503, None, 2),
            RetryAction::TransientRetry { attempt: 2 }
        );
    }

    #[test]
    fn classify_4xx_hard_fails() {
        match classify_response(403, None, 0) {
            RetryAction::Fail(_) => (),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn backoff_grows_exponentially_then_caps() {
        assert_eq!(backoff_seconds(0), 1);
        assert_eq!(backoff_seconds(1), 4);
        assert_eq!(backoff_seconds(2), 16);
        assert_eq!(backoff_seconds(3), 64);
        assert_eq!(backoff_seconds(99), 64);
    }
```

- [ ] **Step 2: Run tests**

Run:
```bash
cargo test -p pipeline-core meteofrance_api
```
Expected: 10 tests PASS (2 URL + 8 retry).

- [ ] **Step 3: Commit**

```bash
git add crates/core/src/meteofrance_api.rs
git commit -m "feat(core): retry classification for meteofrance_api (pure fn)"
```

### Task 8: `MeteoFranceAuth` (token cache + HTTP)

**Files:**
- Modify: `crates/core/src/meteofrance_api.rs`

Pas de TDD ici — le code est dominé par I/O réseau et `tokio::sync::RwLock`. On vérifie manuellement avec un curl à Task 0 et on couvre la logique métier (caching) par tests ciblés.

- [ ] **Step 1: Implement `MeteoFranceAuth`**

Append to `crates/core/src/meteofrance_api.rs` (avant `#[cfg(test)]`):
```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::Duration;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64, // secondes
}

#[derive(Clone, Debug)]
struct CachedToken {
    token: String,
    /// Instant absolu d'expiration (avec marge de sécurité).
    expires_at: DateTime<Utc>,
}

/// Margin avant l'expiration où on commence à rafraîchir le token.
const TOKEN_REFRESH_MARGIN_S: i64 = 60;

pub struct MeteoFranceAuth {
    /// `application_id` long-lived (env `MF_APPLICATION_ID`).
    application_id: String,
    cached: RwLock<Option<CachedToken>>,
    http: reqwest::Client,
}

impl MeteoFranceAuth {
    /// Construit depuis l'env. Échoue si `MF_APPLICATION_ID` est absent.
    pub fn from_env() -> Result<Self, MeteoFranceError> {
        let application_id = std::env::var("MF_APPLICATION_ID")
            .map_err(|_| MeteoFranceError::Auth("MF_APPLICATION_ID missing".into()))?;
        let http = reqwest::Client::builder()
            .user_agent("infoclimat-pipelines/0.1")
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(MeteoFranceError::Transport)?;
        Ok(Self {
            application_id,
            cached: RwLock::new(None),
            http,
        })
    }

    /// Retourne un bearer token valide. Refresh paresseux si <60 s avant
    /// expiration. Thread-safe : plusieurs callers concurrents partagent
    /// le même token (RwLock).
    pub async fn get_token(&self) -> Result<String, MeteoFranceError> {
        // Fast path : token cache encore valide.
        {
            let guard = self.cached.read().await;
            if let Some(c) = guard.as_ref() {
                let now = Utc::now();
                if c.expires_at > now + Duration::seconds(TOKEN_REFRESH_MARGIN_S) {
                    return Ok(c.token.clone());
                }
            }
        }
        // Slow path : refresh.
        self.refresh_token().await
    }

    /// Force un refresh (utilisé sur 401 après un `get_token` qui avait pourtant
    /// un token cache jugé valide — le serveur a peut-être révoqué).
    pub async fn force_refresh(&self) -> Result<String, MeteoFranceError> {
        self.refresh_token().await
    }

    async fn refresh_token(&self) -> Result<String, MeteoFranceError> {
        let resp = self
            .http
            .post(TOKEN_ENDPOINT)
            .header("Authorization", format!("Basic {}", self.application_id))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await?;
        let status = resp.status().as_u16();
        if status != 200 {
            let body = resp.text().await.unwrap_or_default();
            return Err(MeteoFranceError::Auth(format!("token endpoint {status}: {body}")));
        }
        let parsed: TokenResponse = resp.json().await?;
        let expires_at = Utc::now() + Duration::seconds(parsed.expires_in as i64);
        let mut guard = self.cached.write().await;
        *guard = Some(CachedToken {
            token: parsed.access_token.clone(),
            expires_at,
        });
        Ok(parsed.access_token)
    }
}

/// Wrapper Arc pour partager `MeteoFranceAuth` entre tâches tokio.
pub type SharedAuth = Arc<MeteoFranceAuth>;
```

- [ ] **Step 2: Verify it compiles + clippy clean**

Run:
```bash
cargo build -p pipeline-core
cargo clippy -p pipeline-core --all-targets --all-features --locked -- -D warnings
```
Expected: Build OK, clippy clean.

- [ ] **Step 3: Commit**

```bash
git add crates/core/src/meteofrance_api.rs
git commit -m "feat(core): MeteoFranceAuth with lazy token caching"
```

### Task 9: `AromeOmClient::fetch_package` (with retry orchestration)

**Files:**
- Modify: `crates/core/src/meteofrance_api.rs`

- [ ] **Step 1: Add the client implementation**

Append to `crates/core/src/meteofrance_api.rs` (avant `#[cfg(test)]`):
```rust
use bytes::Bytes;

const MAX_TRANSIENT_RETRIES: u32 = 3;
const MAX_RATELIMIT_RETRIES: u32 = 3;

/// Identifiant API d'un territoire AROME-OM. La valeur exacte du `model_id`
/// (utilisé dans le path URL) est résolue à Task 0. `Reunion` correspond
/// vraisemblablement à "AROME-INDIEN" côté API.
#[derive(Debug, Clone, Copy)]
pub enum AromeOmTerritory {
    Reunion,
}

impl AromeOmTerritory {
    pub fn model_id(&self) -> &'static str {
        match self {
            // ⚠️ Valeur exacte confirmée à Task 0. Ajuster si l'API utilise
            // un autre nom (AROME-OM-REUN, AROME-OUTREMER-INDIEN, etc.).
            AromeOmTerritory::Reunion => "AROME-INDIEN",
        }
    }
    pub fn grid_id(&self) -> &'static str {
        // 0.025° pour tous les territoires AROME-OM d'après la doc.
        "0.025"
    }
}

pub struct AromeOmClient {
    base: String,
    api_namespace: String,
    auth: SharedAuth,
    http: reqwest::Client,
}

impl AromeOmClient {
    pub fn new(auth: SharedAuth) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("infoclimat-pipelines/0.1")
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .expect("reqwest client build");
        Self {
            base: PUBLIC_API_BASE.to_string(),
            // ⚠️ Confirmer le namespace exact à Task 0 (DPPaquetAROME-OM ou autre).
            api_namespace: "DPPaquetAROME-OM".to_string(),
            auth,
            http,
        }
    }

    /// Download GRIB2 d'un (territoire, package, run, window). Gère retry,
    /// refresh-token-on-401, et rate limiting.
    pub async fn fetch_package(
        &self,
        territory: AromeOmTerritory,
        package: &str,
        reference_time: DateTime<Utc>,
        time_window: &str,
    ) -> Result<Bytes, MeteoFranceError> {
        let url = build_product_url(
            &self.base,
            &self.api_namespace,
            territory.model_id(),
            territory.grid_id(),
            package,
            reference_time,
            time_window,
        );

        let mut token = self.auth.get_token().await?;
        let mut transient_attempt = 0u32;
        let mut ratelimit_attempt = 0u32;
        let mut already_refreshed_for_401 = false;

        loop {
            let resp = self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await?;
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let action = classify_response(status, retry_after.as_deref(), transient_attempt);

            match action {
                RetryAction::Ok => {
                    let body = resp.bytes().await?;
                    return Ok(body);
                }
                RetryAction::RefreshTokenAndRetry => {
                    if already_refreshed_for_401 {
                        let body = resp.text().await.unwrap_or_default();
                        return Err(MeteoFranceError::Auth(format!("repeated 401: {body}")));
                    }
                    already_refreshed_for_401 = true;
                    token = self.auth.force_refresh().await?;
                }
                RetryAction::BackoffAndRetry { delay_s } => {
                    if ratelimit_attempt >= MAX_RATELIMIT_RETRIES {
                        return Err(MeteoFranceError::RateLimited {
                            retry_after_s: Some(delay_s),
                        });
                    }
                    tracing::warn!(delay_s, "rate limited — backing off");
                    tokio::time::sleep(std::time::Duration::from_secs(delay_s)).await;
                    ratelimit_attempt += 1;
                }
                RetryAction::TransientRetry { attempt } => {
                    if attempt >= MAX_TRANSIENT_RETRIES {
                        let body = resp.text().await.unwrap_or_default();
                        return Err(MeteoFranceError::Http { status, body });
                    }
                    let secs = backoff_seconds(attempt);
                    tracing::warn!(status, attempt, secs, "transient error — retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                    transient_attempt += 1;
                }
                RetryAction::Fail(msg) => {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(MeteoFranceError::Http {
                        status,
                        body: format!("{msg}: {body}"),
                    });
                }
            }
        }
    }
}
```

- [ ] **Step 2: Verify it compiles + clippy clean**

Run:
```bash
cargo build -p pipeline-core
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```
Expected: Build OK, clippy clean.

- [ ] **Step 3: Manual smoke test (uses real network + MF_APPLICATION_ID)**

Create un binaire de smoke test ad-hoc dans `crates/core/examples/mf-smoke.rs`:
```rust
//! cargo run -p pipeline-core --example mf-smoke
//! Vérifie que l'auth et un single fetch marchent contre l'API réelle.

use chrono::{Duration, TimeZone, Utc};
use std::sync::Arc;
use pipeline_core::meteofrance_api::{AromeOmClient, AromeOmTerritory, MeteoFranceAuth};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let auth = Arc::new(MeteoFranceAuth::from_env()?);
    let client = AromeOmClient::new(auth);
    // Run = aujourd'hui 00Z (à ajuster si pas encore publié).
    let run = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let bytes = client.fetch_package(AromeOmTerritory::Reunion, "SP1", run, "00H06H").await?;
    println!("fetched {} bytes for SP1 00H06H", bytes.len());
    Ok(())
}
```

Ajouter `examples` au manifeste si pas déjà fait (`crates/core/Cargo.toml`) — par défaut Cargo découvre les exemples dans `examples/`, rien à modifier.

Run:
```bash
cargo run --release -p pipeline-core --example mf-smoke
```
Expected: `fetched <NB> bytes for SP1 00H06H` avec NB > 100000. Si erreur, ajuster `model_id` ou `api_namespace`.

- [ ] **Step 4: Delete the smoke example (ce n'est pas du code de prod)**

```bash
rm crates/core/examples/mf-smoke.rs
```

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/meteofrance_api.rs
git commit -m "feat(core): AromeOmClient.fetch_package with retry + token refresh"
```

---

## Phase 4 — GRIB2 decoding

### Task 10: Python helper script (`scripts/decode_arome_om_grib.py`)

**Files:**
- Create: `scripts/decode_arome_om_grib.py`
- Modify: `README.md` (mentionner les nouveaux pré-requis Python)

- [ ] **Step 1: Write the script**

Create `scripts/decode_arome_om_grib.py`:
```python
#!/usr/bin/env python3
"""Décode un GRIB2 multi-messages AROME-OM et produit 1 NetCDF par
(variable, leadtime) dans un dossier de sortie.

Appelé par le binaire Rust `arome-om-forecast`. Sortie NetCDF parce que c'est
le seul format que le wrapper Rust sait déjà lire (dep `netcdf` partagée avec
le crate climato).

Usage:
    python decode_arome_om_grib.py \\
        --in /path/to/file.grib2 \\
        --shortnames 2t,2d,10u,10v,prmsl \\
        --out-dir /tmp/decoded \\
        [--unit-convert kelvin-to-celsius]

Pré-requis pip (ajouter au venv projet):
    cfgrib xarray netCDF4

Pré-requis système:
    libeccodes (apt install libeccodes0 libeccodes-tools)
"""

import argparse
import os
import sys
from typing import List

try:
    import cfgrib  # noqa: F401
    import xarray as xr
except ImportError as e:
    print(f"FATAL: missing dependency ({e}). Run: pip install cfgrib xarray netCDF4", file=sys.stderr)
    sys.exit(2)


def decode(grib_path: str, shortnames: List[str], out_dir: str) -> int:
    """Pour chaque shortName, ouvre le GRIB filtré et écrit un NetCDF par
    leadtime (= une dimension `step` côté cfgrib)."""
    os.makedirs(out_dir, exist_ok=True)
    written = 0
    for sn in shortnames:
        try:
            ds = xr.open_dataset(
                grib_path,
                engine="cfgrib",
                backend_kwargs={"filter_by_keys": {"shortName": sn}, "indexpath": ""},
            )
        except Exception as e:
            print(f"WARN: variable {sn!r} not found or unreadable: {e}", file=sys.stderr)
            continue

        var = ds[sn] if sn in ds.data_vars else next(iter(ds.data_vars.values()))
        # `step` peut être un scalaire si une seule leadtime dans la fenêtre.
        steps = ds["step"].values if "step" in ds.coords else [None]
        if steps.shape == ():
            steps = [steps.item()]

        for step in steps:
            sub = var.sel(step=step) if step is not None and "step" in var.dims else var
            lead_h = int(step / 1_000_000_000 / 3600) if step is not None else 0
            out_path = os.path.join(out_dir, f"{sn}_{lead_h:03d}h.nc")
            sub.to_netcdf(out_path)
            written += 1
            print(f"OK: wrote {out_path}", file=sys.stderr)
    return written


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--in", dest="grib_in", required=True)
    p.add_argument("--shortnames", required=True, help="CSV liste shortName")
    p.add_argument("--out-dir", required=True)
    args = p.parse_args()
    shortnames = [s.strip() for s in args.shortnames.split(",") if s.strip()]
    n = decode(args.grib_in, shortnames, args.out_dir)
    if n == 0:
        print("ERROR: no NetCDF produced", file=sys.stderr)
        sys.exit(1)
    print(f"DONE: {n} NetCDF files written to {args.out_dir}", file=sys.stderr)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Make it executable**

```bash
chmod +x scripts/decode_arome_om_grib.py
```

- [ ] **Step 3: Manual smoke test against the fixture from Task 0**

Pré-requis : `pip install cfgrib xarray netCDF4` dans le venv projet, et `libeccodes` installé sur le système.

```bash
source venv/bin/activate  # ou équivalent
python scripts/decode_arome_om_grib.py \
  --in /tmp/aom-sp1-00H06H.grib2 \
  --shortnames 2t,10u,10v \
  --out-dir /tmp/decoded
ls /tmp/decoded/
```
Expected: Plusieurs `.nc` files (un par variable × leadtime). Pour SP1 sur 00H06H avec sortie horaire = 6-7 timesteps × 3 vars = ~20 fichiers.

Vérifier rapidement avec :
```bash
python -c "import xarray as xr; ds=xr.open_dataset('/tmp/decoded/2t_000h.nc'); print(ds)"
```
Expected: Dataset 2D `(latitude, longitude)` avec les bonnes dimensions Réunion.

- [ ] **Step 4: Document the venv setup in README**

Modify `README.md` : Trouver la section "venv requis" (ou créer si absente) et ajouter :
```markdown
### Pipeline `arome-om-forecast`

En plus des deps déjà documentées :

```bash
# Système (Debian-likes)
sudo apt install libeccodes0 libeccodes-tools

# Python (venv projet)
pip install cfgrib xarray netCDF4
```

Le binaire appelle `python3` du PATH (typiquement via `source venv/bin/activate`) sur `scripts/decode_arome_om_grib.py` pour décoder le GRIB2.
```

- [ ] **Step 5: Commit**

```bash
git add scripts/decode_arome_om_grib.py README.md
git commit -m "feat(arome-om): python helper script for GRIB2 decoding via cfgrib"
```

### Task 11: `grib_decoder.rs` Rust wrapper

**Files:**
- Modify: `crates/arome-om-forecast/src/grib_decoder.rs`

- [ ] **Step 1: Implement the wrapper**

Replace content of `crates/arome-om-forecast/src/grib_decoder.rs`:
```rust
//! Wrapper Rust autour de `scripts/decode_arome_om_grib.py`.
//!
//! Cycle : reçoit un GRIB2 sur disque, lance le script Python, lit les NetCDF
//! produits, et applique les conversions d'unité côté Rust avant de retourner
//! des `Array2<f32>` typés par variable et leadtime.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use ndarray::Array2;
use tokio::process::Command;

use crate::variables::{UnitConversion, VariableEntry};

pub struct DecodedSlice {
    pub om_name: &'static str,
    pub leadtime_h: u32,
    pub data: Array2<f32>,
}

/// Décode un fichier GRIB2 multi-messages en N slices `(variable, leadtime, Array2)`.
///
/// `expected_dims` : `(ny, nx)` de `ReunionGrid` — sert à valider chaque slice.
pub async fn decode(
    grib_path: &Path,
    out_dir: &Path,
    variables_of_interest: &[&VariableEntry],
    expected_dims: (usize, usize),
) -> Result<Vec<DecodedSlice>> {
    std::fs::create_dir_all(out_dir).with_context(|| format!("mkdir {out_dir:?}"))?;
    let shortnames: Vec<&str> = variables_of_interest
        .iter()
        .map(|v| v.grib_short_name)
        .collect();
    let shortnames_csv = shortnames.join(",");

    let status = Command::new("python3")
        .arg("scripts/decode_arome_om_grib.py")
        .arg("--in")
        .arg(grib_path)
        .arg("--shortnames")
        .arg(&shortnames_csv)
        .arg("--out-dir")
        .arg(out_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("spawning python helper")?;

    anyhow::ensure!(status.success(), "python helper exited {status}");

    let mut out = Vec::new();
    for entry in std::fs::read_dir(out_dir).with_context(|| format!("readdir {out_dir:?}"))? {
        let entry = entry?;
        let path = entry.path();
        let Some((sn, lead_h)) = parse_filename(&path) else { continue };
        let Some(var) = variables_of_interest.iter().find(|v| v.grib_short_name == sn) else {
            tracing::warn!(?path, %sn, "decoded file for unknown shortName, skipping");
            continue;
        };
        let data = read_netcdf_2d(&path, expected_dims, var.unit_conversion)?;
        out.push(DecodedSlice {
            om_name: var.om_name,
            leadtime_h: lead_h,
            data,
        });
    }
    Ok(out)
}

/// `2t_006h.nc` → `("2t", 6)`. Retourne `None` pour les noms hors-format.
fn parse_filename(path: &Path) -> Option<(String, u32)> {
    let stem = path.file_stem()?.to_str()?;
    // Le shortName peut contenir un chiffre (ex. "10u"), donc on splitte sur le
    // *dernier* `_` et on retire le `h` final.
    let (sn, lead) = stem.rsplit_once('_')?;
    let lead = lead.strip_suffix('h')?;
    let lead_h = lead.parse::<u32>().ok()?;
    Some((sn.to_string(), lead_h))
}

fn read_netcdf_2d(
    path: &Path,
    expected: (usize, usize),
    convert: UnitConversion,
) -> Result<Array2<f32>> {
    let file = netcdf::open(path).with_context(|| format!("open netcdf {path:?}"))?;
    // Le NetCDF produit par xarray a UNE variable de données (le shortName de
    // base) + des coords. On prend la première variable non-coord.
    let var = file
        .variables()
        .find(|v| v.dimensions().len() == 2)
        .ok_or_else(|| anyhow::anyhow!("no 2D variable found in {path:?}"))?;
    let dims = var.dimensions();
    let ny = dims[0].len();
    let nx = dims[1].len();
    anyhow::ensure!(
        (ny, nx) == expected,
        "netcdf dims ({ny},{nx}) != ReunionGrid {expected:?}"
    );
    let flat: Vec<f32> = var
        .get_values::<f32, _>(..)
        .context("reading netcdf data")?;
    let arr = Array2::from_shape_vec((ny, nx), flat)?;
    Ok(apply_unit_conversion(arr, convert))
}

fn apply_unit_conversion(mut arr: Array2<f32>, convert: UnitConversion) -> Array2<f32> {
    match convert {
        UnitConversion::None => arr,
        UnitConversion::KelvinToCelsius => {
            arr.mapv_inplace(|v| if v.is_nan() { v } else { v - 273.15 });
            arr
        }
        UnitConversion::PascalToHectopascal => {
            arr.mapv_inplace(|v| if v.is_nan() { v } else { v / 100.0 });
            arr
        }
        UnitConversion::KgPerM2ToMm => arr, // densité eau ≈ 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_filename_extracts_shortname_and_leadtime() {
        let p = PathBuf::from("/tmp/x/2t_006h.nc");
        assert_eq!(parse_filename(&p), Some(("2t".to_string(), 6)));
        let p = PathBuf::from("/tmp/x/10u_042h.nc");
        assert_eq!(parse_filename(&p), Some(("10u".to_string(), 42)));
    }

    #[test]
    fn parse_filename_rejects_non_matching() {
        assert!(parse_filename(&PathBuf::from("/tmp/foo.txt")).is_none());
        // "no_underscore.nc" → stem "no_underscore" → rsplit_once('_') = ("no","underscore")
        // → strip_suffix('h') fails on "underscore" → None.
        assert!(parse_filename(&PathBuf::from("/tmp/no_underscore.nc")).is_none());
    }

    #[test]
    fn unit_conversion_kelvin_to_celsius_skips_nan() {
        let arr = Array2::from_shape_vec((1, 3), vec![273.15, f32::NAN, 300.0]).unwrap();
        let out = apply_unit_conversion(arr, UnitConversion::KelvinToCelsius);
        assert!((out[[0, 0]] - 0.0).abs() < 1e-4);
        assert!(out[[0, 1]].is_nan());
        assert!((out[[0, 2]] - 26.85).abs() < 1e-4);
    }
}
```

- [ ] **Step 2: Run unit tests**

Run:
```bash
cargo test -p arome-om-forecast grib_decoder
```
Expected: 3 tests PASS.

- [ ] **Step 3: Run clippy**

Run:
```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```
Expected: Clean.

- [ ] **Step 4: Commit**

```bash
git add crates/arome-om-forecast/src/grib_decoder.rs
git commit -m "feat(arome-om): grib_decoder Rust wrapper (python+netcdf bridge)"
```

---

## Phase 5 — Metadata

### Task 12: `arome_om_metadata` module

**Files:**
- Create: `crates/core/src/arome_om_metadata.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Add module to lib.rs**

Modify `crates/core/src/lib.rs`, append:
```rust
pub mod arome_om_metadata;
```

- [ ] **Step 2: Write the module with embedded tests**

Create `crates/core/src/arome_om_metadata.rs`:
```rust
//! Génération des métadonnées JSON du domaine `arome_om_reunion`.
//!
//! Le client `maps/` lit `data_spatial/arome_om_reunion/latest.json` (+
//! `in-progress.json` + `{run}/meta.json`) pour piloter son sélecteur de temps.
//! Schema simplifié vs `anomaly_metadata` (pas de `provisional_times`, pas
//! d'union observé/forecast — produit de pure prévision).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::r2::R2Client;

/// Shape compatible `DomainMetaDataJson` côté client.
#[derive(Debug, Clone, Serialize)]
pub struct ForecastDomainMetadata {
    pub reference_time: String,
    pub valid_times: Vec<String>,
    pub variables: Vec<String>,
}

const META_DOMAIN_PREFIX: &str = "data_spatial/arome_om_reunion";

/// Extrait `(variable, leadtime_h)` d'une clé R2 du type
/// `data_spatial/arome_om_reunion/Y/M/D/HHMMZ/{var}_{HHHh}.om`. Retourne `None`
/// pour les autres clés (meta.json, latest.json, etc.).
pub fn parse_run_key(key: &str) -> Option<(String, u32)> {
    let stem = key.rsplit('/').next()?.strip_suffix(".om")?;
    let (var, lead) = stem.rsplit_once('_')?;
    let lead = lead.strip_suffix('h')?;
    let lead_h = lead.parse::<u32>().ok()?;
    Some((var.to_string(), lead_h))
}

/// `data_spatial/arome_om_reunion/2026/05/28/0000Z/2t_006h.om` → `"2026-05-28T06:00:00Z"`.
///
/// Le `reference_time` est lu depuis la position fixe `Y/M/D/HHMMZ` dans la clé.
pub fn key_to_valid_time(key: &str) -> Option<String> {
    // Repère `data_spatial/arome_om_reunion/Y/M/D/HHMMZ/...`
    let trimmed = key.strip_prefix(META_DOMAIN_PREFIX)?.trim_start_matches('/');
    let mut parts = trimmed.split('/');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    let run_seg = parts.next()?; // ex "0000Z"
    let run_h: u32 = run_seg.strip_suffix('Z')?.get(..2)?.parse().ok()?;
    let (_, lead_h) = parse_run_key(key)?;
    let total_h = run_h as i64 + lead_h as i64;
    let date = chrono::NaiveDate::from_ymd_opt(y, m, d)?;
    let dt = date.and_hms_opt(0, 0, 0)?.and_utc() + chrono::Duration::hours(total_h);
    Some(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

pub async fn update_metadata(
    r2: &R2Client,
    run: DateTime<Utc>,
    variables: &[&'static str],
) -> Result<()> {
    let run_prefix = format!(
        "{META_DOMAIN_PREFIX}/{}/{}/{}/{}Z/",
        run.format("%Y"), run.format("%m"), run.format("%d"), run.format("%H%M"),
    );
    let keys = r2.list_prefix(&run_prefix).await.context("listing run keys")?;

    let mut times: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for k in &keys {
        if let Some(vt) = key_to_valid_time(k) {
            times.insert(vt);
        }
    }
    if times.is_empty() {
        tracing::warn!("no AROME-OM OMfiles found for run — skipping metadata write");
        return Ok(());
    }

    let meta = ForecastDomainMetadata {
        reference_time: run.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        valid_times: times.into_iter().collect(),
        variables: variables.iter().map(|s| s.to_string()).collect(),
    };
    let body = serde_json::to_vec(&meta).context("serializing metadata")?;
    let cc = "public, max-age=300";
    let ct = "application/json";
    let run_meta_key = format!("{run_prefix}meta.json");

    for key in [
        format!("{META_DOMAIN_PREFIX}/latest.json"),
        format!("{META_DOMAIN_PREFIX}/in-progress.json"),
        run_meta_key,
    ] {
        r2.put_bytes(&key, body.clone(), ct, cc)
            .await
            .with_context(|| format!("writing {key}"))?;
    }
    tracing::info!(reference_time = %meta.reference_time, vars = meta.variables.len(), "arome-om metadata written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_run_key_extracts_var_and_lead() {
        let k = "data_spatial/arome_om_reunion/2026/05/28/0000Z/temperature_2m_006h.om";
        assert_eq!(parse_run_key(k), Some(("temperature_2m".to_string(), 6)));
    }

    #[test]
    fn parse_run_key_rejects_non_om() {
        assert!(parse_run_key("foo/bar/meta.json").is_none());
        // "no_lead.om" → stem "no_lead" → rsplit_once('_') = ("no","lead")
        // → strip_suffix('h') fails on "lead" → None.
        assert!(parse_run_key("foo/bar/no_lead.om").is_none());
    }

    #[test]
    fn key_to_valid_time_adds_lead_to_run() {
        let k = "data_spatial/arome_om_reunion/2026/05/28/0000Z/temperature_2m_006h.om";
        assert_eq!(
            key_to_valid_time(k),
            Some("2026-05-28T06:00:00Z".to_string())
        );
    }

    #[test]
    fn key_to_valid_time_crosses_day_boundary() {
        let k = "data_spatial/arome_om_reunion/2026/05/28/1800Z/temperature_2m_012h.om";
        assert_eq!(
            key_to_valid_time(k),
            Some("2026-05-29T06:00:00Z".to_string())
        );
    }

    #[test]
    fn metadata_json_shape() {
        let meta = ForecastDomainMetadata {
            reference_time: "2026-05-28T00:00:00Z".into(),
            valid_times: vec!["2026-05-28T01:00:00Z".into()],
            variables: vec!["temperature_2m".into()],
        };
        let j = serde_json::to_value(&meta).unwrap();
        assert_eq!(j["reference_time"], "2026-05-28T00:00:00Z");
        assert_eq!(j["valid_times"][0], "2026-05-28T01:00:00Z");
        assert_eq!(j["variables"][0], "temperature_2m");
    }
}
```

- [ ] **Step 3: Run tests**

Run:
```bash
cargo test -p pipeline-core arome_om_metadata
```
Expected: 5 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/lib.rs crates/core/src/arome_om_metadata.rs
git commit -m "feat(core): arome_om_metadata module (forecast-only metadata writer)"
```

---

## Phase 6 — Orchestration

### Task 13: `main.rs` — CLI args + run determination

**Files:**
- Modify: `crates/arome-om-forecast/src/main.rs`

- [ ] **Step 1: Replace `main.rs` with the CLI skeleton**

Replace `crates/arome-om-forecast/src/main.rs`:
```rust
//! CLI `arome-om-forecast` — pipeline AROME-OM Réunion (prévision brute).
//!
//! Étapes (cf. `docs/superpowers/specs/2026-05-28-arome-om-forecast-design.md`):
//!  1. Détermine le run target (`floor_3h(now - publication_delay)` ou `--run`).
//!  2. Build le plan (packages × windows).
//!  3. Pour chaque (pkg, window), parallel buffer_unordered :
//!      a. Download GRIB2.
//!      b. Décode (script python) → N slices (var, leadtime, Array2).
//!      c. Écrit OMfile local + upload R2.
//!  4. Update metadata.
//!  5. GC des runs trop vieux.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Timelike, Utc};
use clap::Parser;

use pipeline_core::meteofrance_api::{AromeOmTerritory, MeteoFranceAuth};

const PUBLICATION_DELAY_H: i64 = 4;

#[derive(Debug, Parser)]
#[command(about = "Compute AROME-OM forecast OMfiles (raw values) and upload to R2")]
struct Args {
    /// Territoire AROME-OM (pour l'instant: reunion).
    #[arg(long, default_value = "reunion")]
    territory: String,
    /// Run cible (ISO 8601). Si omis : floor_3h(now - PUBLICATION_DELAY_H).
    #[arg(long)]
    run: Option<DateTime<Utc>>,
    /// Horizon max en heures (multiple de 6, capped par l'horizon du modèle).
    #[arg(long, default_value_t = 42)]
    horizon_h: u32,
    /// Packages à télécharger (CSV).
    #[arg(long, default_value = "SP1,SP2,SP3")]
    packages: String,
    /// Concurrence (downloads parallèles).
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
    /// Dossier de travail (GRIB téléchargés + OMfiles produits).
    #[arg(long)]
    work_dir: PathBuf,
    /// Préfixe R2 cible.
    #[arg(long, default_value = "data_spatial/arome_om_reunion")]
    r2_prefix: String,
    /// Combien de runs garder en R2 (GC).
    #[arg(long, default_value_t = 4)]
    keep_runs_back: u32,
    /// Si présent, n'uploade pas vers R2 (test local).
    #[arg(long)]
    skip_upload: bool,
}

/// `floor_3h(now - publication_delay)`. Renvoie l'heure 00/03/06/09/12/15/18/21
/// la plus récente >= maintenant - PUBLICATION_DELAY_H.
fn latest_run(now: DateTime<Utc>) -> DateTime<Utc> {
    let candidate = now - Duration::hours(PUBLICATION_DELAY_H);
    let h = candidate.hour();
    let floor_h = (h / 3) * 3;
    candidate
        .date_naive()
        .and_hms_opt(floor_h, 0, 0)
        .expect("valid hms")
        .and_utc()
}

fn parse_territory(s: &str) -> Result<AromeOmTerritory> {
    match s.to_lowercase().as_str() {
        "reunion" | "rÉunion" | "réunion" => Ok(AromeOmTerritory::Reunion),
        other => anyhow::bail!("unsupported territory: {other:?} (only 'reunion' for now)"),
    }
}

fn parse_packages(s: &str) -> Result<Vec<&'static str>> {
    let mut out = Vec::new();
    for item in s.split(',') {
        let p = item.trim();
        match p {
            "SP1" => out.push("SP1"),
            "SP2" => out.push("SP2"),
            "SP3" => out.push("SP3"),
            other => anyhow::bail!("unsupported package: {other:?}"),
        }
    }
    Ok(out)
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    pipeline_core::logging::init();
    let args = Args::parse();
    tracing::info!(?args, "starting arome-om-forecast");
    std::fs::create_dir_all(&args.work_dir).context("creating work_dir")?;

    let territory = parse_territory(&args.territory)?;
    let packages = parse_packages(&args.packages)?;
    let run = args.run.unwrap_or_else(|| latest_run(Utc::now()));
    tracing::info!(%run, ?packages, horizon_h = args.horizon_h, "plan parameters");

    let auth = Arc::new(MeteoFranceAuth::from_env().context("init auth")?);
    let _ = (auth, territory, run); // util'd at Task 14

    tracing::info!("plan-only mode — orchestration in Task 14");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn latest_run_floors_to_3h_with_publication_delay() {
        // 2026-05-28 14:23Z → candidate = 10:23Z → floor 09:00Z.
        let now = Utc.with_ymd_and_hms(2026, 5, 28, 14, 23, 0).unwrap();
        let run = latest_run(now);
        assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 28, 9, 0, 0).unwrap());
    }

    #[test]
    fn latest_run_handles_day_boundary() {
        // 2026-05-28 01:00Z → candidate = 21:00 the day before → 21:00Z J-1.
        let now = Utc.with_ymd_and_hms(2026, 5, 28, 1, 0, 0).unwrap();
        let run = latest_run(now);
        assert_eq!(run, Utc.with_ymd_and_hms(2026, 5, 27, 21, 0, 0).unwrap());
    }

    #[test]
    fn parse_territory_accepts_reunion_case_insensitive() {
        assert!(matches!(parse_territory("reunion").unwrap(), AromeOmTerritory::Reunion));
        assert!(matches!(parse_territory("REUNION").unwrap(), AromeOmTerritory::Reunion));
    }

    #[test]
    fn parse_packages_csv() {
        assert_eq!(parse_packages("SP1,SP3").unwrap(), vec!["SP1", "SP3"]);
        assert!(parse_packages("SP1,FOO").is_err());
    }
}
```

- [ ] **Step 2: Run tests**

Run:
```bash
cargo test -p arome-om-forecast
```
Expected: All tests pass (planning, variables, grib_decoder, main).

- [ ] **Step 3: Run clippy + build**

Run:
```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build -p arome-om-forecast
```
Expected: Clean.

- [ ] **Step 4: Smoke test the CLI**

```bash
./target/debug/arome-om-forecast --work-dir /tmp/aom-test --skip-upload 2>&1 | head -20
```
Expected: Tracing output « starting arome-om-forecast », « plan parameters », puis « plan-only mode — orchestration in Task 14 ». Pas de panic.

- [ ] **Step 5: Commit**

```bash
git add crates/arome-om-forecast/src/main.rs
git commit -m "feat(arome-om): CLI args + run determination (no-op orchestration)"
```

### Task 14: `main.rs` — Orchestration loop + GC

**Files:**
- Modify: `crates/arome-om-forecast/src/main.rs`

- [ ] **Step 1: Add the orchestration logic**

Replace the `main()` body and add helper functions in `crates/arome-om-forecast/src/main.rs`. Keep the existing imports, structs, parsing helpers and tests; rewrite from `#[tokio::main]` :
```rust
use futures::stream::{self, StreamExt};
use ndarray::Array2;
use pipeline_core::arome_om_metadata::update_metadata;
use pipeline_core::grid::{Grid, ReunionGrid};
use pipeline_core::meteofrance_api::{AromeOmClient, MeteoFranceError};
use pipeline_core::omfile_io::{OmfileMetadata, write_spatial_omfile};
use pipeline_core::r2::{CACHE_ROLLING, R2Client, R2Config};
use arome_om_forecast::grib_decoder::{self, DecodedSlice};
use arome_om_forecast::planning::{Package, TimeWindow, build_plan};
use arome_om_forecast::variables::{VARIABLES, VariableEntry, variables_for_package};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    pipeline_core::logging::init();
    let args = Args::parse();
    tracing::info!(?args, "starting arome-om-forecast");
    std::fs::create_dir_all(&args.work_dir).context("creating work_dir")?;

    let territory = parse_territory(&args.territory)?;
    let packages: Vec<Package> = parse_packages(&args.packages)?
        .into_iter()
        .map(|p| match p {
            "SP1" => Package::Sp1,
            "SP2" => Package::Sp2,
            "SP3" => Package::Sp3,
            _ => unreachable!("parse_packages guarantees set"),
        })
        .collect();

    let run = args.run.unwrap_or_else(|| latest_run(Utc::now()));
    let plan = build_plan(args.horizon_h, &packages);
    tracing::info!(%run, items = plan.len(), concurrency = args.concurrency, "plan built");

    let auth = Arc::new(MeteoFranceAuth::from_env().context("init auth")?);
    let mf = Arc::new(AromeOmClient::new(auth));
    let r2 = if !args.skip_upload {
        Some(Arc::new(R2Client::new(R2Config::from_env().context("R2 cfg")?).await?))
    } else {
        None
    };
    let grid = ReunionGrid;

    let work_dir = args.work_dir.clone();
    let r2_prefix = args.r2_prefix.clone();

    // Counters partagés.
    let counters = Arc::new(tokio::sync::Mutex::new((0u32, 0u32))); // (written, failures)

    stream::iter(plan.into_iter().map(|(pkg, window)| {
        let mf = mf.clone();
        let r2 = r2.clone();
        let work_dir = work_dir.clone();
        let r2_prefix = r2_prefix.clone();
        let counters = counters.clone();
        async move {
            match process_item(&mf, r2.as_deref(), territory, pkg, window, run, &grid, &work_dir, &r2_prefix).await {
                Ok(n) => {
                    let mut c = counters.lock().await;
                    c.0 += n;
                    tracing::info!(%pkg, %window, n, "item OK");
                }
                Err(e) => {
                    let mut c = counters.lock().await;
                    c.1 += 1;
                    tracing::error!(%pkg, %window, error = %e, "item FAILED");
                    if matches!(e.downcast_ref::<MeteoFranceError>(), Some(MeteoFranceError::Auth(_))) {
                        tracing::error!("auth error — aborting");
                        std::process::exit(2);
                    }
                }
            }
        }
    }))
    .buffer_unordered(args.concurrency)
    .collect::<Vec<_>>()
    .await;

    let (written, failures) = *counters.lock().await;
    tracing::info!(written, failures, "all items done");

    // Metadata + GC seulement si au moins un fichier a été écrit et qu'on uploade.
    if let Some(r2) = r2.as_deref() {
        if written > 0 {
            let var_names: Vec<&'static str> = VARIABLES.iter().map(|v| v.om_name).collect();
            if let Err(e) = update_metadata(r2, run, &var_names).await {
                tracing::error!(error = %e, "metadata update failed");
            }
            if let Err(e) = gc_old_runs(r2, &r2_prefix, run, args.keep_runs_back).await {
                tracing::error!(error = %e, "GC failed");
            }
        }
    }

    if failures > 0 || written == 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn process_item(
    mf: &AromeOmClient,
    r2: Option<&R2Client>,
    territory: AromeOmTerritory,
    pkg: Package,
    window: TimeWindow,
    run: DateTime<Utc>,
    grid: &ReunionGrid,
    work_dir: &std::path::Path,
    r2_prefix: &str,
) -> Result<u32> {
    let run_dir = work_dir.join(format!("{}Z", run.format("%Y%m%dT%H%M")));
    std::fs::create_dir_all(&run_dir)?;
    let grib_path = run_dir.join(format!("{pkg}_{window}.grib2"));

    let bytes = mf
        .fetch_package(territory, pkg.as_api_id(), run, &window.as_api_param())
        .await
        .with_context(|| format!("fetch {pkg} {window}"))?;
    std::fs::write(&grib_path, &bytes).with_context(|| format!("write {grib_path:?}"))?;

    let nc_dir = run_dir.join(format!("nc_{pkg}_{window}"));
    let pkg_id = pkg.as_api_id();
    let vars_of_interest: Vec<&VariableEntry> = variables_for_package(pkg_id).collect();
    let slices = grib_decoder::decode(&grib_path, &nc_dir, &vars_of_interest, (grid.ny(), grid.nx()))
        .await
        .with_context(|| format!("decode {pkg} {window}"))?;

    let mut written = 0u32;
    for slice in slices {
        match write_and_upload_slice(slice, run, &run_dir, r2, r2_prefix, grid).await {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!(error = %e, "slice failed"),
        }
    }
    Ok(written)
}

async fn write_and_upload_slice(
    slice: DecodedSlice,
    run: DateTime<Utc>,
    run_local_dir: &std::path::Path,
    r2: Option<&R2Client>,
    r2_prefix: &str,
    grid: &ReunionGrid,
) -> Result<()> {
    let filename = format!("{}_{:03}h.om", slice.om_name, slice.leadtime_h);
    let local = run_local_dir.join(&filename);
    let meta = OmfileMetadata {
        source: format!("arome_om_reunion_{}", run.format("%Y%m%dT%HZ")),
        generated_at: Utc::now(),
        extra: serde_json::json!({
            "variable": slice.om_name,
            "leadtime_h": slice.leadtime_h,
            "run": run.to_rfc3339(),
        }),
    };
    write_spatial_omfile(&local, &slice.data, grid, &meta)
        .context("write OMfile")?;
    if let Some(r2) = r2 {
        let key = format!(
            "{}/{}/{}/{}/{}Z/{}",
            r2_prefix.trim_end_matches('/'),
            run.format("%Y"), run.format("%m"), run.format("%d"),
            run.format("%H%M"),
            filename,
        );
        r2.upload_file(&key, &local, CACHE_ROLLING).await?;
    }
    Ok(())
}

/// Supprime les préfixes de run plus vieux que `run - keep_runs_back × 3h`.
async fn gc_old_runs(
    r2: &R2Client,
    r2_prefix: &str,
    current_run: DateTime<Utc>,
    keep_runs_back: u32,
) -> Result<()> {
    let cutoff = current_run - Duration::hours(3 * keep_runs_back as i64);
    let all = r2.list_prefix(&format!("{}/", r2_prefix.trim_end_matches('/'))).await?;
    for k in all {
        // On parse `r2_prefix/Y/M/D/HHMMZ/...` et on garde tout >= cutoff.
        let Some(rest) = k.strip_prefix(&format!("{}/", r2_prefix.trim_end_matches('/'))) else {
            continue;
        };
        let mut parts = rest.split('/');
        let Some(y) = parts.next().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let Some(m) = parts.next().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(d) = parts.next().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Some(hhmmz) = parts.next() else { continue };
        let Some(hhmm) = hhmmz.strip_suffix('Z') else { continue };
        let Ok(hh) = hhmm.get(..2).unwrap_or("").parse::<u32>() else { continue };
        let Some(date) = chrono::NaiveDate::from_ymd_opt(y, m, d) else { continue };
        let run_dt = date.and_hms_opt(hh, 0, 0).expect("valid hms").and_utc();
        if run_dt < cutoff {
            if let Err(e) = r2.delete(&k).await {
                tracing::warn!(key=%k, error=%e, "GC delete failed");
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Run unit tests**

Run:
```bash
cargo test -p arome-om-forecast
```
Expected: All previous tests still pass.

- [ ] **Step 3: Run clippy**

Run:
```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```
Expected: Clean.

- [ ] **Step 4: End-to-end smoke test (1 package, 1 window, no upload)**

Pré-requis : `.env` avec `MF_APPLICATION_ID`, venv activé, eccodes installé.

```bash
cargo build --release -p arome-om-forecast
./target/release/arome-om-forecast \
  --packages SP1 --horizon-h 6 --skip-upload --work-dir /tmp/aom-smoke
ls /tmp/aom-smoke/*/
```
Expected: ~7 OMfiles `{var}_{HHH}h.om` dans le dossier du run (1 par variable SP1 × leadtime).

Inspection rapide :
```bash
# Vérifier un OMfile produit (utiliser un script de lecture ou le binaire d'inspection omfile si dispo).
ls -lh /tmp/aom-smoke/*/temperature_2m_*.om
```
Expected: Fichiers ~1-10 KB chacun.

- [ ] **Step 5: Commit**

```bash
git add crates/arome-om-forecast/src/main.rs
git commit -m "feat(arome-om): orchestration loop + GC + metadata update"
```

---

## Phase 7 — Operations

### Task 15: GitHub Actions workflow

**Files:**
- Create: `.github/workflows/arome-om-forecast.yml`

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/arome-om-forecast.yml`:
```yaml
name: arome-om-forecast

on:
  schedule:
    # Lance ~4h après chaque run AROME-OM (00/03/06/09/12/15/18/21Z).
    # Si la cadence réelle est différente (Task 0), ajuster.
    - cron: '0 4,7,10,13,16,19,22,1 * * *'
  workflow_dispatch:

jobs:
  run:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    env:
      MF_APPLICATION_ID: ${{ secrets.MF_APPLICATION_ID }}
      R2_ACCOUNT_ID: ${{ secrets.R2_ACCOUNT_ID }}
      R2_ACCESS_KEY: ${{ secrets.R2_ACCESS_KEY }}
      R2_SECRET_KEY: ${{ secrets.R2_SECRET_KEY }}
      R2_BUCKET: ${{ secrets.R2_BUCKET }}
      RUST_LOG: info
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: "./ -> target"

      - name: Install eccodes (system)
        run: sudo apt-get update && sudo apt-get install -y libeccodes0 libeccodes-tools

      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.11'

      - name: Install cfgrib + xarray
        run: pip install cfgrib xarray netCDF4

      - name: Build cargo binary (release)
        run: cargo build --release -p arome-om-forecast

      - name: Run arome-om-forecast pipeline
        run: |
          mkdir -p work
          ./target/release/arome-om-forecast \
            --territory reunion \
            --packages SP1,SP2,SP3 \
            --horizon-h 42 \
            --work-dir work \
            --r2-prefix data_spatial/arome_om_reunion
```

- [ ] **Step 2: Verify the YAML parses (locally with yq or similar)**

Run:
```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/arome-om-forecast.yml'))" && echo "YAML OK"
```
Expected: `YAML OK`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/arome-om-forecast.yml
git commit -m "ci(arome-om): add scheduled workflow for AROME-OM Réunion pipeline"
```

### Task 16: Document operational setup

**Files:**
- Modify: `README.md` (à la racine)

- [ ] **Step 1: Add a dedicated section**

Modify `README.md` — ajouter (ou compléter) une section pipeline `arome-om-forecast` :
```markdown
## Pipeline `arome-om-forecast` (AROME-OM Réunion, prévision brute)

Publie sur R2 les OMfiles AROME-OM Réunion (prévision brute, ~12 variables surface, horizon 42h). Consommé tel quel par le client `maps/` sous le domaine `arome_om_reunion`.

### Pré-requis

- Compte sur https://portail-api.meteofrance.fr/ + `application_id` long-lived → `MF_APPLICATION_ID` (env var).
- Système : `apt install libeccodes0 libeccodes-tools` (Debian-likes).
- Python venv : `pip install cfgrib xarray netCDF4`.
- Bucket R2 déjà configuré (cf. autres pipelines).

### Lancement local

```bash
source venv/bin/activate
cargo run --release -p arome-om-forecast -- \
  --territory reunion \
  --packages SP1,SP2,SP3 \
  --horizon-h 42 \
  --work-dir work \
  --skip-upload
```

`--skip-upload` produit les OMfiles localement sans toucher R2.

### Cron production

`.github/workflows/arome-om-forecast.yml`, ~8 runs/jour alignés sur la publication AROME-OM.

### Secrets GitHub Actions requis

- `MF_APPLICATION_ID`
- `R2_ACCOUNT_ID`, `R2_ACCESS_KEY`, `R2_SECRET_KEY`, `R2_BUCKET` (partagés avec les autres pipelines)
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(arome-om): operational setup section in README"
```

---

## Phase 8 — Integration test (post-MVP, safe to defer)

### Task 17: GRIB fixture + integration test (`#[ignore]`)

**Files:**
- Create: `crates/arome-om-forecast/tests/grib_decoder_roundtrip.rs`
- Add: `crates/arome-om-forecast/tests/fixtures/sp1_00H06H.grib2` (~200 KB)

- [ ] **Step 1: Reduce the GRIB fixture from Task 0 to a minimal size**

Si la fixture Task 0 Step 8 est >500 KB, filtrer pour ne garder que 2-3 variables × 1 leadtime :
```bash
grib_filter -o crates/arome-om-forecast/tests/fixtures/sp1_00H06H.grib2 \
  'if (shortName is "2t" || shortName is "10u") { write; }' \
  /tmp/aom-sp1-00H06H.grib2
ls -lh crates/arome-om-forecast/tests/fixtures/sp1_00H06H.grib2
```
Expected: ≤200 KB. Si toujours trop gros, restreindre davantage.

- [ ] **Step 2: Write the integration test**

Create `crates/arome-om-forecast/tests/grib_decoder_roundtrip.rs`:
```rust
//! Integration test : décode la fixture GRIB2 réelle via le Python helper et
//! valide les dimensions + une sanity check d'orientation.
//!
//! Marqué `#[ignore]` : la CI n'a pas eccodes installé pour l'instant.
//! Lancer en local : `cargo test -p arome-om-forecast -- --ignored`.

use std::path::PathBuf;

use arome_om_forecast::grib_decoder;
use arome_om_forecast::variables::VARIABLES;
use pipeline_core::grid::{Grid, ReunionGrid};

#[tokio::test]
#[ignore]
async fn decode_fixture_yields_arrays_matching_reunion_grid() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sp1_00H06H.grib2");
    assert!(fixture.exists(), "fixture missing: {fixture:?}");

    let out_dir = tempfile::tempdir().unwrap();
    let vars: Vec<_> = VARIABLES
        .iter()
        .filter(|v| ["2t", "10u"].contains(&v.grib_short_name))
        .collect();

    let slices = grib_decoder::decode(&fixture, out_dir.path(), &vars, (ReunionGrid.ny(), ReunionGrid.nx()))
        .await
        .expect("decode");

    assert!(!slices.is_empty(), "no slices decoded");
    for slice in &slices {
        assert_eq!(slice.data.dim(), (ReunionGrid.ny(), ReunionGrid.nx()));
        // Sanity : T2m en °C autour de la Réunion doit être dans [15, 35].
        if slice.om_name == "temperature_2m" {
            let mid = slice.data[[ReunionGrid.ny() / 2, ReunionGrid.nx() / 2]];
            assert!(
                (15.0..=35.0).contains(&mid),
                "unexpected T2m {mid} (orientation bug?)"
            );
        }
    }
}
```

- [ ] **Step 3: Verify the test runs locally**

```bash
cargo test -p arome-om-forecast -- --ignored
```
Expected: 1 test PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/arome-om-forecast/tests/fixtures/sp1_00H06H.grib2 \
        crates/arome-om-forecast/tests/grib_decoder_roundtrip.rs
git commit -m "test(arome-om): grib_decoder integration test with real GRIB fixture"
```

---

## Done — handoff

- Ouvrir une PR GitHub depuis `feat/arome-om-forecast` vers `main`.
- Préparer côté `maps/` (repo voisin) une PR séparée qui enregistre le nouveau domaine `arome_om_reunion` + ses ~12 variables dans la registry client. Ce step est **hors scope** de ce plan (cross-repo).
- Configurer les secrets GitHub Actions `MF_APPLICATION_ID` sur ce repo avant que le cron démarre.
- Surveiller le premier run cron prod (logs Actions + état du bucket R2).

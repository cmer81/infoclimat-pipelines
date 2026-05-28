# AROME-OM Forecast Pipeline — Design Spec

**Date :** 2026-05-28
**Auteur :** Cédric Mercier (cmer@cmer.fr) + brainstorming Claude
**Statut :** Brouillon — en attente review user

## Contexte et motivation

Le workspace `infoclimat-pipelines` a été conçu dès le départ comme une **plateforme batch transversale**, pas comme un outil dédié aux anomalies de température. Aujourd'hui il ne contient qu'une seule application (anomalies T2m sur ARPEGE Europe), et un seul `impl Grid` (`ArpegeEuropeGrid`). L'objectif de ce spec est d'introduire un **second pipeline de production**, distinct dans son intention (prévision brute, pas anomalie) et dans sa géographie (DOM-TOM, pas Europe), pour deux raisons :

1. Couvrir les utilisateurs Infoclimat **Outre-mer**, aujourd'hui non servis.
2. Exercer pour la première fois la généricité déjà préparée dans `pipeline-core` (le trait `Grid` n'a jamais vu de 2e impl, le crate est nommé `pipeline-core` mais n'a jamais produit autre chose que des anomalies T2m).

Le pilote retenu : **AROME-OM Réunion**, prévision brute, pack de variables surface. C'est le couple qui valide le 2e `Grid` et l'ingestion non-Open-Meteo avec le moins de risque (un seul territoire, une seule source nouvelle).

Ce spec couvre **uniquement** ce premier pipeline. Les extensions naturelles (autres territoires DOM-TOM, autres modèles Météo-France, radar) sont mentionnées comme follow-ups mais hors scope.

## Décisions de cadrage

| Question | Décision | Pourquoi pas l'alternative |
|---|---|---|
| Produit utilisateur | Prévision brute, pas d'anomalie | Une anomalie en DOM-TOM exige une climato tropicale 30 ans par territoire (~1 trimestre de travail) — découplé pour ne pas bloquer la livraison |
| Périmètre géographique | 1 territoire pilote : **Réunion** | Géographie compacte (1 île), grille la plus simple, audience non négligeable |
| Variables | Pack complet surface (~10-12 : T2m, précip, vent 10m + rafale, RH, nuages bas/moyen/haut, MSLP, point de rosée, rayonnement) | « Strict minimum T2m » ne valide pas le multi-variable ; « tout » alourdit le stockage R2 sans usage évident |
| Décodage GRIB2 | Helper Python (`cfgrib` → eccodes) lancé par le binaire Rust | Aligné avec le pattern CDS existant (climato/observed appellent déjà Python). Même décodeur sous-jacent qu'Open-Meteo (libeccodes via SwiftEccodes côté Open-Meteo) |
| Stratégie de refacto | Approche 3 (binaire dédié + 2 abstractions ciblées) | « Calque pur » force du copier-coller au 2e territoire ; « refacto framework » sur-conçoit avec un seul use case |

## Architecture

### Structure de fichiers

```
infoclimat-pipelines/
├── crates/
│   ├── core/                          # pipeline-core
│   │   └── src/
│   │       ├── grid.rs                # + ReunionGrid (2e impl du trait Grid)
│   │       ├── meteofrance_api.rs     # NOUVEAU : OAuth2 + download GRIB2
│   │       ├── arome_om_metadata.rs   # NOUVEAU : meta JSON domaine arome_om_reunion
│   │       └── (autres modules inchangés)
│   └── arome-om-forecast/             # NOUVEAU crate binaire
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                # orchestration
│           ├── grib_decoder.rs        # wrapper Rust autour du script Python
│           ├── planning.rs            # build_plan(run, horizon, packages)
│           └── variables.rs           # registry GRIB shortName → nom OMfile
└── scripts/
    └── decode_arome_om_grib.py        # cfgrib + xarray → 1 NetCDF par (var, leadtime)
```

### Choix structurants

- **`meteofrance_api` vit dans `pipeline-core`**, pas dans le crate binaire. Justification : ce module sera réutilisé par un futur pipeline radar Météo-France. Scope clair (auth + download GRIB2), pas de dépendance vers la grille ou les variables AROME-OM.
- **`ReunionGrid` dans `grid.rs`** à côté de `ArpegeEuropeGrid`. C'est explicitement la 2e impl du trait `Grid` que le module attend depuis sa création.
- **Pas de mutation du crate `temperature-anomaly-forecast`** : il continue de tourner à l'identique en prod. Aucun refacto cross-cutting sur l'existant.
- **Layout R2 aligné sur le pipeline `omProtocol` standard** (`data_spatial/{domain}/{Y}/{M}/{D}/{HHMM}Z/{var}_{HHMM}.om`) — directement consommable par le resolver par défaut du client `maps/`, pas d'intégration custom requise.

## Composants

### `pipeline-core::grid::ReunionGrid`

2e impl du trait `Grid`. Définit une grille lat/lon régulière AROME-OM Réunion.

```rust
pub struct ReunionGrid;
impl Grid for ReunionGrid {
    fn nx(&self) -> usize { /* TBD à confirmer GRIB header */ }
    fn ny(&self) -> usize { /* TBD */ }
    fn lon_min(&self) -> f64 { /* TBD, autour de 53° */ }
    /* ... */
}
```

**TBD impl :** dimensions exactes (`nx`, `ny`, bornes, `dx`/`dy`) à hardcoder après lecture d'un premier GRIB réel. Estimation : ~0.025°, ~160×200 pixels, ~19-23°S × 53-58°E. Sera précisé en première PR.

### `pipeline-core::meteofrance_api`

Client HTTP pour le portail Météo-France. Trois types distincts :

```rust
pub struct MeteoFranceAuth {
    application_id: String,
    cached_token: RwLock<Option<CachedToken>>,
    http: reqwest::Client,
}
impl MeteoFranceAuth {
    pub fn from_env() -> Result<Self>;            // lit MF_APPLICATION_ID
    pub async fn get_token(&self) -> Result<String>;  // refresh paresseux
}

pub struct AromeOmClient { auth: Arc<MeteoFranceAuth>, http: reqwest::Client }
impl AromeOmClient {
    pub async fn fetch_package(
        &self,
        territory: AromeOmTerritory,  // enum: Reunion (autres territoires en follow-up)
        package: Package,             // enum: Sp1, Sp2, Sp3 (Hp*/Ip* en follow-up)
        run: DateTime<Utc>,
        window: TimeWindow,           // ex. (0, 6) => "00H06H"
    ) -> Result<Bytes, MeteoFranceError>;
}
```

**Politique de retry :** voir section « Error handling ». Bearer token rafraîchi paresseusement (refetch si <60s avant expiration ou sur 401 unique).

**TBD impl :** path exact `DPPaquetAROME-OM` (ou `DPPaquetAROME/models/AROME-OM-{TERRITORY}`) — à confirmer sur première requête réelle, isolé dans une fonction `build_product_url` testable.

### `pipeline-core::arome_om_metadata`

Écrit les JSON `data_spatial/arome_om_reunion/{latest,in-progress,run/meta}.json`. Schema simplifié vs `anomaly_metadata` (pas de `provisional_times`, pas d'union observé/forecast) :

```rust
#[derive(Serialize)]
struct ForecastDomainMetadata {
    reference_time: String,    // = run timestamp ISO
    valid_times: Vec<String>,  // pas horaires depuis le run, dérivés du listing R2
    variables: Vec<String>,
}

pub async fn update_metadata(r2: &R2Client, run: DateTime<Utc>) -> Result<()>;
```

Listing R2 du préfixe `data_spatial/arome_om_reunion/{run}/` pour reconstruire `valid_times`. Idempotent.

### `arome-om-forecast` (binaire)

Orchestration. CLI calquée sur `temperature-anomaly-forecast` :

```rust
struct Args {
    /// Territoire AROME-OM (pour l'instant: reunion).
    #[arg(long, default_value = "reunion")] territory: String,
    /// Run cible (défaut: floor_3h(now - publication_delay)).
    #[arg(long)] run: Option<DateTime<Utc>>,
    /// Horizon max en heures (défaut: 42, capped par l'horizon du modèle).
    #[arg(long, default_value_t = 42)] horizon_h: u32,
    /// Packages à télécharger (défaut: SP1,SP2,SP3).
    #[arg(long, default_value = "SP1,SP2,SP3")] packages: String,
    /// Concurrence des downloads.
    #[arg(long, default_value_t = 4)] concurrency: usize,
    #[arg(long)] work_dir: PathBuf,
    #[arg(long, default_value = "data_spatial/arome_om_reunion")] r2_prefix: String,
    #[arg(long)] skip_upload: bool,
}
```

Étapes :
1. Init R2Client, AromeOmClient, ReunionGrid.
2. Determine run (`args.run` ou `floor_3h(now - publication_delay)`).
3. `build_plan(run, horizon_h, packages)` → `Vec<(Package, TimeWindow)>`.
4. `stream::iter(plan).buffer_unordered(concurrency)` :
   a. `fetch_package` → bytes GRIB2 → file `{work_dir}/{run}/{pkg}_{window}.grib2`.
   b. `grib_decoder::decode(path, &variables)` → `Vec<(var, leadtime, Array2<f32>)>`.
   c. Pour chaque OMfile : write local + upload R2 (key `{r2_prefix}/{Y}/{M}/{D}/{HHMM}Z/{var}_{HHMM}.om`).
5. `arome_om_metadata::update_metadata(r2, run)`.
6. `gc_old_runs(r2, keep_back = 4)` — supprime les runs au-delà de N (~1 jour).

### `arome-om-forecast::planning`

```rust
pub fn build_plan(run: DateTime<Utc>, horizon_h: u32, packages: &[Package]) -> Vec<(Package, TimeWindow)>;
pub struct TimeWindow { pub start_h: u32, pub end_h: u32 }  // start_h % 6 == 0, end_h - start_h == 6
```

Fonction pure, testable sans réseau. Pour `horizon_h = 42` : 7 windows × 3 packages = 21 items.

### `arome-om-forecast::grib_decoder`

Wrapper Rust autour de `scripts/decode_arome_om_grib.py` :

```rust
pub async fn decode(
    grib_path: &Path,
    out_dir: &Path,
    variables: &[VariableEntry],
) -> Result<Vec<DecodedSlice>>;
pub struct DecodedSlice {
    pub variable: &'static str,
    pub leadtime_h: u32,
    pub data: Array2<f32>,
}
```

Lance le script via `tokio::process::Command`, lit les NetCDF produits, les charge en `Array2<f32>` (réutilise la dep `netcdf` du crate climato, à déplacer en workspace dep).

### `arome-om-forecast::variables`

Registry statique des ~12 variables :

```rust
pub struct VariableEntry {
    pub grib_short_name: &'static str,  // ex. "2t"
    pub om_name: &'static str,           // ex. "temperature_2m"
    pub unit_conversion: UnitConversion, // KelvinToCelsius | None | ...
}
pub const VARIABLES: &[VariableEntry] = &[...];
```

### `scripts/decode_arome_om_grib.py`

Input : chemin GRIB2, liste de shortNames, dossier de sortie. Pour chaque message GRIB2 dont le `shortName` est dans la liste : `cfgrib.open_dataset` → sélection → conversion d'unité (K→°C si applicable) → écrit `{out_dir}/{shortName}_{leadtime}.nc`.

## Data flow

```
scheduler (cron) ─► $ arome-om-forecast --territory reunion ...
                    │
                    ├─ determine run = floor_3h(now − publication_delay)
                    ├─ build_plan(run, 42h, [SP1,SP2,SP3]) → 21 items
                    │
                    ├─ stream(plan).buffer_unordered(4) :
                    │    ├─ AromeOmClient.fetch_package → bytes GRIB2
                    │    ├─ write {work}/{run}/{pkg}_{win}.grib2
                    │    ├─ grib_decoder.decode → N (var, lead, Array2)
                    │    ├─ pour chaque slice : write OMfile + upload R2
                    │
                    ├─ arome_om_metadata::update_metadata(r2, run)
                    └─ gc_old_runs(r2, keep_back=4)
```

**Trois invariants robustesse :**
1. **Ordre d'écriture** : OMfiles → métadonnées → GC. Garantit qu'on n'annonce jamais dans `valid_times` un fichier non encore écrit (race connue, gérée pareil sur le forecast actuel).
2. **Atomicité par (package, window)** : un échec n'arrête pas les autres items. La métadonnée ne publie que ce qui existe vraiment dans R2.
3. **Token Bearer rafraîchi paresseusement** : `application_id` long-lived dans l'env, token court en mémoire process.

**Délai de publication** (`publication_delay`) : valeur initiale 3-4h, à affiner en regardant la latence réelle des runs. Confirmer en première semaine de prod.

## Error handling

`thiserror` côté lib (`MeteoFranceError`), `anyhow` côté binaire (calque du projet).

```rust
#[derive(Debug, thiserror::Error)]
pub enum MeteoFranceError {
    #[error("auth failed: {0}")]                 Auth(String),
    #[error("rate limited (Retry-After: {retry_after_s:?})")] RateLimited { retry_after_s: Option<u64> },
    #[error("http {status}: {body}")]            Http { status: u16, body: String },
    #[error("transport: {0}")]                   Transport(#[from] reqwest::Error),
    #[error("incomplete response: expected {expected} bytes, got {got}")] Incomplete { expected: u64, got: u64 },
}
```

**Politique de retry (dans `AromeOmClient`) :**

| Code/situation | Action |
|---|---|
| `200` | OK |
| `206` (partial) | concat + continuer (Range resume) |
| `401` | refresh token une fois, rejouer ; sinon `Auth` |
| `429` | sleep(`Retry-After` ou 30s) puis retry, max 3× |
| `5xx`, timeout | backoff expo (1s, 4s, 16s), max 3× |
| autre 4xx | échec dur immédiat (pas de retry) |

**Politique au niveau `main` :**

- **Hard fails** (exit 2, abort) : R2 init KO, MF auth setup KO, `work_dir` non créable.
- **Soft fails** (compteur, continue) : échec d'un item (package, window) après retry ; échec d'un OMfile parmi N ; échec d'un upload individuel.
- **Exit code final** : `0` si `failures == 0` ET ≥1 OMfile écrit ; `1` sinon ; `2` sur hard fail.
- **Métadonnée mise à jour à la fin** dès que la boucle s'est exécutée (i.e. après soft fails inclus) — elle reflète l'état R2 réel. Sur hard fail (abort prématuré), elle n'est pas régénérée ; les fichiers déjà uploadés restent annoncés par la métadonnée du run précédent, donc pas d'incohérence visible côté client.

## Testing

Aligné sur la doctrine projet : tests unitaires `#[cfg(test)]` inline, pas de mock HTTP, validation finale par `cargo run --skip-upload` + inspection.

**Unitaires (no-network) :**

| Module | Tests |
|---|---|
| `grid::ReunionGrid` | dimensions, `lonlat_to_indices` roundtrip sur 4 coins + centre, refus hors bbox |
| `meteofrance_api::url` | `build_product_url(model, grid, package, run, window)` pure fn — format `referencetime`, format `time` (`00H06H`), URL-encoding |
| `meteofrance_api::retry` | `classify(status, headers) -> Action` testée sur status codes synthétiques |
| `arome_om_metadata` | comme `anomaly_metadata` : `parse_key_*`, dédup/tri, shape JSON |
| `variables` | lookup registry round-trip (grib_short ↔ om_name), couverture exhaustive |
| `planning::build_plan` | nombre d'items pour horizon donné, dernière fenêtre couvre horizon, pas de chevauchement |

Le code est délibérément structuré pour rendre testable **sans réseau** : URL building, retry classification, planning sont des fonctions pures séparées de l'HTTP client.

**Intégration (avec fixture) :**

- 1 fixture GRIB2 minimale (≤200 KB, 1 variable × 1 leadtime × ReunionGrid) checkée dans `crates/arome-om-forecast/tests/fixtures/`. Capturée manuellement depuis l'API une fois.
- `tests/grib_decoder_roundtrip.rs` : lance Python helper, vérifie dimensions + 3 valeurs aux indices connus (sanity orientation NE/SW).
- `#[ignore]` par défaut — lancé en local via `cargo test -- --ignored`. CI n'a pas eccodes système installé pour l'instant.

**CI** : aucun changement structurel à `ci.yml`. Job actuel (`clippy --all-targets --all-features --locked -D warnings` + `cargo test --workspace`) couvre les unitaires.

**Validation manuelle (avant merge) :**

1. `cargo run --release -p arome-om-forecast -- --territory reunion --packages SP1 --horizon-h 6 --skip-upload --work-dir /tmp/aom` → produit ~12 OMfiles locaux.
2. Inspection : dimensions des OMfiles == `ReunionGrid`, valeur plausible sur un pixel connu (ex. T2m à Saint-Denis).
3. Run complet (42h, tous packages) avec upload → vérifier layout R2 + cohérence `latest.json`.
4. Côté `maps/`, enregistrer le domaine `arome_om_reunion` + ses variables, vérifier la timeline et le rendu. **Cette étape touche un repo voisin et sera un step à part dans le plan d'implémentation.**

## Suites possibles (hors scope de ce spec)

- Autres territoires AROME-OM : Antilles, Guyane, Nouvelle-Calédonie, Polynésie. Chaque ajout = 1 `Grid` + 1 entrée dans l'enum `AromeOmTerritory`.
- Packages additionnels IP*/HP* (niveaux pression, niveaux isobariques) si demandes utilisateur le justifient.
- Refacto émergent : si on ajoute un 3e provider de forecast (au-delà d'Open-Meteo et MF), on extrait à ce moment-là un trait `ForecastSource` — pas avant.
- Pipeline radar Météo-France : réutilisera `meteofrance_api::MeteoFranceAuth`.
- Anomalies en DOM-TOM : nécessite climato tropicale 30 ans par territoire — projet à part entière.

## Pré-requis opérationnels

- **Compte Météo-France API** + `application_id` (long-lived) → env var `MF_APPLICATION_ID`. Inscription sur https://portail-api.meteofrance.fr/ requise avant la première PR.
- `python3` du PATH avec `cfgrib`, `xarray`, `netCDF4` installés (extension du `venv` projet déjà utilisé pour CDS). `eccodes` système requis (`apt install libeccodes0` sur Debian-likes).
- Bucket R2 existant (déjà configuré pour temperature-anomaly), nouveau préfixe `data_spatial/arome_om_reunion/`.

## Valeurs résolues à Task 0 (probe API du 2026-05-28)

Les 5 inconnues sont maintenant levées. Plusieurs surprises significatives par rapport aux hypothèses initiales :

| # | Hypothèse initiale | Valeur réelle | Impact |
|---|---|---|---|
| 1 | Namespace `DPPaquetAROME-OM` | ✅ confirmé | aucun |
| 1 | Endpoint produit `productARO` | ❌ **`productOMOI`** | renomme l'URL builder + appels |
| 1 | Model id `AROME-INDIEN` | ❌ **`AROME-OM-INDIEN`** | placeholder à corriger |
| 1 | Format `time` = fenêtres 6h (`00H06H`) | ❌ **Leadtimes 1h (`001H`, `002H`, … `048H`)** | refacto `TimeWindow` → `Leadtime` |
| 2 | Grille ~201×161, lon 53-58, lat -23 à -19 | ❌ **1395×899, lon 32.75-67.6, lat -25.9 à -3.45** | grille 40× plus grande (océan Indien entier) |
| 3 | Cadence 4×/j ou 8×/j | ✅ **4×/j (00/06/12/18 UTC)** | cron CI ajusté |
| 4 | Horizon 42-78h | ✅ **48h** (000H à 048H inclus) | default `--horizon-h = 48` |
| 5 | Latence publication 3-4h | ❌ **~6h** (run 06Z → premier fichier 12:27Z) | `PUBLICATION_DELAY_H = 6`, `floor_6h(now - delay)` |

**Conséquence sur la grille :** « ReunionGrid » est un nom trompeur — la grille AROME-OM Océan Indien couvre **Réunion + Mayotte + grand morceau d'océan Indien** (Madagascar, sud de l'Inde, côte est-africaine). Le nom interne reste `ReunionGrid` (user-facing produit), la doc-comment du type explique le périmètre réel.

**Inventaire variables SP1/SP2/SP3 réel** (`grib_ls` sur fichiers 001H du run 2026-05-28T06:00Z) :

- **SP1** (15 messages) : `10wdir, 10si, max_i10fg, prmsl, 10u, 10v, max_10efg, max_10nfg, 2t, 2r, tp, unknown, tsnowp, ssrd, tgrp`
- **SP2** (12 messages) : `2d, 2sh, sp, t (surface), lcc, hcc, mcc, CAPE_INS, blh, tirf, min_2t, max_2t`
- **SP3** (13 messages) : `slhf, sshf, strd, ssr, str, ssrc, strc, iews, inss` (+ 4 `unknown`)

Le registry MVP retenu (12 vars) :
- SP1 (8) : `2t, 2r, 10u, 10v, 10si, max_i10fg, prmsl, tp`
- SP2 (4) : `2d, lcc, mcc, hcc`

(Pas de SP3 au MVP : flux énergétiques peu pertinents pour une carte grand public ; à ajouter au cas par cas si demande utilisateur.)

---

**Post-Task-0 update :** the R2 layout was corrected to match Open-Meteo's `data_spatial` convention — one multi-variable OMfile per leadtime, named `{ISO_valid_time}.om` (e.g. `2026-05-29T0000.om`), with variables as children of root. Replaces the initial design's `{variable}_{leadtime}h.om` (one file per variable).

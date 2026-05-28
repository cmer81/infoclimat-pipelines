# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> Documentation opérationnelle complète (quickstart, déploiement, secrets, build climato 30 ans) : voir `README.md`. Ce fichier se concentre sur l'architecture et les pièges utiles pour modifier le code.

## Vue d'ensemble

Workspace Rust (édition 2024, rust-version 1.85) de **pipelines batch** qui précalculent deux familles de produits météo et publient des OMfiles spatiaux dans un bucket Cloudflare R2 :

1. **Anomalies de température** ARPEGE Europe : `température(jour J) − normale_climatique(jour-de-l'année)`, sur la grille ARPEGE Europe (0.1°, 521×741, °C).
2. **Prévisions brutes AROME-OM Réunion-Mayotte** : 11 variables surface (T2m, RH, vent u/v, rafales, MSLP, précip, point de rosée, nuages bas/moyen/haut) + variable dérivée `precipitation_sum` (cumul depuis le début du run), grille AROME-OM-INDIEN (0.025°, 1395×899), horizon 48 h par pas horaire.

Le client SvelteKit `maps/` (repo voisin, voir `../CLAUDE.md`) consomme les deux directement via accès public R2, sans auth.

## Architecture du workspace

Un crate partagé + quatre binaires CLI :

```
                            ┌─ climatology (one-shot)  ← ERA5 NetCDF (CDS) ─► 366 normales DOY
ERA5/ERA5T (CDS) ─► observed (cron 03h)        ─┐
ARPEGE Europe (Open-Meteo) ─► forecast (cron 4×/j) ─┴─► OMfiles anomalie + meta JSON ─► R2
AROME-OM (MF API GRIB2) ─► arome-om-forecast (cron 4×/j) ─► OMfiles multi-var + meta JSON ─► R2
                                                                                              ↓
                                                                                           maps/
```

- **`crates/core`** (`pipeline-core`) — tout le code réutilisable. Modules :
  - `accumulation` — `deaccumulate_with_nan` (dé-cumul d'un champ accumulé → pas horaire, clampé ≥ 0, NaN propagé).
  - `grid` — grilles `ArpegeEuropeGrid` + `ReunionGrid` (2 impl du trait `Grid`), bbox ERA5.
  - `regrid` — bilinéaire ERA5 → ARPEGE.
  - `climatology` — cache des 366 normales + `day_of_year_index`.
  - `anomaly` — `subtract_with_nan`.
  - `omfile_io` — `write_spatial_omfile` (single-var, anomaly) et `write_multi_variable_omfile` (multi-var, arome-om), plus `read_spatial_omfile`.
  - `r2` — client S3/R2 minimal.
  - `anomaly_metadata` — JSON métadonnées du domaine `anomaly_europe`.
  - `arome_om_metadata` — JSON métadonnées du domaine `arome_om_reunion`.
  - `meteofrance_api` — auth OAuth2 (token cache + refresh), `AromeOmClient.fetch_package`, retry classification, URL builder. Réutilisable pour un futur pipeline radar MF.
  - `logging`.
- **`temperature-anomaly-climatology`** — one-shot `workflow_dispatch`. Lit du NetCDF ERA5, construit les 366 normales lissées 15 j. Seul crate qui dépend de `netcdf`.
- **`temperature-anomaly-observed`** — cron quotidien. Re-télécharge `--refresh-days` jours via CDS (appelle `scripts/download_era5.py`), GC au-delà de `--days-back`.
- **`temperature-anomaly-forecast`** — cron 4×/jour. Fetch les OMfiles horaires ARPEGE d'Open-Meteo, moyenne journalière, pas de CDS, pas de GC (fichiers écrasés en place).
- **`arome-om-forecast`** — cron 4×/jour (`0 1,7,13,19 * * *` UTC). Auth OAuth2 contre `portail-api.meteofrance.fr`, fetch GRIB2 multi-messages (49 leadtimes × 2 packages = 98 fichiers par run), décodage via helper Python (`scripts/decode_arome_om_grib.py` → cfgrib/xarray), regroupement par leadtime, écriture d'**un OMfile multi-variables par timestep**, **incluant la variable dérivée `precipitation_sum`** (cumul de précip depuis le début du run, NaN propagé), upload R2, GC des runs > `keep_runs_back × 6h`.

Les binaires sont fins : l'orchestration vit dans `main.rs` (chacun a un doc-comment d'entête décrivant ses étapes), la logique réutilisable dans `core`. Pour ajouter un pipeline (ex. précipitations, ou autres territoires DOM-TOM), réutiliser `core` et calquer un binaire existant.

## Commandes

```bash
cargo test --workspace                 # 68 tests (le filet de sécurité principal)
cargo test -p pipeline-core grid       # un crate / un test précis (filtre par nom)
cargo build --release -p temperature-anomaly-forecast
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Lancer un pipeline en local (toujours `--skip-upload` pour ne pas écrire en R2 ; voir README pour les jeux d'arguments complets) :

```bash
cargo run --release -p temperature-anomaly-forecast -- \
  --days-ahead 4 --climato-dir climato_out --work-dir work \
  --r2-anomaly-prefix anomaly/temperature_2m/forecast --skip-upload
```

**venv requis pour climato/observed/arome-om** : ces pipelines lancent `python3` du PATH (pour le download CDS et/ou le décodeur GRIB2). `pip install -r scripts/requirements.txt` couvre tout (`cdsapi`, `cfgrib`, `xarray`, `netCDF4`, `numpy`). Activer `source venv/bin/activate` avant. Le `temperature-anomaly-forecast` est le seul à ne pas en avoir besoin (fetch HTTP direct, pas de décodage GRIB).

**Pré-requis système supplémentaires pour `arome-om-forecast`** : `libeccodes0` + `libeccodes-tools` (`sudo apt install libeccodes0 libeccodes-tools` sur Debian-likes). C'est ce que `cfgrib` charge en backend pour décoder les GRIB2 Météo-France.

## CI (GitHub Actions)

Un workflow par binaire dans `.github/workflows/` (4 binaires = 4 workflows en plus du `ci.yml` gate). Patterns :
- *observed/forecast (anomaly)* : build release → **`aws s3 cp` de la climato depuis R2** vers `climato/` → run du binaire.
- *arome-om-forecast* : `apt install libeccodes0 libeccodes-tools` → `setup-python` → `pip install cfgrib xarray netCDF4` → build release → run.
- La climato n'est jamais reconstruite en CI (le workflow climato dépasse la limite 6 h des runners — build initial fait en local).

Secrets attendus :
- `CDS_API_KEY` — Copernicus CDS (climato + observed).
- `MF_APPLICATION_ID` — Météo-France portail-api (arome-om-forecast). Long-lived, format base64(client_id:client_secret), passé en `Authorization: Basic` pour récupérer un bearer OAuth2.
- `R2_ACCOUNT_ID`, `R2_ACCESS_KEY`, `R2_SECRET_KEY`, `R2_BUCKET` — partagés par tous les pipelines.

## Pièges qui touchent le code (à respecter en éditant)

- **Unités** : Open-Meteo `temperature_2m` est déjà en **°C** → le forecast ne convertit PAS. ERA5 NetCDF est en **Kelvin** → climato/observed font K→°C. Ne pas ajouter de `-273.15` côté forecast (régression connue : ~9 °C → -264 °C).
- **R2 + checksums** : le SDK `aws-sdk-s3` 1.x active des trailers CRC32 que R2 rejette (`SignatureDoesNotMatch`). Le client R2 force `request_checksum_calculation(WhenRequired)` — ne pas retirer.
- **Format OMfile produit** : calqué sur les OMfiles Open-Meteo natifs. Deux variantes côte à côte :
  - *Single-var* (pipelines anomaly) — `write_spatial_omfile(path, variable_name, data, grid, meta)` : root vide → 1 child array nommé d'après `variable_name` (typiquement `temperature_2m_anomaly`) → child scalar `metadata` (JSON). Le nom est paramétré, plus hardcodé.
  - *Multi-var* (`arome-om-forecast`) — `write_multi_variable_omfile(path, variables: &[(name, &Array2, scale_factor)], grid, meta)` : root vide → N child arrays (un par variable) → le 1er array porte un child scalar `metadata` partagé. Le client maps/ fait `reader.getChildByName(variable)` pour récupérer la slice.
  - Compression `PforDelta2dInt16`. Le single-var (anomaly) utilise `scale_factor` 100.0 (valeurs petites). Le multi-var utilise un **`scale_factor` par variable** (`VariableEntry.scale_factor` + `variables::scale_factor_for`) : 100 pour T°/RH/vent/nuages, **20 pour `pressure_msl`** (~1013 hPa), **10 pour `precipitation`**, **5 pour `precipitation_sum`** (cumul). ⚠️ En `i16`, la valeur physique max = `32767 / scale_factor` — un facteur 100 plafonnerait à 327, ce qui mettait `pressure_msl` à 100 % NaN et tronquait les cumuls de précip. Ne pas remettre un facteur global.
- **NaN propagés volontairement** : un pixel source manquant → pixel anomalie NaN (`subtract_with_nan`, count==0). C'est voulu.
- **Délai ERA5T ~5 j** : trou attendu entre fin de l'observé et début de la prévision (anomaly) dans la timeline. Pas un bug.
- **Sélection du run forecast (anomaly)** : `floor_6h(now − 6h)` (Open-Meteo publie ~5-6 h après l'heure du run). GC forecast = supprimer les dates passées, sinon le client mal-route un J+0 périmé.
- **Sélection du run AROME-OM** : `floor_6h(now − 6h)` aussi (cadence 4×/j à 00/06/12/18 UTC, latence publication MF ~6 h). `PUBLICATION_DELAY_H = 6` dans `arome-om-forecast/src/main.rs`.
- **GC AROME-OM** : `gc_old_runs(cutoff = current_run − RUN_INTERVAL_H × keep_runs_back)` où `RUN_INTERVAL_H = 6` (cadence) et `keep_runs_back` par défaut 4 → on garde les 4 derniers runs (~24 h de fenêtre).
- **Métadonnées synthétiques (anomaly)** : `reference_time` (aujourd'hui 00Z) est factice — il n'existe pas de vrai « run » pour un produit d'anomalie ; `valid_times` = union observé+prévision. C'est pour satisfaire la machinerie du client `maps/`.
- **Endpoint Météo-France AROME-OM** : `https://public-api.meteofrance.fr/previnum/DPPaquetAROME-OM/v1/models/AROME-OM-INDIEN/grids/0.025/packages/{SP1,SP2,SP3}/productOMOI?referencetime={ISO}&time={NNN}H&format=grib2`. **Le product s'appelle `productOMOI` (Outre-Mer Océan Indien), pas `productARO` comme la métropole**. Le `time` param est un leadtime unique 3-digit (`001H`..`048H`), **pas une fenêtre 6 h** (`00H06H`) comme l'AROME métropole. À garder en tête si tu portes le code sur AROME France un jour.
- **Flip latitude N→S dans le décodeur Python** : les GRIB AROME-OM stockent les rows du nord vers le sud (`latitudeOfFirstGridPoint = -3.45°`, `latitudeOfLastGridPoint = -25.9°`) alors qu'Open-Meteo / le trait `Grid` attendent `row 0 = latMin` (sud). Le helper `scripts/decode_arome_om_grib.py` détecte l'orientation et flippe avec `sub.isel(latitude=slice(None, None, -1))`. Sans ce flip, le client maps/ lit la mauvaise zone (Saint-Denis à -21° pointait sur des pixels océaniques uniformes ~-8°S côté Comores → tout uniforme à ~27 °C). Ne pas retirer.
- **Layout R2 AROME-OM** : `data_spatial/arome_om_reunion/{Y}/{M}/{D}/{HHMM}Z/{YYYY-MM-DDTHHMM}.om` — un fichier multi-variables par leadtime, nommé par son valid_time. Le `parse_run_key` legacy (qui matchait `{var}_{leadtime}h.om`) a été remplacé par un parser qui lit le valid_time directement du nom de fichier via `chrono::NaiveDateTime::parse_from_str(stem, "%Y-%m-%dT%H%M")`.
- **Variables accumulées/intégrées, absentes à leadtime 0** : `max_i10fg`, `tp`, `ssrd`, `tsnowp`, `tgrp` sont des quantités **accumulées depuis H0 du run** (`tp` à l'échéance N = cumul `[H0, N]`) ou intégrées sur l'intervalle, et n'existent **pas** à `time=000H`. Le décodeur Python le détecte (`len(ds.data_vars) == 0`) et continue avec un log INFO ; ne pas reclassifier ça en erreur dure.
- **Précipitation AROME-OM (`precipitation` + `precipitation_sum`)** : ⚠️ le `tp` GRIB est **accumulé depuis le début du run** (PAS horaire). Le flux (`cumul::split_precipitation`) en dérive deux variables : `precipitation` (horaire, convention Open-Meteo) = `tp[N] − tp[N-1]` via `pipeline_core::accumulation::deaccumulate_with_nan` (clampé ≥ 0, le bruit d'arrondi `scale_factor` peut donner de petits négatifs), et `precipitation_sum` (cumul depuis le run) = `tp[N]` tel quel. Le dé-cumul a besoin du `tp` de l'échéance précédente → le flux décode les leadtimes en parallèle mais les **traite dans l'ordre croissant** (`buffered`, pas `buffer_unordered`). H0 = 0 pour les deux variables (tp absent). NaN propagé. Ne pas réintroduire `buffer_unordered` — ça casserait le dé-cumul.
- **`netcdf` feature `static`** : compile HDF5/libnetcdf depuis les sources (cmake requis, ~5 min au 1er build). Évite `libnetcdf-dev`/`libhdf5-dev` sur les machines sans HDF5 système. Utilisé par `temperature-anomaly-climatology` (lecture ERA5) et `arome-om-forecast` (lecture des NetCDF intermédiaires produits par cfgrib).

## Conventions

- **Tenir `README.md` et ce `CLAUDE.md` à jour en même temps que le code.** Toute modif qui change le contrat opérationnel doit être reflétée dans la doc dans la même PR (ou en suivi immédiat). Cas typiques à surveiller :
  - Ajout/suppression d'un pipeline, d'un crate, d'un module dans `pipeline-core`.
  - Changement de défaut CLI (horizon, packages, concurrency, cadence cron…).
  - Nouveau secret CI, nouvelle var d'env, nouveau pré-requis système ou Python.
  - Changement de layout R2 (préfixe, nom de fichier, format OMfile).
  - Ajout/changement d'un endpoint API externe (URL, auth, params).
  - Tout nouveau « piège » découvert en debug — il a sa place dans la section *Pièges connus* du CLAUDE.md.
  Pas besoin de mettre à jour la doc pour des refactos internes, fix de typo, ou ajustements purement de code qui ne changent rien côté utilisateur/opérateur.
- Suivre le skill **`rust-best-practices`** (installé dans `.agents/skills/`, symlink `.claude/skills/`) : pas de `unwrap()`/`expect()` hors tests, `thiserror` pour les libs / `anyhow` pour les binaires, `&str`/`&[T]` en paramètres, itérateurs plutôt que boucles manuelles.
- `.claude/settings.local.json` est gitignoré (permissions locales perso) — ne pas le commiter.

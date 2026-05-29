# Architecture — pipelines batch météo

Règle **transverse** (toujours chargée) : squelette mental du workspace. Pour l'opérationnel complet (quickstart, déploiement, secrets, build climato 30 ans) voir `README.md`.

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

## Piège build transverse — `netcdf` feature `static`

Compile HDF5/libnetcdf depuis les sources (cmake requis, ~5 min au 1er build). Évite `libnetcdf-dev`/`libhdf5-dev` sur les machines sans HDF5 système. Utilisé par `temperature-anomaly-climatology` (lecture ERA5) et `arome-om-forecast` (lecture des NetCDF intermédiaires produits par cfgrib).

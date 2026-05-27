# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> Documentation opérationnelle complète (quickstart, déploiement, secrets, build climato 30 ans) : voir `README.md`. Ce fichier se concentre sur l'architecture et les pièges utiles pour modifier le code.

## Vue d'ensemble

Workspace Rust (édition 2024, rust-version 1.85) de **pipelines batch** qui précalculent des **anomalies de température** et publient des OMfiles spatiaux dans un bucket Cloudflare R2. Le client SvelteKit `maps/` (repo voisin, voir `../CLAUDE.md`) les consomme directement via accès public, sans auth.

Une anomalie = `température(jour J) − normale_climatique(jour-de-l'année)`, sur la grille ARPEGE Europe (0.1°, 521×741, °C).

## Architecture du workspace

Un crate partagé + trois binaires CLI, chacun un étage du même flux de données :

```
                          ┌─ climatology (one-shot)  ← ERA5 NetCDF (CDS) ─► 366 normales DOY
ERA5/ERA5T (CDS) ─► observed (cron 03h)  ─┐
ARPEGE Europe (Open-Meteo) ─► forecast (cron 4×/j) ─┴─► OMfiles anomalie + meta JSON ─► R2 ─► maps/
```

- **`crates/core`** (`pipeline-core`) — tout le code réutilisable, partagé par les trois binaires. Modules : `grid` (grille ARPEGE Europe + bbox), `regrid` (bilinéaire ERA5→ARPEGE), `climatology` (cache des 366 normales + `day_of_year_index`), `anomaly` (`subtract_with_nan`), `omfile_io` (lecture/écriture OMfile spatial), `r2` (client S3/R2), `anomaly_metadata` (génération des JSON lus par le client), `logging`.
- **`temperature-anomaly-climatology`** — one-shot `workflow_dispatch`. Lit du NetCDF ERA5 (modules `build` + `netcdf` propres au crate), construit les 366 normales lissées 15 j. Seul crate qui dépend de `netcdf`.
- **`temperature-anomaly-observed`** — cron quotidien. Re-télécharge `--refresh-days` jours via CDS (appelle `scripts/download_era5.py`), GC au-delà de `--days-back`.
- **`temperature-anomaly-forecast`** — cron 4×/jour. Fetch les OMfiles horaires ARPEGE d'Open-Meteo, moyenne journalière, pas de CDS, pas de GC (fichiers écrasés en place).

Les binaires sont fins : l'orchestration vit dans `main.rs` (chacun a un doc-comment d'entête décrivant ses étapes), la logique réutilisable dans `core`. Pour ajouter un pipeline (ex. précipitations), réutiliser `core` et calquer un binaire existant.

## Commandes

```bash
cargo test --workspace                 # 33 tests (le filet de sécurité principal)
cargo test -p pipeline-core grid       # un crate / un test précis (filtre par nom)
cargo build --release -p temperature-anomaly-forecast
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Lancer un pipeline en local (toujours `--skip-upload` pour ne pas écrire en R2 ; voir README pour les jeux d'arguments complets) :

```bash
cargo run --release -p temperature-anomaly-forecast -- \
  --days-ahead 4 --climato-dir climato_out --work-dir work \
  --r2-anomaly-prefix anomaly/temperature_2m/forecast --skip-upload
```

**venv requis pour climato/observed** : ces pipelines lancent `python3` du PATH pour le download CDS (`cdsapi` n'est pas dans le python système). Activer `source venv/bin/activate` avant. Le forecast n'en a pas besoin (fetch HTTP direct).

## CI (GitHub Actions)

Un workflow par binaire dans `.github/workflows/`. Pattern observed/forecast : build release → **`aws s3 cp` de la climato depuis R2** vers `climato/` → run du binaire. La climato n'est jamais reconstruite en CI (le workflow climato dépasse la limite 6 h des runners — build initial fait en local). Secrets attendus : `CDS_API_KEY`, `R2_ACCOUNT_ID`, `R2_ACCESS_KEY`, `R2_SECRET_KEY`, `R2_BUCKET`.

## Pièges qui touchent le code (à respecter en éditant)

- **Unités** : Open-Meteo `temperature_2m` est déjà en **°C** → le forecast ne convertit PAS. ERA5 NetCDF est en **Kelvin** → climato/observed font K→°C. Ne pas ajouter de `-273.15` côté forecast (régression connue : ~9 °C → -264 °C).
- **R2 + checksums** : le SDK `aws-sdk-s3` 1.x active des trailers CRC32 que R2 rejette (`SignatureDoesNotMatch`). Le client R2 force `request_checksum_calculation(WhenRequired)` — ne pas retirer.
- **Format OMfile produit** : calqué sur les OMfiles Open-Meteo natifs — root vide → child array `temperature_2m_anomaly` (f32, `[ny=521, nx=741]`) → child scalar `metadata` (JSON). Compression `PforDelta2dInt16`, `scale_factor` 100.0.
- **NaN propagés volontairement** : un pixel source manquant → pixel anomalie NaN (`subtract_with_nan`, count==0). C'est voulu.
- **Délai ERA5T ~5 j** : trou attendu entre fin de l'observé et début de la prévision dans la timeline. Pas un bug.
- **Sélection du run forecast** : `floor_6h(now − 6h)` (Open-Meteo publie ~5-6 h après l'heure du run). GC forecast = supprimer les dates passées, sinon le client mal-route un J+0 périmé.
- **Métadonnées synthétiques** : `reference_time` (aujourd'hui 00Z) est factice — il n'existe pas de vrai « run » pour un produit d'anomalie ; `valid_times` = union observé+prévision. C'est pour satisfaire la machinerie du client `maps/`.
- **`netcdf` feature `static`** : compile HDF5/libnetcdf depuis les sources (cmake requis, ~5 min au 1er build). Évite `libnetcdf-dev`/`libhdf5-dev` sur les machines sans HDF5 système.

## Conventions

- Suivre le skill **`rust-best-practices`** (installé dans `.agents/skills/`, symlink `.claude/skills/`) : pas de `unwrap()`/`expect()` hors tests, `thiserror` pour les libs / `anyhow` pour les binaires, `&str`/`&[T]` en paramètres, itérateurs plutôt que boucles manuelles.
- `.claude/settings.local.json` est gitignoré (permissions locales perso) — ne pas le commiter.

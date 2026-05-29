# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Workspace Rust (édition 2024) de **pipelines batch météo** : un crate partagé `pipeline-core` + 4 binaires CLI qui publient des OMfiles spatiaux dans Cloudflare R2 (anomalies de température ARPEGE Europe, et prévisions brutes AROME-OM Réunion-Mayotte). Le client SvelteKit voisin `maps/` (`../CLAUDE.md`) les consomme via accès public R2.

> **Documentation opérationnelle complète** (quickstart, déploiement, secrets, build climato 30 ans) : voir `README.md`.

## Règles ciblées — `.claude/rules/`

Le détail (architecture, pièges, conventions) vit dans `.claude/rules/`, découpé par domaine et **scopé par `paths:`** : chaque fichier ne se charge que quand on touche les fichiers concernés, pour garder ce contexte léger.

| Règle | Chargement |
|---|---|
| `architecture.md` | toujours — vue d'ensemble, crates/binaires, piège build `netcdf static` |
| `doc-and-conventions.md` | toujours — tenir la doc à jour, `settings.local.json`, index des règles |
| `rust-conventions.md` | `**/*.rs` — skill `rust-best-practices`, idiomes |
| `omfile-and-r2.md` | `crates/core/src/omfile_io.rs`, `r2.rs` |
| `arome-om-forecast.md` | `crates/arome-om-forecast/**`, `meteofrance_api.rs`, `accumulation.rs`, `decode_arome_om_grib.py`… |
| `temperature-anomaly.md` | `crates/temperature-anomaly-*/**`, `anomaly*.rs`, `climatology.rs`, `regrid.rs` |
| `python-scripts.md` | `scripts/**/*.py` |
| `ci-workflows.md` | `.github/workflows/**` |

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

> Les pré-requis venv/cfgrib/eccodes (climato/observed/arome-om) sont décrits dans `.claude/rules/python-scripts.md`.

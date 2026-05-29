# Conventions doc & repo (transverse)

- **Tenir `README.md` et la doc `.claude/` à jour en même temps que le code.** Toute modif qui change le contrat opérationnel doit être reflétée dans la doc dans la même PR (ou en suivi immédiat). Cas typiques à surveiller :
  - Ajout/suppression d'un pipeline, d'un crate, d'un module dans `pipeline-core` → mettre à jour `.claude/rules/architecture.md`.
  - Changement de défaut CLI (horizon, packages, concurrency, cadence cron…).
  - Nouveau secret CI, nouvelle var d'env, nouveau pré-requis système ou Python.
  - Changement de layout R2 (préfixe, nom de fichier, format OMfile).
  - Ajout/changement d'un endpoint API externe (URL, auth, params).
  - Tout nouveau « piège » découvert en debug → l'ajouter dans le fichier `.claude/rules/` ciblé correspondant (ou en créer un nouveau avec le bon `paths:`).
  Pas besoin de mettre à jour la doc pour des refactos internes, fix de typo, ou ajustements purement de code qui ne changent rien côté utilisateur/opérateur.
- `.claude/settings.local.json` est gitignoré (permissions locales perso) — ne pas le commiter.

## Organisation des règles `.claude/rules/`

Les règles spécifiques sont découpées par domaine et **scopées par `paths:`** (frontmatter YAML) : elles ne se chargent que quand on touche les fichiers correspondants, pour alléger le contexte par défaut.

| Fichier | Scope (`paths:`) |
|---|---|
| `architecture.md` | *toujours chargé* — vue d'ensemble + crates/binaires + `netcdf static` |
| `doc-and-conventions.md` | *toujours chargé* — ce fichier |
| `rust-conventions.md` | `**/*.rs` |
| `omfile-and-r2.md` | `crates/core/src/omfile_io.rs`, `r2.rs` |
| `arome-om-forecast.md` | `crates/arome-om-forecast/**`, `meteofrance_api.rs`, `arome_om_metadata.rs`, `accumulation.rs`, `scripts/decode_arome_om_grib.py` |
| `temperature-anomaly.md` | `crates/temperature-anomaly-*/**`, `anomaly*.rs`, `climatology.rs`, `regrid.rs` |
| `python-scripts.md` | `scripts/**/*.py` |
| `ci-workflows.md` | `.github/workflows/**` |

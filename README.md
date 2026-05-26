# infoclimat-pipelines

Pipelines batch (Rust + Python + GitHub Actions) qui produisent dans Cloudflare
R2 des données précalculées consommées par le client `maps/`.

Conçu pour héberger plusieurs pipelines à terme, qui partagent le crate `core/`
(R2 client, OMfile io, regridding, grille, logging).

## Pipelines disponibles

### temperature-anomaly

Anomalie journalière de température 2m sur la grille ARPEGE Europe (0.1°, 521×741),
référence climatique ERA5 1991–2020 lissée sur fenêtre glissante 15 jours.

Trois composants :

| Composant | Trigger | Sortie R2 |
|---|---|---|
| `temperature-anomaly-climatology` | `workflow_dispatch` (manuel) | `climatology/temperature_2m/era5_1991-2020/arpege_europe/{doy:03}.om` |
| `temperature-anomaly-observed` | cron 03h UTC | `anomaly/temperature_2m/observed/{YYYY-MM-DD}.om` |
| `temperature-anomaly-forecast` | cron 03/09/15/21h UTC | `anomaly/temperature_2m/forecast/{YYYY-MM-DD}.om` |

Spec design complète : `../docs/superpowers/specs/2026-05-26-anomalies-temperature-design.md`
Plan d'implémentation : `../docs/superpowers/plans/2026-05-26-anomalies-temperature-pipeline.md`

## Structure

```
infoclimat-pipelines/
├── Cargo.toml                                # workspace
├── crates/
│   ├── core/                                 # pipeline-core : grid, regrid, omfile_io, r2, climatology
│   ├── temperature-anomaly-climatology/      # CLI one-shot
│   ├── temperature-anomaly-observed/         # CLI cron quotidien
│   └── temperature-anomaly-forecast/         # CLI cron 4×/jour
├── scripts/
│   ├── download_era5.py                      # client cdsapi (CDS Copernicus)
│   └── requirements.txt
└── .github/workflows/
    ├── temperature-anomaly-climatology.yml
    ├── temperature-anomaly-observed.yml
    └── temperature-anomaly-forecast.yml
```

## Quickstart local

```bash
cp .env.example .env
# remplir CDS_API_KEY + R2_*

cargo test --workspace        # 28 tests doivent passer
```

### Build de climatologie (1 an, test rapide)

```bash
mkdir -p era5_input
python3 -m venv venv && source venv/bin/activate
pip install -r scripts/requirements.txt

CDS_API_KEY=<key> python scripts/download_era5.py --year 2020 \
  --bbox-north 73.0 --bbox-west -33.0 --bbox-south 19.0 --bbox-east 43.0 \
  --output era5_input/era5_2m_temperature_2020.nc

cargo run --release -p temperature-anomaly-climatology -- \
  --input-dir era5_input \
  --output-dir climato_out \
  --r2-prefix climatology/temperature_2m/test/arpege_europe \
  --year-start 2020 --year-end 2020 \
  --skip-upload   # retire ce flag pour pousser vers R2
```

Produit 366 fichiers dans `climato_out/`.

### Observed 1 jour

```bash
cargo run --release -p temperature-anomaly-observed -- \
  --days-back 1 \
  --climato-dir climato_out \
  --work-dir work \
  --r2-anomaly-prefix anomaly/temperature_2m/test_observed \
  --download-script scripts/download_era5.py
```

### Forecast aujourd'hui

```bash
cargo run --release -p temperature-anomaly-forecast -- \
  --days-ahead 0 \
  --climato-dir climato_out \
  --work-dir work \
  --r2-anomaly-prefix anomaly/temperature_2m/test_forecast
```

## Secrets GitHub Actions requis

- `CDS_API_KEY` — Copernicus Climate Data Store ([cds.climate.copernicus.eu](https://cds.climate.copernicus.eu/)).
- `R2_ACCOUNT_ID` — ID compte Cloudflare.
- `R2_ACCESS_KEY` / `R2_SECRET_KEY` — token R2 (lecture+écriture sur le bucket).
- `R2_BUCKET` — nom du bucket (ex: `infoclimat-pipelines`).

## Notes techniques

### `netcdf` build avec `static` feature

Le workspace pin `netcdf = { features = ["static"] }` pour éviter d'avoir à
installer `libnetcdf-dev` + `libhdf5-dev` via apt. Conséquence : la première
compilation embarque cmake et compile HDF5/libnetcdf depuis les sources
(~5 min). Cache cargo de GitHub Actions évite la pénalité sur les runs
suivants.

Si tu préfères une compilation plus rapide localement, installe
`libnetcdf-dev libhdf5-dev` et retire `features = ["static"]` dans le workspace
`Cargo.toml`.

### Modèle de cron forecast

Les runs ARPEGE France sortent à 00/06/12/18Z. Open-Meteo publie les OMfiles
spatiaux ~5-6h après. Le cron est calé sur 03/09/15/21Z (6h après le run le
plus proche). Le code utilise `floor_6h(now - 6h)` pour identifier le run le
plus récent disponible.

### Format des OMfiles produits

Format calqué sur les OMfiles natifs Open-Meteo : root container vide → child
array `temperature_2m_anomaly` (f32, [ny=105, nx=180]) → child scalar
`metadata` (String JSON). Le file-reader JS d'Open-Meteo (utilisé par `maps/`)
navigue via `getChildByName`.

Compression : `PforDelta2dInt16`, scale_factor 100.0 (résolution 0.01 K,
range ±327.67 K — largement suffisant pour anomalies typiquement ±30 K).

## TODO court terme

- Smoke test end-to-end (Task 21 du plan, manuel — nécessite credentials R2 + CDS).
- Intégration côté `maps/` (pseudo-domaine `anomaly_france`, plan séparé à rédiger).
- Pin `omfiles` sur un tag/rev plutôt que `branch = "main"` une fois stabilisé.

## Roadmap

Le crate `core/` est conçu pour être réutilisable. Pipelines envisageables :

- `precipitation-anomaly-*` (même schéma, autre variable).
- Anomalies sigma (z-score) — nécessite de précalculer l'écart-type par DOY
  en plus de la moyenne.
- Élargissement Europe / Global (changer la grille de référence dans
  `core::grid`).

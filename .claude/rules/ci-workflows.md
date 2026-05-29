---
paths:
  - ".github/workflows/**"
---

# CI (GitHub Actions)

Un workflow par binaire dans `.github/workflows/` (4 binaires = 4 workflows en plus du `ci.yml` gate). Patterns :
- *observed/forecast (anomaly)* : build release → **`aws s3 cp` de la climato depuis R2** vers `climato/` → run du binaire.
- *arome-om-forecast* : `apt install libeccodes0 libeccodes-tools` → `setup-python` → `pip install cfgrib xarray netCDF4` → build release → run.
- La climato n'est jamais reconstruite en CI (le workflow climato dépasse la limite 6 h des runners — build initial fait en local).

Secrets attendus :
- `CDS_API_KEY` — Copernicus CDS (climato + observed).
- `MF_APPLICATION_ID` — Météo-France portail-api (arome-om-forecast). Long-lived, format base64(client_id:client_secret), passé en `Authorization: Basic` pour récupérer un bearer OAuth2.
- `R2_ACCOUNT_ID`, `R2_ACCESS_KEY`, `R2_SECRET_KEY`, `R2_BUCKET` — partagés par tous les pipelines.

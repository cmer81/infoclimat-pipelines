---
paths:
  - "scripts/**/*.py"
  - "scripts/requirements.txt"
---

# Helpers Python (download CDS, décodage GRIB2)

- **venv requis pour climato/observed/arome-om** : ces pipelines lancent `python3` du PATH (download CDS et/ou décodeur GRIB2). `pip install -r scripts/requirements.txt` couvre tout (`cdsapi`, `cfgrib`, `xarray`, `netCDF4`, `numpy`). Activer `source venv/bin/activate` avant. `temperature-anomaly-forecast` est le seul à ne pas en avoir besoin (fetch HTTP direct, pas de décodage GRIB).
- **Pré-requis système pour `arome-om-forecast`** : `libeccodes0` + `libeccodes-tools` (`sudo apt install libeccodes0 libeccodes-tools` sur Debian-likes). C'est ce que `cfgrib` charge en backend pour décoder les GRIB2 Météo-France.
- **`scripts/decode_arome_om_grib.py`** : voir aussi la règle AROME-OM (flip latitude N→S obligatoire, détection des variables accumulées absentes à H0).

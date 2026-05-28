# infoclimat-pipelines

Pipelines batch (Rust + Python + GitHub Actions) qui précalculent des données
météo et les publient dans un bucket Cloudflare R2, consommées par le client
`maps/` (Open-Meteo Maps).

Conçu pour héberger plusieurs pipelines à terme, partageant le crate `core/`
(client R2, lecture/écriture OMfile, regridding, grille, logging).

---

## Concepts — c'est quoi une anomalie de température ?

Une **anomalie** = écart entre la température d'un jour et la **normale
climatique** de ce jour. Positif = plus chaud que d'habitude, négatif = plus
froid.

```
anomalie(jour J) = température(J) − normale(jour-de-l'année de J)
```

- **La normale** (au sens OMM) = moyenne sur une période de référence de 30 ans.
  La période officielle en vigueur est **1991–2020**. Ici, pour chaque
  jour-de-l'année (1→366), on calcule la moyenne journalière de chaque année,
  puis la moyenne des 30 années, lissée sur une fenêtre glissante de 15 jours
  (pratique standard NOAA/Copernicus pour gommer le bruit jour-à-jour).
- **La température du jour** vient de deux sources selon qu'on regarde le passé
  ou le futur :
  - **Observé** (passé) : réanalyse **ERA5 / ERA5T** (Copernicus), via l'API CDS.
  - **Prévision** (futur) : modèle **ARPEGE Europe** de Météo-France, via les
    OMfiles spatiaux d'Open-Meteo.

Le tout sur la **grille ARPEGE Europe** (0.1°, 521 lignes × 741 colonnes,
lat 20→72°N, lon −32→42°E), en °C.

### Délai ERA5T (piège important)

ERA5T (la version préliminaire d'ERA5) a un **délai de ~5 jours**. Donc à un
instant donné, les jours J-1 à J-5 ne sont pas encore disponibles côté observé.
La timeline a donc un **trou de ~5 jours** entre la fin de l'observé et le début
de la prévision. C'est attendu, pas un bug.

### Flux de données

```
ERA5 (CDS) ──┐
             ├─► [pipelines Rust] ─► OMfiles d'anomalie ─► R2 ─► client maps/
ARPEGE (OM) ─┘                       + métadonnées JSON
```

---

## Pipelines

### temperature-anomaly

Trois composants, tous écrivant dans le même bucket R2 :

| Composant | Trigger | Rôle | Sortie R2 |
|---|---|---|---|
| `temperature-anomaly-climatology` | `workflow_dispatch` (manuel, one-shot) | Construit les 366 normales depuis 30 ans d'ERA5 | `climatology/temperature_2m/era5_1991-2020/arpege_europe/{doy:03}.om` |
| `temperature-anomaly-observed` | cron 03h UTC | Anomalies passées (ERA5/ERA5T) | `anomaly/temperature_2m/observed/{YYYY-MM-DD}.om` |
| `temperature-anomaly-forecast` | cron 03/09/15/21h UTC | Anomalies prévues (ARPEGE Europe) | `anomaly/temperature_2m/forecast/{YYYY-MM-DD}.om` |

Chaque run observed/forecast régénère aussi les **métadonnées** lues par le
client pour piloter son sélecteur de temps :

```
data_spatial/anomaly_europe/latest.json        # reference_time + valid_times + variables
data_spatial/anomaly_europe/in-progress.json
data_spatial/anomaly_europe/{run}/meta.json
```

`valid_times` = union des dates pour lesquelles un OMfile existe (observé +
prévision). `reference_time` est synthétique (aujourd'hui 00Z) — il n'existe pas
de vrai « run » pour un produit d'anomalie, c'est juste pour satisfaire la
machinerie du client.

Fenêtres :
- **observed** : `--refresh-days` (7) jours re-téléchargés par run, rétention
  `--days-back` (30). Les jours 8→30 persistent en R2 sans re-download ; la GC
  supprime > 30 j.
- **forecast** : J+0→J+4 réécrits à chaque run ; GC des fichiers dont la date est
  passée (sinon un J+0 d'hier traîne et est mal routé par le client).

### Pipeline `arome-om-forecast` (AROME-OM Réunion-Mayotte, prévision brute)

Publie sur R2 les OMfiles AROME-OM Réunion-Mayotte / Océan Indien (prévision brute,
**11 variables surface + `precipitation_sum` dérivée**, horizon **48 h** par pas horaire). Consommé tel quel par
le client `maps/` sous le domaine `arome_om_reunion`.

Couvre la grille AROME-OM-INDIEN native (**1395 × 899 à 0,025°**, ~2,7 km de
résolution), qui s'étend de Madagascar et la côte est-africaine jusqu'au sud de
l'Inde — pas juste les deux îles malgré le nom officiel.

#### Variables exposées (SP1 + SP2)

| Package | Variables (`om_name`) |
|---|---|
| SP1 (7) | `temperature_2m`, `relative_humidity_2m`, `wind_u_component_10m`, `wind_v_component_10m`, `wind_gusts_10m`, `pressure_msl`, `precipitation` |
| SP2 (4) | `dew_point_2m`, `cloud_cover_low`, `cloud_cover_mid`, `cloud_cover_high` |
| Dérivée | `precipitation_sum` : cumul de précipitation depuis le début du run, croissant à chaque échéance (H0 = 0, NaN propagé) |

SP3 est exclu du MVP (flux énergétiques de surface peu pertinents pour une carte
grand public). `parse_packages` rejette `SP3` au startup pour éviter une boucle
d'erreurs silencieuses.

#### Layout R2

Un **OMfile multi-variables par leadtime** (convention Open-Meteo `data_spatial`),
les variables sont attachées comme enfants du root et lues côté client via
`reader.getChildByName(variable)` :

```
data_spatial/arome_om_reunion/{Y}/{M}/{D}/{HHMM}Z/{YYYY-MM-DDTHHMM}.om
data_spatial/arome_om_reunion/latest.json
data_spatial/arome_om_reunion/in-progress.json
data_spatial/arome_om_reunion/{Y}/{M}/{D}/{HHMM}Z/meta.json
```

⚠️ Le GRIB AROME-OM stocke les rangées du **nord au sud** (`latitudeOfFirstGridPoint`
≈ -3.45°) alors qu'Open-Meteo attend `row 0 = latMin` (sud). Le décodeur Python
détecte l'orientation et flippe automatiquement — sans ça, le client lirait des
pixels océaniques uniformes au lieu de la variation piton/côtes (vu en debug :
Saint-Denis affichait ~27 °C uniforme au lieu de la vraie variation 7-31 °C
sur le domaine).

#### Pré-requis

- **Compte Météo-France API** : [portail-api.meteofrance.fr](https://portail-api.meteofrance.fr/) → créer une application, récupérer l'`application_id` long-lived → variable d'environnement `MF_APPLICATION_ID`.
- **Système (Debian-likes)** :

  ```bash
  sudo apt install libeccodes0 libeccodes-tools
  ```

- **Python venv** :

  ```bash
  source venv/bin/activate
  pip install -r scripts/requirements.txt   # cfgrib, xarray, netCDF4, numpy, cdsapi
  ```

  Le binaire appelle `python3` du PATH sur `scripts/decode_arome_om_grib.py` pour
  décoder le GRIB2 ; le venv doit donc être activé avant de lancer le binaire.
  `numpy` est utilisé pour normaliser l'axe latitude lors du flip nord→sud.

- Bucket R2 déjà configuré (cf. autres pipelines).

#### Lancement local

```bash
source venv/bin/activate
cargo run --release -p arome-om-forecast -- \
  --territory reunion \
  --packages SP1,SP2 \
  --horizon-h 48 \
  --work-dir work \
  --skip-upload
```

`--skip-upload` produit les OMfiles localement sans toucher R2. Compter ~30 s
pour un run complet (49 leadtimes × ~10 MB par OMfile multi-var) en
`--concurrency 4`.

#### Cron production

`.github/workflows/arome-om-forecast.yml`, 4 runs/jour à 01/07/13/19 UTC, aligné
sur la publication des runs AROME-OM (00/06/12/18 UTC + ~6 h de latence
publication MF).

#### Secrets GitHub Actions requis

| Secret | Description |
|---|---|
| `MF_APPLICATION_ID` | ID application Météo-France API (long-lived) |
| `R2_ACCOUNT_ID` | Partagé avec les autres pipelines |
| `R2_ACCESS_KEY` | Partagé avec les autres pipelines |
| `R2_SECRET_KEY` | Partagé avec les autres pipelines |
| `R2_BUCKET` | Partagé avec les autres pipelines |

---

## Structure

```
infoclimat-pipelines/
├── Cargo.toml                                # workspace
├── crates/
│   ├── core/                                 # pipeline-core : grid, regrid, omfile_io,
│   │                                         #   r2, climatology, anomaly_metadata, logging
│   ├── temperature-anomaly-climatology/      # CLI one-shot (lit NetCDF ERA5)
│   ├── temperature-anomaly-observed/         # CLI cron quotidien (CDS jour par jour)
│   ├── temperature-anomaly-forecast/         # CLI cron 4×/jour (Open-Meteo)
│   └── arome-om-forecast/                   # CLI cron 4×/jour (AROME-OM Réunion-Mayotte, MF API)
├── scripts/
│   ├── download_era5.py                      # client cdsapi (CDS Copernicus)
│   ├── decode_arome_om_grib.py               # décodeur GRIB2 AROME-OM (cfgrib)
│   ├── build_climatology.sh                  # build climato 30 ans (download + build)
│   └── requirements.txt
└── .github/workflows/
    ├── ci.yml                                # gate PR : clippy + tests
    ├── temperature-anomaly-climatology.yml   # workflow_dispatch
    ├── temperature-anomaly-observed.yml      # cron quotidien
    ├── temperature-anomaly-forecast.yml      # cron 4×/jour
    └── arome-om-forecast.yml                 # cron 4×/jour
```

---

## Quickstart local

```bash
cp .env.example .env
# remplir CDS_API_KEY + R2_*  (R2_BUCKET = infoclimat-modeles-data en prod)

cargo test --workspace        # 68 tests doivent passer
```

### Build de la climatologie

⚠️ Les téléchargements CDS lancés par le pipeline appellent `python3` du PATH.
En local il faut donc le venv activé (`cdsapi` n'est pas dans le python système) :

```bash
python3 -m venv venv && source venv/bin/activate
pip install -r scripts/requirements.txt
```

Build complet 30 ans (download des 30 NetCDF Europe ~26 GB, puis build + upload) :

```bash
nohup ./scripts/build_climatology.sh > climato_build.log 2>&1 &
tail -f climato_build.log
```

Ou test rapide sur 1 an, sans upload :

```bash
mkdir -p era5_input
python scripts/download_era5.py --year 2020 \
  --bbox-north 73.0 --bbox-west -33.0 --bbox-south 19.0 --bbox-east 43.0 \
  --output era5_input/era5_2m_temperature_2020.nc

cargo run --release -p temperature-anomaly-climatology -- \
  --input-dir era5_input --output-dir climato_out \
  --r2-prefix climatology/temperature_2m/test/arpege_europe \
  --year-start 2020 --year-end 2020 --skip-upload
```

Produit 366 fichiers (~180 KB chacun) dans `climato_out/`.

### Observed / Forecast (test local)

```bash
# Observed : --refresh-days = jours re-téléchargés, --days-back = rétention/GC.
# (venv activé requis pour CDS)
cargo run --release -p temperature-anomaly-observed -- \
  --refresh-days 7 --days-back 30 \
  --climato-dir climato_out --work-dir work \
  --r2-anomaly-prefix anomaly/temperature_2m/observed \
  --download-script scripts/download_era5.py

# Forecast (pas de CDS, fetch Open-Meteo direct) :
cargo run --release -p temperature-anomaly-forecast -- \
  --days-ahead 4 --climato-dir climato_out --work-dir work \
  --r2-anomaly-prefix anomaly/temperature_2m/forecast
```

Ajouter `--skip-upload` pour tester sans écrire dans R2.

---

## Déploiement (production)

### Pipelines — GitHub Actions

Crons automatiques une fois les **secrets** configurés sur le repo
(Settings → Secrets and variables → Actions) :

- `CDS_API_KEY` — clé Copernicus CDS ([cds.climate.copernicus.eu](https://cds.climate.copernicus.eu/)). Utilisé par `temperature-anomaly-observed`.
- `MF_APPLICATION_ID` — ID application Météo-France API long-lived ([portail-api.meteofrance.fr](https://portail-api.meteofrance.fr/)). Utilisé par `arome-om-forecast`.
- `R2_ACCOUNT_ID` — ID compte Cloudflare.
- `R2_ACCESS_KEY` / `R2_SECRET_KEY` — token R2 read+write (le secret fait **64 hex**).
- `R2_BUCKET` — `infoclimat-modeles-data`.

La climato est un **one-shot** : se lance à la main (`workflow_dispatch`), pas en
cron. ⚠️ Le workflow climato télécharge 30 ans séquentiellement → dépasse la
limite 6 h des runners GitHub ; le build initial a été fait en local (cf.
`build_climatology.sh`). À paralléliser avant tout re-run en CI.

### Client maps/ — variable runtime

Le client n'affiche le domaine « Anomalie T° (Europe) » que si l'URL **publique**
du bucket lui est fournie. En prod (conteneur Docker), c'est une variable
d'environnement **runtime** (pas build-time) :

```
MODELS_BUCKET_URL=https://pub-xxxx.r2.dev
```

L'entrypoint `maps/docker-entrypoint.d/40-runtime-env.sh` l'injecte dans
`runtime-config.js` au démarrage. Dans le stack Portainer (`compose.portainer.yml`),
elle est passée au service `maps`. Vérif : `https://<domaine>/runtime-config.js`
doit montrer `MODELS_BUCKET_URL: "https://pub-…"`.

Le bucket doit être en **accès public** (R2 → Settings → Public access, sous-domaine
`r2.dev` ou custom domain), car le client fetch les OMfiles directement, sans auth.

---

## Notes techniques (pièges connus)

- **Open-Meteo `temperature_2m` est en °C**, pas en Kelvin. Le forecast ne fait
  donc PAS de conversion ; la climato/observed lisent ERA5 NetCDF (en Kelvin) et
  convertissent K→°C. (Un `-273.15` en trop transformait ~9 °C en -264 °C.)
- **R2 rejette les checksums trailer CRC32** que le SDK aws-sdk-s3 1.x active par
  défaut (→ `SignatureDoesNotMatch`). Le client R2 force
  `request_checksum_calculation(WhenRequired)`.
- **Cache-control** : climato = `immutable` (jamais modifiée) ; anomalies
  observed/forecast = `max-age=900` (réécrites à chaque run).
- **`netcdf` feature `static`** : compile HDF5/libnetcdf depuis les sources
  (cmake requis, ~5 min au premier build, caché ensuite). Évite d'installer
  `libnetcdf-dev`/`libhdf5-dev`. Le crate forecast ne dépend pas de netcdf.
- **Format OMfile produit** : calqué sur les OMfiles natifs Open-Meteo.
  - *Pipelines anomalie* (`temperature-anomaly-*`) : un seul child array
    `temperature_2m_anomaly` (f32, `[ny=521, nx=741]`) → child scalar
    `metadata` (JSON). Le nom du child est passé en argument à
    `write_spatial_omfile`, plus hardcodé.
  - *Pipeline `arome-om-forecast`* : **N child arrays** (un par variable) sous
    le root, plus un child scalar `metadata` rattaché au premier array. Le
    client maps/ fait `reader.getChildByName(variable)` pour récupérer la
    variable sélectionnée. Utilise `write_multi_variable_omfile`.
  - Compression dans les deux cas : `PforDelta2dInt16`, `scale_factor` 100.0.
- **Run forecast** : `floor_6h(now − 6h)` choisit le dernier run ARPEGE
  supposément publié (Open-Meteo publie ~5-6 h après l'heure du run).

---

## Roadmap

Le crate `core/` est réutilisable pour d'autres pipelines :

- **Autres territoires DOM-TOM AROME-OM** : Antilles, Guyane, Nouvelle-Calédonie,
  Polynésie. ~30 min de code par territoire (probe API pour le `model_id`
  exact, lecture du header GRIB pour les dims, ajout d'une variante à l'enum
  `AromeOmTerritory` et d'une impl de `Grid`). Le `META_DOMAIN_PREFIX`
  hardcodé dans `arome_om_metadata` est à paramétrer en argument avant
  d'attaquer le 2e territoire.
- `precipitation-anomaly-*` (même schéma, autre variable).
- Anomalies normalisées (z-score) — nécessite de précalculer l'écart-type par
  jour-de-l'année en plus de la moyenne.
- Élargissement Global, ou variante haute-résolution AROME France (horizon plus
  court) — changer la grille de référence dans `core::grid`.
- Pipeline radar Météo-France : réutilisera `pipeline-core::meteofrance_api`
  (auth + retry + token cache).
- Pin `omfiles` sur un tag/rev plutôt que `branch = "main"`.
- Paralléliser le download du workflow climato (limite 6 h GitHub).

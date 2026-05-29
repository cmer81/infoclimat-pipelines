---
paths:
  - "crates/temperature-anomaly-climatology/**"
  - "crates/temperature-anomaly-observed/**"
  - "crates/temperature-anomaly-forecast/**"
  - "crates/core/src/anomaly.rs"
  - "crates/core/src/anomaly_metadata.rs"
  - "crates/core/src/climatology.rs"
  - "crates/core/src/regrid.rs"
---

# Pièges anomalies de température ARPEGE Europe (à respecter en éditant)

- **Unités** : Open-Meteo `temperature_2m` est déjà en **°C** → le forecast ne convertit PAS. ERA5 NetCDF est en **Kelvin** → climato/observed font K→°C. Ne pas ajouter de `-273.15` côté forecast (régression connue : ~9 °C → -264 °C).
- **NaN propagés volontairement** : un pixel source manquant → pixel anomalie NaN (`subtract_with_nan`, count==0). C'est voulu.
- **Délai ERA5T ~5 j** : trou attendu entre fin de l'observé et début de la prévision (anomaly) dans la timeline. Pas un bug.
- **Sélection du run forecast (anomaly)** : `floor_6h(now − 6h)` (Open-Meteo publie ~5-6 h après l'heure du run). GC forecast = supprimer les dates passées, sinon le client mal-route un J+0 périmé.
- **Métadonnées synthétiques (anomaly)** : `reference_time` (aujourd'hui 00Z) est factice — il n'existe pas de vrai « run » pour un produit d'anomalie ; `valid_times` = union observé+prévision. C'est pour satisfaire la machinerie du client `maps/`.

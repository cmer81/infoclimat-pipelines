---
paths:
  - "crates/core/src/omfile_io.rs"
  - "crates/core/src/r2.rs"
---

# Pièges OMfile & R2 (à respecter en éditant)

- **R2 + checksums** : le SDK `aws-sdk-s3` 1.x active des trailers CRC32 que R2 rejette (`SignatureDoesNotMatch`). Le client R2 force `request_checksum_calculation(WhenRequired)` — ne pas retirer.
- **Format OMfile produit** : calqué sur les OMfiles Open-Meteo natifs. Deux variantes côte à côte :
  - *Single-var* (pipelines anomaly) — `write_spatial_omfile(path, variable_name, data, grid, meta)` : root vide → 1 child array nommé d'après `variable_name` (typiquement `temperature_2m_anomaly`) → child scalar `metadata` (JSON). Le nom est paramétré, plus hardcodé.
  - *Multi-var* (`arome-om-forecast`) — `write_multi_variable_omfile(path, variables: &[(name, &Array2, scale_factor)], grid, meta)` : root vide → N child arrays (un par variable) → le 1er array porte un child scalar `metadata` partagé. Le client maps/ fait `reader.getChildByName(variable)` pour récupérer la slice.
  - Compression `PforDelta2dInt16`. Le single-var (anomaly) utilise `scale_factor` 100.0 (valeurs petites). Le multi-var utilise un **`scale_factor` par variable** (`VariableEntry.scale_factor` + `variables::scale_factor_for`) : 100 pour T°/RH/vent/nuages, **20 pour `pressure_msl`** (~1013 hPa), **10 pour `precipitation`**, **5 pour `precipitation_sum`** (cumul). ⚠️ En `i16`, la valeur physique max = `32767 / scale_factor` — un facteur 100 plafonnerait à 327, ce qui mettait `pressure_msl` à 100 % NaN et tronquait les cumuls de précip. Ne pas remettre un facteur global.

#!/usr/bin/env bash
#
# Construit la climatologie complète ERA5 1991-2020 sur la grille ARPEGE Europe
# et l'uploade sur R2. Conçu pour tourner en background local (pas de limite de
# temps comme GitHub Actions). Idempotent : saute les années déjà téléchargées.
#
# Usage :
#   cp .env.example .env   # remplir CDS_API_KEY + R2_*
#   nohup ./scripts/build_climatology.sh > climato_build.log 2>&1 &
#   tail -f climato_build.log
#
# Variables d'env (depuis .env ou l'environnement) :
#   CDS_API_KEY, R2_ACCOUNT_ID, R2_ACCESS_KEY, R2_SECRET_KEY, R2_BUCKET
#
# Override des années via args : ./scripts/build_climatology.sh 2011 2020

set -euo pipefail

YEAR_START="${1:-1991}"
YEAR_END="${2:-2020}"
INPUT_DIR="era5_input"
OUTPUT_DIR="climato_out"
R2_PREFIX="climatology/temperature_2m/era5_${YEAR_START}-${YEAR_END}/arpege_europe"

# bbox Europe (couvre la grille ARPEGE Europe avec marge ~1°)
BBOX_NORTH=73.0
BBOX_WEST=-33.0
BBOX_SOUTH=19.0
BBOX_EAST=43.0

# Charge .env si présent
if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

if [ -z "${CDS_API_KEY:-}" ]; then
  echo "ERROR: CDS_API_KEY non défini (mettre dans .env ou l'env)" >&2
  exit 1
fi

mkdir -p "$INPUT_DIR"

echo "=== Phase 1/2 : téléchargement ERA5 ${YEAR_START}-${YEAR_END} ==="
for year in $(seq "$YEAR_START" "$YEAR_END"); do
  out="$INPUT_DIR/era5_2m_temperature_${year}.nc"
  if [ -s "$out" ]; then
    echo "[$year] déjà présent ($(du -h "$out" | cut -f1)), skip"
    continue
  fi
  echo "[$year] téléchargement…"
  python scripts/download_era5.py \
    --year "$year" \
    --bbox-north "$BBOX_NORTH" --bbox-west "$BBOX_WEST" \
    --bbox-south "$BBOX_SOUTH" --bbox-east "$BBOX_EAST" \
    --output "$out"
done

echo "=== Phase 2/2 : build climatologie + upload R2 ==="
cargo run --release -p temperature-anomaly-climatology -- \
  --input-dir "$INPUT_DIR" \
  --output-dir "$OUTPUT_DIR" \
  --r2-prefix "$R2_PREFIX" \
  --year-start "$YEAR_START" \
  --year-end "$YEAR_END"

echo "=== Terminé. 366 OMfiles dans $OUTPUT_DIR/ et sur R2 sous $R2_PREFIX/ ==="

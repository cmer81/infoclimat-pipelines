#!/usr/bin/env bash
#
# Télécharge les NetCDF ERA5 annuels avec un parallélisme borné.
#
# Usage:
#   ./scripts/download_era5_parallel.sh 1991 2020 3
#
# Les fichiers déjà présents et non vides sont sautés. Chaque téléchargement
# passe par un fichier temporaire, puis est renommé seulement en cas de succès.

set -euo pipefail

YEAR_START="${1:-1991}"
YEAR_END="${2:-2020}"
JOBS="${3:-3}"

INPUT_DIR="era5_input"
BBOX_NORTH=73.0
BBOX_WEST=-33.0
BBOX_SOUTH=19.0
BBOX_EAST=43.0

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

download_year() {
  year="$1"
  out="$INPUT_DIR/era5_2m_temperature_${year}.nc"
  tmp="${out}.tmp.$$"

  if [ -s "$out" ]; then
    echo "[$year] déjà présent ($(du -h "$out" | cut -f1)), skip"
    return 0
  fi

  echo "[$year] téléchargement..."
  rm -f "$tmp"
  python scripts/download_era5.py \
    --year "$year" \
    --bbox-north "$BBOX_NORTH" --bbox-west "$BBOX_WEST" \
    --bbox-south "$BBOX_SOUTH" --bbox-east "$BBOX_EAST" \
    --output "$tmp"
  mv "$tmp" "$out"
  echo "[$year] terminé ($(du -h "$out" | cut -f1))"
}

export -f download_year
export INPUT_DIR BBOX_NORTH BBOX_WEST BBOX_SOUTH BBOX_EAST

seq "$YEAR_START" "$YEAR_END" | xargs -n1 -P "$JOBS" bash -c 'download_year "$@"' _

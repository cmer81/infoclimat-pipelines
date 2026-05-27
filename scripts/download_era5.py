#!/usr/bin/env python3
"""Télécharge ERA5 hourly 2m_temperature pour une période et bbox données.

Usage annuel (climato) :
    CDS_API_KEY=... python download_era5.py \
        --year 2020 \
        --bbox-north 52.0 --bbox-west -6.5 --bbox-south 40.5 --bbox-east 12.5 \
        --output out/era5_2m_temperature_2020.nc

Usage journalier (observed) :
    python download_era5.py --year 2026 --month 5 --day 26 \
        --bbox-north 52.0 --bbox-west -6.5 --bbox-south 40.5 --bbox-east 12.5 \
        --output out/era5_2026-05-26.nc
"""

import argparse
import os
import sys
from pathlib import Path

import cdsapi


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser()
    p.add_argument("--year", type=int, required=True)
    p.add_argument("--month", type=int, default=None,
                   help="if set, restrict to single month")
    p.add_argument("--day", type=int, default=None,
                   help="if set, restrict to single day (requires --month)")
    p.add_argument("--bbox-north", type=float, required=True)
    p.add_argument("--bbox-west", type=float, required=True)
    p.add_argument("--bbox-south", type=float, required=True)
    p.add_argument("--bbox-east", type=float, required=True)
    p.add_argument("--output", type=Path, required=True)
    return p.parse_args()


def main() -> int:
    args = parse_args()

    api_key = os.environ.get("CDS_API_KEY")
    api_url = os.environ.get("CDS_API_URL", "https://cds.climate.copernicus.eu/api")
    if not api_key:
        print("CDS_API_KEY env var required", file=sys.stderr)
        return 1
    if args.day is not None and args.month is None:
        print("--day requires --month", file=sys.stderr)
        return 1

    args.output.parent.mkdir(parents=True, exist_ok=True)

    months = [f"{args.month:02d}"] if args.month is not None \
        else [f"{m:02d}" for m in range(1, 13)]
    days = [f"{args.day:02d}"] if args.day is not None \
        else [f"{d:02d}" for d in range(1, 32)]

    client = cdsapi.Client(url=api_url, key=api_key)
    request = {
        "product_type": ["reanalysis"],
        "variable": ["2m_temperature"],
        "year": [str(args.year)],
        "month": months,
        "day": days,
        "time": [f"{h:02d}:00" for h in range(0, 24)],
        "area": [args.bbox_north, args.bbox_west, args.bbox_south, args.bbox_east],
        "data_format": "netcdf",
        "download_format": "unarchived",
    }

    try:
        client.retrieve("reanalysis-era5-single-levels", request, str(args.output))
    except Exception as exc:  # noqa: BLE001
        msg = str(exc)
        # Cas attendu : ERA5/ERA5T n'a pas encore publié cette date (délai ~5j).
        # On sort avec un code distinct (3) + message propre, sans traceback,
        # pour que l'appelant Rust le traite comme "skipped" et non "failed".
        if "not available yet" in msg.lower() or "revise the period" in msg.lower():
            print(f"not-available-yet: data not published for the requested period",
                  file=sys.stderr)
            return 3
        # Vraie erreur (auth, réseau, requête invalide) : message concis, code 1.
        print(f"cds-error: {msg.splitlines()[-1] if msg else exc}", file=sys.stderr)
        return 1

    print(f"downloaded {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

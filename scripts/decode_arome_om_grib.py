#!/usr/bin/env python3
"""Décode un GRIB2 AROME-OM et produit 1 NetCDF par (variable, leadtime)
dans un dossier de sortie. Fonctionne avec des GRIB mono-leadtime (un fichier
par heure de prévision, format réel de l'API) ou multi-messages.

Appelé par le binaire Rust `arome-om-forecast`. Sortie NetCDF parce que c'est
le seul format que le wrapper Rust sait déjà lire (dep `netcdf` partagée avec
le crate climato).

Usage:
    python decode_arome_om_grib.py \\
        --in /path/to/file.grib2 \\
        --shortnames 2t,2d,10u,10v,prmsl \\
        --out-dir /tmp/decoded

Pré-requis pip (ajouter au venv projet):
    cfgrib xarray netCDF4

Pré-requis système:
    libeccodes (apt install libeccodes0 libeccodes-tools)
"""

import argparse
import os
import sys
from typing import List

try:
    import cfgrib  # noqa: F401
    import xarray as xr
except ImportError as e:
    print(f"FATAL: missing dependency ({e}). Run: pip install cfgrib xarray netCDF4", file=sys.stderr)
    sys.exit(2)


def decode(grib_path: str, shortnames: List[str], out_dir: str) -> int:
    """Pour chaque shortName, ouvre le GRIB filtré et écrit un NetCDF par
    leadtime (= une dimension `step` côté cfgrib)."""
    os.makedirs(out_dir, exist_ok=True)
    written = 0
    for sn in shortnames:
        try:
            ds = xr.open_dataset(
                grib_path,
                engine="cfgrib",
                backend_kwargs={"filter_by_keys": {"shortName": sn}, "indexpath": ""},
            )
        except Exception as e:
            print(f"WARN: variable {sn!r} not found or unreadable: {e}", file=sys.stderr)
            continue

        if len(ds.data_vars) == 0:
            # Cas courant à leadtime=0 pour les variables cumulatives
            # (max_i10fg, tp, ssrd, ...) dont le `stepRange = 0-1` n'a pas
            # de valeur à l'instant initial.
            print(f"INFO: variable {sn!r} has no data at this leadtime (cumulative var at lead 0?)", file=sys.stderr)
            continue
        var = ds[sn] if sn in ds.data_vars else next(iter(ds.data_vars.values()))
        # `step` peut être un scalaire si une seule leadtime dans la fenêtre.
        steps = ds["step"].values if "step" in ds.coords else [None]
        if steps.shape == ():
            steps = [steps.item()]

        for step in steps:
            sub = var.sel(step=step) if step is not None and "step" in var.dims else var
            lead_h = int(step / 1_000_000_000 / 3600) if step is not None else 0
            out_path = os.path.join(out_dir, f"{sn}_{lead_h:03d}h.nc")
            sub.to_netcdf(out_path)
            written += 1
            print(f"OK: wrote {out_path}", file=sys.stderr)
    return written


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--in", dest="grib_in", required=True)
    p.add_argument("--shortnames", required=True, help="CSV liste shortName")
    p.add_argument("--out-dir", required=True)
    args = p.parse_args()
    shortnames = [s.strip() for s in args.shortnames.split(",") if s.strip()]
    n = decode(args.grib_in, shortnames, args.out_dir)
    if n == 0:
        print("ERROR: no NetCDF produced", file=sys.stderr)
        sys.exit(1)
    print(f"DONE: {n} NetCDF files written to {args.out_dir}", file=sys.stderr)


if __name__ == "__main__":
    main()

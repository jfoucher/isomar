"""Converts Shom's tidal-stream atlases ("Atlas de courants de marée 2D au format netcdf", Licence Ouverte
2.0, credit "Shom", DOI 10.17183/ATLASCOURANTS2D_NETCDF) into the `.isoatlas` files the app reads
(`currents::atlas`), and writes the zone index the website serves them with.

Usage, from the repository root, with h5py and numpy installed (the downloads are untrusted: each zone's
7z is extracted in a folder of its own, and this script runs with `python3 -I` from outside it):

    python3 -I scripts/gen-tidal-atlas.py [--ports] [--step 15] OUT ZONE_DIR...

ZONE_DIR is a folder holding an extracted zone (named after it, as the 7z is: ATLASCOURANTS2D_NETCDF_FINISTERE,
or any folder above one). OUT receives OUT/<zone>/<grid>.isoatlas and OUT/index.json; the website
publishes it as atlases/ (scripts/deploy-website.sh). Not committed: Shom's data, regenerated from the
source.

Every 2 km, 500 m and 250 m grid is kept; the 100 m and 30 m harbour grids only with --ports. Steps finer
than --step minutes (15) are thinned to it. The size of each zone is printed."""
import argparse
import glob
import hashlib
import json
import os
import re
import struct
import zlib

import h5py
import numpy as np

# Each reference port's name and where it is, for the tide the atlas is phased by.
PORTS = {
    "BREST": ("Brest", 48.383, -4.495),
    "ROSCOFF": ("Roscoff", 48.718, -3.966),
    "CONCARNEAU": ("Concarneau", 47.873, -3.907),
    "SAINT-MALO": ("Saint-Malo", 48.640, -2.028),
    "CHERBOURG": ("Cherbourg", 49.651, -1.636),
    "LE-HAVRE": ("Le Havre", 49.482, 0.107),
    "CALAIS": ("Calais", 50.969, 1.868),
    "DUNKERQUE": ("Dunkerque", 51.048, 2.366),
    "BOULOGNE": ("Boulogne", 50.727, 1.577),
}

# The grids' places as they are written, where capitalising the file's words would not do.
PLACES = {
    "BAIE-DE-SEINE": "Baie de Seine",
    "GOLFE-NORM-BRET": "Golfe normand-breton",
    "FINISTERE": "Finistère",
    "PAS-DE-CALAIS": "Pas de Calais",
    "RADE-BREST": "Rade de Brest",
    "PORT-BREST": "Port de Brest",
    "LE-HAVRE": "Le Havre",
}

ZONES = {
    "PAS_DE_CALAIS": ("pas-de-calais", "Pas de Calais"),
    "MANCHE": ("manche", "Manche"),
    "BAIE_DE_SEINE": ("baie-de-seine", "Baie de Seine"),
    "GOLFE_NORMAND_BRETON": ("golfe-normand-breton", "Golfe normand-breton"),
    "BRETAGNE_NORD": ("bretagne-nord", "Bretagne Nord"),
    "FINISTERE": ("finistere", "Finistère"),
    "GASCOGNE": ("gascogne", "Gascogne"),
}

MISSING = -32768


def attr(h, k):
    v = h.attrs.get(k)
    if isinstance(v, bytes):
        return v.decode()
    if isinstance(v, np.ndarray) and v.size == 1:
        return v.item()
    return v


def grid_name(path, space):
    """'...R1111100_IROISE-560-250M-5min.nc' -> ('iroise-250m', 'Iroise 250 m')."""
    stem = os.path.basename(path)
    m = re.search(r"_R\d+_(.+?)-\d{3}-", stem)
    place = m.group(1) if m else stem
    words = PLACES.get(place, " ".join(w.capitalize() for w in place.split("-")))
    size = space.lower().replace("km", " km").replace("m", " m").replace("k m", "km").replace("  ", " ")
    return f"{place.lower()}-{space.lower()}", f"{words} {size.strip()}"


def convert(path, zone, step_min):
    h = h5py.File(path, "r")
    space = attr(h, "spacestep")
    port = attr(h, "Portref")
    if port not in PORTS:
        raise SystemExit(f"{path}: reference port {port} is not in PORTS")
    lat = np.asarray(h["latitude"][:], dtype=np.float64)
    lon = np.asarray(h["longitude"][:], dtype=np.float64)
    coeffs = [float(c) for c in h["coeff"][:]]
    minutes = np.asarray(h["time"][:], dtype=np.float64) - 360.0
    keep = [i for i, m in enumerate(minutes) if abs((m + 360.0) % step_min) < 1e-6] if np.min(np.diff(minutes)) < step_min else list(range(len(minutes)))
    u = np.asarray(h["U"][:, keep, 0, :, :], dtype=np.float64)
    v = np.asarray(h["V"][:, keep, 0, :, :], dtype=np.float64)
    fill = float(h["U"].attrs.get("_FillValue", [-9999.0])[0])
    if lat[-1] > lat[0]:
        # Rows north first.
        u, v, lat = u[:, :, ::-1, :], v[:, :, ::-1, :], lat[::-1]
    dy = abs(lat[-1] - lat[0]) / (len(lat) - 1)
    dx = (lon[-1] - lon[0]) / (len(lon) - 1)

    def cm(a):
        bad = ~np.isfinite(a) | (np.abs(a - fill) < 1e-3)
        out = np.clip(np.round(np.where(bad, 0.0, a) * 100.0), -32767, 32767).astype("<i2")
        out[bad] = MISSING
        return out

    slug, name = grid_name(path, space)
    header = {
        "name": name,
        "zone": zone,
        "port": PORTS[port][0],
        "port_lat": PORTS[port][1],
        "port_lon": PORTS[port][2],
        "low_water": bool(attr(h, "isBM")),
        "coeffs": coeffs,
        "minutes": [float(minutes[i]) for i in keep],
        "ncols": len(lon),
        "nrows": len(lat),
        "xll": float(lon[0] - dx / 2),
        "yll": float(min(lat[0], lat[-1]) - dy / 2),
        "dx": float(dx),
        "dy": float(dy),
        "release": str(attr(h, "Release_date") or ""),
    }
    js = json.dumps(header, ensure_ascii=False).encode()
    body = zlib.compress(cm(u).tobytes() + cm(v).tobytes(), 9)
    data = b"ISOATLAS" + struct.pack("<II", 1, len(js)) + js + body
    bounds = [header["xll"], header["yll"], header["xll"] + dx * len(lon), header["yll"] + dy * len(lat)]
    return slug, data, bounds, header


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("dirs", nargs="+")
    ap.add_argument("--ports", action="store_true", help="keep the 100 m and 30 m harbour grids")
    ap.add_argument("--step", type=float, default=15.0, help="the finest step kept, minutes")
    a = ap.parse_args()
    index_path = os.path.join(a.out, "index.json")
    zones = {}
    if os.path.exists(index_path):
        zones = {z["id"]: z for z in json.load(open(index_path))["zones"]}
    for d in a.dirs:
        for top in sorted(set(glob.glob(os.path.join(d, "**", "ATLASCOURANTS2D_NETCDF_*"), recursive=True) + glob.glob(os.path.join(d, "ATLASCOURANTS2D_NETCDF_*")))):
            if not os.path.isdir(top):
                continue
            key = os.path.basename(top).replace("ATLASCOURANTS2D_NETCDF_", "")
            if key not in ZONES:
                print("skipping", top)
                continue
            zid, zname = ZONES[key]
            os.makedirs(os.path.join(a.out, zid), exist_ok=True)
            files, bounds = [], None
            for nc in sorted(glob.glob(os.path.join(top, "*.nc"))):
                space = re.search(r"-(\d+(?:M|KM))-", os.path.basename(nc))
                if space and space.group(1) in ("100M", "30M") and not a.ports:
                    continue
                slug, data, b, header = convert(nc, zid, a.step)
                rel = f"{zid}/{slug}.isoatlas"
                with open(os.path.join(a.out, rel), "wb") as f:
                    f.write(data)
                files.append({"name": rel, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
                bounds = b if bounds is None else [min(bounds[0], b[0]), min(bounds[1], b[1]), max(bounds[2], b[2]), max(bounds[3], b[3])]
                print(f"  {rel}: {header['name']}, {header['nrows']}x{header['ncols']}, {len(header['minutes'])} steps, {header['port']}{' (low water)' if header['low_water'] else ''}, {len(data) / 1e6:.1f} MB")
            total = sum(f["bytes"] for f in files)
            zones[zid] = {"id": zid, "name": zname, "bounds": [round(x, 4) for x in bounds], "files": files, "bytes": total}
            print(f"{zname}: {len(files)} grids, {total / 1e6:.1f} MB")
    order = [z[0] for z in ZONES.values()]
    out = {"version": 1, "credit": "Shom", "licence": "Licence Ouverte 2.0", "doi": "10.17183/ATLASCOURANTS2D_NETCDF", "zones": sorted(zones.values(), key=lambda z: order.index(z["id"]) if z["id"] in order else 99)}
    with open(index_path, "w") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()

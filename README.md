# Isomar

A native chart plotter for sailors, written in Rust: a GNOME app (GTK 4 and libadwaita) on Linux, and the same engine under native shells on macOS, iPhone and iPad (SwiftUI) and Android (Compose). A clean-sheet rewrite of what OpenCPN does.

**Status: early but usable for planning.** Not for navigation.

## What it does

- **Charts:** S-57 ENC cells (with updates) drawn with a simplified S-52 presentation; BSB/KAP rasters; an open-data planning chart (Open Waters seamap tiles for land, seamarks and anchorages, with place names, land relief and, optionally, EMODnet depths), labelled not for navigation.
- **One-click weather:** right-click the chart → *Get weather here / for this view / for an area*. It fetches only the area and parameters you pick (wind, gusts, pressure, waves, …), shows the size first, and draws wind barbs, wind-speed colours, labelled isobars and waves with a timeline. Four models: NOAA GFS, ECMWF IFS and AIFS, and DWD ICON-EU.
- **Weather routing:** *Weather route…* on any route computes a way with isochrones from the forecast, a polar, land avoidance, engine use, and wind and wave limits. It can run on several models at once and compare them, check a route against the 31 GEFS ensemble members, and *Best departure* scans a range of departures and ranks them.
- **Currents:** Copernicus Marine currents fetched for the passage, and Shom tidal-stream atlases you install; drawn as arrows and sailed in by the router.
- **Polars and performance:** a library of polars (six built in, plus your own); a performance log that learns your boat's speeds from the instruments.
- **Navigation:** waypoints, routes (draw, drag, reverse, insert points), tracks, GPX import/export, go-to, measure, anchor watch, active-route steering data, and an automatic route that adds the waypoints a route needs to keep clear of land and of water shallower than the safety depth.
- **The boat:** NMEA 0183 (TCP, UDP, serial, log replay), gpsd, Signal K, AIS with collision alerts, instruments, a data monitor, and a built-in demo boat.
- **Tides:** right-click → *Tides here…* shows the high and low waters and the curve at the nearest station (Open Waters / TICON harmonic constants).
- **Charts for this area:** a built-in catalogue of free sources with A/B/C quality ratings; one click downloads, safely unpacks and loads them.
- Day, dusk and night palettes.

## Charts: what you can and can't use

The app ships a free **planning chart** built from open data. You can also import real charts you have licensed in an open format: plain S-57 ENC cells (for France, SHOM sells them per cell) and KAP rasters, from folders you choose under *Charts…*. Encrypted or app-locked charts (o-charts, Navionics, Garmin) are not supported.

## Building

```sh
cargo run                      # the GNOME app
cargo test
scripts/fetch-testdata.sh      # optional: free French sample charts into testdata/
```

Needs GTK 4.18+ and libadwaita 1.7+ development files (Fedora: `gtk4-devel libadwaita-devel gettext p7zip-plugins`).

The other shells are built by scripts: `scripts/mac-build.sh` (macOS), `scripts/ios-sim.sh` and `scripts/ios-device.sh` (iPhone and iPad), `scripts/android-build.sh` (Android). The code is in `crates/` (`isomar-core`, the domain code; `isomar-app`, the engine every shell drives; `isomar-gtk`; `isomar-ffi` and `isomar-android`, the doors for the other shells), `macos/` and `android/`.

## Not done yet

S-101, a plugin API, and the translation catalogue (the app is in English only).

## Licence

GPL-3.0-or-later (COPYING), with an additional permission for app stores (COPYING.appstore).

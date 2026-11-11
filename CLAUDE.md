# CLAUDE.md

Isomar is a native GNOME chart plotter for sailors, a from-scratch Rust rewrite of the ideas in OpenCPN: Rust (edition 2024), GTK 4 (`v4_18`), libadwaita (`v1_7`), plus a SwiftUI macOS shell over the same engine. Built the way `~/development/gitree` is built: look there for conventions when something is unclear. Licence: GPL-3.0-or-later. The app was called Chart until 2026-10-31 (`datadir::migrate_legacy` moves the old folders).

The detail of each feature (label placement rules, data-source research, router tuning history) lives in the module doc comments and in git history of this file; read the code before changing a feature.

## Commands

```sh
cargo run                           # --release for a fast build
cargo test                          # core and app need no display
cargo clippy --all-targets          # kept warning-free
cargo test --release -- --ignored --nocapture        # live checks (NOAA, real forecast, every NZ chart)
cargo test --release passages -- --ignored --nocapture   # routing regression passages; FRONTS=<dir> writes SVGs of the fronts
cargo run --release -p isomar-core --example routebench -- R11|R12|R14|R20 [route|window|both]   # its `result` line must not change unless routes are meant to; R20 is the tidal case (a made-up race in the Raz de Sein)
scripts/screenshot.sh out.png [delay-ms]   # headless GNOME Shell screenshot; runs target/debug/isomar, so `cargo build` first
scripts/bench.sh [--runs N] [--timing] [--warm] [view...]   # pan/zoom CPU; method and reference figures in design/perf/
scripts/profile.sh <proc|pid> <secs> [out] [filter]        # eu-stack sampler (perf is not installed)
scripts/mac-test.sh                 # core + app tests and the Swift smoke test on a Mac over SSH
scripts/mac-build.sh [out.png]      # build the macOS app on a Mac, run it once, copy a screenshot back
scripts/fetch-testdata.sh [--list|--all|name...]   # free French chart data into testdata/ (gitignored)
scripts/flatpak.sh / scripts/release.sh / scripts/deploy-website.sh   # packaging and release: release.sh builds the Flatpak, the notarised Mac app, the APK (signed with ~/.config/isomar-release/android-release.jks: every APK must use that key) and the iOS archive it uploads to App Store Connect (macos/scripts/ios-release.sh); the website links TestFlight for iOS
scripts/deps/build-mobile-deps.sh android-arm64|android-x86_64|ios|ios-sim|ios-sim-x86_64   # static cairo/pango/glib stack into ~/dev/isomar-deps/prefix-<platform> (Android on Linux with the NDK, iOS on a Mac)
scripts/android-build.sh [debug|release] [abi...]   # crates/isomar-android (JNI over isomar-ffi) + android/ Gradle app; APK in android/app/build/outputs/apk
scripts/ios-sim.sh [out.png]   # IsomarTouch target (macos/IsomarTouch + the shared macos/Isomar views) built for and run in the iOS simulator on either Mac (x86_64 on the Mac Pro, arm64 on Apple silicon; first run builds that Mac's simulator deps), screenshot copied back; ENV="ISOMAR_DEMO=1 ISOMAR_SHEET=medium|large"
scripts/ios-device.sh                # the same, signed (team 24G23598DF, Automatic) and installed and launched on the iPhone paired with the Mac; DEVICE_ID, ENV, BUILD=0, LAUNCH=0; both go through macos/scripts/ios-build.sh sim|device
```

Generators (outputs are committed): `scripts/gen-blosc-fixtures.py` (synthetic Blosc test frames, needs numcodecs), `scripts/gen-s57names.py` (S-57 names from GDAL), `scripts/gen-polars.py` (`data/polars/*.csv`), `scripts/draw-glyphs.py` (value icons), `cargo xtask naturalearth testdata/naturalearth` (`data/labels/naturalearth.json.gz`), `cargo xtask capes testdata/osm-capes [--coves]` (`data/labels/capes|coves.json.gz`, ODbL), `cargo xtask tokens --swift out.swift --kotlin out.kt`, `macos/scripts/make-icon.sh`. Not committed: `scripts/gen-tidal-atlas.py` converts Shom's tidal-stream NetCDF into `website/atlases/` (gitignored, published by `deploy-website.sh`).

System deps (Fedora): `gtk4-devel libadwaita-devel gettext p7zip-plugins`.

Useful dev env vars (full list in `crates/isomar-app/src/debug.rs` and `crates/isomar-gtk/src/ui/debug.rs`): `ISOMAR_SCREENSHOT=<png>`, `ISOMAR_DEBUG_VIEW=<lat>,<lon>,<zoom>`, `ISOMAR_DEMO` (boat, route and marks only in a hand-checked demo area: Quiberon or the Iroise), `ISOMAR_DEMO_BOAT=harbour|sea|off` (screenshots: `sea` under way only, `off` otherwise; `ISOMAR_PALETTE=day` too, or the evening palette follows the clock), `ISOMAR_DEMO_WEATHER[=gfs|ifs|aifs]`, `ISOMAR_DEMO_CURRENTS` (a made-up current round the Iroise), `ISOMAR_ATLAS_URL` (where the Shom atlases are, `file://` for a local folder), `ISOMAR_OFFLINE`, `ISOMAR_DIALOG=<name>`, `ISOMAR_SIDEBAR=<page>`, `ISOMAR_PALETTE=day|dusk|night`, `ISOMAR_SIZE=WxH`, `ISOMAR_HIDE`/`ISOMAR_SHOW=<layers>`, `ISOMAR_DEBUG_TIMING` (frame and per-tile timings, why tiles redrew), `ISOMAR_BENCH=pan|zoom`, `ISOMAR_TILE_THREADS=<n>` (0 = inline), `ISOMAR_STATE_DIR`/`ISOMAR_CONFIG_DIR`, `ISOMAR_NO_SAVE`.

## Architecture

**Crates** (GTK-free down to GTK):
- `crates/isomar-core`: domain code, no UI, unit-tested without a display. `geo`, `chart/` (shapefile coast store, grids, S-57 `iso8211`+`s57`, BSB/KAP, vector tiles, seamarks, places), `labels/` (lettering along paths), `nav/` (routes, GPX, SQLite store), `comm/` (NMEA, AIS, connections, `Hub`), `session` (everything that changes under way; feed it events and a clock, it returns notices), `weather/` (GRIB2 read and write, store, `source` (the forecast models), planner/download), `routing/` (polars and library, isochrone router, seaway, weather window), `currents/` (Copernicus Marine Zarr with a Blosc/LZ4 decoder, Shom atlases phased by the tide, `CurrentField` nesting them), `perf/` (performance log, polar learning), `tides`, `registry`, `install`, `config`, `i18n`. Feature `testutil` exports test builders.
- `crates/isomar-app`: the toolkit-free engine any shell drives (cairo and pango only): `engine` (view, layers, tile compositing, input entry points), `render` (tile cache and workers), `layers/`, `app::AppState`, `shell::Shell` (what a shell implements: toast, error, sound, confirm, menus, info, open dialog), `host` (start-up, 4 Hz `poll`, shutdown), `vm/` (plain serialisable records for every page and piece of chrome), `flows/` (multi-step operations, each a job), `jobs`, `api::call(app, name, json)` (JSON door to everything), `probe` (what is under the pointer), `router` (weather-route preview), `platform` (hooks: `spawn`, `timeout_once`, `bg`), `chips`, `units`.
- `crates/isomar-gtk`: the GTK app (binary `isomar`), GNOME layout per `design/ia.md`: `window`, `header`, `sidebar` + `pages/`, `alerts`, `overlays`, `popovers`, `canvas.rs` (`ChartView`, feeds the `Engine`). Re-exports `isomar_core` as `crate::core`.
- `crates/isomar-ffi`: hand-written C ABI (`include/isomar.h`) for the macOS shell; main thread only; engine requests reach Swift as JSON events.
- `macos/`: the Swift app (XcodeGen `project.yml`); `Model.swift`, `MainView.swift`, `Sidebar.swift`, `ChartView.swift`, etc. A feature usually needs both shells: the vm record, then GTK and Swift views of it.

**Hard boundaries and rules**
- Core is GTK-free; GTK code is only in `isomar-gtk`. GTK views are plain structs in `Rc`, closures hold `Weak`; no GObject subclassing except `ChartView`.
- Blocking work never runs on the main loop: `ui::spawn(async {...})` for main-loop futures, `ui::bg(|| ...).await` / `platform::bg` for blocking work; only `Send` data crosses.
- Never call `AppState::refresh()` or `tick()` while holding a borrow of `session`.
- Long work is a job (`isomar_app::jobs`) shown in the activity indicator, never a progress dialog. Cancel stops at once: flows await through `JobHandle::unless_cancelled`; work handed to a thread stops when its future is dropped (`jobs::on_drop`). A multi-stage flow is one job from its first stage.
- Dialogs are panels inside the main window (`float`), not windows or sheets; `Panel::set_content` gives their header bar title and close button. Only About, shortcuts and file choosers are real windows.
- Every user-visible string goes through `crate::i18n`: `gettext("…")`, `gettext_f("Delete {name}?", &[("name", &x)])`.
- Colours come from design tokens (`data/design/tokens.json`, generated into `tokens::Token` by `isomar-app/build.rs`, CSS by the GTK `build.rs`, Swift by xtask): `pal.set(cr, Token::X)`, never a new hex. Overlay layers follow `design/design-system/overlay-spec.md`, marks and lights `chart-symbols.md`, value icons `iconography.md`.
- Chart text that may be in a local language is laid out with Pango, not cairo's text API (only Pango falls back per character).
- Settings: JSON in `$XDG_CONFIG_HOME/isomar/settings.json` (`core/config.rs`). Navigation data: SQLite `~/.local/share/isomar/navobj.sqlite`. Caches (tiles, coast index, weather, tides) in `~/.cache/isomar`.

**Chart rendering (tiles)**
- Static layers are drawn into 256-pt tiles aligned to the world grid at the view's zoom (`render.rs`), on `isomar-tile-N` worker threads, cached in a 192 MB LRU; dynamic layers (boat, nav, router, hover, labels that follow the screen) are drawn each frame into an `OverlayBuffer` cut into 128 px chunks. `Engine::frame` hands a shell a `Scene` (tile placements by id + changed chunks); GTK and the Mac turn them into GPU textures, so a pan copies no pixels. `Engine::draw` (one cairo image) is for tests and screenshots.
- A static `Layer` has `prepare(&TileArea)` (main thread, once per batch: requests data, decides everything that depends on the view centre) returning an `Arc<dyn TilePaint>` snapshot whose `paint` runs on a worker per tile. Paints must not touch `Rc`/`RefCell`; shared data is `Arc`. Anything greedy that depends on neighbours must weigh candidates past the tile edge so tiles agree; per-pixel work stays inside the tile's own pixels, the 64 pt margin is for symbols and labels straddling the edge.
- Redraws: a layer reports where its data changed (`Layer::changes_since`, `layers::Changes`); bumping everywhere redraws every tile, so bump an area. Layer order is set in `window.rs` and `data.rs`. `Layer::plane` puts weather in its own tile plane.
- Don't draw dynamic layers through cairo recording surfaces to learn where they drew: it allocated gigabytes on a real session.
- Measure with `ISOMAR_DEBUG_TIMING` and `ISOMAR_BENCH`/`scripts/bench.sh` rather than guessing.

**Names on the chart** are placed only at label steps (every whole zoom, every 2 from zoom 15) and scaled in between, so names and the marks they avoid change together (`layers::names`). Sources: Natural Earth (bundled, spines precomputed by xtask), OpenFreeMap places tiles, ENC `OBJNAM`, bundled OSM capes and coves. Collisions are settled by a pairwise rule independent of the tile asking, so every tile agrees. Marks and lights come first; names go round them. See `layers/{names,regions,waters,towns,capes}.rs` and `core/labels/`.

**Weather routing** (`flows::routing`, `core/routing/`): finds the way by water on a coarse mask first, downloads the forecast for that area, then runs `isochrone::explore`. Pruning is by cells of sea, progress measured by water (`seaway::SeaField`). Check any router change with the `passages` tests and `routebench`, not just unit tests. Routes keep their plan (`RoutePlan`, with the forecast cut to the path) for Statistics and Replay. Best departure (`flows::departures`, `routing::window`) scans departures in parallel and ranks by weighted cost.

**Forecast models** (`core/weather/source.rs`, design and what each phase became in `design/weather-models.md`): GFS, ECMWF IFS, ECMWF AIFS, and the regional DWD ICON-EU (nested in GFS outside its area and after its end, `routing::nested`). Each model's forecast is kept apart (`WeatherState::store` is the shown one, `others` the rest), one is shown at a time. A weather route or best-departure scan runs on every model ticked: one search each with one land mask (`flows::routing::compute_models`), compared by sailing each best way through every forecast (`routing::along`, `routing::compare`); departures rank by the models' average or worst cost. A saved route keeps its model and its arrival on each forecast (`RoutePlan::source`, `checks`). Under way `perf::model_check` scores each loaded model against the instruments. "Ensemble check" sails a route through the 31 GEFS members (`flows::ensemble`).

**Currents** (`core/currents/`, `isomar-app/src/currents/`, `flows::currents`; plan and what each phase became in `design/currents-plan.md`): their own source, shared by every wind model (`ModelWeather::env` wraps the wind in `WithCurrents`). Finest first: an imported grid, the Shom atlases (tide by phase from the reference port's high water and Brest's coefficient, plus SMOC's residual), Copernicus NW Shelf, IBI, Med, Baltic, SMOC; blended at the edges. A route or scan fetches Copernicus for its passage in the same job (anonymous ARCO Zarr, byte ranges of each chunk's Blosc blocks, cached as GRIB2 per update); a Shom zone is only ever installed by the user. The router sails in the wind over the water, reads the current mid-leg, may hold for the stream (`Settings::allow_hold`), and the departure scan refines round the best where the stream is strong. Without currents the router's arithmetic is unchanged to the bit: keep it so (`has_currents`).

**Polars and performance**: one active polar (`Settings::polar`) from the library (`core/routing/library.rs`: six built-ins from `data/polars`, user ones in `<state dir>/polars`); routing sails with it and the performance log (`core/perf/`) learns into it alone. Engine RPM sets sailing/motoring as a fact; without it the app only suggests. A model's wind teaches factors only, never the polar's shape.

## The two-machine setup (macOS)

Mac work is built over SSH (Tailscale) on one of two Macs; `scripts/mac-lib.sh` picks the first that answers (`MAC=user@host` pins one): `100.68.219.52` (Intel Mac Pro, no Homebrew bottles, uses `~/dev/isomar-deps/prefix-<arch>` built by `macos/deps/build-deps.sh`) and `100.74.8.70` (Apple silicon). The repo is rsynced to `~/dev/isomar`; the GTK crate cannot build there. `mac-build.sh` takes `ENV="ISOMAR_DEMO=1 ..."`, `BUILD=0`, `SIZE`, `DELAY`; the Mac app can be driven with `ISOMAR_SCRIPT` (see `macos/` for the verbs). Every SSH prints a harmless tmux complaint. macOS has no libintl: `i18n` reads `.mo` files itself.

## Chart data and licensing rules

- **Never commit chart data.** `testdata/` is gitignored. SHOM `eval` datasets (S-101, S-102) are evaluation-licence only: not redistributable.
- The app does not bundle SHOM S-57; it ships an open-data planning chart (labelled not for navigation) and imports S-57/KAP files users licensed themselves.
- OSM-derived tiles are fetched at run time and never bundled; the only bundled OSM data are `data/labels/capes.json.gz` and `coves.json.gz` (ODbL). Tests build tiles with `vtile::testutil`, never real ones.
- Avoid TPXO, FES, IOC sea level and WDPA (no redistribution): never bundle or redistribute them. Fetching Copernicus SMOC at run time (its tide comes from FES2014) is allowed under the Copernicus licence (decided 2026-11-09). Free coastal S-57 exists only for the US (NOAA, CC0).
- Never ask about or suggest inland-waterway data.
- S-57 object and attribute codes must be checked against OpenCPN's `s57objectclasses.csv` / `s57attributes.csv`; don't trust memory. Class, attribute and value names come from the generated `s57names.rs`. A class a layer draws must be added to `featureinfo::reportable` to be clickable.
- S-57 updates apply only if numbered exactly current + 1 (a test checks the Iroise cell reproduces its consolidated reissue).
- Online sources in use: Open Waters seamap tiles (land, seamarks, anchorages; CC BY 4.0 + ODbL), OpenFreeMap (places), AWS Terrarium (relief), EMODnet (depths, off by default), NOAA NOMADS (GRIB; one global `Throttle`, it blocks clients past ~120 requests/min), Copernicus Marine (currents, anonymous ARCO Zarr on `s3.waw3-1.cloudferro.com`, found through STAC; credit "E.U. Copernicus Marine Service Information"), Shom tidal atlases (converted and hosted on the website; credit "Shom", Licence Ouverte), ECMWF open data (IFS and AIFS, CC BY 4.0; whole-globe fields by byte range from each step's `.index`, cropped before caching, not throttled), Open Waters tides. Each is credited on screen (`CreditLayer`) and cached under `~/.cache/isomar`.
- Test-data files are untrusted downloads: each lives in its own directory, scripts that read them stay outside it.

## Testing gotchas

- A test that reads `testdata/` with a relative path silently skips under `cargo test` (the working directory is the crate's); use `CARGO_MANIFEST_DIR/../..`.
- `clippy` and `test` don't rebuild `target/debug/isomar`; `cargo build` before `screenshot.sh`.
- The SHOM Molene area at zoom 14 (`ISOMAR_DEBUG_VIEW=48.395,-4.945,14`) shows most ENC symbology; at zoom 12 the 1:25,000 cells are not drawn.

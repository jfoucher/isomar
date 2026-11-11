# Currents in the weather router: implementation plan

Plan, 2026-11-09, from the research in `currents.md`; the owner's decisions are in §0. Phases are in the order they
should land; each one ships something usable and leaves `routebench` and `passages` unchanged unless the
phase says otherwise.

## 0. Decisions (owner, 2026-11-09)

1. **SMOC's tide (FES2014) is allowed as a run-time fetch** under the Copernicus licence, credited to
   Copernicus. CLAUDE.md's FES rule now covers bundling and redistribution only. Without it, waters outside
   the SHOM atlases and the European shelf models (New Zealand among them) would get no tidal streams.
2. **SHOM atlases are converted by a generator and hosted as static files on the website**
   (`deploy-website.sh`), downloaded per zone on demand. Etalab allows this with credit. Not bundled; the
   device never reads the original NetCDF/7z.
3. **Currents are on by default** wherever a source covers the passage, with a switch in the routing panels
   and in Settings; the route card says currents were used.
4. **All six phases are planned;** NOAA S-111 stays in "Later".
5. Copernicus is read anonymously, with an optional login in Settings as a fallback (§3.2) in case anonymous
   access closes.

## 1. Design

### 1.1 Where currents live

Currents are **their own source, beside the wind models, shared by all of them** (`currents.md` §1.7).

- **Core: `crates/isomar-core/src/currents/`**, a new module beside `weather/`:
  - `mod.rs`: `CurrentSource` (`Smoc`, `NwShelf`, `Ibi`, `Med`, `Baltic`, `Atlas(AtlasId)`, `Imported`,
    later `NoaaOfs(..)`), with `label`, `credit`, `domain`, `res`, `has_tide`, and `priority` for nesting.
  - `field.rs`: `CurrentField`, the composed current at any point and time (§1.2). This is what the router
    and the chart read.
  - `copernicus.rs`: the dataset table (product, dataset, variable names, domain, grid), STAC discovery and
    the subset reader (§3).
  - `zarr.rs`: Zarr v2 metadata (`.zmetadata`, `.zarray`), chunk keys, chunk geometry.
  - `blosc.rs`: a Blosc 1 decoder (header, block offsets, LZ4 blocks, byte unshuffle) on top of `lz4_flex`.
  - `atlas.rs`: SHOM tidal-stream atlases: the format we convert them to, and sampling by tidal phase and
    coefficient (§4).
  - `coefficient.rs`: high and low waters at an atlas's reference port, and the day's coefficient from
    Brest.
- **Gridded forecasts reuse `WeatherStore`.** SMOC and the shelf models are hourly grids over time, which is
  exactly what `WeatherStore` holds: areas side by side, newest run wins, time interpolation, `clipped`,
  `within`, and the plan encoding. The app keeps one `WeatherStore` per `CurrentSource`, as `WeatherState`
  does per wind model.
- **Params.** `Param::CurrentU/V` (GRIB2 10/1/2, 10/1/3) always means the **total current** a source gives.
  Add `Param::ResidualU/V` (local-use numbers 10/1/192 and 10/1/193 in our cache files only) for SMOC's
  `uo`/`vo` without the tide, which the SHOM atlases are added to. SMOC's `CurrentU/V` is stored as
  `uo + utide` (the Stokes drift is left out, see `currents.md`).
- **App: `crates/isomar-app/src/currents/`**: `CurrentsState { stores: BTreeMap<CurrentSource, WeatherStore>,
  atlases: Vec<Arc<Atlas>>, tides: RefTides, shown: bool, fill: bool }`, held by `AppState` beside
  `weather`. It follows `WeatherState::time` (no clock of its own) and builds an `Arc<CurrentField>` snapshot
  for the router and the layer whenever something loads (`revision` bumps).
- **Imported GRIBs.** Fields from discipline 10 that `read_fields` returns go to `CurrentSource::Imported`
  instead of the wind model's store. `StoreEnv::current` then returns `None`. The current now comes only
  from the currents layer of the environment (§1.3), so it is never counted twice.

### 1.2 Nesting: `CurrentField::sample(p, t) -> Option<(east_kn, north_kn)>`

From finest to coarsest, the first source that covers the point wins, blended at the edges like
`routing::nested` (`BLEND_CELLS` of the finer grid, about 3 cells, and its last `BLEND_HOURS` of time):

1. **SHOM atlas tide + SMOC residual** (`ResidualU/V`). The finest atlas grid covering `p` first (30 m > 100 m >
   250 m > 500 m > 2 km). The residual is added only where SMOC is loaded; without it, the atlas tide alone.
2. **NW Shelf**, then **IBI**, total current (NW Shelf wins where both cover: 1.5 km against 3 km).
3. **Med / Baltic / Black Sea** totals (little tide, but real circulation).
4. **SMOC** `uo + utide`.
5. **Imported** GRIB currents. If the user imported one, it goes first instead: someone who imports a
   current GRIB means it.

Three details matter more than they look:

- **Sampling near the coast.** `Grid::sample` returns `None` when any of the four corners is NaN. Currents
  are NaN over land, so on a 9 km grid the whole last cell before the coast would have no current, which is
  where it matters most. Add `Grid::sample_valid`: bilinear over the valid corners with renormalised
  weights, `None` only when all four are NaN. Currents use it; wind doesn't change.
- **A blend whose finer side is missing** (NaN at a SHOM grid's ragged mesh edge) falls through to the next
  source, not to zero.
- **Time.** Copernicus fields are hourly, so linear interpolation in u, v is fine (`currents.md` §1.4). Atlas
  steps are 1 h, 30 min or 5 min of tidal phase, interpolated in phase (§4.3).

Cost: each `sample` is a few bounds checks and 4–12 bilinear lookups. Measure before optimising (§2.5).

### 1.3 Into the router: `WithCurrents`

```rust
/// The wind, waves and gusts of `wind`, the current of `currents`: every wind model sails on the same current.
pub struct WithCurrents<'a> { pub wind: &'a dyn Environment, pub currents: &'a CurrentField }
```

It implements `Environment` by forwarding everything except `current`. `flows::routing::ModelWeather::env()`
is the one place every route, cross-check, scan and plan check gets its environment from, so wrapping
there gives all of them currents at once: `compute_with`, `compute_models`, `check_route`,
`CrossCheck::run`, `window::scan`, ensembles. `ModelWeather` gains `currents: Option<Arc<CurrentField>>`,
cut to the passage area (`CurrentField::within(b)`).

## 2. Phase 1: the router's physics (no new data)

This can be tested with synthetic fields and imported GRIBs before any download exists. With no current
loaded, every change below reduces to today's arithmetic, so the `routebench` result lines and the
`passages` tests must not move. Check that first and after each step.

### 2.1 Wind over water for the polar (`currents.md` §1.2)

In `isochrone::explore`, per front node (not per heading, so it costs nothing):

```
air  = to_uv(tws, twd)            // ground-relative wind, towards
c0   = env.current(node, t)
wow  = air − c0                   // the wind the boat sails in
(tws_w, twd_w) = from_uv(wow)
```

`tws_w` and `twd_w` feed `polar.at_wind`, the TWA of each heading, and the `max_wind_kn` check. Move
`to_uv`/`from_uv` out of `nested.rs` into `geo` (or `routing::vector`) so the router, `along`, `nested` and
the stats share them. `Node::tws_kn`/`twd` record the **wind over water** (what the boat sailed in); add
`Node::current: (f64, f64)` (east, north kn at mid-leg) so stats, replay and the chart can show it. The plan
wire format (`nodes: Vec<[f64; 9]>`) becomes `[f64; 11]`. `decode` accepts 9 (currents 0) or 11.

### 2.2 Mid-leg sampling (§1.3)

A predictor–corrector per candidate: step with `c0` to `p1`, sample `c_mid` at the midpoint of `node→p1` at
`t + dt/2`, then recompute the ground vector and end point with `c_mid`. To keep it cheap:

- Correct only the candidates that survive the `behind` filter (most of an all-round fan is dropped there
  before land or weather are looked up), then re-run `open` on the corrected end.
- Skip the corrector when no current source covers the passage (`CurrentField` empty): today's loop exactly.

The `left_out` nearest-candidate path takes the same correction.

### 2.3 Shorter legs where the stream is strong

`passage::settings_for_distance` gives 15 min legs under 60 nm, 30 min to 300 nm. Within a tidal race a 30 min
leg is still 3 nm. Rather than an adaptive step (fronts are synchronous in time, so that would be a large
change), cap `step_minutes` at 15 when the passage's peak current (sampled along the way-by-water path at a
few times) exceeds 2 kn and the passage is under 300 nm. Measure the time cost with `routebench` on a
tidal case (§2.6) before choosing the threshold.

### 2.4 `routing::along` (cross-check, ETA bracket, ensembles)

Today the cross-track current is "made up by the helm" and ignored. Do it properly:

- Wind over water as in §2.1.
- **Crab angle:** to hold the course over ground, the boat heads off by `asin(c_cross / speed)`. Speed made
  good = `sqrt(speed² − c_cross²) + c_along`. When `|c_cross| ≥ speed` the boat can't hold the line: sail the
  step at the best it can and count `Sailed::swept_hours`.
- Iterate once: the polar speed depends on the TWA from the heading, not from the course.
- Record `Node::current` and `Node::heading` (now different from `cog`).

### 2.5 Statistics (`routing::stats`)

Add to `Stats`:

- `fair_hours`, `foul_hours`: current along the track at least 0.5 kn with or against it.
- `current_gain_nm`: the integral of current along the track, which is what the stream gave or took.
- `max_current_kn` and where it was.
- `wind_against_tide_hours`: wind at least `W` kn blowing within 60° of opposite the set of a current of at
  least `C` kn (defaults 15 kn, 1.5 kn).

The route statistics vm (`vm/stats.rs`) and the three shells' statistics views get a "Current" section.

### 2.6 Tests and benches

- Unit tests: 15 kn wind against a 3 kn stream sails at the polar's 18 kn speed; a beam stream changes the
  TWA; `along` with a cross current is slower than without and arrives on the line.
- **A synthetic tidal race** in `routing::testutil` or `weather::testgrib`: a 1.5 nm wide band with an M2
  stream (`4 kn · sin(2πt/12.42 h)`, reversing), sea either side. Tests: (1) mid-leg sampling changes the
  outcome against start-of-leg sampling when the leg crosses the band; (2) the router goes through at fair
  stream when it has the choice of a longer way round.
- **`routebench` gets a tidal case**, `R20`: Alderney Race or Raz de Sein with the synthetic race (no real
  data in tests), so later phases have a pinned `result` line and a time to compare against.
- Time `routebench` before and after, and run `scripts/profile.sh` if the corrector costs more than about
  10%. If it does, add a per-step snapshot (`CurrentField::at(t)` that
  works out the time weights once, so the per-point lookup is only bilinear).

## 3. Phase 2: currents as a source, on the chart (data from GRIB import and demo)

### 3.1 Core and app state

`currents::{mod, field}`, `Param::ResidualU/V`, `Grid::sample_valid`, `CurrentsState`, `WithCurrents` in
`ModelWeather::env()`, imported discipline-10 fields routed to `CurrentSource::Imported` (§1.1). Add
`ISOMAR_DEMO_CURRENTS` (a synthetic tidal field round the demo position, through the race builder) to
`debug.rs` for screenshots and the Mac/phone simulators.

### 3.2 Settings (`core/config.rs`)

```rust
pub struct CurrentPrefs {
    /// Route with currents where a source covers the passage.
    pub route_with: bool,                 // default true
    /// Atlas zones the user has downloaded (ids), so the app reloads them at start.
    pub atlases: Vec<String>,
    /// Optional Copernicus Marine login, only if anonymous reads stop working.
    pub copernicus_user: Option<String>,  // password in the keyring, never in settings.json
}
```

### 3.3 The chart layer

- **`overlay-spec.md`** gets a "Currents" row (design first): arrows on a 48 px grid like the barbs, length
  and weight by speed in three steps (< 1 kn thin and short, 1–3 kn, > 3 kn thick), each with the halo;
  at zoom 12 and up the speed as a number in `caption` at the tail ("2.4"); an optional `current-0…current-4`
  speed field that **replaces** the wind or wave field (one field at a time, as the spec already says), so
  `Fill` gains `Current`.
- **Tokens:** `current-arrow` and the `current-*` ramp in `data/design/tokens.json`. The GTK CSS and
  `tokens::Token` follow from the build scripts; Swift and Kotlin from `cargo xtask tokens`.
- **`CurrentLayer`** (`isomar-app/src/currents/layer.rs`), a static layer in the weather's tile plane
  (`Layer::plane`), redrawn when `WeatherState::time` or the field's revision changes; `prepare` takes the
  `Arc<CurrentField>` and the time; `paint` samples per arrow. Sampled once per arrow, so the 250 m atlas
  grids cost nothing extra at low zoom. Its order sits with the weather layer in `window.rs`/`data.rs`.
- **Readout:** `weather::readout` adds a chip "Current 2.1 kn → 075°" (a new `Icon::Current` value glyph by
  `scripts/draw-glyphs.py`, per `iconography.md`), with the source in the tooltip.
- **Timeline:** plays when only currents are loaded (today it needs a forecast).
- **Credits:** `Credit::currents: RefCell<Vec<&'static str>>`, filled by the layer and by a routed plan:
  "E.U. Copernicus Marine Service Information", "Shom". The footer vm picks them up for the Mac and phones.

### 3.4 vm and shells

- `vm::layers`: a "Currents" toggle beside wind, waves and pressure; `Fill::Current` in the field picker.
- Weather sidebar page: a "Currents" group listing the sources covering the view (name, grid, last update,
  how far ahead, "loaded" or a download size), and "Get currents here".
- Routing and Best departure panels: a "Currents" switch row under Forecasts, saying which source covers
  the passage ("Shom atlas + Copernicus", "Copernicus NW Shelf 1.5 km"), and a dimmed "none here" when
  nothing does.
- GTK (`pages/`, `overlays`), Swift (`Sidebar.swift`, `Routing.swift`), Compose (`ui/Pages.kt`,
  `ui/Sheets.kt`), driven by the vm records. `api::call` verbs `currents.get`, `currents.show`,
  `currents.route_with` for `ISOMAR_SCRIPT`.

## 4. Phase 3: Copernicus Marine (SMOC, NW Shelf, IBI, Med, Baltic)

### 4.1 Discovery

`copernicus::discover(product, dataset)` reads `stac.marine.copernicus.eu/metadata/<PRODUCT>/product.stac.json`
and the dataset item, and returns the two Zarr store URLs, the variables and the time extent. Cache it under
`~/.cache/isomar/currents/stac/` for a day. Dataset ids carry version suffixes that change; the table in
`copernicus.rs` names product and dataset prefix, and discovery picks the current version. Read the
consolidated `.zmetadata` once per store per day: `shape`, `chunks`, `dtype` (`<f4`), `compressor` (Blosc
`lz4`, shuffle), `fill_value`, `order` (C), and the `time`, `latitude`, `longitude` coordinate arrays (small,
one chunk each).

Variable names in the shelf datasets (`uo`/`vo`?) are checked in their STAC items when the table is filled
in, not assumed from SMOC.

### 4.2 Reading a subset

Always the **geo-chunked** store (time-chunked is hundreds of MB for a passage):

1. From the passage area (`Area`, rounded outward to whole chunks) and `[now − 1 h, until]`, compute the
   chunk keys (`<var>/<t>.<y>.<x>`). The time axis spans years, so the forecast is in the last one or two time
   chunks.
2. **Byte ranges inside a chunk.** A chunk is one Blosc frame: a 16-byte header (version, flags, type size,
   `nbytes`, `blocksize`, `cbytes`) followed by the offsets of its blocks, and each block is compressed on its
   own. In a C-order `(time, lat, lon)` chunk, the forecast hours are one run of blocks at the end. So:
   one range request for the header and offsets (`16 + 4·nblocks` bytes), then one for the blocks covering
   the hours wanted. Fall back to the whole chunk when the frame is `memcpyed`, a single block, or the
   server ignores `Range`. Expected saving 5–10× (`currents.md` §2); measure it.
3. Decode the blocks (`blosc.rs`: per block, split streams if the flag says so, each prefixed with its
   compressed length; LZ4 via `lz4_flex`; then byte unshuffle by `typesize`), take the hours and cells
   wanted, NaN at `fill_value`, m/s.
4. For SMOC: fetch `uo`, `vo`, `utide`, `vtide`; store `CurrentU/V = uo + utide` and `ResidualU/V = uo`. Skip
   `utide`/`vtide` (half the bytes) where a SHOM atlas or a shelf model covers the whole area.
5. Cache the cropped result as GRIB2 through `gribwrite` (as the ECMWF fields are), one file per source,
   area and update day, under `~/.cache/isomar/currents/`. `Field::run` is the store's `Last-Modified` day.
   Chunks are rewritten daily, so a cached subset is good until the next day's update.

Requests go through `ureq` with the app's normal user agent; a per-host concurrency of 8 (no throttle known,
but be polite). It all runs in `platform::bg`, cancellable through the job's drop guard, like `fetch_plan`.

### 4.3 Planning and the flows

- `currents::plan(area, until) -> CurrentPlan`: which sources cover the area (finest first), which variables
  each needs given what covers it, the chunk count and an estimated download from the measured chunk sizes.
  `weather::planner`'s `Group::Current` "unavailable" goes; the weather page's "Get currents here" uses this.
- **`flows::routing::ensure_currents(app, area, until)`**, called once in `route` and in `flows::departures`
  after `forecasts_for`, inside the same job ("Getting the currents"). A failure is a toast, and the route
  goes on without currents (it says so on the card), never an error.
- `RoutePlan` keeps the current it was routed on so Replay and Statistics work after the forecast is gone:
  `RoutePlan::currents: WeatherStore` holding the **composed** field resampled on a corridor round the path
  (the path ± 2 nm, at the finest source's step capped at 0.005°, hourly), as the wind is cut today. Measure
  its size on a Channel crossing; if it is large, keep only `Node::current` and drop the field.

### 4.4 Tests

- `blosc.rs` against small fixtures written by `numcodecs` (a generator script under `scripts/`, the fixtures
  committed; they're synthetic arrays, not data): whole-frame and single-block decodes, shuffle on and off,
  split streams.
- `zarr.rs`: chunk keys and the block range for given hours, against a synthetic `.zarray`.
- `field.rs`: nesting precedence, edge blends, NaN falling through, `sample_valid` at a coast.
- Live (`--ignored`): discover and fetch SMOC and NW Shelf for the Channel for the next 48 h, and check that the
  peak in the Alderney Race at springs is over 4 kn on NW Shelf and smaller on SMOC (the grid smears it);
  report bytes fetched with and without block ranges.

## 5. Phase 4: SHOM tidal-stream atlases

### 5.1 Conversion (generator, outputs not committed)

`scripts/gen-tidal-atlas.py <7z-dir> <out-dir>` (Python with `h5py`/`netCDF4`, like the other generators; the
downloads live in their own directory, the script outside it, run with `python -I`):

- Per grid file: read `U`, `V` `(coeff, time, Z, Y, X)`, the attributes `Portref`,
  `code_du_port_de_reference`, `isBM`, `spacestep`, `timestep`, the grid origin and step.
- **Which grids to keep:** every 2 km, 500 m and 250 m grid; the 100 m and 30 m port grids only on request (they
  are most of Finistère's 776 MB and matter for a harbour approach, not a passage). Steps: keep 30 min,
  resample 5 min to 30 min except in the races (Raz de Sein, Fromveur, the Four, Alderney, Barfleur), where 15
  min is kept.
- **Format** (`.isoatlas`): a small JSON header (zone, grid, reference port and its id, `isBM`, coefficients,
  phase steps, SHOM credit and DOI) followed by `i16` cm/s for U and V (−32768 for no data), deflated.
  Sampling reads it straight into an `Arc<Atlas>`.
- Print the size per zone, which decides between hosting and bundling (§0.2).

### 5.2 Getting the atlases into the app

With (a): the website serves `atlases/index.json` (zone id, name, bounds, grids, size, sha256, source date) and
the files. The weather page's Currents group lists "Tidal streams (Shom)" zones covering the view, each with
its size and Download/Remove. A route whose passage crosses a zone that isn't installed offers it in the
card ("The Shom atlas for Finistère would resolve the Raz de Sein: 4 MB"), rather than downloading on its own.
Installed atlases go in `<data dir>/atlases/` (they are reference data, not a cache) and are listed in
`CurrentPrefs::atlases`.

### 5.3 Sampling an atlas at a time

1. **The reference port's high (or low, `isBM`) waters** around `t`, from Open Waters tides at the port's
   position (`core/tides`, through its cache). A table in `coefficient.rs` gives each `Portref` a position.
   For a route, fetch them once for the passage's time span plus a day, in `ensure_currents`.
2. **Phase:** take the high water nearest `t`, `phase = t − HW` in hours, kept in −6..+6. When the actual
   interval between the two highs isn't 12.42 h, scale the phase so −6 and +6 meet the low water between
   them, as the paper atlases are used.
3. **Coefficient** from Brest's tide: `C ≈ 100 · range / 6.1 m` for the tide in question. Check the formula
   against SHOM's published coefficients for a dozen dates in a test, and use the coefficient of the tide
   whose high water was chosen in step 2.
4. **Interpolate** linearly in phase between the two nearest steps, then linearly in coefficient between 45 and
   95; outside that range, extrapolate linearly but clamp to 30..120 (the atlases' own advice for big tides is
   to scale by coefficient).
5. Add SMOC's residual (`ResidualU/V`) when it is loaded.

The tide service is a network call. Offline, with no cached extremes for the reference port, the atlas
can't be phased, and the field falls through to the next source (and the card says why).

### 5.4 Tests

- A synthetic atlas: phase and coefficient interpolation reproduce the grid at the steps, the midpoint between
  steps, and coefficient 70 is the mean of 45 and 95.
- The coefficient formula against SHOM's table.
- Nesting: atlas tide + residual inside the zone, NW Shelf outside, a blend at the grid edge, the 250 m grid
  inside the 2 km one.
- Live (`--ignored`): with the Finistère atlas installed, the Raz de Sein at spring HW+3 gives the
  atlas's charted rate.

## 6. Phase 5: tidal gates (departure, waiting, wind against tide)

### 6.1 Departures finer than the scan (`routing::window`)

Scanning every 30 minutes would multiply the work by six. Instead, keep the scan at `WindowPrefs::step_hours`
(3 h by default), then **refine** around the best: `window::refine(scan, around: 3, step: 1 h then 30 min)`
reruns the best three departures at ±1 h, then the best of those at ±30 min. That's about 12 more searches
per model, and the ranking and bars include the refined departures. Do it only when the passage has currents
(peak over 1 kn along the way). Measure on the tidal routebench case (`routebench R20 window`), and record
the time it adds.

### 6.2 Waiting for the stream (`isochrone`)

A front point that reaches a race while the stream is foul either creeps against it or is pushed back into a
visited cell and dropped, so the route that waits for the turn is never found. Add a **hold** candidate per
front node: position unchanged, `sog` 0, `Node::holding = true`, allowed when

- `Settings::allow_hold` (on when currents are used), and
- the boat can stem the current there: the best polar speed at the wind over water (or the engine) is at
  least `|c| + 0.5 kn`. Anchoring where the depth allows comes later, once the router has depths.

A hold lands in its own cell, which is on the edge (open water ahead), so the existing "an edge point may
stay in its cell" rule keeps it. Waiting at the start is what the departure scan is for: a hold is allowed
only after the first hour, or the router just finds a later departure. `Stats::hold_hours`, and
`window::Weights::holding` (per hour, default 0.5: waiting off a headland isn't free). The track draws holds
as a ring with the time ("waits 1 h 20 for the stream").

Check the `passages` tests: with no current the hold candidate isn't generated, so nothing moves.

### 6.3 Wind against tide

`window::Weights` gains `wind_against_tide` (per hour, default 1.0, from `Stats::wind_against_tide_hours`,
with the wind and current thresholds of §2.5). It is a weight, not a router limit; a limit can come later if
the weight isn't enough. The Best-departure row shows it as a chip when non-zero.

## 7. Phase 6: under way

- **Tier C correction (`perf/filter.rs`):** without a speed log, subtract the forecast current (if loaded,
  and only from a tide-resolving source: atlas or shelf) from the ground vector before the sample is used,
  and keep tier C's 0.3 weight. Without a current source, nothing changes.
- **Measured set and drift** with a log and a heading: `ground − (STW along heading)` averaged over windows
  of a few minutes (a degree of heading error is 0.1 kn at 6 kn, so short windows are noise). Shown on the
  instruments page as "Set 075° 1.2 kn".
- **Scoring the current source** like `perf::model_check` scores wind models: `perf::current_check` compares
  the measured set and drift with the field's over the last hours, per source; the weather page shows
  "Currents within 0.3 kn, 15° (Shom)", and an alert when the measured stream is far off the forecast for
  half an hour.

## 8. Later

- **NOAA S-111** (US waters, public domain): needs an HDF5 reader. A small pure-Rust subset (superblock v2,
  object headers, chunked datasets with deflate) would also open the SHOM NetCDF files on the device.
  Worth doing when there's a US user to test with.
- NOAA CO-OPS current predictions (points) as a check on the grid near US stations.
- Ifremer MARS2D 250 m (Brittany): check the licence of each record first.
- Anchoring holds where the depth is known (ENC soundings, EMODnet).

## 9. Risks

- **Copernicus changes the stores** (dataset version, chunking, closing anonymous access). Discovery through
  STAC absorbs version changes; the login fallback (§0.4) covers closing; a live `--ignored` test catches it.
- **Download sizes at sea**: a SMOC box is ~15–30 MB before block ranges. Show the size before downloading,
  skip `utide` where finer sources cover, and keep the cache for the day.
- **The router gets slower**: the corrector and shorter legs. Measure each step with `routebench`; the
  per-time snapshot is the fallback.
- **False precision:** a 9 km SMOC cell in a 2 km race. The card names the source ("Copernicus global 9 km:
  races are smoothed"), and the route card advises the atlas when one exists.
- **Not for navigation:** SHOM's own words go in the about text for the atlas source; the planning-only pill
  already covers the rest.

## 10. Order of work, summarised

| Phase | Ships | Needs |
|---|---|---|
| 1. Router physics | Wind over water, mid-leg current, crab angle in `along`, current stats, `R20` bench | Nothing |
| 2. Source and layer | `currents` module, `WithCurrents`, imported GRIB currents, chart arrows, readout, credits, settings, vm and 3 shells | 1 |
| 3. Copernicus | STAC, Zarr, Blosc with block ranges, SMOC + NW Shelf + IBI + Med/Baltic, `ensure_currents`, plans keep currents | 2 |
| 4. SHOM atlases | Generator, hosting on the website, phase and coefficient, nesting with the SMOC residual | 3 (residual) |
| 5. Tidal gates | Refined departures, hold candidate, wind-against-tide weight | 1, ideally 4 |
| 6. Under way | Tier C correction, measured set and drift, current scoring | 3 or 4 |

When each phase is built, note what it became under a §11 "Built" heading here, with what was measured
(bytes per passage, routing time with and without currents), as `weather-models.md` §5 does.

## 11. Built

1. **Router physics.** *Built 2026-11-09.*
   - `geo::wind_over_water` (with `wind_uv`/`wind_from_uv`, moved out of `routing::nested`, and `set_of`).
   - `isochrone::explore` sails each front node in the wind over the water and reads the current again
     at the middle of each leg that survives the `behind` filter. It skips both when
     `Environment::has_currents` is false, so without currents its arithmetic is unchanged to the bit.
   - `Node::current` records the current met on each leg; saved plans keep it as two extra values on a
     node, written only where the current isn't zero, so plans without one read as before.
   - `along::sail` crabs across a cross current to hold the line, and counts `swept_hours` where it can't.
   - `Stats` gains fair and foul hours, the miles the current gave, the strongest current and hours of
     wind against tide. The statistics table gets a "Current" section only for routes that met a current.
   - `passage::for_currents` and `peak_current_kn` cut legs to 15 min under 300 nm when the stream along
     the way reaches 2 kn. They're not wired in yet (phase 3 has the data); the threshold is still to be
     measured on a real coastal passage.
   - `currents::synthetic::Race` (a made-up M2 race) and `WithRace` back the tests and the routebench
     case `R20` (Camaret to Audierne through a race in the Raz de Sein; `BENCH_NO_RACE=1` to compare).
     A scan test checks that the best departure takes the race with the flood.

   Measured: R11, R12 and R14 `result` lines unchanged. R20 explores in about 120 ms with the race and
   105 ms without, the same 21 µs per searched node, so the mid-leg sample costs nothing measurable; the
   extra time is the extra fronts the foul stream causes.
   R20 `result`: `arrives after 230 min, 27.32 nm; best 03 09:00, ten best 0309:4.8 0212:5.6 0306:5.9 0315:6.2 0412:6.2 0312:6.5 0303:6.7 0409:7.3 0300:7.4 0406:7.6`.

2. **Currents as a source, on the chart.** *Built 2026-11-09.*
   - `currents::{CurrentSource, field::CurrentField, WithCurrents}`. The field nests the sources finest
     first, blends over `BLEND_CELLS` (3) of the finer grid, and falls through where a finer one has no
     value. Atlases go in as `field::Tidal`. `Grid::sample_valid` and `WeatherStore::sample_valid` give
     the last cell before a coast its current. `Param::ResidualU/V` (local 10/1/192, 10/1/193).
   - `flows::routing::ModelWeather` carries `currents` (the field cut to the passage, when routes use
     currents) and `env()` wraps every model in `WithCurrents`, so routes, comparisons, scans and plan
     checks all sail on the same current. `StoreEnv::current` was left as it was: no wind store holds
     currents now, and `WithCurrents` replaces the current rather than adding to it, so nothing counts
     twice.
   - The app opens no GRIB files of its own (only downloads), so there were no imported currents to
     reroute. `CurrentSource::Imported` stays for when it does.
   - `isomar-app/src/currents/`: `CurrentsState` inside `WeatherState` (it follows its time),
     `CurrentLayer` in the weather tile plane (arrows in three weights, numbers from zoom 12,
     `Fill::Current`), the readout chip, credits, the timeline spanning currents alone,
     `ISOMAR_DEMO_CURRENTS`.
   - Design: the overlay-spec rows, `current-arrow` and `current-0…4` tokens, the `current` glyph.
   - `config::CurrentPrefs { route_with, fetch_copernicus, atlases }`. The Copernicus login of the plan was
     not built: anonymous reads work, and a password would need a keyring the app does not have.
   - The weather page's Currents group (sources, arrows, "Sail in the current", "Get currents for this
     view", the atlas zones, the scores under way) and a "Sail in the current" switch in the routing
     panels, in GTK, Swift and Compose. `api` verbs `currents_get`, `currents_arrows`,
     `currents_route_with`, `currents_fetch`, `currents_clear`, `atlas_install`, `atlas_remove`.
   - Saved plans keep only `Node::current` (statistics need nothing more). Resampling the field into the
     plan was dropped: replay shows the currents loaded, not the plan's.

3. **Copernicus Marine.** *Built 2026-11-09.*
   - `currents::blosc`: a Blosc 1 decoder written here (header, block offsets, split streams, LZ4,
     unshuffle), not `lz4_flex`: it is small and adds no dependency. Blocks of a frame are written in the
     order threads finished them, so a block ends at the next larger offset, not at block k+1's.
     Fixtures from `scripts/gen-blosc-fixtures.py` (numcodecs, synthetic values).
   - `currents::zarr`: consolidated metadata, chunk keys, dtypes `<f4 <f8 <i2 <i4 <i8`, CF scale and offset,
     fill values, and the four time units the stores use (hours since 1950, seconds since 1970, minutes
     since 1900, days since 1900).
   - `currents::copernicus`: the dataset table (NW Shelf `cmems_mod_nws_phy-cur_anfc_1.5km-2D_PT1H-i`,
     IBI `…0.027deg-2D_PT1H-m`, Med `…cur_anfc_4.2km-2D_PT1H-m`, Baltic `cmems_mod_bal_phy_anfc_PT1H-i`, its
     surface level picked from 56, SMOC `…merged-uv_PT1H-i`), STAC discovery taking the newest version
     that has a store (the Med's 202611 item had none yet), `choose` (a dataset is skipped where a finer one
     holds the whole area; SMOC then only for an atlas's residual), ranged reads with eight connections,
     and the cropped result cached as GRIB2 keyed by the store's `Last-Modified`.
   - `flows::currents::ensure` in the route and best-departure jobs ("Getting the currents"); past 40 MB
     of estimate, round the passage's ends only. `tide_settings` wires `passage::for_currents` and
     `peak_current_kn` in. The route card names the current it sailed in.
   - The weather planner still says GFS has no currents: true, and currents come by their own flow.

   Measured on 2026-11-09, the Alderney Race (`live_alderney_race`): NW Shelf, 148 hours over
   0.75°×0.5°, 18 chunks, **2.2 MB** in 1.3 s (whole chunks would be about 8.3 MB); SMOC with its tide,
   219 hours, 8 chunks, **1.2 MB** in 1.2 s (about 3.2 MB whole). Peaks 8.0 kn on the shelf model and
   6.0 kn on SMOC. From the cache, 0.14 s. Byte ranges save 2.7–3.7×, less than the 5–10× hoped:
   a block holds 1024 hours, so the forecast's block is mostly analysis.
   The router on that current (`live_route_through_the_alderney_race`, no land, every two hours over a
   tide): 363 min with no current, **194 to 356 min** with it. Beating into the wind, even a foul stream
   raises the wind over the water, so the times swing with the tide rather than straddle the calm one.

4. **Shom atlases.** *Built 2026-11-09.*
   - `scripts/gen-tidal-atlas.py` (h5py) writes `.isoatlas` (JSON header, deflated `i16` cm/s) and
     `atlases/index.json` into `website/atlases/` (gitignored); `deploy-website.sh` publishes it as it is.
     Every 2 km, 500 m and 250 m grid kept, 5-minute steps thinned to 15 minutes, the 100 m and 30 m
     harbour grids only with `--ports`. Sizes: Pas de Calais 5.6 MB, Manche 3.3, Baie de Seine 4.2, Golfe
     normand-breton 2.5, Bretagne Nord 1.1, Finistère 14.1, Gascogne 1.9: **33 MB** in all. Not deployed:
     that is the owner's step (`scripts/deploy-website.sh`).
   - `currents::coefficient` (phase from the reference extreme, scaled so ±6 h meet halfway; Brest's
     coefficient as 100 × half-range ÷ 3.05 m) and `currents::atlas` (sampling, phasing through
     `tides::lookup`, zones, install from the index with each file's SHA-256 checked; `sha2` was already
     in the lock). Le Havre's atlas counts from low water.
   - The app loads and phases the installed zones at start, lists the zones on the weather page, and a
     route over a missing zone says it would help, never downloading it unasked.

   Checked (`live_iroise_atlas`): the Iroise 250 m grid at mean springs gives **6.4 kn** in the Raz de
   Sein (charts: about 6.5) and **7.8 kn** in the Fromveur (about 8). Brest's coefficients from Open
   Waters over the fortnight from 2026-11-10 run 32 to 94 (`live_brest_coefficients`); they were not
   checked against Shom's published table, which was not to hand.

5. **Tidal gates.** *Built 2026-11-09.*
   - `window::REFINE` (the best 3 at ±1 h, then the best at ±30 min) through `around_best`, `scan_since`
     and `Scan::merge`, when the stream along the way reaches `REFINE_PEAK_KN` (1 kn); the same new
     departures on every model, so the scans stay side by side. Departure bars are now laid out by time.
   - The hold candidate (`Settings::allow_hold`, on with currents): after the first hour, where the
     stream is at least 0.5 kn and the boat can stem it with 0.5 kn to spare. A hold is a leg with no
     speed over the ground (`Node::holding`), so neither nodes nor saved plans changed shape.
     `Stats::hold_hours`, the ring and "waits 1 h 20 for the stream" on the chosen route.
   - `Weights::holding` (0.5) and `Weights::wind_against_tide` (1.0), both 0 for "Fastest", sliders in
     the three shells; wind against tide is always among a departure row's chips when there is any.

   Measured on R20: refinement adds 7 departures and 450 ms to a 1.4 s scan, and moves the best from 09:00
   (cost 4.8) to 08:00 (4.2). The wind-against-tide weight alone changes the costs of the departures that
   meet it (0212 5.6 → 5.9, 0315 6.2 → 6.4). No hold is taken in R20: creeping into the stream always
   beats waiting in it there. R11, R12 and R14 unchanged.
   R20 `result` now: `arrives after 230 min, 27.32 nm; best 03 08:00, ten best 0308:4.2 030730:4.3 0307:4.3 030830:4.3 0310:4.5 0309:4.8 0305:5.7 0306:5.9 0212:5.9 0315:6.4`.

6. **Under way.** *Built 2026-11-09.*
   - Tier C correction (`perf::filter`): the forecast current at each window (`ModelWind::current`, only
     from a source that resolves races; schema 7 adds `f_cur_e`, `f_cur_n`) is taken out of the speed and
     the wind.
   - `Sample::stream` (ground less water) on every logged window (`session::Written` became a struct),
     `perf::current_check::CurrentTracker`: set and drift over 5 minutes (the Instruments page's "Set and
     drift" tile and a `set` dashboard readout), each source scored in 5-minute buckets over 6 hours (the
     weather page's Currents group), and an alert (`alerts::Source::CurrentDrift`) when the source in use
     has been more than 1 kn off for half an hour.
   - Leeway counts in the measured stream; nothing corrects for it yet.

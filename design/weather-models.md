# Several forecast models in the weather router

Research and proposal, 2026-11-06. All six phases are built (see §5 for what each became and what was
left out).

Today the router sails on one forecast: NOAA GFS 0.25° (and GFS-Wave), fetched through the NOMADS grib
filter, which crops on the server. This note covers the free models that could join it, what each is good
for, how to get each one, what has to change in the code, and how to show the user several answers to the
same question.

## 1. The models

### Global models (deterministic)

| Model | Centre | Grid as distributed | Runs, length, step | Free source | Licence | Notes |
|---|---|---|---|---|---|---|
| **GFS** | NOAA | 0.25° lat/lon | 4/day, 384 h, 1 h to 120 h then 3 h | NOMADS grib filter (crops on the server); AWS `noaa-gfs-bdp-pds` with `.idx` | Public domain | What we have. One global `Throttle`: ~120 requests/min. |
| **ECMWF IFS** | ECMWF | 0.25°, and 0.1° since 2026-10-20 | 00/12z: 360 h (3 h to 144, then 6 h); 06/18z: 144 h | `data.ecmwf.int/forecasts/…` plus AWS, Azure and Google mirrors; one GRIB2 file per step, with a JSON-lines `.index` giving `_offset`/`_length` per field | CC BY 4.0 | Opened at full resolution on 2025-10-01. CCSDS packing, which our `grib` crate already decodes. **Global fields only, no cropping.** |
| **ECMWF AIFS Single** | ECMWF | 0.25° | 4/day, 360 h, **6 h steps** | Same server, `aifs-single/0p25/oper` | CC BY 4.0 | Machine-learning model, published as soon as it runs, so it lands first. No gusts. |
| **AIGFS** (GraphCast-based) | NOAA | 0.25° | 4/day, 384 h, 6 h | NOMADS `com/aigfs/prod`, with `.idx` byte ranges | Public domain | Went operational in Dec 2025. **No grib-filter product**, so no cropping. |
| **ICON** | DWD | Native icosahedral (~13 km) only | 4/day, 180 h (00/12z) | `opendata.dwd.de`, one bz2 file per parameter per step | DWD open data, credit "Deutscher Wetterdienst" | No lat/lon version: we would have to regrid 2.9 M triangles ourselves. ~3.9 MB per field per step. Probably not worth it given ICON-EU. |
| **ARPEGE** | Météo-France | 0.25° global (GLOB025), 0.1° Europe (EURAT01) | 4/day, 102 h, 1 h then 3 h | data.gouv.fr "Paquets Arpège" (whole packages, 0.7–3.3 GB); Météo-France API portal (crops, needs a free key); AWS `mf-nwp-models` (community mirror) | Etalab Licence Ouverte 2.0 | Good Atlantic and European model; parent of AROME. |
| **GEM GDPS** | ECCC Canada | 15 km lat/lon | 2/day, 240 h | `dd.weather.gc.ca/…/model_gem_global/15km/grib2/lat_lon/` (paths recently gained a date prefix) | Open Government Licence – Canada | One file per variable per step, global. |
| **UKMO Global 10 km** | Met Office | ~10 km | 00/12z: 168 h, 06/18z: 67 h | AWS `met-office-global-deterministic`, **NetCDF**; GRIB2 only from the paid DataHub | Check the licence before use | Often rated close to ECMWF offshore. Needs a NetCDF reader. |
| **BoM ACCESS-G, JMA GSM, KMA, CMA GRAPES** | Australia, Japan, Korea, China | Various | Various | No straightforward free GRIB for routing, except through Open-Meteo (§2) | Various | ACCESS-G would be the useful one for New Zealand and the Pacific. |

### Regional models (high resolution, short range)

They resolve what the global models cannot: acceleration round capes, funnelling in straits, sea breezes,
the coast's effect on gradient wind. They only cover a box and only go out 2–5 days, so each one has to be
**nested in a global parent**.

| Model | Covers | Grid | Length | Source | Parent |
|---|---|---|---|---|---|
| **AROME** | France and its approaches (55.4N–37.5N, 12W–16E) | 0.025° and 0.01° | 42–51 h, hourly | data.gouv.fr packages / API portal | ARPEGE |
| **AROME Outre-Mer** | Antilles, Guyane, Réunion/Mayotte, **Nouvelle-Calédonie, Polynésie** | ~2.5 km | ~48 h | data.gouv.fr "Données PNT" | ARPEGE |
| **ICON-EU** | Europe and the Mediterranean | 0.0625° lat/lon | 120 h (hourly to 78 h, then 3 h) | `opendata.dwd.de/weather/nwp/icon-eu/`, one bz2 file per field per step, ~1.1 MB | ICON (use GFS or ECMWF) |
| **HRRR** | Continental US | 3 km | 18 h (48 h at 00/06/12/18z) | NOMADS grib filter (`hrrr_2d`, crops) | GFS |
| **NAM 3 km / HRDPS** | North America | 3 km / 2.5 km | 60 h / 48 h | NOMADS / ECCC datamart | GFS / GDPS |
| **MEPS (MetCoOp)** | Nordics | 2.5 km | 66 h | MET Norway THREDDS (NetCDF) | ECMWF |
| **UKV** | British Isles | 2 km | 120 h | Met Office (paid DataHub) | UKMO Global |

Our own waters: Brittany has AROME, ARPEGE-Europe and ICON-EU. **New Zealand has no free high-resolution
model** (MetService's and MetOcean's are commercial), so New Zealand passages compare global models only. For
the Pacific crossings New Zealand sailors make, AROME Nouvelle-Calédonie and Polynésie are free and worth having.

### Ensembles

| Ensemble | Members | Grid | Source | Cropped on the server? |
|---|---|---|---|---|
| **GEFS** | 31 | 0.25° to 240 h, 0.5° to 384 h | NOMADS `gefs_atmos_0p25s` / `0p50a` filter | **Yes** |
| **ECMWF ENS** | 51 | 0.25° (and 0.1°) | data.ecmwf.int `enfo` | No: ~0.75 MB per member per field per step, so about 5 GB for 10 days of wind |
| **AIFS ENS** | 51 | 0.25° | data.ecmwf.int `aifs-ens` | No |
| **CMCE (Canada)** | 21 | 0.5° | NOMADS `cmcens` filter | **Yes** |
| **FNMOC (US Navy)** | 21 | 0.5° / 1° | NOMADS `fens` filter | **Yes** |

### Accuracy, honestly

- **No good public study compares 10 m wind over the open sea across all these models.** The WMO Lead Centre
  for Deterministic NWP Verification (run by ECMWF) exchanges standard scores, mostly upper air and over land.
  What follows is the consensus of the published scorecards and of the routing community, not a measurement.
- **Global, days 1–10:** ECMWF IFS leads on almost every headline score. The Met Office model is close
  (PredictWind's claim that it matches ECMWF offshore is uncited marketing). GFS, ICON, ARPEGE and GEM come
  next, roughly half a day to a day of lead time behind ECMWF at days 4–6. The gap has narrowed over the years.
- **The machine-learning models** (AIFS, GraphCast/AIGFS) match or beat IFS on large-scale scores from day 3
  onwards. Their fields are **smoother**: peak winds in fronts, lows and tropical cyclones are underestimated,
  and they come every 6 h, so a front crossing a passage is interpolated across 6 h. Good for "where is the
  high on Thursday", worse for "how hard will it blow at the cape".
- **Regional models** win inside 48 h near coasts and in complex topography, and are no better than their
  parent past about 48 h. Their skill comes from resolving the coast, not from better large-scale dynamics.
- **Ensembles** are the only source of calibrated uncertainty. A handful of deterministic models (a "poor man's
  ensemble") underestimates the spread, but **agreement or disagreement between models** is the signal sailors
  have always used, and it costs far less to download.
- **Which model is "right" depends on the weather of the day** more than on the model. On a given passage the
  best model is not knowable in advance, which shapes the UI below: present the disagreement, help choose a
  route that holds up under all of them, then check the models against the boat's own instruments once under way.

## 2. Getting the data: bandwidth is the real constraint

Rough wind-only size (U and V) for a five-day passage in a 10°×10° box, at 3-hourly steps where available:

| Source | Size | Why |
|---|---|---|
| GFS via NOMADS filter | **~0.3 MB** (41 requests) | Cropped on the server |
| ECMWF IFS 0.25° | **~60 MB** | 41 steps × 1.47 MB, global fields by byte range |
| ECMWF AIFS 0.25° | ~30 MB | 6-hourly, so half the steps |
| ECMWF IFS 0.1° | ~300 MB | 41 × 7.3 MB |
| ICON-EU | ~95 MB | Whole-Europe bz2 files |
| ARPEGE via data.gouv packages | GBs | Packages mix every parameter; the API portal crops but needs a key |
| GEFS 31 members via NOMADS | ~10 MB, but **~1,300 requests ≈ 11 min** under the throttle | Cropped, but one request per member per step |

Measured on 2026-11-05 00z: IFS 0.25° `10u` = 738,617 bytes per step; 0.1° `10u` = 3,660,237; ENS member
`10u` ≈ 750,000.

Ashore on broadband, 60 MB per model is fine. At sea on Starlink it is tolerable; on Iridium or a phone's
roaming data it is not. Options:

1. **Crop on arrival**: decode each global field and keep only the passage box (plus margin) in the cache.
   This doesn't save bandwidth, but the disk cache stays as small as it is now. Do this anyway.
2. **Byte ranges from the index** (`.index` for ECMWF, `.idx` for NOAA): one request per field and only the
   fields we need, never the 100 MB step files. ECMWF allows 500–2000 simultaneous connections, so no throttle.
3. **Open-Meteo's open-data bucket** (`s3://openmeteo`, CC BY 4.0) has ~35 forecast models in one format:
   ECMWF IFS/AIFS, GFS, ICON, ICON-EU, ARPEGE, AROME, UKMO Global and UKV, GEM, JMA, KMA, CMA, **BoM ACCESS-G**,
   the HARMONIE runs, plus ECMWF WAM, GFS-Wave and Météo-France waves and currents. `data_spatial/<model>/<run>/<time>.om`
   files are chunked and compressed, so a sub-area can be read by range. On paper this solves both coverage
   (NZ gets ACCESS-G) and bandwidth. **The blocker is licensing:** the only reader (`omfiles` / `om-file-format`)
   is **GPL-2.0-only**, which can't be combined with our GPL-3.0-or-later, and couldn't carry our App Store
   permission anyway. It would need a clean-room reader written from a format spec, or permission from Open-Meteo.
   Their HTTP API is point-based and free for non-commercial use only, so it isn't a route either.
4. **A cropping proxy** of our own (a small service that range-reads the global files and returns a cropped
   GRIB). It works for every model and is the only way ECMWF becomes cheap at sea, but we would be running
   infrastructure.

Recommendation: ship options 1 and 2 first, show the download size per model in the setup, and default to
models that are cheap in the area (GFS everywhere, regional models when they are small). Look into
Open-Meteo's licence separately (an email to them costs nothing).

## 3. What changes in the code

### The forecast gets a model dimension (core)

- **`weather::store::WeatherStore` mixes sources today.** `add` keys series by grid shape, and `sample`
  takes "the newest run where areas overlap". An ECMWF field cropped to the same 0.25° box as a GFS one would
  replace it; overlapping boxes would make the router sail on GFS in one place and ECMWF in the next. Fields
  need a `Source { model: ModelId, member: Option<u8> }`, and the app keeps a `Forecasts` map of
  `Source → WeatherStore`. Inside one source the current newest-run logic stays as it is.
- **`planner::Model`** becomes a registry entry (`ModelSpec`): id, label, credit line, licence, coverage (a
  box, or global), grid step, run hours and publication delay (`candidate_runs` is hard-coded to 6-hourly with
  4.5 h delay today), step schedule per run (replaces `forecast_hours`' GFS-only rule), parameters it has,
  parent model for nesting, and a **fetcher**: `NomadsFilter` (today's code), `IndexedRanges` (ECMWF
  `.index`, NOAA `.idx`), `WholeFile { bz2 }` (DWD).
- **`cache::Cache`** names files by model as well as by query; `load` restores each into its own source.
- **Nesting** (`routing::isochrone::Environment` for a regional model): a `NestedEnv { inner, outer }` that
  samples the regional model inside its box and before its horizon, the parent elsewhere, and blends linearly
  across the last ~5 grid cells and the last ~3 hours so the router sees no wall of wind at the edge. To the
  user this is one forecast, "AROME → ARPEGE".
- **GRIB**: CCSDS already works (ECMWF). Turn on the crate's pure-Rust `jpeg2000-unpack-with-hayro` and
  `png-unpack-with-png-crate` features for the centres that use them. Rotated or icosahedral grids are out of scope.
- **Credits**: `CreditLayer` names each model shown or routed on (ECMWF's CC BY wording, "Deutscher
  Wetterdienst", Météo-France Etalab).

### Routing on several models

- **One `Search` (land mask, passage area), many explorations.** `flows::routing` already separates the
  passage-wide `Search` from the departure (that is how Best departure shares one mask). Routing K models means
  K calls to `isochrone::explore` with K `StoreEnv`s, run in parallel. The land mask and the way by water are
  computed once.
- **Different horizons.** ARPEGE ends at 102 h, ICON-EU at 120 h, UKMO at 168 h, IFS at 360 h, GFS at 384 h.
  The router already stops at the horizon and reports how far it got (`alternatives::Alternative.arrives`,
  `remaining_nm`). The comparison has to say "ARPEGE: forecast ends 40 nm short" instead of pretending it arrived.
- **Sailing a fixed route through a forecast** (new, `routing::along`): given the saved route's points, a
  departure, the polar and an `Environment`, sail leg by leg (tacking or gybing on the leg when the wind is
  inside the polar's best VMG angle, motoring under the same rule as the router) and return the timed track.
  This is much cheaper than a search (one boat, no fronts), so running it K×K times is cheap. It is what
  gives an ETA bracket for a chosen route (§4.3).
- **Best departure.** `window::scan` per model: D departures × K models searches. With 24 departures and 4
  models that is 96 explorations. Mitigations: run every model on the same departure grid in one parallel pool;
  let the user pick fewer models for this scan; and measure with `routebench` before deciding defaults.
- **`RoutePlan`** keeps the forecast it was routed on, cut to the path. It would keep one per model
  (`Vec<(Source, WeatherStore)>`), so Replay and the ETA bracket still work after the forecasts are gone. Each
  is small once cut to the path.
- **Regression checks**: `passages` and `routebench` stay single-model (GFS testgrib), so their `result`
  lines must not move. New tests use `testgrib` to build two synthetic models that disagree (a veering front
  arriving 12 h apart) and check that the comparison, the bracket and the ranking say what they should.

## 4. The UI

The new dimension is "which forecast". The rule throughout: **a model is a colour, an alternative is a dash
pattern**, so the two never fight (`ALT_COLOURS` × `ALT_DASHES` already exist; the model colours can reuse
the `alt-*` tokens, and an alternative within a model keeps its model's colour with its own dash).

### 4.1 Setup: choosing the models

The "Forecast" row in the Weather route and Best departure panels becomes a **Forecasts** group listing only
the models that cover the passage:

```
Forecasts                                         3 selected
┌──────────────────────────────────────────────────────────┐
│ [x] ● ECMWF IFS  0.25°   run 00 UTC · to Sat 18 Nov  loaded │
│ [x] ● GFS        0.25°   run 06 UTC · to Wed 22 Nov  0.3 MB │
│ [x] ● AROME → ARPEGE     2.5 km 48 h, then 11 km      34 MB │
│ [ ] ● AIFS (AI)  0.25°   6-hourly · to Sat 18 Nov    30 MB │
│ [ ] ● ICON-EU    6.5 km  to Sat 11 Nov               95 MB │
│ Total to download: 34 MB                     About models › │
└──────────────────────────────────────────────────────────┘
```

- The colour dot is the colour that model keeps in every result.
- Each row gives the run age, how far ahead it reaches (dimmed when shorter than the passage), and either
  "loaded" or the download size. On a metered connection the total is in the warning colour.
- The selection is remembered (`Settings::routing_models`), with a sensible first default per region: GFS +
  ECMWF everywhere, plus the regional model where one exists.
- "About models" opens a short page: what each model is good at, in a sentence each (the accuracy notes above).
- Phones: the same list as a sheet, with the rows as toggles.

### 4.2 Weather route results: one track per model

On the chart, one track per model in its colour, the playback boat on each (the timeline already plays them
all), and the selected model's fronts and wind barbs under them. **Where the tracks split is itself the
information**: "three go north of Belle-Île, one goes south" means the choice of side depends on the forecast.

The card:

```
Weather route · 3 forecasts                                   ⋯
Models agree on the way: all pass north of Belle-Île.
Arrival between Sat 14:20 and Sat 19:05 (4 h 45 spread).

● ECMWF IFS     arrives Sat 16:10 · 41 h · max 22 kn · 3 h motor   ✓
● GFS           arrives Sat 14:20 · 39 h · max 19 kn               ✓
● AROME→ARPEGE  arrives Sat 19:05 · 44 h · max 27 kn · 6 h motor

[ Compare ]          [ Discard ]          [ Use ECMWF route ]
```

- The first lines are generated: whether the ways agree (do the tracks share a side of each island or headland
  within N nm, or differ in the side of a weather feature), and the arrival spread.
- Selecting a row focuses that model: its fronts and its wind overlay, and clicking a front adds an alternative
  to *that* model (same colour, new dash).
- **Compare** opens the statistics table with one column per model (the table already takes columns per
  alternative), plus the cross-check below.

### 4.3 "Did I pick the right model?": cross-check and ETA bracket

Your instinct is right, with one change: **the user should not be choosing a model at all, but a route that
holds up under every model.** The way to show that is to sail each candidate route through every forecast
(§3, `routing::along`, which re-sails the fixed route rather than re-routing):

```
                      sailed through
route made on     ECMWF     GFS      AROME→ARP    worst
● ECMWF           41 h      40 h     45 h         45 h   ← most robust
● GFS             43 h      39 h     49 h         49 h
● AROME→ARPEGE    42 h      41 h     44 h         44 h   ← best worst case
```

- The diagonal is each model's own optimum. Off the diagonal is what happens if you follow route A and model
  B turns out to be the true weather. The **worst column** is the decision aid: pick the route whose worst case
  is least bad. The panel marks it and says it in words ("The AROME route never loses more than an hour
  whichever forecast is right; the GFS route loses 10 h if AROME is right").
- With K ≤ 6 that is at most 36 cheap fixed-route sails.
- Comfort counts too: the same matrix can show max wind or wave height instead of hours (a segmented
  control: Time · Max wind · Waves · Motor), because "fastest under ECMWF, but 35 kn if GFS is right" matters
  more than an hour.

Once the user saves a route, its **ETA becomes a bracket** wherever an ETA is shown:

- Route properties / Passage page: "ETA Sat 16:10 (14:20–19:05 over 3 forecasts)".
- On the chart during playback (and in the plan's replay), a **ghost boat per model** along the *same* line,
  so the spread shows as a stretch of track: "by Friday noon you'll be somewhere between here and here".
- The plan keeps the per-model sails, so the bracket survives until the forecasts are refreshed, then is
  recomputed with the new runs. When a new run moves the bracket a lot, that is worth a notice.

### 4.4 Best departure across models

The bar chart has one bar per departure. With several models each departure has K costs. Two views:

**Range bars (default):** each departure's bar becomes a range from its best to its worst model, with a dot
per model in its colour. A short tight bar is a departure that's good whatever the weather does; a tall
spread bar is a gamble.

```
cost
 ▲      ●
 │  ●   ┃  ●          ●
 │  ┃   ┃  ┃   ●  ●   ┃
 │  ┃   ●  ┃   ┃  ┃   ┃      ← tight and low: robust
 │  ●      ●   ●  ●   ●
 └──Fri 06──────Sat 06──────Sun 06──▶
```

**Heatmap (toggle, and the phone default):** rows are models, columns are departures, cells are coloured by
cost with night shaded as now. Agreement shows as vertical stripes of the same colour. A top row "All" shows
the ranking score.

**Ranking** gets a choice: *Typical* (the mean cost across models) or *Cautious* (the worst model's cost).
The list rows add per-model chips: "Sat 12:00 · arrives +38–44 h · ECMWF 26 · GFS 28 · AROME 33". "Preview"
opens that departure in the multi-model route view of §4.2.

### 4.5 Under way: which model is tracking reality

This is where "which model was right" actually gets answered. `perf::forecast_check` already measures the
one model's wind against the instruments (speed bias and rms, direction, by forecast age: hindcast, day 1,
day 2, older). With a model dimension on the samples' forecast wind, the same check runs per model at the
boat's position and time:

```
Forecasts vs. your wind (last 6 h)
● ECMWF   within 3 kn, 10°          ✓ closest
● GFS     8 kn light, backed 25°
● AROME   within 4 kn, 15°
```

- Shown in the weather sidebar page and as a chip on the passage page; when the routed model is clearly
  behind another one for some hours, a notice suggests re-routing on the one tracking best.
- Kept per model and per sea area in the performance store, it becomes the user's own skill table over the
  seasons ("in the Hauraki Gulf, ECMWF has been closest 70% of the time").
- Only meaningful with a wind instrument and a compass/GPS heading; without one the panel says so.

## 5. Phasing

1. **Model dimension, ECMWF IFS and AIFS.** *Built 2026-11-06.* `weather::source::Source` (GFS, IFS, AIFS:
   runs, publication delay, published steps, groups each has); `WeatherState` keeps one store per model and
   shows one; `planner::Fetch::Ranges` reads ECMWF fields by byte range from the step's `.index` and
   `download::crop` cuts them (across the antimeridian too) before `gribwrite` caches them as small GRIB2;
   ECMWF requests skip the NOMADS throttle. A short ECMWF run (06/18 UTC, 144 h) is continued from the run
   before. Routes and scans download a whole-globe model only as far as the passage needs
   (`routing::passage_hours`). Pickers in the Get weather card, the weather page and the routing page (GTK,
   Swift, Compose); the chart label, timeline, stale-forecast alert and credit line name the model; saved plans
   record it. Measured: a 12-hour IFS request for Biscay fetched 16 MB and cached 75 KB; a 450 nm route on
   IFS fetched 46 steps (about 94 MB) in 16 s.
2. **Multi-model weather route + cross-check + ETA bracket.** *Built 2026-11-06.* `routing::along` sails a
   fixed route through a forecast (tacking at the best VMG angle, motoring by the router's rule, counting
   hours over the wind limit); `routing::compare::CrossCheck` sails every model's best way through every
   model's forecast and picks the route whose worst case is least bad, preferring one that keeps under the
   wind limit in every forecast; `compare::agree` says whether the ways part. `flows::routing::compute_models`
   runs one search per model with one land mask; `RouterPreview::runs` holds them, one shown at a time
   (`focus`), the others drawn in their colours with their boats. The card lists the models, says whether
   they agree, the spread of arrivals and the robust route; "Cross-check" opens the matrix. A route saved
   from it keeps `RoutePlan::checks` (the saved route sailed through each forecast), shown as a sentence on
   the route page and a section of its statistics. Measured: a 450 nm route on GFS, IFS and AIFS routed
   in 15 s once downloaded.
3. **Best departure across models.** *Built 2026-11-06.* One scan per model of the same departures,
   ranked by `window::combined_costs` (average, or with `WindowPrefs::cautious` the worst model). Bars carry
   a dot per model and the range between them; rows list each model's cost; previewing a departure routes
   it on every model. No heatmap: the range bars read well enough at phone width. Measured: 25 departures
   of a 120 nm passage on three models in 9 s.
4. **Regional models with nesting.** *Built 2026-11-06, ICON-EU only.* `Source::IconEu` (`parent` GFS,
   `domain` 29.5–70.5 N, 23.5 W–62.5 E), `Fetch::Files` (DWD's whole-domain bz2 files, cropped),
   `routing::nested::NestedEnv` (the regional model inside its area and before its end, GFS around and after,
   blended over the last degree and the last three hours). HRRR is on a Lambert grid the decoder does not
   read; AROME still needs the Météo-France API-key question. ICON-EU is heavy: about 4.5 MB a step for wind,
   gusts and pressure over Europe, so routes leave pressure out of whole-domain downloads.
5. **Model scoring under way.** *Built 2026-11-06.* Each logged window carries the instruments' true wind
   (`Session::perf_written`); `perf::model_check::ModelTracker` compares every loaded model with it over the
   last six hours; the weather page lists them, closest first; `alerts::Source::ForecastDrift` says when
   another model has been clearly closer (a third less error, over half an hour) than the one in use.
6. **Ensembles.** *Built 2026-11-06, as a check of a route rather than routing per member.* "Ensemble check"
   on a route page downloads the 31 GEFS members over the passage through the NOMADS filter (0.5°, about a
   kilobyte a file, 6-hourly) and sails the route through each (`routing::ensemble::spread`): the arrival
   one member in ten beats, half, nine in ten, the strongest wind, how many exceed the limit. Measured: 124
   member files in 74 s. CMCE and FNMOC could join the same way.

Decided 2026-11-06: no cropping server for ECMWF; its fields are fetched whole-globe by byte range and
cropped on the device. Still open: whether to ask Open-Meteo about the reader's licence, and whether the
Météo-France API portal's per-user key is acceptable in the app.

## Sources

- ECMWF open data: https://www.ecmwf.int/en/forecasts/datasets/open-data, files at https://data.ecmwf.int/forecasts/
- Open-Meteo ECMWF page (IFS open at full resolution since 2025-10-01): https://open-meteo.com/en/docs/ecmwf-api
- NOAA AI models: https://www.nco.ncep.noaa.gov/pmb/products/aigefs/, https://emc.ncep.noaa.gov/users/verification/global/aigfs/prod/main.php, https://www.unidata.ucar.edu/news/ai-driven-global-model-output-availability
- NOMADS grib filters (gfs, gefs, cmcens, fens, hrrr): https://nomads.ncep.noaa.gov
- DWD open data: https://opendata.dwd.de/weather/nwp/icon/grib/, https://opendata.dwd.de/weather/nwp/icon-eu/grib/
- Météo-France: https://www.data.gouv.fr/datasets/paquets-arpege-resolution-0-25deg, https://www.data.gouv.fr/datasets/paquets-arome-resolution-0-01deg, https://www.data.gouv.fr/datasets/donnees-pnt-retention-14-jours, https://registry.opendata.aws/meteo-france-models/
- ECCC GDPS: https://eccc-msc.github.io/open-data/msc-data/nwp_gdps/readme_gdps-datamart_en/
- Met Office on AWS: https://registry.opendata.aws/met-office-global-deterministic/
- Open-Meteo open data: https://github.com/open-meteo/open-data, reader licence https://github.com/open-meteo/rust-omfiles
- WMO Lead Centre for Deterministic NWP Verification: https://wmolcdnv.ecmwf.int
- Ensemble routing: https://navimetrix.com/en/faq/routing/how-to-launch-and-interpret-an-ensemble-routing/, https://trid.trb.org/View/2773652

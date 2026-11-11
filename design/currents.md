# Currents in the weather router

Research, 2026-11-09. Nothing is built yet; the implementation is to be planned from this note.

For a small yacht the current often matters as much as the wind: a 2–3 kn tidal stream is half the boat's
speed, and in the Channel or round Brittany the time you reach a race decides the passage. This note
covers what the router already does with currents, what it gets wrong, the free current forecasts that
exist, how good they are, and what fetching them costs.

## 1. What the code does today

The arithmetic is half there; the data is not.

- `routing::isochrone::Environment::current` returns the surface current as (east, north) knots. The
  isochrone adds it to each heading's water vector to get speed and course over ground, and records
  `heading` apart from `cog` (`isochrone.rs`, the candidate loop). `MAX_CURRENT_KN` (10 kn) bounds the
  fastest leg. A test checks that a favourable current shortens a passage.
- `routing::along::sail` (route replay, comparisons) projects the current on the course: along it helps
  or hinders, across it the helm is assumed to make up for it.
- `routing::nested` passes the current through (inner model where it has one, outer otherwise);
  `Bounded` and `StoreEnv` forward it.
- `WeatherStore::current` reads GRIB2 discipline 10 parameters 1/2 and 1/3 (`Param::CurrentU`,
  `CurrentV`, `Group::Current`).
- The download planner reports `Group::Current` as unavailable (`weather/planner.rs`): only a GRIB the
  user imports could bring currents in. Nothing draws currents on the chart.

### What is missing or wrong

1. **No source.** See §2.
2. **The polar is fed the wind over the ground.** Forecast wind is relative to the ground; the boat sails
   in the wind relative to the water. The true wind for the polar is `wind − current` (as vectors). With
   15 kn of wind against a 3 kn stream the boat sails in 18 kn, at a different angle. This changes the
   speed, the wind angle, and the `max_wind_kn` limit.
3. **One sample per leg.** The current is read at the start of each leg. A 30-minute leg is about 3 nm;
   Raz de Sein, Fromveur or the Alderney Race are 1–2 nm wide and turn within the hour. Read the current
   at mid-leg (predictor–corrector: step with the start value, sample at the midpoint, step again), or cut
   legs shorter where the current changes fast across or along them.
4. **Time interpolation is linear in u, v between fields.** Fine for hourly or finer data. With 3-hourly
   data a semi-diurnal stream is badly aliased, so a current source must be hourly or better.
5. **Tidal gates.** The best-departure scan (`routing::window::departures`) steps every 3 hours, a
   quarter of a tidal cycle; with currents it needs 30–60 minutes. The isochrone also has no way to wait:
   no "anchor" or "heave to" candidate for the stream to turn (anchoring only where the depth allows).
6. **Wind against tide.** Short, steep seas where the wind blows against a strong stream. A candidate cost
   for the weather window ranking (hours of wind over X kn against a current over Y kn) and maybe a limit.
7. **Separate from the wind model.** A route sailed on GFS and on ECMWF (`compute_models`, `along`,
   `compare`) should use the same current field. Currents are their own source, not a property of each
   wind model.
8. **Under way.** Without a speed log the performance log learns from speed over ground
   (`perf/filter.rs`, Tier C), so it learns the tide as a polar error; a current forecast could correct
   that. With a log, GPS SOG/COG against STW and heading gives the measured set and drift, which could
   score the current source as `perf::model_check` scores the wind models.

## 2. The sources

| Source | Area | Grid | Tides | Length, step | Access | Licence |
|---|---|---|---|---|---|---|
| **Copernicus SMOC** (`GLOBAL_ANALYSISFORECAST_PHY_001_024`, dataset `cmems_mod_glo_phy_anfc_merged-uv_PT1H-i`) | Global, 80°S–90°N | 1/12° (~9 km) | Yes, as separate variables | 10 days, hourly, new run daily | Anonymous ARCO Zarr on S3 (checked) | Copernicus Marine: free, commercial use allowed, credit required |
| **Copernicus NW Shelf** (`NWSHELF_ANALYSISFORECAST_PHY_004_013`, `cmems_mod_nws_phy-cur_anfc_1.5km-2D_PT1H-i`, also `_PT15M-i`) | 46–62.7°N, 16°W–13°E | 1.5 km (0.0135° × 0.0303°) | Yes (AMM15, tidal) | ~6 days, hourly instantaneous and 15 min | Same | Same |
| **Copernicus IBI** (`IBI_ANALYSISFORECAST_PHY_005_001`, `cmems_mod_ibi_phy_anfc_0.027deg-2D_PT1H-m`, also `_PT15M-i`) | 26–56°N, 19°W–5°E | 1/36° (~3 km) | Yes | 10 days, hourly means and 15 min | Same | Same |
| Copernicus Med (`MEDSEA_ANALYSISFORECAST_PHY_006_013`) | Mediterranean | 4.2 km | Weak tides | Hourly and 15 min | Same | Same |
| Copernicus Black Sea, Marmara, Baltic | Regional | 2.5 km, 500 m, ~2 km | Little or no tide | Hourly and 15 min | Same | Same |
| **SHOM tidal-stream atlases** ("Atlas de courants de marée 2D au format netcdf") | Pas de Calais, Manche, Baie de Seine, Golfe normand-breton, Bretagne Nord, Finistère, Gascogne | 2 km down to 30 m | Tide only | Valid on any date; 5, 30 or 60 min steps | NetCDF4 in 7z on data.gouv.fr / services.data.shom.fr | Etalab Licence Ouverte 2.0, credit "Shom", DOI 10.17183/ATLASCOURANTS2D_NETCDF |
| NOAA OFS as S-111 (`noaa-s111-pds` on AWS) | US coast and Great Lakes: cbofs, dbofs, gomofs, ngofs2, nyofs, sfbofs, tbofs, wcofs, the Great Lakes, rtofs_east/west | ~0.01° and coarser | Yes | 36–48 h (120 h for two Great Lakes), 4 runs a day | Public S3, S-111 HDF5, regular-grid (DCF-2) files available | Public domain |
| NOAA RTOFS (global HYCOM) | Global | 1/12° | Unclear; the public 2D files are 3-hourly | ~8 days | NOMADS, NetCDF | Public domain |
| NOAA CO-OPS current predictions | US stations | Points | Yes (harmonic) | Any date | Keyless JSON API | Public domain |
| Ifremer MARC (MARS2D 250 m: Bretagne Sud, Manche Ouest…) | Brittany and western Channel | 250 m | Yes, plus surge | Forecasts, 15 min | THREDDS `tds1.ifremer.fr` (not checked) | Some records Licence Ouverte, others view-only |
| Open-Meteo marine | Global, points | 0.08° (it is SMOC) | Yes | 10 days, hourly | Point API, many points per call | Free API non-commercial; "not suitable for coastal navigation" |

ECMWF open data and GFS carry no ocean currents.

### Copernicus SMOC

SMOC (Surface Merged Ocean Currents) adds three models and keeps them apart:

- `uo`, `vo`: general circulation (Mercator GLO12, no tides);
- `utide`, `vtide`: tidal current (FES2014);
- `vsdx`, `vsdy`: Stokes drift from the wave model (MFWAM);
- `utotal`, `vtotal`: the sum.

All in m/s, at 0.49 m depth. The STAC item gave a time extent ending 2026-11-18T23:00Z when checked on
2026-11-09: a 10-day forecast.

Stokes drift moves the top few decimetres; a keelboat's hull sits deeper. Take `uo + utide` rather than
`utotal`.

CLAUDE.md says to avoid FES (no redistribution). Fetching SMOC at run time under the Copernicus licence
does not redistribute FES, but this is the owner's call.

### Copernicus access

The STAC catalogue (`https://stac.marine.copernicus.eu/metadata/<PRODUCT>/product.stac.json`, then
`<dataset>/dataset.stac.json`) lists each dataset's assets. Two ARCO Zarr (v2) stores per dataset, both
readable anonymously over HTTPS on `s3.waw3-1.cloudferro.com`; listing is refused (a missing chunk answers
403), so chunk keys are computed. Compression is Blosc with LZ4, shuffle, float32. The server answers byte
ranges. The official toolbox wants an account, but it reads the same Zarr stores on the client: there is
no server-side subsetting either way.

| Store | SMOC chunk | NW Shelf chunk | IBI chunk |
|---|---|---|---|
| `timeChunked.zarr` (one hour, a wide area) | 1 h × 512 × 2048 cells (~43° × 171°), ~590 KB per variable | 1 h × whole domain (1240 × 958), ~930 KB | 1 h × whole domain (1078 × 871) |
| `geoChunked.zarr` (a small area, every hour) | 4272 h × 16 × 8 cells (1.33° × 0.67°), ~405 KB | 3720 h × 16 × 16 cells (0.22° × 0.48°), ~460 KB | 3336 h × 16 × 16 cells |

Sizes measured on 2026-11-09. The chunks are rewritten daily (`last-modified` that day). Time is in
`hours since 1950-01-01`; the time axis of a store spans years of analysis plus the forecast.

What a passage costs:

- SMOC, a 200 nm box (~4° × 4°), geo-chunked, two variables: ~15 MB; four variables (`uo`, `vo`,
  `utide`, `vtide`): ~30 MB.
- NW Shelf, a 1° × 2° box (a Channel crossing), geo-chunked, two variables: ~23 MB.
- Time-chunked over 6–10 days of hourly steps: hundreds of MB. Not usable.

Byte ranges could cut the geo-chunked figures 5–10×: a Blosc chunk is compressed in independent blocks
with their offsets in the header, and in a time-major chunk the forecast hours are one run of blocks. That
needs a small Blosc + LZ4 decoder in Rust (e.g. `lz4_flex` plus the Blosc header).

### SHOM atlases

Checked on the Bretagne Nord file (`ATLASDYN-SURF_TELEMAC2D_R1124100_ROSCOFF-563-250M-30min.nc`):

- Dimensions `coeff = 2` (45 mean neaps, 95 mean springs), `time` = 13, 25 or 145 steps (1 h, 30 min or
  5 min) from HW−6 h to HW+6 h at the reference port, `Z = 1`, regular WGS84 grid.
- Variables `U`, `V` (m/s, fill −9999), `(coeff, time, Z, Y, X)`.
- Attributes: `Portref` (e.g. ROSCOFF, BREST, CONCARNEAU), `code_du_port_de_reference`, `isBM` (whether
  the reference is low water), `spacestep`, `timestep`.
- Built from TELEMAC 2D/3D runs on irregular meshes refined round islands and capes, interpolated to
  regular grids of 2 km, 500 m, 250 m, 100 m or 30 m. Several nested grids per zone (Finistère: 500 m,
  Iroise 250 m, Rade de Brest 100 m, Port de Brest 30 m, 5-minute steps).

Download sizes (7z): Bretagne Nord 13 MB, Golfe normand-breton 16 MB, Gascogne 23 MB, Manche 44 MB, Pas de
Calais 86 MB, Baie de Seine 94 MB, Finistère 776 MB.

To use one at a given time: find the reference port's high water before and after (the app already has
tide extremes from the Open Waters tides service, `core/tides`), the hour relative to it, and the day's
coefficient (from the Brest range). Then interpolate in time between steps and in coefficient between 45
and 95, as the paper atlases are used. SHOM: "A n'utiliser qu'en complément des cartes et ouvrages
nautiques officiels."

These are too big to fetch at run time as published, but Etalab allows redistribution with credit: an
xtask could cut them to the grids worth keeping and ship them, or the app could fetch per zone on demand.

## 3. Accuracy

- **The tide is the predictable part; grid size limits it.** A 9 km cell smears a race a few km wide and
  badly underestimates its peak. The 1.5–3 km shelf models do better (the Met Office reported a better
  tidal signal against HF radar after moving to 1.5 km); SHOM's 250 m and 30 m grids resolve the races.
- **Non-tidal currents** (wind-driven flow, eddies, meanders) have less skill. Model surface currents
  compared with ADCP in Galway Bay were within ~12–14 cm/s where the tide dominates, with much poorer
  agreement on residual flows. Good for strategy (Gulf Stream, Agulhas, Canaries current), noisy for
  tactics.
- **The atlas method** ignores wind-driven current, surge and the difference between a day's two tides;
  small in the Channel and Biscay.
- No source is certified for navigation.

## 4. A possible shape (to plan)

- Currents as their own source in the weather state, beside the wind models, used by every wind model's
  route.
- Nested like ICON-EU in GFS (`routing::nested`), the finest that covers the point winning:
  SHOM tide + SMOC `uo/vo` (no tide counted twice, since SMOC keeps it apart) → NW Shelf or IBI total →
  SMOC `uo + utide`. NOAA S-111 for US waters.
- Router: wind over water for the polar (§1.2), mid-leg sampling (§1.3), 30–60 min departure steps when
  currents matter (§1.5), then maybe a wait candidate and a wind-against-tide cost.
- A current layer on the chart (arrows, per `overlay-spec.md`) and in the route statistics.
- Credits: "E.U. Copernicus Marine Service Information" and "Shom" in `CreditLayer`.

## Sources

- [Copernicus global product](https://data.marine.copernicus.eu/product/GLOBAL_ANALYSISFORECAST_PHY_001_024),
  [user manual](https://documentation.marine.copernicus.eu/PUM/CMEMS-GLO-PUM-001-024.pdf),
  [SMOC poster](https://marine.copernicus.eu/sites/default/files/wp-content/uploads/2019/04/Poster_SMOC_EGU2018APRIL.pdf),
  STAC catalogue at `stac.marine.copernicus.eu`
- [NW Shelf product](https://data.marine.copernicus.eu/product/NWSHELF_ANALYSISFORECAST_PHY_004_013),
  [NWS producer](https://marine.copernicus.eu/about/producers/nws-mfc),
  [IBI product](https://data.marine.copernicus.eu/product/IBI_ANALYSISFORECAST_PHY_005_001),
  [IBI user manual](https://documentation.marine.copernicus.eu/PUM/CMEMS-IBI-PUM-005-001.pdf)
- [Copernicus sign-up](https://help.marine.copernicus.eu/en/articles/4220332-how-to-sign-up-for-copernicus-marine-service),
  [citing Copernicus Marine](https://help.marine.copernicus.eu/en/articles/4444611-how-to-cite-copernicus-marine-products-and-services),
  [CMEMS licence](https://eds.hub.copernicus.eu/licences/CMEMS-licence),
  [cmemsarco (anonymous Zarr reads)](https://github.com/hypertidy/cmemsarco)
- SHOM atlases: [Finistère](https://www.data.gouv.fr/datasets/atlas-de-courants-de-maree-2d-au-format-netcdf-finistere),
  [Manche](https://www.data.gouv.fr/datasets/atlas-de-courants-de-maree-2d-au-format-netcdf-manche),
  [Pas de Calais](https://www.data.gouv.fr/datasets/atlas-de-courants-de-maree-2d-au-format-netcdf-pas-de-calais),
  [Iroise 250 m record](https://services.data.shom.fr:/geonetwork/srv/api/records/ATLASDYN-SURF_TELEMAC3D-SURF_R1111100_IROISE-560-250M-15min.xml);
  the product description PDF inside each 7z
- [Ifremer MARS2D Bretagne Sud 250 m](https://sextant.ifremer.fr/geonetwork/srv/api/records/870ebddf-71ca-4e35-9c12-44d64d916f11)
- [Open-Meteo marine API](https://open-meteo.com/en/docs/marine-weather-api)
- [NOAA S-111 on AWS](https://noaa-s111-pds.s3.amazonaws.com/README.html),
  [CO-OPS current stations](https://mapservices.weather.noaa.gov/static/rest/services/NOS_Observations/CO_OPS_Products/MapServer/4)
- [RTOFS on ERDDAP](https://coastwatch.pfeg.noaa.gov/erddap/griddap/ncepRtofsG2DFore3hrlyProg),
  [RTOFS v2.5 notice](https://www.weather.gov/media/notification/pdf_2025/scn25-47_RTOFS_v.2.5.pdf),
  [RTOFS v3 notice](https://www.weather.gov/media/notification/pdf_2026/pns26-17_RTOFS_v3.pdf)
- [Galway Bay model and HF radar against ADCP](https://www.codar.com/images/about/2015ODonncha_JMS.pdf),
  [EMODnet: verifying surface-current forecasts with HF radar](https://emodnet.ec.europa.eu/en/use-case/verification-high-resolution-ocean-surface-velocity-forecast-using-hf-radar-data)

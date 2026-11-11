# Chart symbols

Navigation aids and chart data are drawn in the style of French (SHOM) paper charts, which follow the international INT 1 symbology: coloured light flares, big arcs for light sectors, buoy and beacon shapes with topmarks, buff land, blue depth tints, italic soundings. Meaning follows INT 1 and the S-52 "paper chart" symbol set exactly; only the drawing is ours.

## Rights and sources

- **SHOM Ouvrage 1D** (the French edition of INT 1, free to download) is a copyrighted SHOM publication. Free download is not a licence to reuse it, and its artwork is not under SHOM's Licence Ouverte (that covers listed datasets such as the buoyage shapefiles, not this book). **Do not trace, extract or copy symbols from 1D.** Use it to read, as a visual reference.
- **The symbology itself** (what a cardinal topmark or a light flare looks like) is an international standard, IHO S-4 / INT 1, that every chart plotter implements. Drawing our own glyphs to that specification is how OpenCPN and others do it.
- **U.S. Chart No. 1** (NOAA / NGA) shows the same INT 1 symbols and is a U.S. government work in the public domain: the reference to measure glyphs against.
- Every glyph in this system is drawn from scratch to those specifications and can be shipped under the app's own licence. This is not legal advice; for certainty, SHOM's licensing office can confirm in writing.

## Relation to S-52

- ENC cells (S-57) use the S-52 **paper chart point symbols** lookup and **symbolized boundaries**, with **full-length light sectors** available as an option. These are S-52's own INT 1-derived choices, so symbol meaning is untouched.
- Colour is the one deliberate departure: Day uses the INT 1 paper colours below instead of the S-52 DAY table. The S-52 colour tables stay available (Preferences → Chart style: Paper / ECDIS). Dusk and Night use the same dimming logic as S-52 DUSK/NIGHT.
- The open-data planning map (OSM seamarks, coastline, depths) uses exactly the same glyphs and colours, so a sailor sees one chart whatever the source.

## Lights

| Element | Spec |
|---|---|
| Position | A 1.5mm dot in `chart-ink` (the light's exact position). |
| Flare (the "tear") | `flare-length` teardrop, point at the position, pointing 135° (south-east) by default; filled `light-white` (white and yellow lights), `light-red`, `light-green` or `light-other`, with a 0.5px `chart-ink` outline. Multicolour lights get one flare per colour, fanned 15° apart. Several lights on one structure fan out clockwise. |
| Sector light | Arc band `stroke-sector-arc` at `sector-radius`, in each sector's colour, outlined by a 0.5px `chart-ink` line on both edges; sector limits as dashed `chart-ink` lines from the light, drawn to the arc (or to the nominal range with Full-length light sectors). Obscured sectors: dashed thin `chart-ink` arc. |
| All-round major light | Flare plus, at large scale or when selected, a full 360° ring at `sector-radius` in the light colour (S-52 major lights rule). |
| Directional / leading lights | The leading line in solid `chart-ink` within the sector, dashed beyond; the narrow sector drawn as above. |
| Characteristic text | `chart-light` beside the light at large scale or when selected ("Fl(3)WRG.12s15M"). At smaller scales it lives in the aid popover; this keeps the old no-labels-on-aids decision except when there is room. |

## Buoys (IALA region A)

Buoys lean 15° (INT 1 style); beacons stand upright. Height `buoy-height`, a small open circle at the base marks the position. Colours: `mark-red`, `mark-green`, `mark-yellow`, `mark-black`, `mark-white`, outlined in `chart-ink`.

| Mark | Shape | Colour | Topmark |
|---|---|---|---|
| Port lateral | Can (cylinder) | Red | Red can |
| Starboard lateral | Cone (conical) | Green | Green cone, point up |
| North cardinal | Pillar or spar | Black over yellow | Two black cones, points up |
| East cardinal | Pillar or spar | Black, yellow band, black | Two cones, base to base |
| South cardinal | Pillar or spar | Yellow over black | Two cones, points down |
| West cardinal | Pillar or spar | Yellow, black band, yellow | Two cones, point to point |
| Isolated danger | Pillar or spar | Black, red band, black | Two black spheres |
| Safe water | Sphere, pillar or spar | Red and white vertical stripes | Red sphere |
| Special mark | Any shape | Yellow | Yellow X |
| Emergency wreck marking | Pillar or spar | Blue and yellow vertical stripes | Yellow upright cross |

Lit buoys get a flare from the buoy's position. Colour bands are horizontal bands across the leaning body; stripes are vertical.

## Beacons

Upright: stake or perch (single line), lattice or tower beacon (narrow trapezoid outline), cairn. Same colour bands and topmarks as the buoy table. A light on a beacon or tower is shown by its flare from the structure's position.

## Depths, land and dangers

| Element | Spec |
|---|---|
| Land | `chart-land`, built-up `chart-land-built`, coastline `chart-coast` 1px |
| Drying | `chart-drying`, drying heights underlined (`sounding`) |
| 0 to 5 m | `chart-shallow` |
| 5 m to safety contour | `chart-medium` |
| Deeper | `chart-deep` |
| Contours | `chart-contour` 0.5px with depth labels; safety contour `chart-safety-contour` 1.5px |
| Soundings | `sounding` italic in `chart-sounding`, decimetres as a smaller subscript; depths shallower than the safety depth in `chart-ink` bold |
| Rocks, wrecks, obstructions | INT 1 glyphs in `chart-ink`; dangers inside the safety contour get the dotted danger line |
| Restricted areas, cables, pipelines | `chart-magenta` (INT 1). This is why overlays never use magenta. |

## Night and Dusk

Chart colours keep their meaning in every palette: red, green and yellow lights stay red, green and yellow, dimmed (`light-red` night ≈ luminance 0.08). Land and water darken so the chart stays the darkest thing on screen; black marks get a `chart-ink` outline so they don't vanish into the dark sea.

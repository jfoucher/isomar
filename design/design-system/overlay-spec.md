# Overlay spec

Everything Chart draws over the chart, rendered by the shared renderer, identical on GNOME and macOS. Three rules apply to every item:

- **Halo.** Every stroke and label sits on `overlay-halo` at `stroke-halo` each side. Chart (S-57/S-52 and INT 1) lines never have halos.
- **No magenta.** No overlay uses a hue from 280° to 340°. `chart-magenta` and `light-other` are the chart's.
- **Upright labels** in `chart-label` / `chart-label-strong`; the chart's own names are italic.

## Own ship

| Element | Spec |
|---|---|
| Symbol | Pointed hull outline, `ownship-length`, filled `own-ship`, aligned to heading (COG if no heading). Above true scale, the hull is drawn to scale from boat length and beam. |
| Heading line | `own-ship`, 1.5px, to the edge of the viewport (option: 3 min ahead). |
| COG/SOG vector | `own-ship`, `stroke-vector`, length = distance run in the vector time (default 6 min) with a tick each minute; open arrowhead. |
| Track trail | `own-ship-trail`, dotted 1.5/3, last 2 h by default. |
| Fix stale (> 30 s) | Hull outline only (no fill), dashed, `stale`; a "?" beside it and the age ("Fix 2 min old") in the status. |
| Fix lost (> 2 min) | As stale, plus a warning (alarm when navigating a route). Last known position kept. |

## Routes and waypoints

| State | Line | Waypoints |
|---|---|---|
| Inactive | `route-inactive`, `stroke-route`, dashed 6/4 | 7px rings |
| Active | `route`, `stroke-route-active`, solid; past legs at 40% | Next waypoint filled `route` with its number in `on-route`; arrival circle dashed `route` at the arrival radius |
| Being edited | `route`, `stroke-route`, solid | 12px square handles with `overlay-halo`; a "+" midpoint handle on each leg; leg readout follows the pointer |
| Selected | Any of the above plus a 3px `selection` ring outside the halo | |
| XTE corridor | `route-corridor` fill between dashed edges at ± XTE limit, active leg only | |

Leg labels (bearing and distance, "247° · 12.4 nm") sit on a `route` chip with `on-route` text, centred on the leg, only on the active route or on hover, and only when the leg is longer than the label plus 48px.

Standalone marks: a `route` square with a dot, name to the right.

## AIS

| State | Symbol |
|---|---|
| Class A, under way | Outline isosceles triangle `ais` pointing to COG/heading, `stroke-vector`; COG vector (same time as own ship) |
| Class B | Same, 20% smaller |
| At anchor / moored | Outline circle `ais` |
| AtoN, SART | AtoN: diamond `ais`; SART: circle with cross in `alarm` |
| Dangerous (CPA/TCPA inside limits) | Filled triangle `ais-danger`, blinking `danger-blink`; CPA line from both vessels to the CPA points (dashed `ais-danger`); name in `chart-label-strong` |
| Acknowledged danger | Filled `ais-danger`, steady, no blink |
| Lost | `ais-lost` dashed outline with a cross; removed after 10 min |

Labels: name (or MMSI), shown on hover, when selected, when dangerous, or always at large scale if there is room.

## Anchor watch

Anchor symbol (`own-ship`) at the drop point, swing circle dashed `anchor-circle` at the alarm radius, a faint track inside the circle. Dragging: circle solid `alarm` 3px, the boat outside it, and the alarm level fires.

## Measure

Line `measure` 2px with an open circle at each end; a readout chip (`card-bg`, `body-numeric`) at the pointer: "247° T · 3.82 nm". Bearing true or magnetic per the units preference.

## Weather

| Layer | Spec |
|---|---|
| Wind-speed field | `wind-00` … `wind-48` at 55% opacity, smooth-banded; never stroked. Legend shows numbers in knots. |
| Wind barbs | `wind-barb` 1.5px, standard barbs (half barb 5 kn, full 10 kn, pennant 50 kn) on a grid every 48px; arrow mode for beginners. Each barb has the halo. |
| Gusts | Optional number under the barb in `caption`; never a second colour field. |
| Isobars | `isobar` 1px, every 4 hPa, labels on the line ("1016"); H and L centres in `chart-label-strong`. |
| Waves | `wave-0` … `wave-8` field (replaces wind field when chosen; only one field at a time) plus direction arrows. |
| Currents | `current-arrow` arrows pointing the way the stream sets, on a grid every 48px offset from the barbs by half a step; three weights by speed (under 1 kn 1px and 14px long, 1–3 kn 1.5px and 22px, over 3 kn 2.5px and 30px), each with the halo; under 0.1 kn nothing. From zoom 12 the speed in `caption` at the tail ("2.4"). |
| Current field | `current-0` … `current-4` at 55% opacity, only one field at a time (replaces wind or waves when chosen). |
| Pointer readout | Card at the pointer: wind dir/speed/gust, waves, pressure, the current with its source, and the forecast time with its age. |

Stale forecast: field is hatched with a 45° `stale` line pattern and the timeline shows "Forecast run 06 UTC, 31 h old".

## Weather router

| Element | Spec |
|---|---|
| Past fronts | `isochrone`, `stroke-front`, every step |
| Front at the clock | `isochrone-now`, `stroke-front-now`, haloed; hover shows a ghost track to that point; click creates an alternative |
| Alternatives | `alt-1` … `alt-6`, `stroke-route`, each with its dash pattern (see token usage) so they are told apart without colour; legend chip in the card |
| Boat at the clock | `router-boat` small hull on each alternative, oriented to its heading |
| Destination | `route` waypoint ring with a flag |
| Failure | "No way forward" / "Forecast ends before arrival": the affected part of the track in `stale`, dashed, with an inline warning in the card |

## Selection and hover

Hover: `hover` disc (hit-min) behind the object and the pointer cursor. Selected: `selection` ring outside the halo; the object's own colour does not change. Every clickable overlay has a hit area of at least `hit-min` (planning) or `hit-min-underway`.

## Map furniture

- **Footer**: one strip at the bottom of the chart holds, left to right, the scale bar (nm with metres on a second scale, ink `own-ship`), the data credits (`caption`, `text-muted`, collapsible to "© OpenStreetMap and 4 more"), and the not-for-navigation pill. Nothing overlaps; the credits truncate first.
- **Not for navigation**: a `radius-full` pill, `warning` glyph + "Planning only" in `caption-heading`, shown whenever no licensed official ENC covers the view; tapping it explains why. Quiet, never a banner.
- **North / course up**: a compass button in the map control cluster; shows the chart's rotation; click toggles north-up / course-up.
- **Grid labels**: `chart-grid`, whole minutes, inside the viewport edge with a halo.

## Label priority

When labels collide, the lower priority label is hidden (never moved onto a symbol). From highest:

1. Alarm objects (dangerous AIS name, anchor drag readout)
2. Own ship (vector time, fix age when stale)
3. Active route: next waypoint name, then active leg label
4. AIS names (dangerous first, then by range)
5. Selected / hovered object
6. Other waypoint names and inactive route leg labels
7. Weather labels (isobar values, H/L)
8. Chart text (S-57 names, soundings, light characteristics)
9. Grid labels

Labels never cover own ship, a waypoint symbol or an aid symbol. Priority 1–3 are placed first with up to 8 candidate positions; 4 and below try 4 positions, then hide. Collision boxes include the halo.

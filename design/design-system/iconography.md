# Iconography

GNOME uses symbolic icons (16px, monochrome, recoloured by the theme) from the Adwaita icon theme; macOS, iOS and iPadOS use SF Symbols (SF Symbols 4, the set available on macOS 13 and iOS 16); Android uses Material Symbols Rounded (weight 400, optical size 24, grade 0 in Day and −25 in Dusk and Night so light-on-dark icons don't look heavier). Every concept below has one name in each set, or a custom glyph drawn once per platform style. Icons are never the only label: every icon button has a tooltip and an accessible name, and text labels appear wherever there is room.

Names marked † were added after the oldest platform release Chart targets or come from outside the core set; check them in the Adwaita icon theme shipped with GNOME 48, the SF Symbols app (filtered to macOS 13 and iOS 16) and the Material Symbols catalogue before use, and fall back to the custom glyph if absent.

## Concepts

| Concept | Where it appears | GNOME (Adwaita symbolic) | macOS, iOS, iPadOS (SF Symbols) | Android (Material Symbols) |
|---|---|---|---|---|
| Sidebar | Header bar, toolbar | `sidebar-show-symbolic` | `sidebar.left` | — |
| Main menu | GNOME header bar; action capsule on touch | `open-menu-symbolic` | menu bar on macOS; `ellipsis.circle` on iPhone and iPad | `more_vert` |
| Layers | Header, map controls | custom **Layers** | `square.3.layers.3d` | `layers` |
| Follow boat (off / on) | Map controls | custom **Follow** (outline / filled) | `location` / `location.fill` | `navigation` (fill 0 / 1) |
| Centre on boat | Context menu | `find-location-symbolic` | `location.circle` | `my_location` |
| Course up / north up | Map controls | custom **Course up** | `location.north.line` / `location.north.line.fill` | `explore` (fill 0 / 1) |
| Measure | Map controls, Navigate menu | custom **Measure** | `ruler` | `straighten` |
| Zoom in / out | Map controls | `zoom-in-symbolic` / `zoom-out-symbolic` | `plus` / `minus` | `add` / `remove` |
| Route, new route | Passage page, header | custom **Route** (+ `list-add` badge) | `point.topleft.down.curvedto.point.bottomright.up` | `route` (+ `add`) |
| Waypoint, mark | Passage page, popovers | `mark-location-symbolic` | `mappin` / `mappin.and.ellipse` | `location_on` |
| Track, record track | Passage page | custom **Track**; `media-record-symbolic` while recording | `point.3.connected.trianglepath.dotted` †; `record.circle` | `conversion_path` †; `radio_button_checked` |
| Activate route / play | Route rows, timeline | `media-playback-start-symbolic` | `play.fill` | `play_arrow` |
| Pause / stop | Timeline, navigating card | `media-playback-pause-symbolic` / `media-playback-stop-symbolic` | `pause.fill` / `stop.fill` | `pause` / `stop` |
| Skip waypoint | Navigating card | `media-skip-forward-symbolic` | `forward.end.fill` | `skip_next` |
| Anchor watch | Navigate menu, task card | custom **Anchor** | custom **Anchor** | `anchor` |
| Weather | Header, sidebar page | `weather-few-clouds-symbolic` | `cloud.sun` | `partly_cloudy_day` |
| Wind | Weather page, readouts | `weather-windy-symbolic` | `wind` | `air` |
| Tides | Weather page, popover | custom **Tide** | `water.waves` | `waves` |
| AIS, traffic | Sidebar page, list rows | custom **AIS target** | custom **AIS target** (`ferry` † for vessel details) | custom **AIS target** (`directions_boat` for vessel details) |
| Instruments | Sidebar page | custom **Instruments** | `gauge.with.dots.needle.33percent` † | `speed` |
| Charts | Sidebar page | custom **Charts** | `map` | `map` |
| Connections | Header indicator, window | `network-wired-symbolic` | `cable.connector` | `cable` |
| Offline | Activity indicator | `network-offline-symbolic` | `wifi.slash` | `cloud_off` |
| Activity running | Header | `GtkSpinner` | `ProgressView()` (small) | `CircularProgressIndicator` |
| Import / export GPX | Passage page, File menu | `document-open-symbolic` / `document-save-symbolic` | `square.and.arrow.down` / `square.and.arrow.up` | `file_download` / `file_upload` |
| Undo | Route editing | `edit-undo-symbolic` | `arrow.uturn.backward` | `undo` |
| More | Rows | `view-more-symbolic` | `ellipsis.circle` | `more_vert` |
| Close | Cards, popovers | `window-close-symbolic` | `xmark` | `close` |
| Preferences | Primary menu | `preferences-system-symbolic` | `gearshape` | `settings` |
| Polar | Boat preferences, weather route | custom **Polar** | custom **Polar** | custom **Polar** |
| Isochrone, weather route | Route detail, Navigate menu | custom **Isochrone** | custom **Isochrone** | custom **Isochrone** |
| Best departure | Route detail, Navigate menu | custom **Best departure** | custom **Best departure** | custom **Best departure** |
| Alarm | Banners, notifications | `dialog-error-symbolic` drawn as an octagon (custom **Alarm**) | `exclamationmark.octagon.fill` | `report` (octagon) |
| Warning, planning only | Banners, footer pill | `dialog-warning-symbolic` | `exclamationmark.triangle.fill` | `warning` |
| Info | Toasts | `dialog-information-symbolic` | `info.circle.fill` | `info` |
| OK, fresh | Badges | `emblem-ok-symbolic` | `checkmark.circle.fill` | `check_circle` |

## Custom glyphs

Each custom glyph is drawn once per style: GNOME, Apple and Android. The CustomGlyphs component shows both versions side by side.

- **GNOME version:** 16 × 16 grid, 2 px strokes or solid shapes on whole pixels, flat ends, no detail below 2 px, single colour. Delivered as `<name>-symbolic.svg` in the app's icon resource; GTK recolours it.
- **Android version:** 24 × 24 dp grid on Material Symbols metrics (2 dp strokes at weight 400, rounded ends), with an outline and a filled variant where the concept has an on state; delivered as vector drawables.
- **macOS, iOS and iPadOS version:** built as a custom SF Symbol (template with Ultralight, Regular and Black masters, small/medium/large scales), round caps and joins, stroke weight matched to the text weight beside it, optical centre on the text's cap height. Delivered in the asset catalogue as a symbol image.

| Glyph | Brief |
|---|---|
| Polar | Half of a polar diagram: the wind axis vertical, two nested boat-speed curves bulging to the right. Reads as "boat performance". | custom **Polar** |
| Isochrone | A start point at lower left with three concentric arcs spreading up and right, the outer one heavier. Reads as "weather route". |
| Best departure | A short time axis with four bars of different heights, one bar marked (solid when the others are outlines). Reads as "choose a time". | custom **Best departure** |
| Anchor | Classic stock anchor: ring, shank, stock, two flukes. Neither icon set has one. |
| AIS target | Isosceles triangle pointing up with a short vector line from its apex; matches the chart symbol. |
| Follow / Course up | Navigation arrow (outline = off, solid = following); course up adds a short line above the arrow, like a lubber line. |
| Layers | Two stacked diamonds; GNOME only (Apple and Material have their own). | `layers` |
| Measure | Ruler at 45° with three ticks; GNOME only. | `straighten` |
| Route | Two end dots joined by a three-segment polyline. |
| Track | Dotted wandering line. |
| Tide | Two stacked waves; GNOME only. |
| Instruments | Half gauge with a needle on a base line; GNOME only. | `speed` |
| Charts | Folded map with three panels; GNOME only. | `map` |
| Alarm | Solid octagon with a cut-out exclamation mark, so the alarm glyph differs in shape from the warning triangle on both platforms. | `report` (octagon) |

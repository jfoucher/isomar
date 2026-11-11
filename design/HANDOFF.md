# Chart redesign: handoff for Claude Code

This folder is the complete design for Chart's redesign, written for an implementer working in the repository (`~/development/chart`: Rust, GTK 4.18, libadwaita 1.7, GTK-free core in `src/core/`). It is self-contained: everything you need is in these files. The design was produced from the original handoff document, which described the current app, its problems and the owner's answers to open questions; this file restates what matters from it.

The design covers five targets that share one design system and one chart: GNOME, macOS, iPhone and iPhone Duo, iPad, and Android phones and tablets. **Part 1** (phases 1 to 9) is the GNOME app in this repository. **Part 2** (phases 10 to 15) is the touch apps; it depends on Part 1's token generator, alert model and renderer, and starts only when the owner says so.

Start by reading `CLAUDE.md` in the repo, then this file, then the files it points to. Work phase by phase (below), on its own branch, and stop at the end of each phase for review. Ask the owner when the design and the code disagree; do not silently pick one.

## What is in this folder

| Path | What it is | Use it for |
| --- | --- | --- |
| `HANDOFF.md` | This file | The plan, constraints and acceptance checks |
| `tokens.json` | Every design token: colours per palette (Day, Dusk, Night), type styles, spacing, radii, control sizes, stroke widths, chart geometry, timings, shadows. Each token has a `usage` note | The single source for every value. Generate code from it; never copy hex values by hand |
| `ia.md` | Design principles, window model, planning vs under-way modes, where every capability lives, menus and shortcuts, status signals | Structure of the UI |
| `design-system/foundations.md` | Palettes, type, spacing, control sizes, motion, how tokens reach code | Read first among the design-system files |
| `design-system/platform-mapping.md` | Every token mapped to libadwaita CSS variables, AppKit/SwiftUI, iOS/iPadOS SwiftUI and Material 3 for Compose; palette switching and Night overrides per platform; Liquid Glass on iOS | Writing the generated stylesheets and theme files |
| `design-system/alert-hierarchy.md` | Alarm, warning, info, hint: colour, glyph, placement, motion, sound, how each ends; Night rules; background behaviour | The alert model and its UI |
| `design-system/overlay-spec.md` | Everything Chart draws over the chart: own ship, routes, AIS, anchor watch, measure, weather, router, selection, map furniture, label priority | The renderer |
| `design-system/chart-symbols.md` | INT 1 / SHOM paper-chart look for lights, buoys, beacons, depths; rights and sources | The renderer's chart symbology |
| `design-system/iconography.md` | Each UI concept's Adwaita symbolic name, SF Symbol and Material Symbol, plus briefs for custom glyphs | Icons |
| `screens/` | 107 PNG renders: 67 desktop screens (GNOME and macOS) and 40 touch screens (iPhone, iPhone Duo folded and open, iPad, Android phone, Android tablet), in Day, Dusk and Night. `screens/INDEX.md` lists them | Visual target for layout, hierarchy and copy |
| `symbols/` | PNGs of each design-system component in Day and Night, and `bundle.js`, the SVG drawing code that produced them | Reference geometry for buoys, flares, sectors, own ship, AIS, routes |

The screens are mock-ups drawn in HTML, not screenshots of GTK, SwiftUI or Compose. Match their structure, copy and token use; use each platform's real widgets and metrics rather than copying pixels.

## Decisions already made

These were settled with the owner. Do not reopen them in code.

- **One left sidebar, no right inspector.** Five pages: Passage (routes, marks, tracks; replaces the Navigation page), Weather, Traffic, Charts (merges "Charts…" and "Charts for this area"), Instruments. Objects (aids, vessels, tides, waypoints) open in a popover on the chart; their full detail is pushed inside the page that owns them.
- **Night is dim blue-grey, alarms are red.** Red appears in Night chrome only for alarms.
- **Under-way layout switches on automatically** when a route is started, and off when navigation stops.
- **Instruments become a configurable dashboard** that can float over the chart.
- **One timeline** docked over the bottom of the chart for both the forecast time and the router clock.
- **macOS app:** SwiftUI-first, macOS 13 Ventura minimum, no Liquid Glass. Not part of Part 1, but the token generator must be able to emit Swift and Kotlin later.
- **Shared renderer** for the chart canvas on every platform, if it is fully compatible with macOS, iOS and Android. Write the renderer against tokens, not GTK colours.
- **Linux tablets are not a first-release target** for the GNOME app, but its hit areas still follow `hit-min` / `hit-min-underway`. Phones and tablets are served by the iOS/iPadOS and Android apps in Part 2.
- **Touch apps (Part 2):** one layout model driven by window width. Under 600 pt/dp (phones, iPhone Duo folded, narrow tablet windows): the chart full screen and the five pages in a bottom sheet. iPhone Duo open: pages in the left half, chart on the right, nothing on the crease. 600 and wider: iPad sidebar, Android navigation rail plus page panel. See the touch section of `ia.md`.
- **iOS and iPadOS:** SwiftUI, iOS 16 minimum, built against the current SDK. No system bars over the chart, opaque sheets, Chart's own floating controls, so Liquid Glass (which iOS 27 SDK builds can no longer opt out of) never touches the chart.
- **Android:** Kotlin, Jetpack Compose, Material 3, dynamic colour off (the palette is a safety feature), edge to edge.
- **Chart symbology looks like SHOM paper charts** (INT 1): coloured light flares, big sector arcs, leaning buoys with topmarks, buff land, blue depth bands, italic soundings. This changes the S-52 colour table for Day; keep the S-52 tables available as a preference ("Chart style: Paper / ECDIS"). Symbol meanings never change.
- **No glyph is traced or copied from SHOM Ouvrage 1D.** It is copyrighted. Draw every glyph from the INT 1 specification; U.S. Chart No. 1 (public domain) is the reference to measure against. `symbols/bundle.js` shows working geometry.

## Hard constraints

- **Overlays never read as chart objects.** Every stroke and label Chart draws over the chart sits on a 1.5 px `overlay-halo`. No overlay uses a hue between 280° and 340° (that is `chart-magenta`).
- **Every surface follows the palette**, chrome included: sidebar, popovers, menus, dialogs, toasts, separate windows. "Follow system" maps light → Day, dark → Dusk; Night is never automatic.
- **Colour is never the only signal.** Alert levels differ by glyph shape; router alternatives by dash pattern; AIS states by fill and outline; stale data by hatching and an age in words.
- **Contrast:** text pairs in `tokens.json` already pass WCAG AA in all three palettes (checked). Don't introduce new colours; add a token instead.
- **Not for navigation:** a quiet "Planning only" pill in the footer whenever no licensed official ENC covers the view, plus one line in About, Tides, Auto route and Weather route. Never a blocking dialog.
- **Translations:** every string through gettext; sentence case in source strings; layouts survive 40% longer text and RTL. Units are suffixes, never baked into labels.
- **Performance:** no blur or animation behind panels over the live chart; floating cards use an opaque `card-bg`. The canvas keeps its budgeted redraw of slow layers.
- **Reduce motion:** pulses become static frames, slides become crossfades; alarm sound is unchanged.

## Phases

Each phase ends with: `cargo build`, `cargo clippy`, tests passing, and screenshots from `scripts/screenshot.sh` (with the `CHART_*` variables listed in `CLAUDE.md`) in Day and Night, compared against the screens named. Note anything you could not match and why.

### Phase 1: Token pipeline and palettes

Goal: one `tokens.json` drives every colour in the app, and Night reaches every surface.

1. Copy `tokens.json` into the repo (suggested: `data/design/tokens.json`).
2. Add a generator (a `build.rs` step or an `xtask`) that emits:
   - three stylesheets (`style-day.css`, `style-dusk.css`, `style-night.css`) redefining libadwaita's CSS variables on `:root`, mapped as in `design-system/platform-mapping.md`, plus Chart's own variables;
   - a Rust module of colours per palette for the renderer (replacing the "darken for Dusk, blue-grey for Night" function in `src/ui/palette.rs`);
   - structure it so Swift and Kotlin emitters can be added in Phase 10.
3. Switch palettes by setting `AdwStyleManager:color-scheme` (`FORCE_LIGHT` for Day, `FORCE_DARK` for Dusk and Night) and swapping one `GtkCssProvider` on the display at application priority.
4. Keep the existing `data/style.css` for structural rules only; move every colour into the generated sheets.

Done when: switching to Night recolours the chart, sidebar, header bar, popovers, the right-click menu, every dialog and the Data monitor window, with no bright surface left. Compare: `screens/main-gnome-wide-night.png`, `screens/dialogs-gnome-preferences-night.png`.

### Phase 2: Alert model and alarms

Goal: the four-level alert hierarchy in `design-system/alert-hierarchy.md`.

1. In the core, one alert model: level (alarm, warning, info, hint), source object, message, actions, state (active, snoozed with time left, acknowledged), and the retrigger rule (collision: not again for the same target once acknowledged unless CPA worsens by half).
2. Alarms: `AdwBanner` restyled with `alarm-bg` / `on-alarm` and an octagon glyph; a `stroke-alarm-frame` frame around the chart viewport pulsing at `alarm-pulse`; a bundled alarm sound repeating every `alarm-repeat` until acknowledged; Snooze and Acknowledge. One alarm banner at a time; more stack as a count.
3. The current bug: the alarm style in `style.css` does not take effect on the banner. Find out why (likely the selector does not reach `AdwBanner`'s internal node) and fix it as part of this phase.
4. Warnings: `warning-bg` banner below any alarm, one chime, clears itself. Info: `AdwToast` restyled with `info-bg`. Hints ("Drawing a route…") become a small card, never a banner.
5. In the background: `GNotification` with urgent priority through the portal, with an Acknowledge action; sound keeps repeating.
6. Add shallow-water and lost-fix alarms to the same model; off-course as a warning.

Done when: the collision alert looks like `screens/alarms-gnome-collision-night.png`, the hint no longer looks like an alarm, and every row of `screens/alarms-states-day-night.png` is reproducible.

### Phase 3: Overlay spec in the renderer

Goal: everything in `design-system/overlay-spec.md`, identical in all palettes.

Own ship (hull, heading line, COG/SOG vector with minute ticks, trail, stale and lost fix), routes (inactive, active with dimmed past legs, next waypoint numbered with arrival circle, XTE corridor, leg chip, editing handles, selection ring), AIS (normal, dangerous blinking with CPA line and point, acknowledged, lost, at anchor, Class B), anchor watch, measure, weather (wind field from the `wind-*` ramp at 55% opacity, barbs with halos, isobars, waves), router (past fronts, current front heavy and clickable, six alternatives with dash patterns, boats at the clock, failure segments), footer strip (scale bar, credits that collapse first, Planning only pill, pointer position), grid labels in whole minutes, and the label priority and collision rules.

Done when: `screens/main-gnome-wide-day.png`, `screens/routing-gnome-3-preview.png` and `symbols/*` match, and leg labels, AIS labels and soundings no longer overlap.

### Phase 4: Paper-chart symbology

Goal: `design-system/chart-symbols.md`.

Light flares (135°, one per colour, `light-*` tokens), sector arcs at `sector-radius` with ink edges and dashed limits (option: full nominal range), all-round major lights; IALA A buoys leaning 15° with bands, stripes and topmarks; upright beacons; land, drying and depth bands; italic soundings with decimetre subscripts; magenta reserved for chart features. Add the "Chart style: Paper / ECDIS" preference. Apply the same look to the open-data planning map.

Done when: `symbols/Lights-*.png`, `symbols/Marks-*.png` and `symbols/Depths-*.png` are matched in Day and Night.

### Phase 5: Window and sidebar

Goal: `ia.md` window model.

Sidebar moves to the start side (`AdwOverlaySplitView`, overlay below 1000 px); `AdwInlineViewSwitcher` with the five pages; header bar buttons (New route, Weather, Layers, activity indicator, primary menu) and the connection dot in the subtitle; primary menu restructured; Layers popover grouped (Chart data, Base map, Overlays, Downloads with age and size); map control cluster (follow with three states, north/course up, layers, measure, zoom); activity indicator with a jobs popover; object popovers for aids, vessels and tides (with tide curve); planning-only pill.

Done when: all `screens/main-gnome-*.png` boards are matched, including the 400 px narrow layout.

### Phase 6: Under-way layout

Goal: `ia.md` "Planning and under way" and `screens/flows-gnome-underway-*.png`.

Automatic switch on route start and stop; sidebar hidden; navigating card; docked dashboard with `readout-xl` / `readout-l`; large controls; follow on; header shows Stop navigation. Instrument dashboard configurable from the Instruments page and pinnable over the chart in planning mode.

### Phase 7: Flows

In order of priority: weather routing (`screens/routing-gnome-1…5`), best departure (`screens/routing-gnome-bd-*`), get weather with the area chosen on the chart and the docked timeline (`screens/flows-gnome-weather-*`), charts page with coverage on the chart (`screens/flows-gnome-charts.png`), first-run welcome, route drawing with undo and leg readout, connections indicator.

### Phase 8: Dialog patterns and preferences

Task dialogs as in `screens/dialogs-gnome-task.png`: `AdwDialog`, Cancel and one primary action in the header, `AdwPreferencesGroup` rows, units as suffixes, `AdwSpinRow` for numbers, a calendar-and-time popover instead of typed "YYYY-MM-DD HH:MM", Advanced folded in an `AdwExpanderRow`, the planning-only line at the foot. They become bottom sheets at 400 px. Preferences as `AdwPreferencesDialog` with pages General, Units, Chart, Alarms, Boat, Data (`screens/dialogs-gnome-preferences.png`). Apply the template to every existing dialog.

### Phase 9: Icons

Use the Adwaita names in `design-system/iconography.md`; add the custom symbolic SVGs (16 px grid, 2 px strokes) to the app resources. Names marked † need checking against the icon theme in GNOME 48.

## Part 2: Touch platforms

Not started until the owner says so. Before writing code, propose to the owner how the apps are structured. The suggestion behind these designs: one SwiftUI multiplatform app for macOS, iOS and iPadOS; one Kotlin and Compose app for Android; both calling the Rust core through generated bindings (for example UniFFI); the shared renderer drawing into a native surface on each. That is a proposal, not a decision.

Each phase ends with a build on a simulator or emulator and a real device where it matters (Night brightness, glare, alarms), screenshots in Day and Night, and a comparison with the screens named.

### Phase 10: Token generator for Swift and Kotlin

Extend the Phase 1 generator to emit `Tokens.swift` (a `ChartPalette` per theme, injected through the SwiftUI environment) and `ChartTheme.kt` (a Material `ColorScheme` plus `ChartColors` per theme), mapped as in `design-system/platform-mapping.md`. Done when Day, Dusk and Night recolour every surface of a minimal shell, including sheets, menus and settings, with nothing bright left at Night.

### Phase 11: Phone shell

Chart full screen and edge to edge; status pill and action capsule floating at the top (`floating-bar-height`); map controls (`map-control-touch`); footer strip above the sheet; bottom sheet with peek, half and full heights (`sheet-peek`; iOS `.presentationDetents` with `.presentationBackground`, Android `BottomSheetScaffold`); the five pages in the sheet; object details in the sheet; long-press actions. Hit areas never below `hit-min-touch` / `hit-min-android`. Compare `screens/iphone-planning-day.png`, `iphone-passage-medium.png`, `iphone-route-detail.png`, `iphone-lookup-light.png` and the `android-phone-*` equivalents.

### Phase 12: Under way and alarms on phones

Automatic switch on route start; portrait (navigating card, 2 × 2 dashboard) and landscape (readout column) layouts; keep the screen on; Dim screen at Night; the alarm banner, frame and sound with 52-tall Snooze and Acknowledge; background behaviour (below). Compare `screens/*-underway-*.png`, `*-alarm-collision.png`.

### Phase 13: Tablets

iPad: `NavigationSplitView` with the sidebar of the desktop design. Android: navigation rail (`nav-rail-width`) plus page panel. Portrait opens the panel over the chart; windows narrower than 600 use the phone layout. Compare `screens/ipad-*.png`, `android-tablet-*.png`.

### Phase 14: iPhone Duo

Folded uses the phone layout. Open (about 890 × 626 pt) puts the pages, task cards, the navigating card and readouts in the left half and the chart in the right half; nothing interactive sits on the crease. Folding and unfolding keeps the open page, the chart position and any running task. Split View with another app uses the phone layout. Compare `screens/duo-*.png`.

### Phase 15: Flows on touch

Get weather (area handles on the chart, form in the sheet), weather route (alternatives in the sheet, timeline above it), best departure (full sheet), settings. Compare `screens/*-get-weather.png`, `*-weather-route.png`, `*-best-departure.png`, `*-weather-timeline.png`, `*-settings-night.png`.

### Background watching

Anchor and collision alarms are only worth having on a phone if Chart keeps watching with the screen off. Check this before promising either:

- **iOS:** Chart can only keep reading the boat's data and sounding alarms in the background under a supported background mode (for example continuous location updates while an anchor watch or a route is active). Check App Review's rules for the mode chosen. Critical alerts, which sound through the mute switch and Focus, need an entitlement Apple must grant; request it early.
- **Android:** a foreground service with a persistent notification, declared with the right type (location or connected device), and a notification channel carrying the alarm sound. A full-screen alarm over the lock screen needs the full-screen-intent permission, which Google Play restricts to alarm and calling apps; check whether an anchor alarm qualifies.

## Open items to resolve in the code

- **Best departure weights.** The nine slider names in the screens are placeholders derived from the route statistics. Use the nine weights the core actually has and their real names.
- **Weather model.** Screens show "GFS 0.25°" as the source; show whatever the core downloads.
- **Collision thresholds, anchor radius, safety depth** come from Preferences; the numbers in the screens are examples.
- **iPhone Duo sizes** in the screens are approximate, worked out from the published pixel counts at @3x. Use the sizes the iOS 27 SDK reports.
- **Liquid Glass on iOS.** The owner's "no Liquid Glass" holds for everything over the chart. Menus, keyboards and system alerts may still render as glass on iOS 27 SDK builds; check them at Night on a device and replace any that are too bright with Chart's own views.
- **Not-for-navigation coverage test.** The pill needs a "does a licensed official ENC cover the visible area" query in the core; it was a TODO in the original app.

## Out of scope here

Part 1 does not build the macOS, iOS or Android apps; it only makes sure the generator and renderer don't block them. Also out of scope: the app icon and renaming the app.

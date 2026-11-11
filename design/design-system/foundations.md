Chart is a chart plotter for sailors that ships as native apps on five targets: GNOME (GTK 4.18, libadwaita 1.7), macOS (SwiftUI-first, macOS 13 Ventura and later, no Liquid Glass), iPhone and iPhone Duo, iPad (SwiftUI, iOS 16 and later), and Android phones and tablets (Jetpack Compose, Material 3). This system is the shared half: colour, type, spacing and control sizes that each shell maps onto its own widgets, and the overlay and chart-symbol spec that the shared renderer draws identically everywhere. The chart canvas looks the same everywhere; everything around it follows its platform.

Five further sections sit beside this one: **Platform mapping** (every token to libadwaita, AppKit/SwiftUI, iOS/iPadOS and Material 3), **Alert hierarchy**, **Overlay spec**, **Chart symbols** and **Iconography** (every concept's Adwaita, SF Symbols and Material Symbols name, plus briefs for the custom glyphs).

## Rules that settle disagreements

1. **The chart comes first.** Chrome stays quiet and low-contrast (`text-muted`, `border`, `sidebar-bg`); colour and motion are spent on state changes.
2. **Safety over style.** An alarm, a depth or a stale forecast reads correctly at a glance in sun glare and at night before anything looks pretty. Day is tuned for glare: `text` on `view-bg` is 17:1.
3. **Every surface follows the palette.** Day, Dusk and Night apply to the canvas, the sidebar, menus, popovers, dialogs and toasts. A bright dialog at night is a safety bug, not a cosmetic one.
4. **Say how fresh it is.** Anything that can go stale (fix, forecast, tiles, chart edition, tide) carries its age in words in `caption`, coloured `stale` when old, never colour alone.
5. **Ours never looks like the chart.** Every line and label Chart draws over the chart sits on a 1.5px `overlay-halo`; no overlay uses a hue between 280° and 340° (that band belongs to `chart-magenta`).
6. **Plain words.** Sentence case in the source strings; any uppercase comes from a style (`caption-heading`), so French and German survive. Units are suffixes in `readout-unit`, never part of the label.

## Palettes

Three themes, selected by the user (Display: Follow system / Day / Dusk / Night). Follow system maps light → Day and dark → Dusk; Night is never automatic.

- **Day** — light, high contrast for direct sun. Paper-white chart, near-black ink, `route` signal orange.
- **Dusk** — dark chrome and a dark chart, comfortable in a cabin at evening. Equivalent to a normal dark mode.
- **Night** — dim blue-grey, red-free chrome to protect dark adaptation. Normal content stays at or below relative luminance 0.5 (`own-ship` is the brightest thing on screen, `text` is 0.29). Red appears in two places only: chart light flares and marks (`light-red`, `mark-red`, dimmed) and the alarm level (`alarm-bg`, `alarm`), which is the one element allowed to break the brightness ceiling.

Accent (`accent`, `accent-bg`, `focus-ring`, `selection`) is a calm blue used only for interaction state. It never colours a chart object.

Semantic status colours always come with a second signal: `alarm` with an octagon glyph and pulse, `warning` with a triangle, `ok` with a word or filled dot, `stale` with hatching and an age.

## Type

One family, Adwaita Sans (`sans`), on both platforms for the canvas: the shared renderer embeds it (SIL OFL) so chart labels are pixel-identical. Chrome uses the system face: Adwaita Sans on GNOME, SF Pro on macOS. Inter is the preview stand-in here (Adwaita Sans is derived from it).

- Every number that can change uses tabular figures: `readout-*`, `body-numeric`, `chart-grid`. GNOME `.numeric`, SwiftUI `.monospacedDigit()`.
- `readout-xl` is for at most two values on screen (depth and SOG under way). `readout-l` for COG and wind, `readout-m` for planning tiles and the navigating card.
- Chart text scales with the chart text size preference on top of the system text size; layouts reflow, never clip.
- Soundings are italic (`sounding`), names of marks and areas italic (`chart-name`), overlay labels upright (`chart-label`). Upright vs. sloping is one more cue that a label is ours.

## Spacing, shape and control sizes

4px grid: `space-2xs` … `space-3xl`. Native list rows, header bars and dialogs keep their platform metrics; the grid applies to what Chart lays out itself (cards over the chart, readout dashboards, timeline).

Two control scales, switched automatically when a route is started (under-way layout) and back when it stops:

| | Planning | Under way |
|---|---|---|
| Buttons and rows | `control-regular` 34px | `control-underway` 48px |
| Map controls | `map-control` 40px | `map-control-underway` 56px |
| Chart object hit area | `hit-min` 24px | `hit-min-underway` 44px |
| Card margin to chart edge | `space-l` | `space-xl` |

Phones and tablets replace the planning scale with platform touch minimums: every control and chart object has at least `hit-min-touch` (44 pt, Apple) or `hit-min-android` (48 dp); map controls are `map-control-touch` (48) in planning and `map-control-underway` (56) under way. On phones the pages live in a bottom sheet whose lowest detent is `sheet-peek`; the status pill and action capsule floating over the chart are `floating-bar-height` tall.

Radii: `radius-s` for controls, `radius-m` for cards and popovers, `radius-l` for the map control cluster and dashboard tiles, `radius-full` for round map buttons and pills.

## Motion

`alarm-pulse`, `danger-blink`, `panel-slide`, `toast-duration`. Nothing animates in normal state. Router front playback steps the clock; it never animates the whole chart. With reduce motion (GNOME `gtk-enable-animations` off, macOS and iOS Reduce Motion, Android Remove animations) pulses become static thick frames and slides become 100 ms crossfades; alarm sounds are unchanged.

## How tokens reach code

One `tokens.json` generates every shell's sources at build time: `data/style-day.css`, `style-dusk.css`, `style-night.css` for GTK (CSS custom properties, libadwaita 1.6+), `Tokens.swift` for macOS, iOS and iPadOS (a `ChartPalette` value per theme), `ChartTheme.kt` for Android (a Material `ColorScheme` plus `ChartColors` per theme), and `palette.rs` for the shared renderer. Nobody edits generated files.

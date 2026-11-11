# Platform mapping

Each shell maps the shared tokens onto its own styling. Five targets share one `tokens.json`:

| Shell | Devices | Toolkit |
|---|---|---|
| GNOME | Linux laptops and desktops | GTK 4.18, libadwaita 1.7 |
| macOS | Macs | SwiftUI-first, macOS 13 Ventura and later |
| iOS and iPadOS | iPhone, iPhone Duo (folded and open), iPad | SwiftUI, the same app target as macOS where possible; iOS 16 and later, built against the current SDK |
| Android | Phones and tablets | Kotlin, Jetpack Compose, Material 3; dynamic colour off |

The table values are the ones the generator writes; the text under each heading is how a theme switch reaches every surface.

## Switching palettes

**GNOME.** Day sets `AdwStyleManager:color-scheme` to `FORCE_LIGHT`; Dusk and Night set `FORCE_DARK`. Each palette is one generated stylesheet (`style-day.css`, `style-dusk.css`, `style-night.css`) that redefines libadwaita's CSS variables on `:root`; the app swaps a single `GtkCssProvider` on the display at `GTK_STYLE_PROVIDER_PRIORITY_APPLICATION`, so every window, popover, `AdwDialog` and toast changes at once, including the Data monitor and Connections windows. Follow system listens to `AdwStyleManager:dark`.

**macOS.** Every token is an `NSColor(name:dynamicProvider:)` whose provider reads `ChartTheme.current`. Changing theme sets `NSApp.appearance` (`.aqua` for Day, `.darkAqua` for Dusk and Night), which re-resolves every dynamic colour in AppKit and SwiftUI. SwiftUI reads the same colours as `Color(nsColor:)` and gets `.tint(.chartAccentBg)` at the root. Follow system leaves `NSApp.appearance` nil and maps the effective appearance to Day or Dusk.

**iOS and iPadOS.** One generated `ChartPalette` value per theme, injected at the root through the SwiftUI environment (`.environment(\.chartPalette, …)`), so a Dusk → Night switch re-renders every view even though both are dark. The root also sets `.preferredColorScheme(.light)` for Day and `.dark` for Dusk and Night, so system controls, keyboards and menus pick the matching appearance, and `.tint(palette.accentBg)`. UIKit views bridge through `UIColor(dynamicProvider:)` reading the same theme.

**Android.** One `ColorScheme` per theme built from the tokens (`lightColorScheme` for Day, `darkColorScheme` for Dusk and Night) and passed to `MaterialTheme`; dynamic colour (Material You) is never used, because the palette is a safety feature. Colours Material has no role for (warning, ok, stale, chart and overlay inks) live in a `ChartColors` object provided through a `CompositionLocal`. System bar icons follow the theme (`isAppearanceLightStatusBars` true only in Day). The app draws edge to edge and pads controls with the window insets.

**Night needs overrides on every platform,** because some system surfaces are brighter than the Night ceiling:

- No vibrancy or translucent materials at night: macOS sidebar, toolbar and cards switch from `NSVisualEffectView` materials to solid `sidebar-bg`, `headerbar-bg`, `popover-bg` fills.
- The chart's context menu (right-click, long-press) is Chart's own list in Night on macOS and iOS (`NSMenu` and `UIMenu` ignore custom colours); on Android a `ModalBottomSheet` styled from the tokens is used in every palette.
- No system alert dialogs for confirmations in Night (`NSAlert`, `UIAlertController`); an in-sheet confirmation row takes their place. Android `AlertDialog` is styled from the tokens and may be used.
- iOS and Android only: Night offers **Dim screen**, which lowers the screen brightness while Chart is in front (`UIScreen.brightness`, `WindowManager.LayoutParams.screenBrightness`) and restores it when Chart leaves the foreground.

**Liquid Glass on iOS and iPadOS.** Apps built against the iOS 27 SDK can no longer opt out of Liquid Glass (`UIDesignRequiresCompatibility` is ignored), and supporting iPhone Duo means building against that SDK. Chart is designed so this never touches the chart: there is no system navigation bar, tab bar or toolbar over the chart on iPhone; the bottom sheet takes an opaque fill with `.presentationBackground(palette.dialogBg)`; floating controls, cards and the dashboard are Chart's own views on solid `card-bg`. What stays system-drawn and may render as glass: the iPad sidebar toolbar, menus in Day and Dusk, keyboards and system alerts. Check each of these on device in Night.

## Surfaces

| Token | GNOME (libadwaita 1.7) | macOS (AppKit / SwiftUI) | iOS / iPadOS (SwiftUI) | Android (Material 3) |
|---|---|---|---|---|
| `window-bg` | `--window-bg-color` | `windowBackgroundColor` | sheet and screen background (replaces `systemGroupedBackground`) | `background`, `surface` |
| `view-bg` | `--view-bg-color` | `controlBackgroundColor` | grouped list rows (`.listRowBackground`) | `surfaceContainerLowest` (list groups, text fields) |
| `sidebar-bg` | `--sidebar-bg-color` | `.sidebar` material (Day, Dusk); solid (Night) | iPad sidebar column (`.scrollContentBackground(.hidden)` + this fill) | `surfaceContainerLow` (navigation rail, page panel) |
| `headerbar-bg` | `--headerbar-bg-color` | unified toolbar; solid in Night | iPad sidebar header | `surfaceContainer` (top app bar on settings screens) |
| `card-bg` | `--card-bg-color`, `.osd` override | solid overlay view | Chart's own floating views: status pill, action capsule, map controls, cards, dashboard | same, as `Surface(color = ChartColors.cardBg)` |
| `popover-bg` | `--popover-bg-color` | `NSPopover` content | `.popover` and menus (`.presentationBackground`) | `surfaceContainerHigh` (menus, dropdowns) |
| `dialog-bg` | `--dialog-bg-color` | sheet background | bottom sheet (`.presentationBackground`) and full-screen sheets | `surfaceContainerHigh` (dialogs, bottom sheets via `containerColor`) |
| `scrim` | `--shade-color` | overlay sidebar only | sheet dimming above the medium detent | `scrim` |
| `border` | `--border-color` | `separatorColor` | `separator` | `outlineVariant` |
| `border-control` | entry and button outline | custom stroke in Night | text field and toggle-off outline | `outline` |

## Text, accent, focus

| Token | GNOME | macOS | iOS / iPadOS | Android |
|---|---|---|---|---|
| `text` | `--window-fg-color` and friends | `labelColor` | `palette.text` (replaces `.primary`) | `onSurface`, `onBackground` |
| `text-muted` | `.dim-label`, explicit colour | `secondaryLabelColor` | `palette.textMuted` (replaces `.secondary`) | `onSurfaceVariant` |
| `text-disabled` | `:disabled` colour | `disabledControlTextColor` | disabled labels, explicit colour | disabled content, explicit colour (no alpha) |
| `accent` | `--accent-color` | `controlAccentColor` via `.tint` | `.tint` on links and plain buttons | `ChartColors.accent` for text buttons and links |
| `accent-bg` / `on-accent` | `--accent-bg-color` / `--accent-fg-color` | `.borderedProminent` | `.borderedProminent`, toggle on, selected segment | `primary` / `onPrimary` (filled buttons, switches, selection) |
| `focus-ring` | `:focus-visible` outline | keyboard focus ring | hardware-keyboard focus on iPad | focus indication for keyboards and D-pads |
| `selection` | selected rows; renderer | selected rows; renderer | selected rows; renderer | `secondaryContainer` on selected rows; renderer |

## Status and alerts

| Token | GNOME | macOS | iOS / iPadOS | Android |
|---|---|---|---|---|
| `alarm-bg` / `on-alarm` | `AdwBanner.alarm` | custom banner | custom banner under the top inset + frame | `error` / `onError` on Chart's banner |
| `alarm` | `--error-color` | replaces `systemRed` | replaces `.red` | `ChartColors.alarm` |
| `warning-bg` / `on-warning` | `AdwBanner.warning` | custom banner | custom banner | `ChartColors.warningBg` / `onWarning` |
| `warning` | `--warning-color` | replaces `systemOrange` | replaces `.orange` | `ChartColors.warning` |
| `info-bg` / `on-info` | `AdwToast` | in-window HUD | in-app HUD above the sheet | `Snackbar` with these colours |
| `ok` | `--success-color` | replaces `systemGreen` | replaces `.green` | `ChartColors.ok` |
| `stale` | `.stale` badge | `ChartBadge(.stale)` | `ChartBadge(.stale)` | `ChartBadge(stale)` |

Alarms when Chart is not in front: macOS time-sensitive notification and Dock bounce; iOS a time-sensitive notification, or a critical alert if Apple grants the Critical Alerts entitlement (it plays through the mute switch and Focus); Android a high-importance notification channel with the alarm sound, from the foreground service that keeps the boat connection open. Each platform's background-execution rules decide whether Chart can keep watching at all; see the handoff.

## Type

| Style | GNOME | macOS | iOS / iPadOS | Android |
|---|---|---|---|---|
| `readout-xl` / `-l` / `-m` | `.readout-xl` + `.numeric` | `.system(size:weight:.semibold).monospacedDigit()` | same, sizes through `@ScaledMetric` | `TextStyle(fontSize = 56/40/28.sp, fontWeight = SemiBold, fontFeatureSettings = "tnum")` |
| `readout-unit` | `.readout-unit` | `.subheadline` | `.subheadline` | `titleSmall` |
| `title-1` | `.title-1` | `.largeTitle` | `.largeTitle` | `headlineSmall` |
| `title-2` | `.title-2` | `.title` | `.title2.bold()` | `titleLarge` |
| `title-3` | `.title-3` | `.title3.bold()` | `.title3.bold()` | `titleMedium` |
| `heading` | `.heading` | `.headline` | `.headline` | `titleSmall` |
| `body`, `body-numeric` | default, `.numeric` | `.body`, `.monospacedDigit()` | `.body`, `.monospacedDigit()` | `bodyLarge`, + `"tnum"` |
| `caption`, `caption-heading` | `.caption`, `.caption-heading` | `.caption`, semibold | `.footnote`, `.caption.weight(.semibold)` | `bodySmall`, `labelMedium` |
| `monospace` | `.monospace` | `.monospaced` | `.monospaced` | `FontFamily.Monospace` |
| chart styles | shared renderer, Adwaita Sans | same | same | same |

Map styles by role, not by size: body is 11pt on GNOME, 13pt on macOS, 17pt on iOS and 16sp on Android. Sizes in `tokens.json` are the GNOME pixel values and the canvas values. On iOS and Android every style follows the system text size (Dynamic Type, font scale); layouts reflow, never clip.

## Spacing, radius, sizes

| Token | GNOME | macOS | iOS / iPadOS | Android |
|---|---|---|---|---|
| `space-*` | box `spacing`, margins | `.padding()`, stack `spacing:` | `.padding()`, stack `spacing:` | `Modifier.padding`, `Arrangement.spacedBy` (dp) |
| `radius-s` | native 6px | native | native (controls) | native |
| `radius-m` | `.card`, popover | 10pt | 10pt grouped lists and cards | 12dp cards; dialogs and sheets keep Material's 28dp |
| `radius-l`, `radius-full` | `.map-cluster`, `.osd.circular` | `RoundedRectangle`, `Capsule` | same | `RoundedCornerShape(16.dp)`, `CircleShape` |
| `control-regular` | 34px native | `.regular` | replaced by `hit-min-touch` (44pt) | replaced by `hit-min-android` (48dp) |
| `control-underway` | 48px | 44pt | 56pt (`map-control-underway`) | 56dp |
| `map-control*` | 40 / 56px | 40 / 56pt | `map-control-touch` 48pt / 56pt | 48dp / 56dp |
| `hit-min*` | renderer | renderer | renderer, never below 44pt | renderer, never below 48dp |
| `sheet-peek` | — | — | lowest sheet detent on iPhone | `BottomSheetScaffold` `sheetPeekHeight` |
| `nav-rail-width` | — | — | — | `NavigationRail` on tablets |
| `floating-bar-height` | — | — | status pill and action capsule | same |

## Overlay, chart and ramps

Tokens from `overlay-halo` down to `mark-white`, the `stroke`, `chart` and ramp tokens are read only by the shared renderer (`palette.rs`), which draws the same pixels on every platform. Dialog charts (route statistics, best-departure timeline, polar) read the ramp tokens through the generated CSS, Swift or Kotlin colours. On high-density phone screens the renderer keeps the token sizes in points or dp, so a 1.5 pt halo stays 1.5 pt.

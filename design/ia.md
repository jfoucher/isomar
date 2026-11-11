# Principles and information architecture

## Design principles

Six principles settle every disagreement, in this order.

| Principle | What it decides on screen |
| --- | --- |
| **1. The chart comes first** | One window, the chart filling it. The sidebar overlays the chart below 1000 px instead of shrinking it. Cards float over the chart and never push it; under way the sidebar hides by default. |
| **2. Safety over style** | Alarms, depth and stale data are designed first and checked in Day glare and Night. Day uses near-black on white. An alarm is the only red in Night chrome. |
| **3. Calm until it matters** | Normal state has no colour beyond the chart and the orange route. Motion and sound are reserved for alarms. Hints never use banner colours. |
| **4. Say what is known and how fresh it is** | Every readout, forecast, tile set, chart edition and tide shows its source or age in words beside it, and turns `stale` past a threshold. The planning-only pill appears whenever no official ENC covers the view. |
| **5. Plain words** | Sentence case in source strings, units as suffixes, sailor's words over jargon ("Collision risk", not "CPA alert"). Every label fits 40% longer French or German. |
| **6. Native, not lowest common denominator** | Same structure, same chart, same words. Chrome, menus, dialogs and shortcuts follow each HIG. |

Two rules sit under all six: every capability has a visible entry point (the right-click menu is a shortcut, never the only way in), and nothing is modal except an alarm or a dialog the sailor opened.

## Window model

One window: a header bar, a left sidebar and the chart. Everything else floats over the chart in fixed places. There is no right inspector; objects open in a popover on the chart, and their full detail opens in the sidebar page that owns them.

```
+------------------------------------------------------------------+
| Header bar: sidebar toggle · title + SOG/connection · New route · |
|             Weather · Layers · activity · primary menu            |
+--------------+---------------------------------------------------+
| Sidebar      |  [ Alarm / warning banners, full width          ] |
| (left)       |  [ Task card ]                [ Object popover ]  |
| page switch: |   navigating / router         aid, AIS, tide,     |
|  Passage     |   preview / editing / measure  waypoint           |
|  Weather     |                                                   |
|  Traffic     |                 CHART CANVAS              [ map ] |
|  Charts      |                                           [ ctl ] |
|  Instruments |  [ Instrument dashboard ]                 [     ] |
|              |  [ Timeline: forecast time and router clock     ] |
|              |  Footer: scale bar · credits · Planning only · pos |
+--------------+---------------------------------------------------+
```

Floating layers, top of the stack first: alarm and warning banners, hint card, task card (top left), object popovers, instrument dashboard (bottom left), map control cluster (bottom right), timeline (bottom, above the footer, only when a forecast is loaded or a router preview is open; one control for both), footer strip.

| Part | GNOME (GTK 4.18, libadwaita 1.7) | macOS (SwiftUI, macOS 13+) |
| --- | --- | --- |
| Window | `AdwApplicationWindow` + `AdwToolbarView` | `WindowGroup`, unified toolbar, chart under the toolbar |
| Sidebar | `AdwOverlaySplitView`, sidebar at the start, collapses to overlay below 1000 px | `NavigationSplitView` sidebar column, 300–360 pt |
| Page switcher | `AdwInlineViewSwitcher` (icons, tooltips, accessible names) at the sidebar top; page title under it | Segmented control with SF Symbols in the sidebar header |
| Detail in a page | `AdwNavigationView` push (route, vessel, chart set) | `NavigationStack` push in the sidebar column |
| Floating cards | `GtkOverlay` children, `.card` on `card-bg` | `.overlay` views with a solid `card-bg` fill |
| Object popover | `GtkPopover` pointing at the object | `.popover` anchored to the object's rect |
| Banners | `AdwBanner` restyled per alert level | Custom banner view under the toolbar |
| Long-lived tools | Data monitor, Connections: separate `AdwWindow` | Separate `Window` scenes |

## Planning and under way

Under way starts automatically when a route is started (Navigate to, Activate route, Navigate here) and ends when navigation stops or the route completes. A manual switch (View menu, Ctrl+U / ⌘U) holds until the next route starts or stops.

| | Planning | Under way |
| --- | --- | --- |
| Sidebar | Open on the last page | Hidden; opens as an overlay |
| Instruments | Instruments page, or a floating dashboard if pinned | Dashboard docked bottom left: depth and SOG in `readout-xl`, COG and wind in `readout-l` |
| Task card | Router preview, route editing, measure | Navigating card: next waypoint, BTW, DTW, XTE, VMG, TTG, ETA, Skip, Stop |
| Follow boat | Off | On, course-up if chosen last time |
| Control sizes | `control-regular`, `map-control`, `hit-min` | `control-underway`, `map-control-underway`, `hit-min-underway` |
| Header bar | New route, Weather, Layers | Stop navigation, Layers |
| Right-click menu | Full | Navigation actions first: Navigate here, Set anchor watch, Tides here |

The switch animates with `panel-slide` (crossfade with reduce motion) and announces "Under way layout" to screen readers. The chart's scale and layers do not change.

## Where every capability lives

| Capability | Home | Visible entry points | Right-click shortcut |
| --- | --- | --- | --- |
| Palette (Follow system, Day, Dusk, Night) | Primary menu / View menu | GNOME: toggle row at the top of the primary menu; macOS: View › Palette, ⌘⇧1/2/3 | — |
| Layers (13 today) | Layers popover, grouped: Chart data, Base map, Overlays, Downloads | Header Layers button and map control cluster | — |
| Measure distance | Task card | Map control cluster; M | Measure from here |
| Routes, waypoints, tracks | Passage page | Sidebar | — |
| New route, record track | Passage page + header | Header New route; Passage buttons; Ctrl/⌘+N | New route from here |
| Import / export GPX | Passage page ⋯ menu | File menu (macOS) | — |
| Route / waypoint properties | Sidebar detail (pushed) | Row ›; double-click on the chart | Properties… |
| Activate, navigate to point | Navigating card | Route row play button; Navigate menu | Activate, Navigate here |
| Instruments | Instruments page + floating dashboard | Sidebar; pin switch | — |
| Get weather | Weather page + area on the chart | Header Weather button; Weather page | Get weather here / for this view |
| Weather timeline and readout | Timeline over the chart | Appears when a forecast is loaded | — |
| Weather route | Sheet, then router preview card | Route detail "Weather route…"; Navigate menu | Weather route… |
| Best departure | Sheet, results, then router preview card | Route detail "Best departure…"; Navigate menu | Best departure… |
| Auto route | Sheet | Route detail "Auto route…"; Navigate menu | Auto route… |
| Tides here | Popover with tide curve | Weather page "Tides nearby" | Tides here… |
| Charts for this area, Charts… | Charts page (merged) | Sidebar; planning-only pill | Charts for this area… |
| AIS vessel info | Popover, then Traffic detail | Traffic page rows; click a target | Vessel information |
| Buoy, beacon, light info | Popover | Click the aid | — |
| Anchor watch | Task card + anchor overlay | Navigate menu; Instruments page | Set / Clear anchor watch |
| Connections | Separate window | Connection status dot in the header subtitle | — |
| Data monitor | Separate window | Connections window; Window menu (macOS) | — |
| Boat polar | Preferences › Boat | Weather route sheet "Edit polar…" | — |
| Preferences | Preferences dialog / Settings window | Primary menu; ⌘, | — |
| Copy position | — | Footer position (click to copy) | Copy position |
| Collision and anchor alarms | Banner + frame + sound | — | Acknowledge on the target |
| About, not-for-navigation statement | About dialog | Planning-only pill | — |

## Menus, commands and shortcuts

**GNOME header bar**, left to right: sidebar toggle · title "Chart" with subtitle (e.g. "6.0 kn · NMEA connected" with a status dot) · New route · Weather · Layers · activity indicator (only while something runs) · primary menu.

**GNOME primary menu:** palette toggle row · Under way layout · Import GPX… · Export GPX… · Connections · Data monitor · Preferences · Keyboard Shortcuts · About Chart.

**macOS menu bar:**

| Menu | Commands |
| --- | --- |
| Chart | About Chart, Settings… (⌘,), Hide, Quit |
| File | New Route (⌘N), Record Track (⌘⇧R), Import GPX… (⌘I), Export GPX… (⌘E) |
| Edit | Undo, Redo, Cut, Copy, Paste, Delete, Copy Position (⌘⇧C), Select All |
| View | Show Sidebar (⌘⌥S), Passage / Weather / Traffic / Charts / Instruments (⌘1–5), Layers ›, Palette › Day, Dusk, Night (⌘⇧1–3), Under Way Layout (⌘U), Follow Boat (F), Course Up (C), Zoom In / Out (⌘+ / ⌘−), Enter Full Screen |
| Navigate | Activate Route, Navigate Here, Skip Waypoint, Stop Navigation, Set Anchor Watch, Weather Route…, Best Departure…, Auto Route…, Measure (M) |
| Weather | Get Weather… (⌘⇧W), Update Forecast, Tides Here…, Next / Previous Time Step (⌘→ / ⌘←) |
| Window | Connections, Data Monitor, standard items |
| Help | Chart Help, Keyboard Shortcuts, Not for Navigation |

GNOME uses the same keys with Ctrl and lists them in the shortcuts dialog (Ctrl+?). Single-letter keys work only when the chart has focus.

## Status, freshness and not for navigation

| Signal | Where it shows | Detail on click |
| --- | --- | --- |
| Connection | Status dot in the header subtitle: `ok` connected, `stale` no data for 10 s, none = no source | Connections window |
| Position fix age | Own ship (dashed when stale) + instrument tile age | Data monitor |
| Long-running work | Activity indicator in the header (spinner + count); one toast when done | Popover listing each job with progress and Cancel |
| Offline | Activity indicator shows "Offline"; network actions say "Needs a connection" | Popover: what is cached, what is not |
| Forecast age | Timeline badge "Run 06 UTC · 5 h old"; Weather page; warning banner past useful life | Weather page |
| Tile and depth age | Layers popover per downloaded layer | Charts page |
| Chart edition | Charts page per set | Charts page detail |
| Tide prediction | Tide popover: source, licence, "Predicted" | — |
| Planning only | Footer pill whenever no licensed official ENC covers the view; one quiet line in About, Tides, Auto route, Weather route | Popover explanation + "Charts for this area…" |
| Pointer position and depth | Footer, right | Click copies the position |

None of these block. Only alarms interrupt.

## Phones, foldables and tablets

Chart also runs on iPhone, iPhone Duo, iPad, Android phones and Android tablets, with the same five pages, the same chart and the same words. What changes is where the pages live and how big the controls are. The layout follows the window's width, not the device name, so an iPad in Split View or an iPhone Duo folded gets the phone layout.

| Width class | Devices | Where the pages live | Over the chart |
| --- | --- | --- | --- |
| Compact (under 600 pt / dp) | iPhone, Android phone, iPhone Duo folded, any tablet window narrower than 600 | A bottom sheet with three heights: peek (page switcher and one status line, `sheet-peek`), half, full. Object details and task cards open in the same sheet | Status pill (title, speed, connection dot) and action capsule (Weather, Layers, More) at the top; map controls on the right; footer strip above the sheet |
| Open foldable (about 890 × 626 pt) | iPhone Duo open | The left half is the sidebar; the chart fills the right half. Task cards, the navigating card and readouts dock in the left half, never on the crease | Action capsule and map controls in the right half |
| Regular (600 and wider) | iPad, Android tablet | iPad: left sidebar (`NavigationSplitView`). Android: navigation rail (`nav-rail-width`) plus a page panel. In portrait the panel opens over the chart | Status pill, action capsule with New route, map controls with zoom, footer |

Every desktop entry point has a touch home: header buttons move to the action capsule; the primary menu becomes More (Settings, Connections, Import and export, About); New route stays on the Passage page; the connection status is the dot in the status pill.

**Gestures.** Long-press is right-click: it opens the actions for that place or object (a context menu on iOS in Day and Dusk, Chart's own list at Night and on Android). Tap an object to see it in the sheet (phone) or a popover (tablet). Pinch and double-tap zoom. Dragging the chart turns Follow off. Two-finger rotate is off by default; the compass control sets north up or course up.

**Under way on a phone.** Portrait: the navigating card at the top; depth, SOG, COG and wind in a 2 × 2 dashboard at the bottom; the sheet hidden behind a swipe from the bottom edge. Landscape, for a cockpit mount: readouts and Stop in a column on the left. Controls are 56 pt / dp, the screen stays on, and Night offers Dim screen.

**Alarms on touch devices.** A full-width banner under the top inset, the frame around the screen and the repeating sound, with Snooze and Acknowledge at least 52 tall. When Chart is in the background, iOS uses a time-sensitive notification, or a critical alert if Apple grants the entitlement; Android uses a high-importance channel posted by the foreground service that keeps the boat connection open.

# Alert hierarchy

Four levels. Each differs in colour, glyph shape, placement, motion, sound and persistence, so a level is recognisable with any one channel missing (colour-blind, Night, muted laptop, glancing from the helm).

| Level | Examples | Colour | Glyph | Placement | Motion | Sound | Ends |
|---|---|---|---|---|---|---|---|
| **Alarm** | Collision risk, anchor dragging, shallow water, position lost while navigating | `alarm-bg` / `on-alarm` | Octagon with ! | Full-width banner pinned to the top of the chart **and** a `stroke-alarm-frame` frame around the chart viewport; the object on the chart turns `ais-danger` / `alarm` | Frame and banner edge pulse at `alarm-pulse` (1 Hz); dangerous target blinks at `danger-blink` | Alarm sound, repeats every `alarm-repeat` until acknowledged | Acknowledge or Snooze (2 min collision, 5 min anchor). Collision does not retrigger for the same target once acknowledged unless CPA worsens by half |
| **Warning** | Forecast is stale, off course (XTE over limit), wind over boat limit, data source lost, not-for-navigation outside official ENC | `warning-bg` / `on-warning` | Triangle with ! | Banner below any alarm, or inline in the panel that owns the data | None | One chime when it appears | Clears itself when the condition clears; Dismiss hides it for this session |
| **Info** | Forecast downloaded, arrived at waypoint 3, route completed | `info-bg` / `on-info` | Circle with i | Toast (GNOME) / HUD (macOS), bottom centre above the timeline | Slide in | None (arrival: soft tick) | `toast-duration` |
| **Hint** | Drawing a route: click to add points | `card-bg`, `text-muted` | None or the tool's own icon | Inline: a small card at the top of the chart or the status line | None | None | When the mode ends |

Only one alarm banner shows at a time; further alarms stack as a count ("2 alarms") on the banner, newest first. Warnings never cover an alarm. Hints never use a banner, so the old problem of "Drawing a route" looking like a collision is gone.

## Alarm in Night

Night keeps red out of the chrome so that the alarm owns it. The alarm stays unmistakable because:

1. **It is the only saturated red** in Night chrome, and the only element above the Night brightness ceiling: `on-alarm` text reaches relative luminance 0.8 where normal Night text is 0.29. The banner is deliberately bright enough to wake a sleeping watch-keeper; red light is the least damaging to dark adaptation, which is why the alarm, not the chrome, gets it.
2. **Shape:** octagon glyph, full-width banner, frame around the chart. Nothing else in the app frames the viewport.
3. **Motion:** 1 Hz pulse of the frame (4 → 8px). Reduce motion: static 8px frame.
4. **Sound:** repeating alarm tone until acknowledged, at system alert volume.
5. **After acknowledgement** the banner drops to its dim state (`alarm` text on `card-bg`) so the screen returns to Night levels while the condition is still shown.

## Background behaviour

| | GNOME | macOS |
|---|---|---|
| Alarm, app in background | `GNotification` priority `URGENT` through the notification portal, with Acknowledge action; sound keeps repeating | `UNNotification` with `.timeSensitive` interruption level, Acknowledge action; `requestUserAttention(.criticalRequest)` bounces the Dock icon until focused; sound keeps repeating |
| Warning, app in background | Normal notification once | Normal notification once |
| Info | Nothing (shown when the window returns, if still relevant) | Nothing |

Sounds ship with the app (GNOME `GtkMediaFile`, macOS `NSSound`) so they work offline and in a Flatpak sandbox.

package io.github.isomar.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Anchor
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CloudDownload
import androidx.compose.material.icons.outlined.CropFree
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DirectionsBoat
import androidx.compose.material.icons.outlined.FiberManualRecord
import androidx.compose.material.icons.outlined.FileDownload
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.MoreHoriz
import androidx.compose.material.icons.outlined.Navigation
import androidx.compose.material.icons.outlined.Place
import androidx.compose.material.icons.outlined.Report
import androidx.compose.material.icons.outlined.Route
import androidx.compose.material.icons.outlined.Sailing
import androidx.compose.material.icons.outlined.Timeline
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.isomar.Chip
import io.github.isomar.Detail
import io.github.isomar.Model
import io.github.isomar.Page
import io.github.isomar.SourceRow
import io.github.isomar.TaskSheet
import io.github.isomar.WeatherArea

/** A page of the sheet: its title, then what it lists, scrolling. */
@Composable
fun PageView(page: Page) {
    PageColumn {
        Text(page.title, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp))
        when (page) {
            Page.Passage -> PassagePage()
            Page.Weather -> WeatherPageView()
            Page.Traffic -> TrafficPage()
            Page.Charts -> ChartsPageView()
            Page.Instruments -> InstrumentsPage()
        }
        Spacer(Modifier.height(32.dp))
    }
}

@Composable
fun PageColumn(content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()), content = content)
}

// Passage

@Composable
private fun PassagePage() {
    val p = Model.passage
    p.following?.let { f ->
        SectionHeader("Following")
        BoxedList {
            ListRow(f.name, chips = f.detail, icon = Icons.Outlined.Navigation, selected = true) {
                IconButton(onClick = { Model.send("underway_stop") }) { Icon(Icons.Filled.Stop, "Stop navigating") }
            }
        }
    }
    SectionHeader("Routes") {
        TextButton(onClick = { Model.command("new_route") }) {
            Icon(Icons.Outlined.Add, null, Modifier.size(18.dp)); Spacer(Modifier.size(4.dp)); Text("New")
        }
    }
    if (p.routes.isEmpty()) EmptyText("No routes. Use New, then tap the chart for each turn.")
    else BoxedList {
        p.routes.forEachIndexed { i, r ->
            if (i > 0) ListDivider()
            ListRow(
                r.name, chips = r.detail, more = r.planned, icon = Icons.Outlined.Route, selected = r.active, dim = !r.visible,
                onClick = { Model.command("zoom_route", r.id); Model.detail = Detail.Route(r.id) },
            ) {
                IconButton(onClick = { Model.send("start_route", Model.args("id" to r.id)) }) { Icon(Icons.Filled.PlayArrow, "Follow this route") }
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = c(Tok.textMuted), modifier = Modifier.padding(end = 8.dp))
            }
        }
    }
    SectionHeader("Marks")
    if (p.waypoints.isEmpty()) EmptyText("No marks. Touch and hold the chart, then Drop waypoint here.")
    else BoxedList {
        p.waypoints.forEachIndexed { i, w ->
            if (i > 0) ListDivider()
            ListRow(w.name, subtitle = w.detail, icon = Icons.Outlined.Place, onClick = { Model.command("zoom_waypoint", w.id) }) {
                RowMenu(
                    "Navigate to" to { Model.command("goto_waypoint", w.id) },
                    "Properties" to { Model.detail = Detail.Waypoint(w.id) },
                    "Delete…" to { Model.command("delete_waypoint", w.id) },
                )
            }
        }
    }
    SectionHeader("Tracks")
    if (p.tracks.isEmpty()) EmptyText("Recorded tracks appear here.")
    else BoxedList {
        p.tracks.forEachIndexed { i, t ->
            if (i > 0) ListDivider()
            ListRow(t.name, chips = t.detail, icon = Icons.Outlined.Timeline, dim = !t.visible, onClick = { Model.command("zoom_track", t.id) }) {
                RowMenu(
                    (if (t.visible) "Hide" else "Show") to { Model.command("toggle_track", t.id) },
                    "Delete…" to { Model.command("delete_track", t.id) },
                )
            }
        }
    }
    Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilledTonalButton(onClick = { Model.command("record_track") }) {
            Icon(if (p.recording) Icons.Filled.Stop else Icons.Outlined.FiberManualRecord, null, Modifier.size(18.dp))
            Spacer(Modifier.size(6.dp)); Text(if (p.recording) "Stop recording" else "Record track")
        }
        OutlinedButton(onClick = { Model.pickFile("gpx") }) {
            Icon(Icons.Outlined.FileDownload, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Import GPX")
        }
    }
}

/** The "…" of a row, with its actions. */
@Composable
fun RowMenu(vararg items: Pair<String, () -> Unit>) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Outlined.MoreHoriz, "More") }
        DropdownMenu(open, onDismissRequest = { open = false }) {
            for ((label, run) in items) DropdownMenuItem(text = { Text(label) }, onClick = { open = false; run() })
        }
    }
}

// Weather

@Composable
private fun WeatherPageView() {
    val w = Model.weather
    // The model shown, and routed on: each keeps its own forecast.
    if (w.models.isNotEmpty()) {
        SectionHeader("Model")
        ModelPicker(w.model) { Model.send("weather_show_model", Model.args("model" to it)) }
    }
    if (w.loaded) {
        Column(Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) {
            Text("${w.modelLabel} loaded", fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
            w.run?.let { Text("Run $it", fontSize = 13.sp, color = c(Tok.textMuted)) }
            if (w.from != null && w.to != null) Text("${w.from} to ${w.to}", fontSize = 13.sp, color = c(Tok.textMuted))
        }
    } else {
        EmptyText("No forecast yet. Get weather for what is on screen, or draw an area on the chart.")
    }
    Row(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        // While a download runs: off, with a spinner, and pressing it shows the running jobs.
        Button(onClick = { Model.job("weather") { Model.call<WeatherArea>("weather_defaults", Model.args("kind" to "view"))?.let { Model.sheet = TaskSheet.GetWeather(it) } } }, modifier = Modifier.jobBusy(Model.busy("weather"))) {
            JobLabel("Get weather", Model.busy("weather"), Icons.Outlined.CloudDownload)
        }
        OutlinedButton(onClick = { Model.send("weather_pick") }) {
            Icon(Icons.Outlined.CropFree, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Draw an area")
        }
    }
    Model.weatherReadout?.let { r ->
        SectionHeader("Here")
        BoxedList {
            Column(Modifier.padding(14.dp)) {
                ChipsText(r.chips, size = 14.sp, color = c(Tok.text), maxLines = 4, style = Numeric)
                Text(r.time, fontSize = 12.sp, color = c(Tok.textMuted))
            }
        }
    }
    SectionHeader("Show")
    BoxedList {
        // One field colours the sea, picked like a radio; barbs, wave arrows and isobars go over it.
        Column(Modifier.padding(horizontal = 16.dp, vertical = 10.dp)) {
            Text("Colour", fontSize = 16.sp)
            Text(if (w.fill == "waves" && w.noWaves != null) w.noWaves else "What colours the sea", fontSize = 13.sp, color = c(Tok.textMuted), modifier = Modifier.padding(bottom = 8.dp))
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                val fills = listOf("none" to "None", "wind" to "Wind", "waves" to "Waves", "current" to "Current")
                fills.forEachIndexed { i, (id, label) ->
                    SegmentedButton(
                        selected = w.fill == id,
                        onClick = { Model.send("weather_fill", Model.args("fill" to id)) },
                        shape = SegmentedButtonDefaults.itemShape(i, fills.size),
                    ) { Text(label) }
                }
            }
        }
        ListDivider()
        SwitchRow("Wind barbs", w.barbs) { Model.send("weather_toggle", Model.args("name" to "barbs")) }
        ListDivider()
        SwitchRow("Wave arrows", w.waveArrows, subtitle = w.noWaves ?: "") { Model.send("weather_toggle", Model.args("name" to "wave_arrows")) }
        ListDivider()
        SwitchRow("Isobars", w.pressure) { Model.send("weather_toggle", Model.args("name" to "pressure")) }
    }
    w.currents?.let { cur ->
        SectionHeader("Currents")
        EmptyText(cur.note)
        cur.tracking.forEach { EmptyText(it) }
        Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
            OutlinedButton(onClick = { Model.send("currents_fetch") }, modifier = Modifier.jobBusy(Model.busy("currents"))) {
                JobLabel("Get currents for this view", Model.busy("currents"), Icons.Outlined.CloudDownload)
            }
        }
        BoxedList {
            cur.sources.forEachIndexed { i, s ->
                if (i > 0) ListDivider()
                Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
                    Text(s.label, fontSize = 15.sp)
                    Text(s.detail, fontSize = 13.sp, color = c(Tok.textMuted))
                }
            }
            if (cur.sources.isNotEmpty()) ListDivider()
            SwitchRow("Current arrows", cur.arrows) { on -> Model.send("currents_arrows", Model.args("on" to on)) }
            ListDivider()
            SwitchRow("Sail in the current", cur.routeWith, subtitle = "Weather routes and best departures") { on -> Model.send("currents_route_with", Model.args("on" to on)) }
            cur.atlases.forEach { a ->
                ListDivider()
                SwitchRow("Shom atlas: ${a.name}", a.installed, subtitle = a.detail) { on -> Model.send(if (on) "atlas_install" else "atlas_remove", Model.args("id" to a.id)) }
            }
        }
    }
    if (w.loaded) {
        SectionHeader("Forecast")
        Row(Modifier.padding(horizontal = 16.dp)) {
            OutlinedButton(onClick = { Model.send("weather_clear") }) {
                Icon(Icons.Outlined.Delete, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Clear the forecast")
            }
        }
        EmptyText("Tap the chart to read the forecast there. The time control above the sheet plays it.")
    }
    if (w.tracking.isNotEmpty()) {
        SectionHeader("Forecasts against your wind")
        BoxedList {
            w.tracking.forEachIndexed { i, t ->
                if (i > 0) ListDivider()
                Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
                    Text(if (t.best) "${t.label} · closest" else t.label, fontSize = 15.sp, fontWeight = if (t.best) FontWeight.SemiBold else FontWeight.Normal)
                    Text(t.text, fontSize = 13.sp, color = c(Tok.textMuted))
                }
            }
        }
    }
    SectionHeader("Tides")
    EmptyText("Touch and hold the chart, then Tides here.")
}

@Composable
fun SwitchRow(label: String, on: Boolean, subtitle: String = "", toggle: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable { toggle(!on) }.padding(start = 16.dp, end = 12.dp).heightIn(min = 56.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
            Text(label, fontSize = 16.sp)
            if (subtitle.isNotEmpty()) Text(subtitle, fontSize = 13.sp, color = c(Tok.textMuted))
        }
        Switch(checked = on, onCheckedChange = toggle)
    }
}

// Traffic

@Composable
private fun TrafficPage() {
    val vs = Model.vessels
    SectionHeader(if (vs.size == 1) "1 vessel" else "${vs.size} vessels")
    if (vs.isEmpty()) EmptyText("AIS targets appear when a receiver is connected.")
    else BoxedList {
        vs.forEachIndexed { i, v ->
            if (i > 0) ListDivider()
            ListRow(
                v.name, chips = v.detail, icon = if (v.alert) Icons.Outlined.Report else Icons.Outlined.DirectionsBoat,
                iconTint = c(if (v.alert) Tok.alarm else Tok.ais),
                onClick = { Model.command("show_vessel", v.mmsi) },
            ) {
                if (v.alert) TextButton(onClick = { Model.command("acknowledge", v.mmsi) }) { Text("Acknowledge") }
                IconButton(onClick = { Model.detail = Detail.Vessel(v.mmsi) }) { Icon(Icons.Outlined.Info, "Vessel information") }
            }
        }
    }
}

// Instruments

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun InstrumentsPage() {
    SailSwitchRow()
    FlowRow(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp), maxItemsInEachRow = 2) {
        for (t in Model.tiles) {
            Column(
                Modifier.weight(1f).heightIn(min = Size.readoutTile * 0.7f).clip(RoundedCornerShape(Radius.m)).background(c(Tok.viewBg)).padding(12.dp),
            ) {
                Text(t.title.uppercase(), fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = c(Tok.textMuted))
                Text(t.value, fontSize = 26.sp, fontWeight = FontWeight.SemiBold, style = Numeric, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(t.unit, fontSize = 12.sp, color = c(Tok.textMuted))
            }
        }
    }
    SectionHeader("Performance")
    BoxedList {
        ListRow("Performance and polar", subtitle = "What the boat does in each wind, learnt from the log", icon = Icons.Outlined.Sailing, onClick = { Model.detail = Detail.Performance }) {
            Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = c(Tok.textMuted), modifier = Modifier.padding(end = 8.dp))
        }
    }
    SectionHeader("Anchor watch")
    Row(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilledTonalButton(onClick = { Model.command("anchor_here") }) { Icon(Icons.Outlined.Anchor, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Set here") }
        OutlinedButton(onClick = { Model.command("stop_watching") }) { Icon(Icons.Outlined.Close, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Clear") }
    }
}

/** Sailing or motoring, for the performance log: set by the engine's RPM when there is one, else by hand. */
@Composable
private fun SailSwitchRow() {
    val s = Model.sailSwitch
    if (s.mode.isEmpty()) return
    Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            val modes = listOf("sailing" to "Sailing", "motoring" to "Motoring")
            modes.forEachIndexed { i, (id, label) ->
                SegmentedButton(
                    selected = s.mode == id,
                    enabled = !s.byEngine,
                    onClick = { Model.send("perf_set_mode", Model.args("mode" to id))?.let { Model.show(it) } },
                    shape = SegmentedButtonDefaults.itemShape(i, modes.size),
                ) { Text(label) }
            }
        }
        (s.note ?: s.hint)?.let { Text(it, fontSize = 12.sp, color = c(Tok.textMuted), modifier = Modifier.padding(top = 4.dp)) }
    }
}

// Charts

@Composable
private fun ChartsPageView() {
    LaunchedEffect(Unit) { Model.showPage(Page.Charts) }
    val ch = Model.charts
    if (ch.planningOnly) {
        Row(Modifier.padding(horizontal = 20.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(Icons.Outlined.WarningAmber, null, tint = c(Tok.warning), modifier = Modifier.size(18.dp))
            Text("Planning only here: no official chart you added covers this area.", fontSize = 13.sp)
        }
    }
    SectionHeader("In use here")
    if (ch.inUse.isEmpty()) EmptyText(if (ch.vector + ch.rasters == 0) "No charts loaded." else "None of the loaded charts is drawn here at this zoom.")
    else BoxedList {
        ch.inUse.forEachIndexed { i, u ->
            if (i > 0) ListDivider()
            Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
                Text(u.name, fontSize = 15.sp)
                ChipsText(listOf(Chip(null, u.kind, u.kind)) + u.chips, size = 12.sp)
            }
        }
    }
    if (ch.planningOnly || ch.here.isNotEmpty()) SectionHeader(if (ch.planningOnly) "For this area" else "Downloaded for this area")
    if (ch.planningOnly && ch.here.isEmpty()) EmptyText("No chart source is known for this region.")
    if (ch.here.isNotEmpty()) BoxedList {
        ch.here.forEachIndexed { i, s ->
            if (i > 0) ListDivider()
            SourceLine(s)
        }
    }
    SectionHeader("Your chart folders") {
        TextButton(onClick = { Model.pickFile("charts") }) { Icon(Icons.Outlined.Add, null, Modifier.size(18.dp)); Spacer(Modifier.size(4.dp)); Text("Add") }
    }
    if (ch.folders.isEmpty()) EmptyText("S-57 cells (.000) and BSB/KAP charts you licensed: Add copies them into the app.")
    else BoxedList {
        ch.folders.forEachIndexed { i, f ->
            if (i > 0) ListDivider()
            ListRow(f.substringAfterLast('/'), subtitle = f, icon = Icons.Outlined.Folder) {
                IconButton(onClick = { Model.send("charts_remove_folder", Model.args("path" to f)); Model.showPage(Page.Charts) }) { Icon(Icons.Outlined.Delete, "Forget this folder") }
            }
        }
    }
    SectionHeader("Loaded charts")
    EmptyText("Vector charts (S-57): ${ch.vector}\nRaster charts (BSB/KAP): ${ch.rasters}")
}

@Composable
private fun SourceLine(s: SourceRow) {
    Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(s.rating, fontSize = 11.sp, fontWeight = FontWeight.Bold, modifier = Modifier.clip(RoundedCornerShape(Radius.full)).background(c(Tok.hover)).padding(horizontal = 6.dp))
            Text(s.name, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
        }
        ChipsText(listOf(Chip(null, s.ratingText, s.ratingText)) + s.chips, size = 12.sp, maxLines = 3)
        if (s.note.isNotEmpty()) Text(s.note, fontSize = 12.sp, color = c(Tok.textMuted))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            val label = when (s.action) { "install" -> "Install"; "installing" -> "Installing…"; "installed" -> "Installed"; "update" -> "Update"; "on" -> "On"; "turn_on" -> "Turn on"; else -> "Open website" }
            // While its install runs: off, with a spinner, and pressing it shows the running jobs.
            val busy = Model.busy("chart:${s.id}")
            FilledTonalButton(onClick = {
                when (s.action) {
                    "install", "update", "installing" -> Model.job("chart:${s.id}") { if (s.action != "installing") Model.send("charts_install", Model.args("id" to s.id)) }
                    "turn_on" -> { Model.send("charts_service_on"); Model.showPage(Page.Charts) }
                    "website" -> Model.openUrl(s.url)
                }
            }, enabled = s.action !in listOf("on", "installed"), modifier = Modifier.jobBusy(busy)) { JobLabel(label, busy) }
            if (s.installed) OutlinedButton(onClick = { Model.send("charts_uninstall", Model.args("id" to s.id)) }) { Text("Remove") }
        }
    }
}

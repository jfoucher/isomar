package io.github.isomar.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ArrowDownward
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.isomar.Model
import io.github.isomar.Native
import io.github.isomar.Page
import io.github.isomar.StatsTable
import io.github.isomar.TaskSheet
import io.github.isomar.WeatherArea
import io.github.isomar.WeatherEstimate
import io.github.isomar.bool
import io.github.isomar.num
import io.github.isomar.str
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** The task sheets over everything: full height, with their actions at the bottom. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TaskSheets() {
    val s = Model.sheet ?: return
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(onDismissRequest = { Model.sheet = null }, sheetState = state, containerColor = c(Tok.dialogBg)) {
        when (s) {
            is TaskSheet.GetWeather -> GetWeatherSheet(s.area)
            is TaskSheet.AutoRoute -> AutoRouteSheet(s.id)
            is TaskSheet.Stats -> StatsSheet(s.table)
            is TaskSheet.Tides -> TidesSheet(s.lat, s.lon)
            TaskSheet.PlanningOnly -> PlanningOnlySheet()
            TaskSheet.Settings -> SettingsSheet()
            TaskSheet.Connections -> ConnectionsSheet()
            TaskSheet.Monitor -> MonitorSheet()
            TaskSheet.About -> AboutSheet()
        }
    }
}

@Composable
private fun SheetBody(title: String, intro: String = "", content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        Text(title, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(horizontal = 20.dp))
        if (intro.isNotEmpty()) Text(intro, fontSize = 14.sp, color = c(Tok.textMuted), modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp))
        content()
    }
}

@Composable
private fun Actions(action: String, enabled: Boolean = true, onAct: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Spacer(Modifier.weight(1f))
        TextButton(onClick = { Model.sheet = null }) { Text("Cancel") }
        Button(onClick = onAct, enabled = enabled) { Text(action) }
    }
}

// Get weather

@Composable
private fun GetWeatherSheet(area: WeatherArea) {
    var north by remember { mutableStateOf(area.north) }
    var south by remember { mutableStateOf(area.south) }
    var west by remember { mutableStateOf(area.west) }
    var east by remember { mutableStateOf(area.east) }
    var horizon by remember { mutableIntStateOf(2) }
    var step by remember { mutableIntStateOf(1) }
    val groups = remember { mutableStateMapOf<String, Boolean>().apply { Model.weather.groups.forEach { put(it.id, it.default) } } }
    var forecastModel by remember { mutableStateOf(Model.weather.model) }
    val args = Model.args(
        "north" to north, "south" to south, "west" to west, "east" to east, "horizon_index" to horizon, "step_index" to step,
        "groups" to groups.filterValues { it }.keys.sorted(), "model" to forecastModel,
    )
    var estimate by remember { mutableStateOf(WeatherEstimate("—", "—", "—", "—", false)) }
    LaunchedEffect(args.toString()) { Model.call<WeatherEstimate>("weather_estimate", args)?.let { estimate = it } }
    SheetBody("Get weather", "Downloads the forecast for the area you choose.") {
        SectionHeader("Area")
        Row { Column(Modifier.weight(1f)) { Field("North", north) { north = it } }; Column(Modifier.weight(1f)) { Field("South", south) { south = it } } }
        Row { Column(Modifier.weight(1f)) { Field("West", west) { west = it } }; Column(Modifier.weight(1f)) { Field("East", east) { east = it } } }
        SectionHeader("Model")
        ModelPicker(forecastModel) { forecastModel = it }
        SectionHeader("Forecast length")
        Choice(Model.weather.horizons, horizon) { horizon = it }
        SectionHeader("Time step")
        Choice(Model.weather.steps, step) { step = it }
        SectionHeader("Data")
        BoxedList {
            Model.weather.groups.forEachIndexed { i, g ->
                if (i > 0) ListDivider()
                SwitchRow(g.label, groups[g.id] == true) { groups[g.id] = it }
            }
        }
        SectionHeader("What will be downloaded")
        BoxedList {
            for ((l, v) in listOf("Source" to estimate.source, "Forecast run" to estimate.run, "Download size" to estimate.size, "Files" to estimate.files, "Not available" to estimate.missing).filter { it.second.isNotEmpty() }) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 8.dp)) {
                    Text(l, fontSize = 14.sp, color = c(Tok.textMuted), modifier = Modifier.weight(1f))
                    Text(v, fontSize = 14.sp, style = Numeric)
                }
            }
        }
        Actions("Download", enabled = estimate.ok) {
            val e = Model.send("weather_download", args)
            if (e != null) Model.error = io.github.isomar.ErrorRequest("Could not get the weather", e) else Model.sheet = null
        }
    }
}

@Composable
private fun Choice(items: List<String>, selected: Int, pick: (Int) -> Unit) {
    if (items.isEmpty()) return
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        items.forEachIndexed { i, label ->
            SegmentedButton(selected = i == selected, onClick = { pick(i) }, shape = SegmentedButtonDefaults.itemShape(i, items.size)) { Text(label, maxLines = 1, fontSize = 13.sp) }
        }
    }
}

// Auto route

@Composable
private fun AutoRouteSheet(id: Long) {
    var safety by remember { mutableStateOf("5") }
    var clearance by remember { mutableStateOf("0.15") }
    var download by remember { mutableStateOf(true) }
    var offline by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        val d = Model.dict("auto_defaults") ?: return@LaunchedEffect
        safety = "${d.num("safety_depth") ?: 5.0}".removeSuffix(".0")
        clearance = "${d.num("clearance_nm") ?: 0.15}"
        offline = d.bool("offline") ?: false
        if (offline) download = false
    }
    SheetBody(
        "Auto route",
        "Adds waypoints between the points of this route so that the legs keep clear of land and of water shallower than the safety depth. Your own points are kept. Depths are from the chart datum: the tide is not added, and this is not for navigation.",
    ) {
        SectionHeader("Safety")
        Field("Safety depth (m)", safety, number = true) { safety = it }
        Field("Keep clear of land by (nm)", clearance, number = true) { clearance = it }
        SectionHeader("Data")
        BoxedList {
            SwitchRow("Download missing depth data", download && !offline, subtitle = "EMODnet, about half a megabyte for each half degree") { if (!offline) download = it }
        }
        Actions("Compute") {
            Model.send("auto_run", Model.args("id" to id, "safety_depth" to (safety.toDoubleOrNull() ?: 5.0), "clearance_nm" to (clearance.toDoubleOrNull() ?: 0.15), "download" to download))
            Model.sheet = null
        }
    }
}

// Statistics

@Composable
private fun StatsSheet(t: StatsTable) {
    // One alternative at a time, picked by number (the one ticked on the chart first), its sections as boxed
    // lists of label and value: a table of small columns did not fit a phone, nor look like the rest.
    val alts = Model.routerCard?.alternatives?.takeIf { it.size == t.heads.size }
    var column by remember { mutableIntStateOf(alts?.indexOfFirst { it.selected }?.coerceAtLeast(0) ?: 0) }
    SheetBody("Route statistics") {
        if (t.heads.size > 1) {
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
                t.heads.forEachIndexed { i, h ->
                    SegmentedButton(selected = column == i, onClick = { column = i }, shape = SegmentedButtonDefaults.itemShape(i, t.heads.size)) { Text(h) }
                }
            }
            Row(Modifier.padding(horizontal = 20.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                alts?.getOrNull(column)?.let { DashSwatch(it.colour, it.dash) }
                Text(alts?.getOrNull(column)?.title ?: t.heads.getOrElse(column) { "" }, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
            }
        }
        for (s in t.sections) {
            SectionHeader(s.title)
            BoxedList {
                s.rows.forEachIndexed { i, r ->
                    if (i > 0) ListDivider()
                    val label = runCatching { r[0].jsonPrimitive.contentOrNull }.getOrNull() ?: ""
                    val cells = runCatching { r[1].jsonArray.map { it.jsonPrimitive.contentOrNull ?: "" } }.getOrNull() ?: emptyList()
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(label, fontSize = 15.sp, modifier = Modifier.weight(1f))
                        Text(cells.getOrElse(column) { "" }, fontSize = 15.sp, style = Numeric, textAlign = androidx.compose.ui.text.style.TextAlign.End)
                    }
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(16.dp)) { Spacer(Modifier.weight(1f)); Button(onClick = { Model.sheet = null }) { Text("Done") } }
    }
}

// Tides

@Composable
private fun TidesSheet(lat: Double, lon: Double) {
    var info by remember { mutableStateOf<JsonObject?>(null) }
    var failed by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(lat, lon) {
        val text = withContext(Dispatchers.IO) { Native.tideInfo(lat, lon) }
        val o = runCatching { Model.json.parseToJsonElement(text).jsonObject }.getOrNull()
        if (o == null) failed = "No answer" else if (o["error"] != null) failed = o.str("error") else info = o
    }
    val i = info
    when {
        i != null -> SheetBody(i.str("title").ifEmpty { "Tides" }) {
            val pts = (i["curve"] as? JsonArray)?.mapNotNull { p -> p.jsonArray.let { a -> a[0].jsonPrimitive.doubleOrNull?.let { t -> a[1].jsonPrimitive.doubleOrNull?.let { h -> t to h } } } } ?: emptyList()
            if (pts.size > 1) {
                val now = i.num("now") ?: 0.0
                val line = c(Tok.ais); val mark = c(Tok.route); val axis = c(Tok.border)
                Canvas(Modifier.fillMaxWidth().height(150.dp).padding(16.dp)) {
                    val t0 = pts.first().first; val t1 = pts.last().first
                    val lo = pts.minOf { it.second }; val hi = pts.maxOf { it.second }
                    fun x(t: Double) = ((t - t0) / (t1 - t0) * size.width).toFloat()
                    fun y(h: Double) = (size.height - (h - lo) / (hi - lo).coerceAtLeast(0.1) * size.height).toFloat()
                    drawLine(axis, Offset(0f, size.height), Offset(size.width, size.height), 1f)
                    val path = Path().apply { pts.forEachIndexed { k, (t, h) -> if (k == 0) moveTo(x(t), y(h)) else lineTo(x(t), y(h)) } }
                    drawPath(path, line, style = Stroke(2.dp.toPx()))
                    if (now in t0..t1) drawLine(mark, Offset(x(now), 0f), Offset(x(now), size.height), 2.dp.toPx())
                }
            }
            Rows((i["station"] as? JsonArray)?.rows() ?: emptyList())
            val extremes = (i["extremes"] as? JsonArray)?.map { it.jsonObject } ?: emptyList()
            if (extremes.isNotEmpty()) {
                SectionHeader(i.str("next_title").ifEmpty { "Next" })
                BoxedList {
                    extremes.forEachIndexed { k, e ->
                        if (k > 0) ListDivider()
                        Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                            Icon(if (e.bool("high") == true) Icons.Outlined.ArrowUpward else Icons.Outlined.ArrowDownward, e.str("name"), Modifier.size(18.dp))
                            Text(e.str("time"), fontSize = 14.sp, style = Numeric, modifier = Modifier.padding(start = 10.dp).weight(1f))
                            Text(e.str("height"), fontSize = 14.sp, style = Numeric)
                        }
                    }
                }
            }
            for (g in (i["groups"] as? JsonArray) ?: JsonArray(emptyList())) {
                val a = g.jsonArray
                val title = a.firstOrNull()?.jsonPrimitive?.contentOrNull ?: ""
                if (title.isNotEmpty()) SectionHeader(title)
                Rows((a.lastOrNull() as? JsonArray)?.rows() ?: emptyList())
            }
        }
        failed != null -> SheetBody("No tide information", failed!!) {}
        else -> Row(Modifier.padding(24.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
            Text("Looking up the tides…")
        }
    }
}

private fun JsonArray.rows(): List<List<String>> = map { r -> r.jsonArray.map { it.jsonPrimitive.contentOrNull ?: "" } }

@Composable
private fun Rows(rows: List<List<String>>) {
    if (rows.isEmpty()) return
    Spacer(Modifier.height(8.dp))
    BoxedList {
        rows.forEachIndexed { k, r ->
            if (k > 0) ListDivider()
            Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 10.dp)) {
                Text(r.getOrElse(0) { "" }, fontSize = 14.sp, color = c(Tok.textMuted), modifier = Modifier.weight(0.4f))
                Text(r.getOrElse(1) { "" }, fontSize = 14.sp, modifier = Modifier.weight(0.6f))
            }
        }
    }
}

// Planning only

@Composable
private fun PlanningOnlySheet() {
    SheetBody("Planning only") {
        Row(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Icon(Icons.Outlined.WarningAmber, null, tint = c(Tok.warning))
            Text(
                "No official chart that you have added covers this part of the map. What you see here is built from open data (OpenStreetMap coastlines, seamarks and EMODnet depths). It is good for planning and not for navigation.",
                fontSize = 15.sp,
            )
        }
        Text("Add the electronic charts you have licensed, or install a free official source for your area, on the Charts page.", fontSize = 14.sp, color = c(Tok.textMuted), modifier = Modifier.padding(horizontal = 20.dp))
        Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = { Model.sheet = null; Model.showPage(Page.Charts) }) { Text("Open Charts") }
            Spacer(Modifier.weight(1f))
            Button(onClick = { Model.sheet = null }) { Text("Close") }
        }
    }
}

// Settings

/** The settings a phone needs, read from and written to the engine's settings (`settings_patch`). */
@Composable
private fun SettingsSheet() {
    val values = remember { mutableStateMapOf<String, Any?>() }
    LaunchedEffect(Unit) {
        for ((k, v) in Model.settings) {
            val p = v as? kotlinx.serialization.json.JsonPrimitive
            values[k] = when {
                p == null -> v
                p.isString -> p.content
                p.content == "true" || p.content == "false" -> p.content == "true"
                else -> p.doubleOrNull
            }
        }
    }
    fun set(key: String, v: Any) {
        values[key] = v
        Model.send("settings_patch", Model.args(key to v))
    }
    fun str(key: String, d: String) = values[key] as? String ?: d
    fun num(key: String, d: Double) = values[key] as? Double ?: d
    fun flag(key: String) = values[key] as? Boolean ?: false

    SheetBody("Settings") {
        SectionHeader("Display")
        Choice(listOf("System", "Day", "Dusk", "Night"), listOf("auto", "day", "dusk", "night").indexOf(Model.mode).coerceAtLeast(0)) { Model.changeMode(listOf("auto", "day", "dusk", "night")[it]) }
        SectionHeader("Screen")
        BoxedList {
            SwitchRow("Dim screen at night", Model.dimAtNight, subtitle = "Lowers brightness while Isomar is open at Night") { Model.dimAtNight = it }
            ListDivider()
            SwitchRow("Keep screen on under way", Model.keepOnUnderway) { Model.keepOnUnderway = it }
        }
        SectionHeader("Chart")
        BoxedList {
            SwitchRow("Under-way layout when a route starts", flag("underway_auto")) { set("underway_auto", it) }
            ListDivider()
            SwitchRow("Light sectors at their full range", flag("full_sectors")) { set("full_sectors", it) }
            ListDivider()
            Picker("Text size on the chart", listOf("75 %", "100 %", "125 %", "150 %", "200 %"), listOf(75.0, 100.0, 125.0, 150.0, 200.0), num("text_scale", 100.0)) { set("text_scale", it) }
            ListDivider()
            Picker("Chart style", listOf("Paper", "ECDIS"), listOf("paper", "ecdis"), str("chart_style", "paper")) { set("chart_style", it) }
        }
        SectionHeader("Units")
        BoxedList {
            Picker("Depth", listOf("Metres", "Feet", "Fathoms"), listOf("m", "ft", "fa"), str("depth_unit", "m")) { set("depth_unit", it) }
            ListDivider()
            Picker("Speed", listOf("Knots", "km/h", "m/s"), listOf("kn", "km/h", "m/s"), str("speed_unit", "kn")) { set("speed_unit", it) }
            ListDivider()
            Picker("Distance", listOf("Nautical miles", "Kilometres", "Miles"), listOf("nm", "km", "mi"), str("distance_unit", "nm")) { set("distance_unit", it) }
            ListDivider()
            Picker("Bearings", listOf("True", "Magnetic"), listOf(false, true), flag("bearing_magnetic")) { set("bearing_magnetic", it) }
        }
        SectionHeader("Alarms")
        BoxedList {
            Stepper("Collision: closest approach under", "%.1f nm", num("ais_cpa_nm", 0.5), 0.1, 0.1..5.0) { set("ais_cpa_nm", it) }
            ListDivider()
            Stepper("Collision: within", "%.0f min", num("ais_tcpa_min", 20.0), 1.0, 2.0..60.0) { set("ais_tcpa_min", it) }
            ListDivider()
            Stepper("Anchor circle", "%.2f nm", num("anchor_radius_nm", 0.05), 0.01, 0.01..1.0) { set("anchor_radius_nm", it) }
            ListDivider()
            Stepper("Off course beyond", "%.2f nm", num("xte_limit_nm", 0.1), 0.02, 0.02..2.0) { set("xte_limit_nm", it) }
            ListDivider()
            Stepper("Shallow water under (0 is off)", "%.0f m", num("shallow_alarm_m", 0.0), 1.0, 0.0..30.0) { set("shallow_alarm_m", it) }
            ListDivider()
            Stepper("Wind above (0 is off)", "%.0f kn", num("wind_limit_kn", 0.0), 1.0, 0.0..60.0) { set("wind_limit_kn", it) }
        }
        SectionHeader("Depths and vectors")
        BoxedList {
            Stepper("Safety contour", "%.0f m", num("safety_depth", 5.0), 1.0, 1.0..50.0) { set("safety_depth", it) }
            ListDivider()
            Stepper("Course vector", "%.0f min", num("vector_minutes", 6.0), 1.0, 1.0..30.0) { set("vector_minutes", it) }
        }
        SectionHeader("Boat")
        BoxedList {
            Stepper("Length", "%.1f m", num("boat_length_m", 10.0), 0.5, 3.0..60.0) { set("boat_length_m", it) }
            ListDivider()
            Stepper("Beam", "%.1f m", num("boat_beam_m", 3.4), 0.1, 1.0..15.0) { set("boat_beam_m", it) }
            ListDivider()
            SwitchRow("Record performance under way", flag("perf_recording"), subtitle = "Learns what the boat really does, for weather routing") { set("perf_recording", it) }
        }
        Row(Modifier.fillMaxWidth().padding(16.dp)) { Spacer(Modifier.weight(1f)); Button(onClick = { Model.sheet = null }) { Text("Done") } }
    }
}

@Composable
private fun <T> Picker(label: String, labels: List<String>, values: List<T>, current: T, pick: (T) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val i = values.indexOf(current).coerceAtLeast(0)
    androidx.compose.foundation.layout.Box {
        ListRow(label, onClick = { open = true }) {
            Text(labels[i], fontSize = 15.sp, color = c(Tok.textMuted), modifier = Modifier.padding(end = 12.dp))
        }
        androidx.compose.material3.DropdownMenu(open, onDismissRequest = { open = false }) {
            labels.forEachIndexed { k, l -> androidx.compose.material3.DropdownMenuItem(text = { Text(l) }, onClick = { open = false; pick(values[k]) }) }
        }
    }
}

@Composable
private fun Stepper(label: String, format: String, value: Double, step: Double, range: ClosedFloatingPointRange<Double>, set: (Double) -> Unit) {
    val round = { v: Double -> Math.round(v / step) * step }
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp).height(56.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(label, fontSize = 15.sp, modifier = Modifier.weight(1f))
        TextButton(onClick = { set(round((value - step).coerceIn(range))) }) { Text("−", fontSize = 20.sp) }
        Text(String.format(java.util.Locale.getDefault(), format, value), fontSize = 15.sp, style = Numeric)
        TextButton(onClick = { set(round((value + step).coerceIn(range))) }) { Text("+", fontSize = 20.sp) }
    }
}

// Connections

/** A connection as the editor holds it (the engine's ConnectionConfig, `connections` / `connection_save`). */
private data class Conn(
    val id: String, val name: String = "", val kind: String = "tcp", val host: String = "192.168.1.1", val port: String = "10110",
    val url: String = "http://192.168.1.1:3000", val path: String = "", val rate: String = "0", val enabled: Boolean = true,
    val filterMode: String = "off", val filterItems: String = "", val describe: String = "", val status: String = "",
) {
    fun json(): JsonObject {
        // The engine's `Kind`, tagged by "type" (crates/isomar-core/src/comm/source.rs).
        val k: Map<String, Any> = when (kind) {
            "udp" -> mapOf("type" to "udp", "port" to (port.toIntOrNull() ?: 10110))
            "gpsd" -> mapOf("type" to "gpsd", "host" to host, "port" to (port.toIntOrNull() ?: 2947))
            "signalk" -> mapOf("type" to "signal_k", "url" to url)
            "file" -> mapOf("type" to "file", "path" to path, "rate_hz" to (rate.toDoubleOrNull() ?: 0.0))
            "simulator" -> mapOf("type" to "simulator", "lat" to 48.0, "lon" to -4.5, "cog" to 90.0, "sog" to 6.0)
            else -> mapOf("type" to "tcp", "host" to host, "port" to (port.toIntOrNull() ?: 10110))
        }
        return Model.args("id" to id, "name" to name.ifEmpty { id }, "kind" to k, "enabled" to enabled, "filter_mode" to filterMode, "filter_items" to filterItems)
    }

    companion object {
        fun from(row: JsonObject): Conn? {
            val cfg = row["config"] as? JsonObject ?: return null
            val base = Conn(
                id = cfg.str("id"), name = cfg.str("name"), enabled = cfg.bool("enabled") ?: true,
                filterMode = cfg.str("filter_mode").ifEmpty { "off" }, filterItems = cfg.str("filter_items"),
                describe = row.str("describe"), status = row.str("status"),
            )
            val k = cfg["kind"] as? JsonObject ?: return base
            return when (k.str("type")) {
                "tcp" -> base.copy(kind = "tcp", host = k.str("host"), port = k.str("port"))
                "udp" -> base.copy(kind = "udp", port = k.str("port"))
                "gpsd" -> base.copy(kind = "gpsd", host = k.str("host"), port = k.str("port"))
                "signal_k" -> base.copy(kind = "signalk", url = k.str("url"))
                "file" -> base.copy(kind = "file", path = k.str("path"), rate = k.str("rate_hz"))
                "serial" -> base.copy(kind = "serial", path = k.str("path"))
                "simulator" -> base.copy(kind = "simulator")
                else -> base
            }
            return base
        }
    }
}

private val kinds = listOf("tcp" to "TCP client", "udp" to "UDP listener", "gpsd" to "gpsd", "signalk" to "Signal K", "file" to "NMEA file", "simulator" to "Simulator")

/** The boat's instruments: NMEA 0183 over the boat's Wi-Fi (TCP or UDP), gpsd, Signal K, a file, the simulator. */
@Composable
fun ConnectionsSheet() {
    var rows by remember { mutableStateOf(emptyList<Conn>()) }
    var editing by remember { mutableStateOf<Conn?>(null) }
    LaunchedEffect(editing) {
        while (editing == null) {
            rows = runCatching { Model.json.parseToJsonElement(Model.callText("connections")).jsonArray.mapNotNull { Conn.from(it.jsonObject) } }.getOrElse { emptyList() }
            kotlinx.coroutines.delay(1000)
        }
    }
    val e = editing
    if (e != null) {
        ConnectionEditor(e) { saved ->
            if (saved != null) Model.send("connection_save", saved.json())?.let { Model.error = io.github.isomar.ErrorRequest("Could not save the connection", it) }
            Model.connectionsChanged()
            editing = null
        }
        return
    }
    SheetBody("Connections", "Instruments and GPS send NMEA 0183 over the boat's network or from a file. Without any, the phone's own GPS gives the position.") {
        if (rows.isEmpty()) EmptyText("No connections yet.")
        else BoxedList {
            rows.forEachIndexed { i, c ->
                if (i > 0) ListDivider()
                val tint = c(if (c.status.startsWith("Connected")) Tok.ok else if (c.status.startsWith("Disc")) Tok.alarm else Tok.textDisabled)
                ListRow(c.name, subtitle = listOf(c.describe, c.status).filter { it.isNotEmpty() }.joinToString(" · "), icon = Icons.Outlined.Circle, iconTint = tint, dim = !c.enabled, onClick = { editing = c }) {
                    androidx.compose.material3.IconButton(onClick = { Model.send("connection_remove", Model.args("id" to c.id)); Model.connectionsChanged() }) {
                        Icon(Icons.Outlined.Delete, "Remove")
                    }
                }
            }
        }
        SectionHeader("USB")
        LaunchedEffect(Unit) { io.github.isomar.UsbNmea.refresh() }
        val usb = io.github.isomar.UsbNmea
        if (usb.devices.isEmpty()) EmptyText("Plug in a USB serial adapter (NMEA 0183 instruments, a GPS, an AIS receiver) with a USB-C cable or adapter.")
        else BoxedList {
            Picker("Speed", listOf("4800 baud (NMEA)", "9600 baud", "38400 baud (AIS)", "115200 baud"), listOf(4800, 9600, 38400, 115200), usb.baud) { usb.baud = it }
            for (d in usb.devices) {
                ListDivider()
                val on = usb.open == d
                SwitchRow(d, on, subtitle = if (on) "Reading at ${usb.baud} baud" else "") { if (it) usb.connect(d) else usb.close() }
            }
        }
        Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            androidx.compose.material3.OutlinedButton(onClick = { Model.sheet = TaskSheet.Monitor }) { Text("Data monitor") }
            Spacer(Modifier.weight(1f))
            Button(onClick = { editing = Conn(id = java.util.UUID.randomUUID().toString().take(8)) }) { Text("Add") }
        }
    }
}

@Composable
private fun ConnectionEditor(start: Conn, done: (Conn?) -> Unit) {
    var c by remember { mutableStateOf(start) }
    SheetBody(start.name.ifEmpty { "New connection" }) {
        Field("Name", c.name) { c = c.copy(name = it) }
        Spacer(Modifier.height(8.dp))
        BoxedList { Picker("Kind", kinds.map { it.second }, kinds.map { it.first }, c.kind) { c = c.copy(kind = it) } }
        Spacer(Modifier.height(8.dp))
        when (c.kind) {
            "tcp", "gpsd" -> { Field("Host", c.host) { c = c.copy(host = it) }; Field("Port", c.port, number = true) { c = c.copy(port = it) } }
            "udp" -> {
                Field("Port", c.port, number = true) { c = c.copy(port = it) }
                EmptyText("Listens on the phone's Wi-Fi for what the boat broadcasts.")
            }
            "signalk" -> Field("URL", c.url) { c = c.copy(url = it) }
            "file" -> { Field("File", c.path) { c = c.copy(path = it) }; Field("Lines per second (0 is 20)", c.rate, number = true) { c = c.copy(rate = it) } }
            "serial" -> EmptyText("Serial ports are not available on this device.")
            else -> EmptyText("A boat that sails east at 6 knots, with some traffic.")
        }
        BoxedList { SwitchRow("Enabled", c.enabled) { c = c.copy(enabled = it) } }
        SectionHeader("Filter")
        Choice(listOf("None", "Only these", "All but these"), listOf("off", "allow", "deny").indexOf(c.filterMode).coerceAtLeast(0)) { c = c.copy(filterMode = listOf("off", "allow", "deny")[it]) }
        if (c.filterMode != "off") Field("Sentence types (RMC, GGA…)", c.filterItems) { c = c.copy(filterItems = it) }
        Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Spacer(Modifier.weight(1f))
            TextButton(onClick = { done(null) }) { Text("Cancel") }
            Button(onClick = { done(c) }) { Text("Save") }
        }
    }
}

/** What arrives, line by line, and how many lines were not NMEA. */
@Composable
fun MonitorSheet() {
    var lines by remember { mutableStateOf(emptyList<JsonObject>()) }
    var bad by remember { mutableIntStateOf(0) }
    var paused by remember { mutableStateOf(false) }
    LaunchedEffect(paused) {
        while (!paused) {
            Model.dict("monitor", Model.args("since" to 0))?.let { d ->
                bad = d.num("bad")?.toInt() ?: 0
                lines = (d["lines"] as? JsonArray)?.map { it.jsonObject }?.takeLast(200) ?: emptyList()
            }
            kotlinx.coroutines.delay(500)
        }
    }
    SheetBody("Data monitor", "$bad lines were not valid NMEA") {
        Column(Modifier.padding(horizontal = 16.dp)) {
            for (l in lines.asReversed()) {
                Text(
                    "${l.str("time")}  ${l.str("source")}  ${l.str("text")}", fontSize = 11.sp, maxLines = 2,
                    fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                    color = if (l.bool("ok") == false) c(Tok.alarm) else c(Tok.text),
                )
            }
        }
        Row(Modifier.fillMaxWidth().padding(16.dp)) {
            TextButton(onClick = { paused = !paused }) { Text(if (paused) "Resume" else "Pause") }
            Spacer(Modifier.weight(1f))
            Button(onClick = { Model.sheet = TaskSheet.Connections }) { Text("Done") }
        }
    }
}

/** What the app is, where its source is, and whose data it draws. */
@Composable
fun AboutSheet() {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val version = remember { runCatching { ctx.packageManager.getPackageInfo(ctx.packageName, 0).versionName }.getOrNull() ?: "" }
    SheetBody("Isomar $version") {
        Text(
            "A chart plotter for sailors. Free software under the GNU General Public License, version 3 or later, with an additional permission for app stores.",
            fontSize = 15.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
        )
        Text(
            "Not for navigation unless an official chart you have licensed covers the area.",
            fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
        )
        Text(
            "Data: OpenStreetMap contributors (ODbL), Open Waters seamap tiles (CC BY 4.0), OpenFreeMap and OpenMapTiles, EMODnet Bathymetry, NOAA GFS forecasts, ECMWF open data forecasts (CC BY 4.0), SHOM open data (Licence Ouverte).",
            fontSize = 13.sp, color = c(Tok.textMuted), modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
        )
        Row(Modifier.fillMaxWidth().padding(16.dp)) {
            TextButton(onClick = { Model.openUrl(SOURCE_URL) }) { Text("Website and source code") }
            Spacer(Modifier.weight(1f))
            Button(onClick = { Model.sheet = null }) { Text("Close") }
        }
    }
}

/** Where the source of each version is published (COPYING.appstore asks the app to say). */
const val SOURCE_URL = "https://isomar.marinetechs.com/"

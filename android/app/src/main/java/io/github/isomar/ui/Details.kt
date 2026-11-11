package io.github.isomar.ui

import android.app.DatePickerDialog
import android.app.TimePickerDialog
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.outlined.AltRoute
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Navigation
import androidx.compose.material.icons.outlined.WbCloudy
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.isomar.Chip
import io.github.isomar.DeparturesInfo
import io.github.isomar.Detail
import io.github.isomar.Model
import io.github.isomar.StatsTable
import io.github.isomar.TaskSheet
import io.github.isomar.bool
import io.github.isomar.num
import io.github.isomar.str
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Locale

@Composable
fun DetailView(d: Detail) {
    when (d) {
        is Detail.Route -> RouteDetail(d.id)
        is Detail.Waypoint -> WaypointDetail(d.id)
        is Detail.Vessel -> VesselDetail(d.mmsi)
        is Detail.Routing -> RoutingDetail(d.id, d.window)
        Detail.Performance -> PerformancePage()
    }
}

/**
 * The detail is in a sheet of its own (a phone's `ItemSheet`), not pushed on a page: no way back to the page,
 * a close button instead, and closing it is `chartDetail`'s, not `detail`'s.
 */
val LocalStandaloneDetail = androidx.compose.runtime.compositionLocalOf { false }

/** Back to the page (or to `parent`), and the title of what is shown; or the title and Close, standing alone. */
@Composable
fun BackBar(title: String, parent: Detail? = null, parentTitle: String? = null) {
    if (LocalStandaloneDetail.current) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(start = 20.dp, end = 4.dp)) {
            Text(title, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, modifier = Modifier.weight(1f))
            IconButton(onClick = { Model.chartDetail = null }) { Icon(Icons.Outlined.Close, "Close") }
        }
        return
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(start = 4.dp, end = 16.dp)) {
        IconButton(onClick = { Model.detail = parent }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, "Back to ${parentTitle ?: Model.page.title}") }
        Text(title, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, maxLines = 2)
    }
}

@Composable
fun Field(label: String, value: String, number: Boolean = false, onChange: (String) -> Unit) {
    OutlinedTextField(
        value, onChange, label = { Text(label) }, singleLine = true,
        keyboardOptions = if (number) KeyboardOptions(keyboardType = KeyboardType.Decimal) else KeyboardOptions.Default,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
    )
}

private fun chipsOf(v: kotlinx.serialization.json.JsonElement?): List<Chip> =
    runCatching { Model.json.decodeFromJsonElement(kotlinx.serialization.builtins.ListSerializer(Chip.serializer()), v!!) }.getOrElse { emptyList() }

private fun fmtG(v: Double) = if (v == v.toLong().toDouble()) "${v.toLong()}" else "$v"

// Route

@Composable
private fun RouteDetail(id: Long) {
    val standalone = LocalStandaloneDetail.current
    var name by remember(id) { mutableStateOf("") }
    var speed by remember(id) { mutableStateOf("") }
    var departure by remember(id) { mutableStateOf("") }
    var visible by remember(id) { mutableStateOf(true) }
    // The form as last loaded or saved: Save is offered only once it differs.
    var saved by remember(id) { mutableStateOf(listOf<Any>()) }
    var summary by remember(id) { mutableStateOf(emptyList<Chip>()) }
    var legs by remember(id) { mutableStateOf(emptyList<JsonObject>()) }
    var plan by remember(id) { mutableStateOf<JsonObject?>(null) }
    var ensemble by remember(id) { mutableStateOf(emptyList<String>()) }
    var stamp by remember { mutableStateOf(0) }
    LaunchedEffect(id, stamp) {
        val d = Model.dict("route_get", Model.args("id" to id))
        if (d == null) { Model.closeDetail(standalone); return@LaunchedEffect }
        name = d.str("name"); speed = fmtG(d.num("speed_kn") ?: 5.0); departure = d.str("departure"); visible = d.bool("visible") ?: true
        saved = listOf(name, speed, departure, visible)
        summary = chipsOf(d["chips"])
        legs = (d["legs"] as? JsonArray)?.map { it.jsonObject } ?: emptyList()
        plan = d["plan"] as? JsonObject
        ensemble = (d["ensemble"] as? JsonArray)?.mapNotNull { it.jsonPrimitive.contentOrNull } ?: emptyList()
    }
    PageColumn {
        BackBar(name)
        ChipsText(summary, size = 13.sp, modifier = Modifier.padding(horizontal = 20.dp))
        Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { Model.send("start_route", Model.args("id" to id)) }) { Icon(Icons.Filled.PlayArrow, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Follow") }
            OutlinedButton(onClick = { Model.command("edit_route", id) }) { Icon(Icons.Outlined.Edit, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Edit") }
            RowMenu(
                "Reverse" to { Model.command("reverse_route", id); stamp++ },
                (if (visible) "Hide" else "Show") to { Model.command("toggle_route", id); stamp++ },
                "Export GPX" to { Model.pickFile("gpx-export:$id:" + (Model.callText("route_file_name", Model.args("name" to name)).trim('"'))) },
                "Delete…" to { Model.command("delete_route", id); Model.closeDetail(standalone) },
            )
        }
        SectionHeader("Plan a passage")
        BoxedList {
            ListRow("Weather routing", subtitle = "The fastest way with the forecast and the boat's polar", icon = Icons.Outlined.WbCloudy, onClick = { Model.showRouting(id, false) }) {
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = c(Tok.textMuted), modifier = Modifier.padding(end = 8.dp))
            }
            ListDivider()
            ListRow("Best departure", subtitle = "When to leave, over the days the forecast covers", icon = Icons.Outlined.Navigation, onClick = { Model.showRouting(id, true) }) {
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = c(Tok.textMuted), modifier = Modifier.padding(end = 8.dp))
            }
            ListDivider()
            ListRow("Auto route", subtitle = "Adds waypoints to keep the route in safe water", icon = Icons.Outlined.AltRoute, busy = Model.busy("autoroute:$id"), onClick = { Model.job("autoroute:$id") { Model.sheet = TaskSheet.AutoRoute(id) } })
            ListDivider()
            ListRow("Ensemble check", subtitle = "Sails the route through 31 GEFS forecasts for the spread of its arrival and wind; a few minutes' download", icon = Icons.Outlined.WbCloudy, busy = Model.busy("ensemble:$id"), onClick = { Model.job("ensemble:$id") { Model.send("route_ensemble", Model.args("id" to id)) } })
        }
        if (ensemble.isNotEmpty()) {
            SectionHeader("Ensemble")
            BoxedList { Text(ensemble.joinToString("\n"), fontSize = 13.sp, modifier = Modifier.padding(14.dp)) }
        }
        plan?.let { p ->
            SectionHeader("Weather routing")
            BoxedList {
                Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    ChipsText(chipsOf(p["summary"]), size = 13.sp, color = c(Tok.text), maxLines = 4)
                    Text(p.str("forecast"), fontSize = 12.sp, color = c(Tok.textMuted))
                    p.str("bracket").takeIf { it.isNotEmpty() }?.let { Text(it, fontSize = 12.sp, color = c(Tok.textMuted)) }
                    if (p.bool("edited") == true) Text("The route has been edited since: this is the passage as it was planned.", fontSize = 12.sp, color = c(Tok.textMuted))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Spacer(Modifier.weight(1f))
                        OutlinedButton(onClick = { Model.call<StatsTable>("route_plan_stats", Model.args("id" to id))?.let { Model.sheet = TaskSheet.Stats(it) } }) { Text("Statistics") }
                        Button(onClick = {
                            val ok = Model.dict("route_replay", Model.args("id" to id))?.bool("ok") ?: false
                            if (!ok) Model.error = io.github.isomar.ErrorRequest("Could not play the passage", "This route has no plan to play back.")
                        }) { Text("Play") }
                    }
                }
            }
        }
        SectionHeader("Route")
        Field("Name", name) { name = it }
        Field("Speed (kn)", speed, number = true) { speed = it }
        DateTimeRow("Departure", departure, optional = true) { departure = it }
        SwitchRow("Show on the chart", visible) { visible = it }
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
            Spacer(Modifier.weight(1f))
            Button(enabled = listOf(name, speed, departure, visible) != saved, onClick = {
                val e = Model.send("route_save", Model.args("id" to id, "name" to name, "speed_kn" to (speed.toDoubleOrNull() ?: 5.0), "departure" to departure, "visible" to visible))
                if (e != null) Model.error = io.github.isomar.ErrorRequest("Could not save the route", e) else { stamp++; Model.show("Route saved") }
            }) { Text("Save") }
        }
        if (legs.isNotEmpty()) {
            SectionHeader("Legs")
            BoxedList {
                legs.forEachIndexed { i, l ->
                    if (i > 0) ListDivider()
                    Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 10.dp)) {
                        Text("${l.str("from")} → ${l.str("to")}", fontSize = 14.sp, modifier = Modifier.weight(1f))
                        Text(l.str("text"), fontSize = 14.sp, style = Numeric)
                    }
                }
            }
        }
        Spacer(Modifier.height(32.dp))
    }
}

// Waypoint

@Composable
private fun WaypointDetail(id: Long) {
    val standalone = LocalStandaloneDetail.current
    var name by remember(id) { mutableStateOf("") }
    var lat by remember(id) { mutableStateOf("") }
    var lon by remember(id) { mutableStateOf("") }
    var notes by remember(id) { mutableStateOf("") }
    var radius by remember(id) { mutableStateOf("") }
    var visible by remember(id) { mutableStateOf(true) }
    var stamp by remember { mutableStateOf(0) }
    LaunchedEffect(id, stamp) {
        val d = Model.dict("waypoint_get", Model.args("id" to id))
        if (d == null) { Model.closeDetail(standalone); return@LaunchedEffect }
        name = d.str("name"); lat = d.str("latitude"); lon = d.str("longitude"); notes = d.str("description")
        radius = fmtG(d.num("arrival_radius_nm") ?: 0.05); visible = d.bool("visible") ?: true
    }
    PageColumn {
        BackBar(name)
        Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { Model.command("goto_waypoint", id) }) { Icon(Icons.Outlined.Navigation, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Navigate to") }
            OutlinedButton(onClick = { Model.command("delete_waypoint", id); Model.closeDetail(standalone) }) { Text("Delete…") }
        }
        Field("Name", name) { name = it }
        Field("Latitude", lat) { lat = it }
        Field("Longitude", lon) { lon = it }
        Field("Arrival (nm)", radius, number = true) { radius = it }
        Field("Notes", notes) { notes = it }
        SwitchRow("Show on the chart", visible) { visible = it }
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
            Spacer(Modifier.weight(1f))
            Button(onClick = {
                val e = Model.send("waypoint_save", Model.args("id" to id, "name" to name, "latitude" to lat, "longitude" to lon, "description" to notes, "arrival_radius_nm" to (radius.toDoubleOrNull() ?: 0.05), "visible" to visible))
                if (e != null) Model.error = io.github.isomar.ErrorRequest("Could not save the mark", e) else stamp++
            }) { Text("Save") }
        }
        Spacer(Modifier.height(32.dp))
    }
}

// Vessel

@Composable
private fun VesselDetail(mmsi: Long) {
    val standalone = LocalStandaloneDetail.current
    var title by remember(mmsi) { mutableStateOf("") }
    var groups by remember(mmsi) { mutableStateOf(emptyList<JsonObject>()) }
    LaunchedEffect(mmsi) {
        while (true) {
            val d = Model.dict("vessel_get", Model.args("id" to mmsi))
            if (d == null) { Model.closeDetail(standalone); return@LaunchedEffect }
            title = d.str("title")
            groups = (d["groups"] as? JsonArray)?.map { it.jsonObject } ?: emptyList()
            delay(2000)
        }
    }
    PageColumn {
        BackBar(title)
        for (g in groups) {
            SectionHeader(g.str("title").ifEmpty { "Vessel" })
            BoxedList {
                val rows = (g["rows"] as? JsonArray)?.map { r -> r.jsonArray.map { it.jsonPrimitive.contentOrNull ?: "" } } ?: emptyList()
                rows.forEachIndexed { i, r ->
                    if (i > 0) ListDivider()
                    Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 10.dp)) {
                        Text(r.getOrElse(0) { "" }, fontSize = 14.sp, color = c(Tok.textMuted), modifier = Modifier.weight(0.45f))
                        Text(r.getOrElse(1) { "" }, fontSize = 14.sp, modifier = Modifier.weight(0.55f))
                    }
                }
            }
        }
        Spacer(Modifier.height(32.dp))
    }
}

// Weather routing and best departure

private val stampFormat = SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.ROOT)

/**
 * The engine's departure text, "YYYY-MM-DD HH:MM" in local time, picked with the system's date and time
 * pickers. An `optional` one may be empty ("Not set", the pickers then start at the next whole hour) and
 * has a button to clear it.
 */
@Composable
private fun DateTimeRow(label: String, value: String, optional: Boolean = false, onChange: (String) -> Unit) {
    val ctx = LocalContext.current
    ListRow(label, subtitle = value.ifEmpty { "Not set" }, onClick = {
        val cal = Calendar.getInstance()
        val parsed = runCatching { stampFormat.parse(value) }.getOrNull()
        if (parsed != null) cal.time = parsed else { cal.add(Calendar.HOUR_OF_DAY, 1); cal.set(Calendar.MINUTE, 0) }
        DatePickerDialog(ctx, { _, y, m, d ->
            TimePickerDialog(ctx, { _, hh, mm ->
                cal.set(y, m, d, hh, mm)
                onChange(stampFormat.format(cal.time))
            }, cal.get(Calendar.HOUR_OF_DAY), cal.get(Calendar.MINUTE), true).show()
        }, cal.get(Calendar.YEAR), cal.get(Calendar.MONTH), cal.get(Calendar.DAY_OF_MONTH)).show()
    }) {
        if (optional && value.isNotEmpty()) IconButton(onClick = { onChange("") }) { Icon(Icons.Outlined.Close, "Clear $label", tint = c(Tok.textMuted)) }
    }
}

@Composable
private fun RoutingDetail(id: Long, window: Boolean) {
    var name by remember(id) { mutableStateOf("") }
    var summary by remember(id) { mutableStateOf(emptyList<Chip>()) }
    var leave by remember(id) { mutableStateOf(stampFormat.format(java.util.Date())) }
    var earliest by remember(id) { mutableStateOf(leave) }
    var days by remember(id) { mutableDoubleStateOf(3.0) }
    var stepHours by remember(id) { mutableDoubleStateOf(3.0) }
    var steps by remember(id) { mutableStateOf(listOf(1.0, 3.0, 6.0)) }
    val weights = remember(id) { mutableStateMapOf<String, Double>() }
    var presets by remember(id) { mutableStateOf(emptyList<Pair<String, Map<String, Double>>>()) }
    var maxWeight by remember(id) { mutableDoubleStateOf(4.0) }
    var engine by remember(id) { mutableStateOf(false) }
    var motor by remember(id) { mutableDoubleStateOf(5.0) }
    var maxWind by remember(id) { mutableDoubleStateOf(35.0) }
    var perf by remember(id) { mutableDoubleStateOf(100.0) }
    var adjust by remember { mutableStateOf(false) }
    val forecasts = remember(id) { mutableStateListOf(Model.weather.model) }
    var cautious by remember(id) { mutableStateOf(false) }
    var currents by remember(id) { mutableStateOf(true) }
    var currentsDetail by remember(id) { mutableStateOf("") }
    LaunchedEffect(id) {
        val ends = Model.dict("route_defaults", Model.args("id" to id))
        (ends?.get("currents") as? JsonObject)?.let { cur ->
            currents = cur.bool("on") != false
            currentsDetail = cur.str("detail")
        }
        val chosen = (ends?.get("models") as? JsonArray)?.mapNotNull { it.jsonPrimitive.contentOrNull } ?: emptyList()
        if (chosen.isNotEmpty()) { forecasts.clear(); forecasts.addAll(chosen) }
        Model.dict("route_get", Model.args("id" to id))?.let { r ->
            name = r.str("name")
            val way = "${ends?.str("from") ?: ""} to ${ends?.str("to") ?: ""}"
            val nm = String.format(Locale.getDefault(), "%.1f nm", r.num("distance_nm") ?: 0.0)
            summary = listOf(Chip(null, way, way), Chip("distance", nm, nm))
        }
        ends?.str("departure")?.takeIf { it.isNotEmpty() }?.let { leave = it }
        val d = Model.dict("window_defaults", Model.args("id" to id)) ?: return@LaunchedEffect
        d.str("earliest").takeIf { it.isNotEmpty() }?.let { earliest = it }
        days = d.num("days") ?: 3.0
        stepHours = d.num("step_hours") ?: 3.0
        steps = (d["steps"] as? JsonArray)?.mapNotNull { it.jsonPrimitive.doubleOrNull } ?: steps
        (d["weights"] as? JsonObject)?.forEach { (k, v) -> (v as? JsonPrimitive)?.doubleOrNull?.let { weights[k] = it } }
        maxWeight = d.num("max_weight") ?: 4.0
        cautious = d.bool("cautious") == true
        presets = (d["presets"] as? JsonArray)?.map { p ->
            val o = p.jsonObject
            o.str("name") to ((o["weights"] as? JsonObject)?.mapNotNull { (k, v) -> (v as? JsonPrimitive)?.doubleOrNull?.let { k to it } }?.toMap() ?: emptyMap())
        } ?: emptyList()
    }
    val boat = mapOf("performance_pct" to perf, "use_engine" to engine, "motor_kn" to motor, "max_wind_kn" to maxWind)
    PageColumn {
        BackBar(if (window) "Best departure" else "Weather routing", parent = Detail.Route(id), parentTitle = "Route")
        ListRow(name, chips = summary, icon = Icons.Outlined.AltRoute)
        // Once found, the departures come first (design/screens/android-phone-best-departure.png): the form
        // that found them is below, to search again.
        val found = Model.departures?.takeIf { it.routeId == id }
        if (window && found != null) DepartureResults(found)
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
            SegmentedButton(selected = !window, onClick = { Model.detail = Detail.Routing(id, false) }, shape = SegmentedButtonDefaults.itemShape(0, 2)) { Text("At a time") }
            SegmentedButton(selected = window, onClick = { Model.detail = Detail.Routing(id, true) }, shape = SegmentedButtonDefaults.itemShape(1, 2)) { Text("Best time") }
        }
        BoxedList {
            if (!window) {
                DateTimeRow("Leave at", leave) { leave = it }
            } else {
                DateTimeRow("Earliest", earliest) { earliest = it }
                ListDivider()
                StepRow("Over", "${days.toInt()} days", { days = (days - 1).coerceAtLeast(1.0) }, { days = (days + 1).coerceAtMost(14.0) })
                ListDivider()
                StepRow("Try a departure", "Every ${stepHours.toInt()} h",
                    { steps.lastOrNull { it < stepHours }?.let { stepHours = it } },
                    { steps.firstOrNull { it > stepHours }?.let { stepHours = it } })
            }
        }
        // A switch per model: several are each routed on and compared.
        SectionHeader("Forecasts")
        BoxedList {
            Model.weather.models.forEachIndexed { i, m ->
                if (i > 0) ListDivider()
                SwitchRow(m.label, m.id in forecasts, subtitle = if (m.loaded) m.detail else "None loaded: downloaded first. ${m.about}") { on -> if (on) forecasts.add(m.id) else forecasts.remove(m.id) }
            }
        }
        BoxedList {
            SwitchRow("Sail in the current", currents, subtitle = currentsDetail) { currents = it }
        }
        if (window && forecasts.size > 1) {
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
                SegmentedButton(selected = !cautious, onClick = { cautious = false }, shape = SegmentedButtonDefaults.itemShape(0, 2)) { Text("Average forecast") }
                SegmentedButton(selected = cautious, onClick = { cautious = true }, shape = SegmentedButtonDefaults.itemShape(1, 2)) { Text("Worst forecast") }
            }
        }
        if (window) {
            SectionHeader("What matters")
            if (presets.isNotEmpty()) {
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
                    presets.forEachIndexed { i, (pn, pw) ->
                        SegmentedButton(selected = pw.all { (k, v) -> weights[k] == v }, onClick = { weights.putAll(pw) }, shape = SegmentedButtonDefaults.itemShape(i, presets.size)) { Text(pn, maxLines = 1) }
                    }
                }
            }
            TextButton(onClick = { adjust = !adjust }, modifier = Modifier.padding(horizontal = 8.dp)) { Text(if (adjust) "Hide the weights" else "Adjust") }
            if (adjust) {
                for ((key, label) in listOf("time" to "Time at sea", "upwind" to "Upwind", "engine" to "Engine", "strong_wind" to "Strong wind", "gusts" to "Gusts", "waves" to "Rough sea", "night" to "Night", "manoeuvres" to "Tacks and gybes", "waiting" to "Leaving soon", "holding" to "Waiting for the stream", "wind_against_tide" to "Wind against tide")) {
                    Column(Modifier.padding(horizontal = 20.dp)) {
                        Text(label, fontSize = 14.sp)
                        Slider(value = (weights[key] ?: 0.0).toFloat(), onValueChange = { weights[key] = (Math.round(it * 4) / 4.0) }, valueRange = 0f..maxWeight.toFloat())
                    }
                }
            }
        }
        SectionHeader("Boat")
        BoxedList {
            PolarRow()
            ListDivider()
            SwitchRow("Use the engine in light wind", engine) { engine = it }
            if (engine) { ListDivider(); StepRow("Engine speed", "${fmtG(motor)} kn", { motor = (motor - 0.5).coerceAtLeast(2.0) }, { motor = (motor + 0.5).coerceAtMost(12.0) }) }
            ListDivider()
            StepRow("Avoid wind above", "${maxWind.toInt()} kn", { maxWind = (maxWind - 1).coerceAtLeast(15.0) }, { maxWind = (maxWind + 1).coerceAtMost(60.0) })
            ListDivider()
            StepRow("Performance", "${perf.toInt()} % of the polar", { perf = (perf - 5).coerceAtLeast(50.0) }, { perf = (perf + 5).coerceAtMost(120.0) })
        }
        // While a route is computed: off, with a spinner, and pressing it shows the running jobs.
        Button(
            onClick = {
                Model.job("route") {
                    val models = Model.weather.models.map { it.id }.filter { it in forecasts }
                    val e = if (!window) Model.send("route_compute", Model.args("id" to id, "departure" to leave, "boat" to boat, "models" to models, "currents" to currents))
                    else Model.send("window_run", Model.args("id" to id, "earliest" to earliest, "days" to days, "step_hours" to stepHours, "weights" to weights.toMap(), "boat" to boat, "models" to models, "cautious" to cautious, "currents" to currents))
                    if (e != null) Model.error = io.github.isomar.ErrorRequest(if (window) "Best departure" else "Weather route", e)
                }
            },
            modifier = Modifier.fillMaxWidth().padding(16.dp).height(Size.hitMinAndroid).jobBusy(Model.busy("route")),
        ) { JobLabel(if (window) "Find the best departure" else "Compute the route", Model.busy("route")) }
        EmptyText("The forecast for the passage is downloaded first if it is not already loaded. Planning only: not for navigation.")

        Spacer(Modifier.height(32.dp))
    }
}

@kotlinx.serialization.Serializable private data class PolarEntry(val id: String = "", val name: String = "")
@kotlinx.serialization.Serializable private data class PolarList(val active: String = "", val polars: List<PolarEntry> = emptyList())

/** Which polar of the library the route sails with: the one in use, which the performance log learns into too. */
@Composable
private fun PolarRow() {
    var list by remember { mutableStateOf(Model.call<PolarList>("polar_list") ?: PolarList()) }
    var open by remember { mutableStateOf(false) }
    val current = list.polars.firstOrNull { it.id == list.active }
    Row(Modifier.fillMaxWidth().clickable { open = true }.padding(horizontal = 16.dp).height(56.dp), verticalAlignment = Alignment.CenterVertically) {
        Text("Polar", fontSize = 16.sp, modifier = Modifier.weight(1f))
        androidx.compose.foundation.layout.Box {
            Text(current?.name ?: "", fontSize = 15.sp, color = c(Tok.accent))
            androidx.compose.material3.DropdownMenu(open, onDismissRequest = { open = false }) {
                for (p in list.polars) androidx.compose.material3.DropdownMenuItem(text = { Text(p.name) }, onClick = {
                    open = false
                    val e = Model.send("polar_activate", Model.args("id" to p.id))
                    if (e != null) Model.error = io.github.isomar.ErrorRequest("Could not use the polar", e)
                    list = Model.call<PolarList>("polar_list") ?: list
                })
            }
        }
    }
}

@Composable
private fun StepRow(label: String, value: String, minus: () -> Unit, plus: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp).height(56.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(label, fontSize = 16.sp, modifier = Modifier.weight(1f))
        TextButton(onClick = minus) { Text("−", fontSize = 20.sp) }
        Text(value, fontSize = 15.sp, style = Numeric)
        TextButton(onClick = plus) { Text("+", fontSize = 20.sp) }
    }
}

/** The results of a best-departure search: the cost of each departure as a bar, and the best ten. */
@Composable
private fun DepartureResults(d: DeparturesInfo) {
    val pick = { i: Int -> Model.send("departure_select", Model.args("i" to i)); Model.send("departure_preview") }
    SectionHeader("Best departures") {
        TextButton(onClick = { Model.call<StatsTable>("departure_compare")?.let { Model.sheet = TaskSheet.Stats(it) } }, enabled = d.canCompare) { Text("Compare") }
    }
    EmptyText(d.summary)
    if (d.legend.isNotEmpty()) {
        Row(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            for (l in d.legend) Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(8.dp).clip(CircleShape).background(c(Tok.byName(l.colour) ?: Tok.route)))
                Spacer(Modifier.size(4.dp)); Text(l.label, fontSize = 12.sp)
            }
        }
    }
    val dotColours = d.legend.associate { it.colour to c(Tok.byName(it.colour) ?: Tok.route) }
    val dotRing = c(Tok.viewBg)
    val night = c(Tok.costNightBand); val grid = c(Tok.border); val axis = c(Tok.borderControl)
    val best = c(Tok.costBest); val mid = c(Tok.costMid); val worst = c(Tok.costWorst); val ink = c(Tok.text)
    Canvas(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).height(110.dp).pointerInput(d) {
            detectTapGestures { p ->
                val f = p.x / size.width
                val i = d.bars.indexOfFirst { f >= it.x && f < it.x + it.w }
                if (i >= 0) pick(i)
            }
        },
    ) {
        val w = size.width; val plot = size.height - 4.dp.toPx()
        for (n in d.nights) if (n.size == 2) drawRect(night, Offset((n[0] * w).toFloat(), 0f), androidx.compose.ui.geometry.Size(((n[1] - n[0]) * w).toFloat().coerceAtLeast(1f), plot))
        for (day in d.days) drawLine(grid, Offset((day.x * w).toFloat(), 0f), Offset((day.x * w).toFloat(), size.height), 1f)
        for (b in d.bars) {
            val h = (b.h * (plot - 6)).toFloat().coerceAtLeast(3f)
            val slot = (b.w * w).toFloat()
            val colour = if (!b.ok) worst else if (b.best) best else if (b.h < 0.6) mid else worst
            val x = (b.x * w).toFloat() + if (slot > 6) 1f else 0f
            drawRoundRect(colour, Offset(x, plot - h), androidx.compose.ui.geometry.Size((slot - if (slot > 6) 2f else 0f).coerceAtLeast(1f), h), CornerRadius(minOf(4f, slot / 2)))
            if (b.selected) drawRoundRect(ink, Offset(x, plot - h), androidx.compose.ui.geometry.Size((slot - 2f).coerceAtLeast(1f), h), CornerRadius(minOf(4f, slot / 2)), style = androidx.compose.ui.graphics.drawscope.Stroke(2.dp.toPx()))
            // Several forecasts: the spread of their costs, and a dot each in its colour.
            val cx = (b.x * w).toFloat() + slot / 2
            b.range?.takeIf { it.size == 2 }?.let { r -> drawLine(ink, Offset(cx, (plot - r[0] * (plot - 6)).toFloat()), Offset(cx, (plot - r[1] * (plot - 6)).toFloat()), 1.5.dp.toPx()) }
            for (dot in b.dots) {
                val y = (plot - dot.h * (plot - 6)).toFloat()
                val radius = (slot / 3f).coerceIn(2f, 3.5f.dp.toPx())
                drawCircle(dotRing, radius + 1f, Offset(cx, y))
                drawCircle(dotColours[dot.colour] ?: ink, radius, Offset(cx, y))
            }
        }
        drawLine(axis, Offset(0f, plot), Offset(w, plot), 1f)
    }
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        for (day in d.days) Text(day.label, fontSize = 11.sp, color = c(Tok.textMuted), modifier = Modifier.weight(1f))
    }
    if (d.rows.isNotEmpty()) {
        Spacer(Modifier.height(8.dp))
        BoxedList {
            d.rows.forEachIndexed { k, r ->
                if (k > 0) ListDivider()
                Column(
                    Modifier.fillMaxWidth().clickable { pick(r.index) }.background(if (r.selected) c(Tok.hover) else androidx.compose.ui.graphics.Color.Transparent)
                        .padding(horizontal = 14.dp, vertical = 10.dp),
                ) {
                    Text(r.title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    ChipsText(r.figures, size = 13.sp, maxLines = 3)
                    ChipsText(r.why, size = 13.sp, maxLines = 3)
                    if (r.models.isNotEmpty()) ChipsText(r.models, size = 13.sp, maxLines = 2)
                }
            }
        }
    }
}

package io.github.isomar

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNamingStrategy
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.serializer

/** A page of the sheet (the desktop's sidebar). */
enum class Page(val title: String) { Passage("Passage"), Weather("Weather"), Traffic("Traffic"), Charts("Charts"), Instruments("Instruments") }

/** What a page pushed on top of itself shows. */
sealed interface Detail {
    data class Route(val id: Long) : Detail
    data class Waypoint(val id: Long) : Detail
    data class Vessel(val mmsi: Long) : Detail
    /** Weather routing for a route: one departure, or the best one. */
    data class Routing(val id: Long, val window: Boolean) : Detail
    /** The performance log, pushed from Instruments. */
    data object Performance : Detail
}

/** A task sheet over everything. */
sealed interface TaskSheet {
    data class GetWeather(val area: WeatherArea) : TaskSheet
    data class AutoRoute(val id: Long) : TaskSheet
    data class Stats(val table: StatsTable) : TaskSheet
    data class Tides(val lat: Double, val lon: Double) : TaskSheet
    data object PlanningOnly : TaskSheet
    data object Settings : TaskSheet
    data object Connections : TaskSheet
    data object Monitor : TaskSheet
    data object About : TaskSheet
}

/**
 * The Kotlin side of the engine, as Model.swift is the Swift side: forwards the engine's callbacks, calls its
 * JSON api and holds what the screens show. Main thread only. It lives as long as the process (the engine is
 * the Application's), so a fold or a rotation keeps the page, the sheet and what was tapped.
 */
object Model {
    val json = Json {
        ignoreUnknownKeys = true
        coerceInputValues = true
        explicitNulls = false
        @OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)
        namingStrategy = JsonNamingStrategy.SnakeCase
    }

    private val main = Handler(Looper.getMainLooper())

    // Chart chrome.
    var footer by mutableStateOf(FooterInfo())
    var status by mutableStateOf("")
    val layers = mutableStateListOf<LayerItem>()
    var mode by mutableStateOf("auto")
    var window by mutableStateOf(WindowInfo())
    var alerts by mutableStateOf(AlertsInfo())
    var underway by mutableStateOf(UnderwayInfo())
    var timeline by mutableStateOf<TimelineInfo?>(null)
    var routerCard by mutableStateOf<RouterCard?>(null)
    var departures by mutableStateOf<DeparturesInfo?>(null)
    var weatherReadout by mutableStateOf<WeatherReadout?>(null)
    var toast by mutableStateOf<String?>(null)
    var follow by mutableStateOf(false)
    var measuring by mutableStateOf(false)

    // Pages.
    var passage by mutableStateOf(Passage())
    var vessels by mutableStateOf(emptyList<Vessel>())
    var tiles by mutableStateOf(emptyList<InstrumentTile>())
    var sailSwitch by mutableStateOf(PerfSwitch())
    var charts by mutableStateOf(ChartsPage())
    var weather by mutableStateOf(WeatherPage())
    var performance by mutableStateOf<PerformanceInfo?>(null)

    // Navigation.
    var page by mutableStateOf(Page.Passage)
    var detail by mutableStateOf<Detail?>(null)
    var sheet by mutableStateOf<TaskSheet?>(null)
    /** What was tapped, shown in the sheet; the actions of a long press. */
    var info by mutableStateOf<InfoRequest?>(null)
    var menu by mutableStateOf<MenuRequest?>(null)
    var confirm by mutableStateOf<ConfirmRequest?>(null)
    var error by mutableStateOf<ErrorRequest?>(null)
    /** A detail page asks to be opened at full height (an engine dialog), counted so each ask is new. */
    var expandRequests by mutableStateOf(0)
    /** A height for the phone's sheet asked for by a script (`peek`, `medium`, `large`), until it is taken. */
    var sheetRequest by mutableStateOf<String?>(null)
    /** A busy job button was pressed: the activity indicator opens its list of running jobs (`job`). */
    var activityShown by mutableStateOf(false)
    /**
     * On a phone, the mark, route or vessel tapped on the chart: its page in a sheet of its own, apart from the
     * pages' sheet and its tabs (`ItemSheet`). A tablet opens it in the panel (`detail`).
     */
    var chartDetail by mutableStateOf<Detail?>(null)
    /** The phone layout is on screen (PhoneScreen): what is tapped goes to `chartDetail`. */
    var phoneLayout = false
    /** Something tapped on the chart is shown in its own sheet (a phone). */
    val chartShown: Boolean get() = info != null || chartDetail != null
    /** A page looked at over a weather route's preview, which comes back when a new one is made. */
    var pagesOverRouter by mutableStateOf(false)
    /** Best departure's preview shown over its results (`BackToRouter`), until "Best departure" goes back. */
    var previewOverDepartures by mutableStateOf(false)
    /** A foldable open like a book: where its hinge starts and how wide it is, in dp from the window's left. */
    var hinge by mutableStateOf<Float?>(null)
    var hingeWidth by mutableStateOf(0f)
    /** The phone's own preferences (not the engine's settings): dim the screen at Night, keep it on under way. */
    var dimAtNight by mutableStateOf(true)
    var keepOnUnderway by mutableStateOf(true)
    /** The tablet's page panel is open. */
    var panelOpen by mutableStateOf(true)
    /** The sheet's height at start ("medium", "large"), from the launch intent (for screenshots). */
    var startSheet: String? = null

    /** Set by the activity: repaint the chart, put text on the clipboard, play a sound. */
    var redraw: () -> Unit = {}
    var copy: (String) -> Unit = {}
    var sound: (String) -> Unit = {}
    var openUrl: (String) -> Unit = {}
    var pickFile: (kind: String) -> Unit = {}
    /** Asks where to save a file called `name`, writes it with `write` (a path to write to; an error or null), then copies it there. */
    var saveAs: (name: String, write: (String) -> String?) -> Unit = { _, _ -> }
    /** The connections changed: the app holds the Wi-Fi's multicast lock while a UDP one is on. */
    var onConnections: () -> Unit = {}

    fun connectionsChanged() = onConnections()

    /** Whether an enabled UDP connection listens (for the multicast lock). */
    fun listensUdp(): Boolean = runCatching {
        json.parseToJsonElement(callText("connections")).let { it as kotlinx.serialization.json.JsonArray }.any { row ->
            val cfg = (row as JsonObject)["config"] as? JsonObject
            cfg?.bool("enabled") != false && (cfg?.get("kind") as? JsonObject)?.str("type") == "udp"
        }
    }.getOrDefault(false)

    private var toastAt = 0L

    fun start() {
        Native.onRedraw = { redraw() }
        Native.onChanged = ::changed
        Native.onUi = ::ui
        mode = Native.mode()
        for (t in listOf(Native.TOPIC_VIEW, Native.TOPIC_STATUS, Native.TOPIC_LAYERS, Native.TOPIC_DATA, Native.TOPIC_LIVE)) changed(t)
    }

    // Calling the engine.

    fun args(vararg pairs: Pair<String, Any?>): JsonObject = buildJsonObject {
        for ((k, v) in pairs) put(k, element(v))
    }

    private fun element(v: Any?): JsonElement = when (v) {
        null -> JsonNull
        is JsonElement -> v
        is String -> JsonPrimitive(v)
        is Number -> JsonPrimitive(v)
        is Boolean -> JsonPrimitive(v)
        is Map<*, *> -> JsonObject(v.entries.associate { (k, x) -> k.toString() to element(x) })
        is List<*> -> kotlinx.serialization.json.JsonArray(v.map(::element))
        else -> JsonPrimitive(v.toString())
    }

    fun callText(name: String, args: JsonObject = JsonObject(emptyMap())): String = Native.call(name, args.toString())

    /** Calls `name` of the engine's api and decodes the answer. */
    inline fun <reified T> call(name: String, args: JsonObject = JsonObject(emptyMap())): T? {
        val text = callText(name, args)
        if (text.isEmpty() || text == "null") return null
        return runCatching { json.decodeFromString(serializer<T>(), text) }.getOrNull()
    }

    /** The answer as a JSON object, for the forms that edit what it describes. */
    fun dict(name: String, args: JsonObject = JsonObject(emptyMap())): JsonObject? =
        runCatching { json.parseToJsonElement(callText(name, args)).jsonObject }.getOrNull()

    /** Calls `name` for what it does; returns an error message if it said one. */
    fun send(name: String, args: JsonObject = JsonObject(emptyMap())): String? {
        val o = runCatching { json.parseToJsonElement(callText(name, args)) as? JsonObject }.getOrNull() ?: return null
        return o["error"]?.jsonPrimitive?.contentOrNull
    }

    fun command(name: String, arg: Long = 0) = Native.command(name, arg)

    /** The settings, as the engine keeps them. */
    val settings: JsonObject get() = dict("settings_get") ?: JsonObject(emptyMap())

    // Topics.

    private fun changed(topic: Int) {
        when (topic) {
            Native.TOPIC_POINTER, Native.TOPIC_VIEW -> refreshFooter()
            Native.TOPIC_STATUS -> status = Native.text(Native.TEXT_STATUS)
            Native.TOPIC_LAYERS -> reloadLayers()
            Native.TOPIC_DATA -> reloadData()
            Native.TOPIC_LIVE -> reloadLive()
        }
        if (topic == Native.TOPIC_VIEW && page == Page.Charts) refreshChartsWhenSettled()
    }

    private val chartsRefresh = Runnable { if (page == Page.Charts) call<ChartsPage>("charts")?.let { charts = it } }

    /** "In use here" follows the centre of the chart: once the view stops moving, while the page shows. */
    private fun refreshChartsWhenSettled() {
        main.removeCallbacks(chartsRefresh)
        main.postDelayed(chartsRefresh, 400)
    }

    private var footerAt = 0L
    private var footerPending = false

    /** The footer follows the view at most ten times a second, and always ends on where it stopped. */
    private fun refreshFooter() {
        val now = android.os.SystemClock.uptimeMillis()
        if (now - footerAt >= 100) {
            footerAt = now
            call<FooterInfo>("footer")?.let { if (it != footer) footer = it }
        } else if (!footerPending) {
            footerPending = true
            main.postDelayed({ footerPending = false; refreshFooter() }, 100 - (now - footerAt))
        }
    }

    fun reloadData() {
        call<Passage>("passage")?.let { if (it != passage) passage = it }
        call<WeatherPage>("weather_page")?.let { if (it != weather) weather = it }
        val card = call<RouterCard>("router_card")
        if (card != routerCard) routerCard = card
        val before = departures
        val d = call<DeparturesInfo>("departures")
        if (d != departures) departures = d
        if (page == Page.Charts) call<ChartsPage>("charts")?.let { charts = it }
        if (detail == Detail.Performance) call<PerformanceInfo>("perf_page")?.let { if (it != performance) performance = it }
        // New results of the route on show: its best departure goes on the chart at once.
        val det = detail
        if (d != null && before == null && det is Detail.Routing && d.routeId == det.id && d.rows.any { it.selected }) send("departure_preview")
    }

    /**
     * Buttons that start a job (`isomar_app::jobs`, the GTK shell's `ui/busy.rs`): while their job runs they
     * look off, with a spinner in place of their icon (`JobLabel`, `jobBusy`), and a press shows the running
     * jobs in the activity indicator instead of starting the work again. `action` is what the engine's jobs
     * name: `route`, `weather`, `autoroute:<id>`, `ensemble:<id>`, `chart:<source id>`, `perf_analyse`,
     * `perf_backfill`.
     */
    fun busy(action: String) = action.isNotEmpty() && window.jobs.any { it.action == action }

    /** What a job button does: `start` unless `action`'s job runs, else shows the running jobs (the phone's sheet lowered to show them). */
    fun job(action: String, start: () -> Unit) {
        if (!busy(action)) return start()
        activityShown = true
        if (phoneLayout) sheetRequest = "medium"
    }

    private fun reloadLive() {
        call<WindowInfo>("window")?.let {
            if (it != window) {
                // A mark, a route or a vessel tapped on the chart opens in the sheet or the panel.
                val p = it.picked
                if (p != null && p != window.picked) openPicked(p)
                window = it
            }
        }
        // Asked for while a job ran, and it has ended: the list must not open by itself with the next one.
        if (activityShown && window.jobs.isEmpty()) activityShown = false
        // Panning by hand ends following, in the engine.
        if (!window.following && follow) follow = false
        call<AlertsInfo>("alerts")?.let { if (it != alerts) alerts = it }
        call<UnderwayInfo>("underway")?.let { if (it != underway) underway = it }
        val t = call<TimelineInfo>("timeline")
        if (t != timeline) timeline = t
        val r = call<WeatherReadout>("weather_readout")
        if (r != weatherReadout) weatherReadout = r
        refreshFooter()
        when (page) {
            Page.Traffic -> call<List<Vessel>>("traffic")?.let { if (it != vessels) vessels = it }
            Page.Instruments -> {
                call<List<InstrumentTile>>("instruments")?.let { if (it != tiles) tiles = it }
                call<PerfSwitch>("perf_switch")?.let { if (it != sailSwitch) sailSwitch = it }
            }
            Page.Passage -> call<Passage>("passage")?.let { if (it != passage) passage = it }
            else -> {}
        }
        Alerting.update(alerts, window)
    }

    private fun reloadLayers() {
        val list = runCatching { json.decodeFromString<List<LayerItem>>(Native.layers()) }.getOrNull() ?: return
        layers.clear()
        layers.addAll(list)
    }

    // Commands the screens use.

    fun setLayer(id: String, visible: Boolean) {
        Native.setLayerVisible(id, visible)
        reloadLayers()
    }

    fun changeMode(m: String) {
        Native.setMode(m)
        mode = Native.mode()
        redraw()
    }

    fun toggleFollow() {
        follow = !follow
        command("follow", if (follow) 1 else 0)
        if (follow) command("centre_boat")
    }

    fun toggleMeasure() {
        measuring = !measuring
        command(if (measuring) "measure_begin" else "cancel")
    }

    fun showPage(p: Page) {
        page = p
        detail = null
        info = null
        reloadData()
        reloadLive()
    }

    fun showRouting(id: Long, window: Boolean? = null) {
        page = Page.Passage
        detail = Detail.Routing(id, window ?: (departures?.routeId == id))
        info = null
        expandRequests++
    }

    fun show(toast: String) {
        this.toast = toast
        val at = android.os.SystemClock.uptimeMillis()
        toastAt = at
        main.postDelayed({ if (toastAt == at) this.toast = null }, io.github.isomar.ui.Timing.toastDuration)
    }

    private fun openPicked(p: Picked) {
        val d = when (p.kind) {
            "waypoint" -> Detail.Waypoint(p.id)
            "route" -> Detail.Route(p.id)
            "vessel" -> Detail.Vessel(p.id)
            else -> return
        }
        info = null
        if (phoneLayout) { chartDetail = d; return }
        page = if (d is Detail.Vessel) Page.Traffic else Page.Passage
        detail = d
        panelOpen = true
    }

    /** Closes what was tapped on the chart (a phone's sheet of its own). */
    fun closeChartItem() {
        if (info != null) closeInfo()
        chartDetail = null
    }

    /** Closes a detail page: the one on a page, or the one in a sheet of its own. */
    fun closeDetail(standalone: Boolean) {
        if (standalone) chartDetail = null else detail = null
    }

    fun closeInfo() {
        info = null
        command("info_closed")
    }

    // Requests from the engine.

    private inline fun <reified T> decode(s: String): T? = runCatching { json.decodeFromString(serializer<T>(), s) }.getOrNull()

    private fun ui(kind: Int, text: String) {
        when (kind) {
            Native.UI_TOAST -> decode<TextRequest>(text)?.let { show(it.text) }
            Native.UI_ERROR -> error = decode<ErrorRequest>(text)
            Native.UI_BEEP -> sound(decode<SoundRequest>(text)?.sound ?: "alarm")
            Native.UI_NOTIFY -> decode<NotifyRequest>(text)?.let { Alerting.finished(it.title, it.body) }
            Native.UI_CONFIRM -> decode<ConfirmRequest>(text)?.let { confirm = it }
            Native.UI_MENU -> menu = decode<MenuRequest>(text)
            Native.UI_INFO -> decode<InfoRequest>(text)?.let { info = it; chartDetail = null }
            Native.UI_CLIPBOARD -> decode<TextRequest>(text)?.let { copy(it.text) }
            Native.UI_DIALOG -> decode<DialogRequest>(text)?.let(::open)
        }
    }

    /** The engine asks for a dialog: on a phone, a page of the sheet or a task sheet. */
    fun open(d: DialogRequest) {
        val id = d.id
        when (d.name) {
            "waypoint_properties" -> if (id != null) { page = Page.Passage; detail = Detail.Waypoint(id); info = null; expandRequests++ }
            "route_properties" -> if (id != null) { page = Page.Passage; detail = Detail.Route(id); info = null; expandRequests++ }
            "vessel" -> if (id != null) { page = Page.Traffic; detail = Detail.Vessel(id); info = null; expandRequests++ }
            "weather_route" -> if (id != null) showRouting(id, false)
            "best_departure" -> if (id != null) showRouting(id, true)
            "auto_route" -> if (id != null) sheet = TaskSheet.AutoRoute(id)
            "get_weather" -> {
                val n = d.north; val s = d.south; val w = d.west; val e = d.east
                if (n != null && s != null && w != null && e != null) {
                    sheet = TaskSheet.GetWeather(WeatherArea(fmt(n), fmt(s), fmt(w), fmt(e)))
                } else {
                    call<WeatherArea>("weather_defaults", args("kind" to "rect"))?.let { sheet = TaskSheet.GetWeather(it) }
                }
            }
            "tides" -> { val la = d.lat; val lo = d.lon; if (la != null && lo != null) sheet = TaskSheet.Tides(la, lo) }
            "charts_here" -> showPage(Page.Charts)
            "preferences", "settings" -> sheet = TaskSheet.Settings
            else -> show("${d.name} is not available here")
        }
    }

    /** A coordinate the way the weather form takes it: decimal degrees are accepted. */
    private fun fmt(v: Double) = String.format(java.util.Locale.ROOT, "%.4f", v)
}

/** A string field of an api answer. */
fun JsonObject.str(key: String): String = (this[key] as? JsonPrimitive)?.contentOrNull ?: ""
fun JsonObject.num(key: String): Double? = (this[key] as? JsonPrimitive)?.contentOrNull?.toDoubleOrNull()
fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.contentOrNull?.toBooleanStrictOrNull()

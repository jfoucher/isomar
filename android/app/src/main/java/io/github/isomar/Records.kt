package io.github.isomar

import kotlinx.serialization.Serializable

// What the engine reports: the JSON of crates/isomar-app/src/vm and api.rs, as the Mac's Model.swift reads
// it. Keys are snake_case in the JSON and camelCase here (Json's naming strategy in Model.kt). Every field
// has a default, so a record that gains a field never stops decoding.

/** A value after the icon that says what it is (`isomar_app::chips::Chip`); `label` is the whole phrase. */
@Serializable data class Chip(val icon: String? = null, val text: String = "", val label: String = "")

@Serializable data class RouteRow(
    val id: Long = 0, val name: String = "", val detail: List<Chip> = emptyList(), val planned: List<Chip> = emptyList(),
    val visible: Boolean = true, val active: Boolean = false,
)
@Serializable data class WaypointRow(val id: Long = 0, val name: String = "", val detail: String = "")
@Serializable data class TrackRow(val id: Long = 0, val name: String = "", val detail: List<Chip> = emptyList(), val visible: Boolean = true)
@Serializable data class FollowingRow(val name: String = "", val detail: List<Chip> = emptyList())
@Serializable data class Passage(
    val routes: List<RouteRow> = emptyList(),
    val waypoints: List<WaypointRow> = emptyList(),
    val tracks: List<TrackRow> = emptyList(),
    val recording: Boolean = false,
    val following: FollowingRow? = null,
)
@Serializable data class Vessel(val mmsi: Long = 0, val name: String = "", val detail: List<Chip> = emptyList(), val alert: Boolean = false)
@Serializable data class InstrumentTile(val title: String = "", val value: String = "", val unit: String = "")

/** `action` names what started it (`isomar_app::jobs`): its button shows it running (`Model.busy`). */
@Serializable data class JobRow(val id: Long = 0, val action: String = "", val title: String = "", val fraction: Double? = null, val detail: String = "", val cancellable: Boolean = false)
@Serializable data class WindowInfo(
    val title: String = "Isomar",
    val subtitle: List<Chip> = emptyList(),
    val link: String = "none",
    val jobs: List<JobRow> = emptyList(),
    val underway: Boolean = false,
    val offline: Boolean = false,
    val recording: Boolean = false,
    val keepAwake: Boolean = false,
    val watching: Boolean = false,
    val following: Boolean = false,
    val routing: Boolean = false,
    /** What was last selected on the chart, to open in the sheet or the panel (`n` changes with each pick). */
    val picked: Picked? = null,
)
@Serializable data class Picked(val kind: String = "", val id: Long = 0, val n: Long = 0)

@Serializable data class Strip(
    val id: Long = 0, val level: String = "", val title: String = "",
    val acknowledge: String? = null, val snooze: String? = null, val dismiss: String? = null, val action: String? = null,
    val count: Int = 0, val countLabel: String? = null, val dim: Boolean = false,
)
@Serializable data class Hint(val title: String = "", val finish: String? = null, val undo: String? = null, val cancel: String? = null)
@Serializable data class AlertsInfo(val alarm: Strip? = null, val warnings: List<Strip> = emptyList(), val hint: Hint? = null)

@Serializable data class ScaleBar(val px: Double = 80.0, val label: String = "")
@Serializable data class PointerInfo(val position: String = "", val chips: List<Chip> = emptyList())
@Serializable data class FooterInfo(
    val scale: ScaleBar = ScaleBar(),
    val metric: ScaleBar = ScaleBar(),
    val credits: List<String> = emptyList(),
    val planningOnly: Boolean = false,
    val planningText: String = "",
    val pointer: PointerInfo? = null,
    val view: List<Chip> = emptyList(),
)

@Serializable data class UCell(val label: String = "", val value: String = "", val unit: String = "")
@Serializable data class NavCard(val number: Int = 0, val name: String = "", val cells: List<UCell> = emptyList(), val canSkip: Boolean = false)
@Serializable data class Readout(val id: String = "", val label: String = "", val value: String = "", val unit: String = "", val note: List<Chip> = emptyList())
@Serializable data class PerfSwitch(val mode: String = "", val byEngine: Boolean = false, val note: String? = null, val hint: String? = null)
@Serializable data class UnderwayInfo(val on: Boolean = false, val card: NavCard? = null, val dashboard: List<Readout> = emptyList(), val sail: PerfSwitch? = null)

@Serializable data class TimelineInfo(
    val mode: String = "", val start: Long = 0, val end: Long = 0, val now: Long = 0, val step: Long = 0,
    val playing: Boolean = false, val speed: Double = 1.0, val label: String = "", val age: String? = null, val stale: Boolean = false,
)

@Serializable data class AltRow(
    val index: Int = 0, val title: String = "", val summary: List<Chip> = emptyList(), val colour: String = "route",
    val dash: List<Double> = emptyList(), val selected: Boolean = false, val arrives: Boolean = false,
)
/** One forecast model's best way, when the passage was routed on several (`vm::router::ModelRow`). */
@Serializable data class ModelRowInfo(
    val index: Int = 0, val label: String = "", val summary: List<Chip> = emptyList(), val colour: String = "route",
    val focused: Boolean = false, val arrives: Boolean = false, val robust: Boolean = false,
)
@Serializable data class RouterCard(
    val title: String = "", val note: String = "", val failure: Boolean = false,
    val alternatives: List<AltRow> = emptyList(), val canAdd: Boolean = false, val replay: Boolean = false,
    val models: List<ModelRowInfo> = emptyList(), val verdict: List<String> = emptyList(),
)

/** One forecast model's cost of a departure, on the bar's scale. */
@Serializable data class DepDot(val h: Double = 0.0, val colour: String = "route")
@Serializable data class DepBar(
    val x: Double = 0.0, val w: Double = 0.0, val h: Double = 0.0, val ok: Boolean = false, val best: Boolean = false, val selected: Boolean = false, val tooltip: String = "",
    val dots: List<DepDot> = emptyList(), val range: List<Double>? = null,
)
@Serializable data class DepRow(val index: Int = 0, val title: String = "", val figures: List<Chip> = emptyList(), val why: List<Chip> = emptyList(), val selected: Boolean = false, val models: List<Chip> = emptyList())
@Serializable data class DepLegend(val label: String = "", val colour: String = "route")
@Serializable data class DepDay(val x: Double = 0.0, val labelX: Double = 0.0, val label: String = "")
@Serializable data class DeparturesInfo(
    val routeId: Long = 0, val summary: String = "", val bars: List<DepBar> = emptyList(), val rows: List<DepRow> = emptyList(),
    val days: List<DepDay> = emptyList(), val nights: List<List<Double>> = emptyList(), val canCompare: Boolean = false,
    val legend: List<DepLegend> = emptyList(), val cautious: Boolean = false,
)

@Serializable data class SourceRow(
    val id: String = "", val name: String = "", val rating: String = "", val ratingText: String = "", val chips: List<Chip> = emptyList(),
    val note: String = "", val action: String = "", val url: String = "", val installed: Boolean = false,
)
@Serializable data class InUseRow(val name: String = "", val kind: String = "", val chips: List<Chip> = emptyList())
@Serializable data class ChartsPage(
    val here: List<SourceRow> = emptyList(), val inUse: List<InUseRow> = emptyList(), val vector: Int = 0,
    val folders: List<String> = emptyList(), val rasters: Int = 0, val planningOnly: Boolean = false,
)

@Serializable data class WeatherArea(val north: String = "", val south: String = "", val west: String = "", val east: String = "")
@Serializable data class WeatherGroup(val id: String = "", val label: String = "", val default: Boolean = false)
/** A forecast model to pick (`flows::weather::model_choices`): what it is and what is loaded of it. */
@Serializable data class ModelChoice(val id: String = "", val label: String = "", val about: String = "", val detail: String = "", val loaded: Boolean = false, val shown: Boolean = false)
/** One loaded model against the measured wind (`flows::perf::tracking`). */
@Serializable data class TrackingRow(val label: String = "", val text: String = "", val best: Boolean = false)
@Serializable data class WeatherPage(
    /** Under way, each loaded model against the wind the instruments measure. */
    val tracking: List<TrackingRow> = emptyList(),
    /** The model shown, which weather routes use unless another is picked. */
    val model: String = "gfs", val modelLabel: String = "GFS", val models: List<ModelChoice> = emptyList(),
    val loaded: Boolean = false, val fields: Int = 0, val run: String? = null, val from: String? = null, val to: String? = null,
    /** What colours the sea ("none", "wind" or "waves"), and what is drawn over it. */
    val fill: String = "wind", val barbs: Boolean = true, val pressure: Boolean = true, val waveArrows: Boolean = true, val picking: Boolean = false, val noWaves: String? = null,
    val rect: WeatherArea? = null, val horizons: List<String> = emptyList(), val steps: List<String> = emptyList(), val groups: List<WeatherGroup> = emptyList(),
    /** Tidal streams and ocean currents, shared by every model (`currents::page`). */
    val currents: CurrentsPage? = null,
)
@Serializable data class CurrentSourceLine(val id: String = "", val label: String = "", val detail: String = "")
@Serializable data class AtlasLine(val id: String = "", val name: String = "", val detail: String = "", val installed: Boolean = false)
@Serializable data class CurrentsPage(
    val loaded: Boolean = false, val arrows: Boolean = true, val routeWith: Boolean = true,
    val sources: List<CurrentSourceLine> = emptyList(), val atlases: List<AtlasLine> = emptyList(), val note: String = "",
    /** Under way, each source against the stream measured. */
    val tracking: List<String> = emptyList(),
)
@Serializable data class WeatherReadout(val chips: List<Chip> = emptyList(), val time: String = "")
@Serializable data class WeatherEstimate(val source: String = "", val run: String = "", val size: String = "", val files: String = "", val ok: Boolean = false, val missing: String = "")

/** A table of figures (`vm::stats`): a section's rows are [label, [cells]]. */
@Serializable data class StatSection(val title: String = "", val rows: List<kotlinx.serialization.json.JsonArray> = emptyList())
@Serializable data class StatsTable(val heads: List<String> = emptyList(), val sections: List<StatSection> = emptyList())

@Serializable data class MenuEntry(val i: Int = 0, val label: String = "")
@Serializable data class MenuRequest(val x: Double = 0.0, val y: Double = 0.0, val title: String = "", val sections: List<List<MenuEntry>> = emptyList())
@Serializable data class InfoRequest(
    val x: Double = 0.0, val y: Double = 0.0, val title: String = "", val subtitle: String? = null,
    val rows: List<List<String>> = emptyList(), val choice: List<Int>? = null,
)
@Serializable data class ConfirmRequest(val id: Long = 0, val heading: String = "", val body: String = "", val ok: String = "OK", val destructive: Boolean = false)
@Serializable data class ErrorRequest(val heading: String = "", val details: String = "")
@Serializable data class TextRequest(val text: String = "")
@Serializable data class SoundRequest(val sound: String? = null)
@Serializable data class NotifyRequest(val title: String = "", val body: String = "")
@Serializable data class DialogRequest(
    val name: String = "", val id: Long? = null, val lat: Double? = null, val lon: Double? = null,
    val north: Double? = null, val south: Double? = null, val west: Double? = null, val east: Double? = null,
)
@Serializable data class LayerItem(val id: String = "", val label: String = "", val visible: Boolean = true)

// The performance log (`vm::performance`).
@Serializable data class PerfLine(val title: String = "", val detail: String = "")
@Serializable data class PerfCalibration(val findings: List<PerfLine> = emptyList(), val evidence: String = "", val canSave: Boolean = false, val saved: Boolean = false)
@Serializable data class PerfCurve(
    val tws: Double = 0.0, val current: List<List<Double>> = emptyList(), val learned: List<List<Double>> = emptyList(),
    val cells: List<List<Double>> = emptyList(), val best: String? = null,
)
@Serializable data class PerfLearned(
    val hours: Double = 0.0, val summary: String = "", val curves: List<PerfCurve> = emptyList(), val coverage: List<List<Double>> = emptyList(),
    val columns: List<Double> = emptyList(), val changes: Int = 0, val caution: String? = null,
)
@Serializable data class PerfFactorRow(val label: String = "", val pct: Double = 0.0, val lo: Double = 0.0, val hi: Double = 0.0, val hours: Double = 0.0, val text: String = "")
@Serializable data class PerfFactorGroup(val title: String = "", val note: String = "", val rows: List<PerfFactorRow> = emptyList())
@Serializable data class PerfSuggestion(val pct: Double = 0.0, val text: String = "", val button: String = "", val applied: Boolean = false)
@Serializable data class PerformanceInfo(
    val recording: Boolean = false, val analysing: Boolean = false, val backfilling: Boolean = false, val hindcastPending: Int = 0,
    val empty: Boolean = true, val summary: String = "", val hours: List<PerfLine> = emptyList(), val calibration: PerfCalibration? = null,
    val learned: PerfLearned? = null, val factors: List<PerfFactorGroup> = emptyList(), val suggestion: PerfSuggestion? = null,
    val forecastCheck: List<PerfLine> = emptyList(), val waves: List<PerfLine> = emptyList(), val passages: List<PerfLine> = emptyList(),
    val routingPct: Double? = null, val canUndoPolar: Boolean = false, val polar: String = "", val error: String? = null,
)

package io.github.isomar

import android.os.Handler
import android.os.Looper
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonPrimitive
import java.io.File

/**
 * Drives the app as a person would, for checks where nothing can be touched (a debuggable build, launched with
 * `--es script 'step;step'`): the steps of macos/Isomar/Script.swift. `shot,<name>` writes `files/isomar-shot`
 * for whoever drives the emulator (`adb shell run-as ... cat`), which takes a screenshot and deletes the file;
 * the script goes on once it is gone. `end` says it is over.
 */
object Script {
    private val main = Handler(Looper.getMainLooper())

    fun run(script: String, files: File) {
        val steps = script.split(";").filter { it.isNotBlank() }.map { it.split(",", limit = 3) }
        val shot = File(files, "isomar-shot")
        var i = 0
        lateinit var next: () -> Unit
        fun after(ms: Long) = main.postDelayed({ next() }, ms)
        next = next@{
            if (i >= steps.size) return@next
            val s = steps[i]
            val arg = s.getOrElse(1) { "" }
            // `waitfor,router|departures|idle`: until the preview is up, the departures are in, or no job runs.
            if (s[0] == "waitfor" && when (arg) {
                    "router" -> Model.routerCard == null
                    "departures" -> Model.departures == null
                    else -> Model.window.jobs.isNotEmpty()
                }) { after(1000); return@next }
            i++
            var wait = 400L
            when (s[0]) {
                "wait" -> wait = ((arg.toDoubleOrNull() ?: 1.0) * 1000).toLong()
                "shot", "end" -> {
                    shot.writeText(if (s[0] == "end") "END" else arg)
                    if (s[0] == "end") return@next
                    var waited = 0
                    fun taken() {
                        if (shot.exists() && waited < 240) { waited++; main.postDelayed({ taken() }, 250) } else after(300)
                    }
                    taken()
                    return@next
                }
                "page" -> Page.entries.firstOrNull { it.name.equals(arg, true) }?.let(Model::showPage)
                "detent" -> Model.sheetRequest = arg
                "route" -> arg.toLongOrNull()?.let { Model.detail = Detail.Route(it) }
                "command" -> Model.command(arg, s.getOrNull(2)?.toLongOrNull() ?: 0)
                "call" -> Native.call(arg, s.getOrElse(2) { "{}" })
                "tapll", "tapll3" -> {
                    val p = s.drop(1).joinToString(",").split(",").mapNotNull { it.toDoubleOrNull() }
                    val at = if (p.size == 2) Model.dict("screen_of", Model.args("lat" to p[0], "lon" to p[1])) else null
                    val x = at?.get("x")?.jsonPrimitive?.doubleOrNull
                    val y = at?.get("y")?.jsonPrimitive?.doubleOrNull
                    if (x != null && y != null) Native.tap(if (s[0] == "tapll3") 3 else 1, 1, x, y)
                }
                "underway" -> Model.send("underway_set", Model.args("on" to (arg == "on")))
                "dismiss" -> { Model.menu = null; Model.info = null; Model.sheet = null }
                "sheet" -> when (arg) {
                    "weather" -> Model.call<WeatherArea>("weather_defaults", Model.args("kind" to "view"))?.let { Model.sheet = TaskSheet.GetWeather(it) }
                    "settings" -> Model.sheet = TaskSheet.Settings
                    "connections" -> Model.sheet = TaskSheet.Connections
                    "about" -> Model.sheet = TaskSheet.About
                    "planning" -> Model.sheet = TaskSheet.PlanningOnly
                    "weatherroute" -> route()?.let { Model.showRouting(it, false) }
                    "bestdeparture" -> route()?.let { Model.showRouting(it, true) }
                }
                "compute" -> route()?.let { id ->
                    val at = java.text.SimpleDateFormat("yyyy-MM-dd HH:mm", java.util.Locale.ROOT).format(java.util.Date(System.currentTimeMillis() + 3_600_000))
                    Model.send("route_compute", Model.args("id" to id, "departure" to at))
                }
                "departures" -> route()?.let { id ->
                    val d = Model.dict("window_defaults", Model.args("id" to id)) ?: return@let
                    Model.showRouting(id, true)
                    Model.send("window_run", buildJsonObject {
                        put("id", kotlinx.serialization.json.JsonPrimitive(id))
                        for (k in listOf("earliest", "days", "step_hours", "weights")) d[k]?.let { put(k, it) }
                        put("boat", JsonObject(emptyMap()))
                    })
                }
                "stats" -> Model.call<StatsTable>("router_stats")?.let { Model.sheet = TaskSheet.Stats(it) }
            }
            after(wait)
        }
        after(2000)
    }

    /** The route a step is about: the one open, the one followed, or the first. */
    private fun route(): Long? = (Model.detail as? Detail.Route)?.id ?: (Model.detail as? Detail.Routing)?.id
        ?: Model.passage.routes.firstOrNull { it.active }?.id ?: Model.passage.routes.firstOrNull()?.id
}

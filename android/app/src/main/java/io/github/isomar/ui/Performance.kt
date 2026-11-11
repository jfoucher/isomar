package io.github.isomar.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size as GSize
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.isomar.ErrorRequest
import io.github.isomar.Model
import io.github.isomar.PerfCurve
import io.github.isomar.PerfLearned
import io.github.isomar.PerfLine
import io.github.isomar.PerformanceInfo
import io.github.isomar.num
import io.github.isomar.str
import kotlin.math.ceil
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.sin

/**
 * The performance log, pushed from Instruments (Performance.swift on the Mac): what the log holds, what it
 * says about the instruments, the polar as learnt, the boat's performance by condition, the forecast
 * against the instruments, and passages planned against sailed (`perf_get`, `perf_page`).
 */
@Composable
fun PerformancePage() {
    var confirmClear by remember { mutableStateOf(false) }
    // Asking for the record starts an analysis when the log is newer than the last one.
    LaunchedEffect(Unit) { Model.performance = Model.call<PerformanceInfo>("perf_get") }
    PageColumn {
        BackBar("Performance")
        val p = Model.performance
        if (p == null) CircularProgressIndicator(Modifier.padding(24.dp)) else Content(p) { confirmClear = true }
        Spacer(Modifier.height(32.dp))
    }
    if (confirmClear) {
        AlertDialog(
            onDismissRequest = { confirmClear = false },
            title = { Text("Clear the performance log?") },
            text = { Text("Every sample logged against the polar in use, and every passage, is deleted. The polars and the calibration are kept.") },
            confirmButton = { TextButton(onClick = { Model.send("perf_clear"); confirmClear = false }) { Text("Clear", color = c(Tok.alarm)) } },
            dismissButton = { TextButton(onClick = { confirmClear = false }) { Text("Cancel") } },
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Content(p: PerformanceInfo, clear: () -> Unit) {
    Row(Modifier.padding(start = 20.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(p.summary, fontSize = 13.sp, color = c(Tok.textMuted), modifier = Modifier.weight(1f))
        // While it analyses: a spinner in place of its icon, and pressing it shows the running jobs.
        val analysing = Model.busy("perf_analyse")
        if (p.analysing && !analysing) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        IconButton(onClick = { Model.job("perf_analyse") { Model.send("perf_analyse") } }, modifier = Modifier.jobBusy(analysing)) {
            if (analysing) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Icon(Icons.Outlined.Refresh, "Analyse again")
        }
    }
    Text("Logged against the polar ${p.polar}", fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp))
    p.error?.let { Text("The log could not be read: $it", fontSize = 13.sp, color = c(Tok.alarm), modifier = Modifier.padding(horizontal = 20.dp)) }

    SectionHeader("Log")
    BoxedList {
        SwitchRow("Record while under way", p.recording) { Model.send("perf_set_recording", Model.args("on" to it)) }
        for (l in p.hours) { ListDivider(); Figure(l) }
    }
    FlowRow(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(onClick = {
            Model.saveAs("performance-log.csv") { path ->
                val d = Model.dict("perf_export_csv", Model.args("path" to path))
                d?.str("error")?.ifEmpty { null }.also { if (it == null) Model.show("${d?.num("rows")?.toInt() ?: 0} samples exported") }
            }
        }) { Text("Export") }
        if (!p.empty) OutlinedButton(onClick = clear) { Text("Clear…") }
    }

    p.calibration?.let { c ->
        SectionHeader("Instruments")
        EmptyText(c.evidence)
        Lines(c.findings)
        FlowRow(Modifier.padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (c.canSave) FilledTonalButton(onClick = { Model.send("perf_calibration_save")?.let { Model.error = ErrorRequest("Could not save the calibration", it) } }) { Text("Correct the analysis with this") }
            if (c.saved) OutlinedButton(onClick = { Model.send("perf_calibration_clear") }) { Text("Forget the saved correction") }
        }
        EmptyText("The correction applies to this analysis only; it does not change what the instruments show.")
    }

    p.learned?.let { Learned(it, p.canUndoPolar) }

    if (p.suggestion != null || p.factors.isNotEmpty()) {
        SectionHeader("Performance")
        p.suggestion?.let { s ->
            Text(s.text, fontSize = 14.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp))
            if (s.applied) EmptyText("Weather routing starts from this.")
            else Button(onClick = { Model.send("perf_apply_factor", Model.args("pct" to s.pct)); Model.show("Weather routing will start from this performance") }, modifier = Modifier.padding(horizontal = 16.dp)) { Text(s.button) }
        }
        for (g in p.factors) {
            var open by remember(g.title) { mutableStateOf(false) }
            BoxedList(Modifier.padding(vertical = 4.dp)) {
                Column(Modifier.fillMaxWidth().clickable { open = !open }.padding(14.dp)) {
                    Text(g.title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    Text(g.note, fontSize = 13.sp, color = c(Tok.textMuted))
                }
                if (open) for (r in g.rows) {
                    ListDivider()
                    Column(Modifier.padding(horizontal = 14.dp, vertical = 8.dp)) {
                        Text(r.label, fontSize = 14.sp)
                        Text(r.text, fontSize = 13.sp, color = c(Tok.textMuted))
                    }
                }
            }
        }
    }

    if (p.forecastCheck.isNotEmpty() || p.hindcastPending > 0) {
        SectionHeader("Forecast against the instruments")
        Lines(p.forecastCheck)
        if (p.hindcastPending > 0) {
            OutlinedButton(onClick = { Model.job("perf_backfill") { Model.send("perf_backfill") } }, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp).jobBusy(Model.busy("perf_backfill"))) {
                JobLabel("Fetch the wind as analysed afterwards", Model.busy("perf_backfill"))
            }
        }
    }
    if (p.waves.isNotEmpty()) { SectionHeader("Waves"); Lines(p.waves) }
    if (p.passages.isNotEmpty()) { SectionHeader("Passages"); Lines(p.passages) }
}

@Composable
private fun Figure(l: PerfLine) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(l.title, fontSize = 15.sp, modifier = Modifier.weight(1f))
        Text(l.detail, fontSize = 15.sp, color = c(Tok.textMuted), style = Numeric)
    }
}

@Composable
private fun Lines(ls: List<PerfLine>) {
    if (ls.isEmpty()) return
    BoxedList(Modifier.padding(vertical = 4.dp)) {
        ls.forEachIndexed { i, l ->
            if (i > 0) ListDivider()
            Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
                Text(l.title, fontSize = 14.sp)
                Text(l.detail, fontSize = 13.sp, color = c(Tok.textMuted))
            }
        }
    }
}

@Composable
private fun Learned(l: PerfLearned, canUndo: Boolean) {
    var column by remember { mutableIntStateOf(minOf(2, (l.columns.size - 1).coerceAtLeast(0))) }
    SectionHeader("Polar")
    EmptyText(l.summary)
    if (l.columns.isNotEmpty()) {
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
            l.columns.forEachIndexed { i, w ->
                SegmentedButton(selected = i == column, onClick = { column = i }, shape = SegmentedButtonDefaults.itemShape(i, l.columns.size)) { Text("${w.toInt()}", fontSize = 12.sp) }
            }
        }
    }
    l.curves.getOrNull(column)?.let { curve ->
        Curve(curve)
        curve.best?.let { EmptyText(it) }
    }
    EmptyText("Grey: the boat's polar now. Blue: as learnt. Dots: what the boat did, the bolder the more there is.")
    Text("Coverage: where the log has data", fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp))
    Coverage(l.coverage, l.columns)
    l.caution?.let { Text(it, fontSize = 13.sp, color = c(Tok.warning), modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) }
    Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (l.changes > 0) Button(onClick = {
            val d = Model.dict("perf_apply_polar")
            val e = d?.str("error")?.ifEmpty { null }
            if (e != null) Model.error = ErrorRequest("Could not change the polar", e) else Model.show("${d?.num("changed")?.toInt() ?: 0} speeds of the polar changed")
        }) { Text("Use the learnt values (${l.changes} cells)") }
        if (canUndo) OutlinedButton(onClick = { Model.send("perf_undo_polar")?.let { Model.error = ErrorRequest("Could not go back", it) } }) { Text("Back to before") }
    }
}

/** Half a polar diagram, starboard side: the wind from the top, speed outwards. */
@Composable
private fun Curve(curve: PerfCurve) {
    val grey = c(Tok.textMuted); val blue = c(Tok.accent); val ring = c(Tok.border)
    Canvas(Modifier.fillMaxWidth().height(240.dp).padding(horizontal = 16.dp, vertical = 8.dp)) {
        val speeds = (curve.current + curve.learned + curve.cells).map { it.getOrElse(1) { 0.0 } }
        val top = (speeds.maxOrNull() ?: 1.0).coerceAtLeast(1.0)
        val step = if (top < 11) 2.0 else 5.0
        val scale = ceil(top / step) * step
        val r = min(size.height / 2 - 8f, size.width * 0.75f)
        val cx = ((size.width - r) / 2).coerceAtLeast(4f); val cy = size.height / 2
        fun at(a: Double, kn: Double): Offset {
            val rr = (kn / scale * r).toFloat()
            return Offset(cx + (sin(Math.toRadians(a)) * rr).toFloat(), cy - (cos(Math.toRadians(a)) * rr).toFloat())
        }
        var k = step
        while (k <= scale + 1e-9) {
            val rr = (k / scale * r).toFloat()
            drawArc(ring, -90f, 180f, false, Offset(cx - rr, cy - rr), GSize(rr * 2, rr * 2), style = Stroke(1f))
            k += step
        }
        fun path(pts: List<List<Double>>) = Path().apply {
            pts.filter { it.size > 1 }.forEachIndexed { i, q -> val o = at(q[0], q[1]); if (i == 0) moveTo(o.x, o.y) else lineTo(o.x, o.y) }
        }
        drawPath(path(curve.current), grey, style = Stroke(1.5.dp.toPx()))
        for (q in curve.cells) if (q.size > 2) drawCircle(blue.copy(alpha = (0.15 + 0.6 * q[2]).toFloat()), 3.dp.toPx(), at(q[0], q[1]))
        drawPath(path(curve.learned), blue, style = Stroke(2.5.dp.toPx()))
    }
}

/** Wind speeds (rows) by 5 degree bins of angle (columns), tinted by how much the log has there. */
@Composable
private fun Coverage(coverage: List<List<Double>>, columns: List<Double>) {
    val blue = c(Tok.accent); val empty = c(Tok.border)
    Column(Modifier.padding(horizontal = 16.dp)) {
        coverage.forEachIndexed { j, row ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("${columns.getOrElse(j) { 0.0 }.toInt()} kn", fontSize = 10.sp, color = c(Tok.textMuted), modifier = Modifier.padding(end = 4.dp).size(width = 36.dp, height = 12.dp))
                Canvas(Modifier.weight(1f).height(12.dp)) {
                    val w = size.width / row.size.coerceAtLeast(1)
                    row.forEachIndexed { i, v ->
                        drawRect(if (v > 0) blue.copy(alpha = (0.15 + 0.85 * v).toFloat()) else empty.copy(alpha = 0.4f), Offset(i * w, 1f), GSize(w - 1f, size.height - 2f))
                    }
                }
            }
        }
    }
}

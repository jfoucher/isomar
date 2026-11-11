package io.github.isomar.ui

import androidx.compose.foundation.background
import io.github.isomar.Readout
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Undo
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Report
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
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
import io.github.isomar.AltRow
import io.github.isomar.Hint
import io.github.isomar.Model
import io.github.isomar.NavCard
import io.github.isomar.RouterCard
import io.github.isomar.StatsTable
import io.github.isomar.Strip
import io.github.isomar.TaskSheet
import io.github.isomar.TimelineInfo

/** The alarm, over the top of the chart: what it is, and Snooze and Acknowledge as big buttons. */
@Composable
fun AlarmCard(a: Strip) {
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(Radius.l)).background(c(Tok.alarmBg)).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Icon(Icons.Outlined.Report, null, tint = c(Tok.onAlarm), modifier = Modifier.size(24.dp))
            val (head, rest) = a.title.split(". ", limit = 2).let { it[0] to it.getOrNull(1) }
            Column(Modifier.weight(1f)) {
                Text(head, color = c(Tok.onAlarm), fontSize = 16.sp, fontWeight = FontWeight.Bold)
                if (rest != null) Text(rest, color = c(Tok.onAlarm), fontSize = 14.sp)
            }
            a.countLabel?.let { Text("${a.count}", color = c(Tok.onAlarm), fontWeight = FontWeight.Bold, modifier = Modifier.padding(4.dp)) }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            a.snooze?.let { s ->
                OutlinedButton(
                    onClick = { Model.command("alert_snooze", a.id) }, modifier = Modifier.weight(1f).height(Size.hitMinAndroid),
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = c(Tok.onAlarm)),
                    border = androidx.compose.foundation.BorderStroke(1.5.dp, c(Tok.onAlarm)),
                ) { Text(s, fontWeight = FontWeight.SemiBold) }
            }
            a.acknowledge?.let { s ->
                Button(
                    onClick = { Model.command("alert_acknowledge", a.id) }, modifier = Modifier.weight(1f).height(Size.hitMinAndroid),
                    colors = ButtonDefaults.buttonColors(containerColor = c(Tok.onAlarm), contentColor = c(Tok.alarmBg)),
                ) { Text(s, fontWeight = FontWeight.SemiBold) }
            }
            a.action?.let { s -> TextButton(onClick = { Model.command("alert_action", a.id) }) { Text(s, color = c(Tok.onAlarm)) } }
            a.dismiss?.let { s -> TextButton(onClick = { Model.command("alert_dismiss", a.id) }) { Text(s, color = c(Tok.onAlarm)) } }
        }
    }
}

/** A warning (or an alarm that was acknowledged): a slim strip with its actions. */
@Composable
fun WarningStrip(w: Strip) {
    val alarm = w.level == "alarm"
    val fg = c(if (w.dim) Tok.alarm else if (alarm) Tok.onAlarm else Tok.onWarning)
    val bg = c(if (w.dim) Tok.cardBg else if (alarm) Tok.alarmBg else Tok.warningBg)
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(Radius.m)).background(bg).padding(start = 12.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(Icons.Outlined.WarningAmber, null, tint = fg, modifier = Modifier.size(18.dp))
        Text(w.title, color = fg, fontSize = 14.sp, fontWeight = FontWeight.Medium, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        w.snooze?.let { TextButton(onClick = { Model.command("alert_snooze", w.id) }) { Text(it, color = fg) } }
        w.acknowledge?.let { TextButton(onClick = { Model.command("alert_acknowledge", w.id) }) { Text(it, color = fg) } }
        w.action?.let { TextButton(onClick = { Model.command("alert_action", w.id) }) { Text(it, color = fg) } }
        w.dismiss?.let { TextButton(onClick = { Model.command("alert_dismiss", w.id) }) { Text(it, color = fg) } }
    }
}

/** What a tool on the chart expects next (drawing a route, measuring), with its Finish, Undo and Cancel. */
@Composable
fun HintCard(h: Hint) {
    FloatingCard(Modifier.fillMaxWidth()) {
        Row(Modifier.padding(start = 14.dp, end = 4.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(h.title, fontSize = 14.sp, modifier = Modifier.weight(1f))
            h.undo?.let { IconButton(onClick = { Model.command("hint_undo") }) { Icon(Icons.AutoMirrored.Outlined.Undo, it) } }
            h.cancel?.let { TextButton(onClick = { Model.command("hint_cancel") }) { Text(it) } }
            h.finish?.let { Button(onClick = { Model.command("hint_finish") }) { Text(it) } }
        }
    }
}

/** Under way, at the top: the next waypoint and the figures to it, and Skip. */
@Composable
fun NavigatingCard(card: NavCard) {
    FloatingCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Box(Modifier.size(30.dp).clip(CircleShape).background(c(Tok.route)), contentAlignment = Alignment.Center) {
                    Text("${card.number}", color = c(Tok.onRoute), fontWeight = FontWeight.Bold, style = Numeric)
                }
                Column(Modifier.weight(1f)) {
                    Text("NEXT WAYPOINT", fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = c(Tok.textMuted))
                    Text(card.name, fontSize = 19.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                FilledTonalButtonIcon(enabled = card.canSkip, onClick = { Model.send("underway_skip") })
            }
            Row(Modifier.fillMaxWidth()) {
                for (cell in card.cells.take(4)) {
                    Column(Modifier.weight(1f)) {
                        Text(cell.label, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = c(Tok.textMuted))
                        Text(cell.value + if (cell.unit.isNotEmpty() && cell.unit.length <= 2) " ${cell.unit}" else "", fontSize = 19.sp, fontWeight = FontWeight.SemiBold, style = Numeric, maxLines = 1)
                    }
                }
            }
        }
    }
}

@Composable
private fun FilledTonalButtonIcon(enabled: Boolean, onClick: () -> Unit) {
    androidx.compose.material3.FilledTonalIconButton(onClick = onClick, enabled = enabled, modifier = Modifier.size(Size.hitMinAndroid)) {
        Icon(Icons.Filled.SkipNext, "Skip to the next waypoint")
    }
}

/**
 * The instruments under way, filling the space they are given: the number of columns that makes the figures
 * largest, so a few readouts are big and a full set of instruments smaller. A last row that is not full shares
 * its width out rather than leave a hole.
 */
@Composable
fun InstrumentGrid(readouts: List<Readout>, modifier: Modifier = Modifier, largest: Dp = 110.dp) {
    BoxWithConstraints(modifier) {
        val gap = 8.dp
        val (cols, figure) = fitInstruments(readouts.size, maxWidth, maxHeight, gap)
        val f = minOf(figure, largest)
        Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(gap)) {
            for (row in readouts.chunked(cols.coerceAtLeast(1))) {
                Row(Modifier.fillMaxWidth().weight(1f), horizontalArrangement = Arrangement.spacedBy(gap)) {
                    for (r in row) InstrumentTile(r, f, Modifier.weight(1f).fillMaxHeight())
                }
            }
        }
    }
}

/** The columns and the figures' height for `n` readouts in `w` by `h`. */
fun fitInstruments(n: Int, w: Dp, h: Dp, gap: Dp): Pair<Int, Dp> {
    var best = Pair(1, 0.dp)
    for (c in 1..maxOf(n, 1)) {
        val r = (n + c - 1) / c
        val tw = (w - gap * (c - 1)) / c
        val th = (h - gap * (r - 1)) / maxOf(r, 1)
        // The figure, its label and its note in the height; four or five characters in the width.
        val f = minOf(th * 0.40f, tw * 0.27f)
        if (f > best.second) best = Pair(c, f)
    }
    return best
}

/** One readout under way, its figure `figure` high and the rest in proportion. */
@Composable
fun InstrumentTile(r: Readout, figure: Dp, modifier: Modifier = Modifier) {
    val d = LocalDensity.current
    val sp = { x: Dp -> with(d) { x.toSp() } }
    val small = maxOf(figure * 0.24f, 11.dp)
    // A word ("Standby") is wider than a figure: it is set smaller to fit.
    val value = figure * minOf(1f, 5f / maxOf(r.value.length, 4))
    FloatingCard(modifier, shape = RoundedCornerShape(Radius.m)) {
        Column(Modifier.fillMaxSize().padding(horizontal = maxOf(figure * 0.22f, 10.dp), vertical = maxOf(figure * 0.14f, 6.dp))) {
            Text(r.label.uppercase(), fontSize = sp(small), fontWeight = FontWeight.SemiBold, color = c(Tok.textMuted), maxLines = 1)
            Spacer(Modifier.weight(1f))
            Row(verticalAlignment = Alignment.Bottom) {
                Text(r.value, fontSize = sp(value), fontWeight = FontWeight.SemiBold, style = Numeric, maxLines = 1, lineHeight = sp(value * 1.1f))
                if (r.unit.isNotEmpty()) Text(r.unit, fontSize = sp(maxOf(figure * 0.3f, 11.dp)), color = c(Tok.textMuted), maxLines = 1, modifier = Modifier.padding(start = 3.dp, bottom = figure * 0.14f))
            }
            Spacer(Modifier.weight(1f))
            if (r.note.isNotEmpty()) ChipsText(r.note, size = sp(small), maxLines = 1)
        }
    }
}

/** Under way beside the chart (a tablet, or the left half of an open foldable): the navigating card and the instruments filling the height. */
@Composable
fun UnderwayPanel(width: Dp, top: Dp, bottom: Dp) {
    Column(
        Modifier.width(width).fillMaxHeight().background(c(Tok.sidebarBg)).padding(start = 12.dp, end = 12.dp, top = top + 12.dp, bottom = bottom + 12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Model.underway.card?.let { NavigatingCard(it) }
        InstrumentGrid(Model.underway.dashboard, Modifier.weight(1f).fillMaxWidth())
    }
}

/** One time control for the forecast and for the router's clock. */
@Composable
fun TimelineCard(t: TimelineInfo) {
    FloatingCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(horizontal = 8.dp, vertical = 4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = { Model.send("timeline_play", Model.args("on" to !t.playing)) }) {
                    Icon(if (t.playing) Icons.Filled.Pause else Icons.Filled.PlayArrow, if (t.playing) "Pause" else "Play")
                }
                Text(t.label, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, style = Numeric, modifier = Modifier.weight(1f))
                var speeds by remember { mutableStateOf(false) }
                Box {
                    OutlinedButton(onClick = { speeds = true }, contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 10.dp)) {
                        Text("${fmtSpeed(t.speed)}×", style = Numeric)
                    }
                    androidx.compose.material3.DropdownMenu(speeds, onDismissRequest = { speeds = false }) {
                        for (x in listOf(0.5, 1.0, 2.0, 4.0)) {
                            androidx.compose.material3.DropdownMenuItem(text = { Text("${fmtSpeed(x)}×") }, onClick = { Model.send("timeline_speed", Model.args("x" to x)); speeds = false })
                        }
                    }
                }
            }
            val end = maxOf(t.end, t.start + 1)
            Slider(
                value = t.now.coerceIn(t.start, end).toFloat(),
                onValueChange = { Model.send("timeline_set", Model.args("t" to it.toLong())) },
                valueRange = t.start.toFloat()..end.toFloat(),
                modifier = Modifier.padding(horizontal = 8.dp),
            )
            t.age?.let { Text(it, fontSize = 12.sp, color = c(if (t.stale) Tok.warning else Tok.textMuted), modifier = Modifier.padding(start = 12.dp, bottom = 4.dp)) }
        }
    }
}

private fun fmtSpeed(x: Double) = if (x == x.toLong().toDouble()) "${x.toLong()}" else "$x"

/** The weather route in the sheet: the alternatives in their colours, and Statistics, Discard and Use. */
@Composable
fun RouterSheet(card: RouterCard) {
    Column(Modifier.fillMaxSize()) {
        Text(card.title, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 20.dp, end = 20.dp, top = 8.dp))
        Row(Modifier.padding(horizontal = 20.dp, vertical = 6.dp), verticalAlignment = Alignment.Top) {
            if (card.failure) Icon(Icons.Outlined.WarningAmber, null, tint = c(Tok.warning), modifier = Modifier.padding(end = 6.dp).size(18.dp))
            if (card.models.isEmpty()) Text(card.note, fontSize = 13.sp, color = c(Tok.textMuted))
        }
        LazyColumn(Modifier.weight(1f)) {
            // Routed on several forecasts: a row per model (tap one to show its search and wind), what they
            // say together, and the cross-check.
            if (card.models.isNotEmpty()) item {
                BoxedList {
                    card.models.forEachIndexed { i, m ->
                        if (i > 0) ListDivider()
                        Row(
                            Modifier.fillMaxWidth().clickable { Model.send("router_focus_model", Model.args("i" to m.index)) }
                                .background(if (m.focused) c(Tok.hover) else androidx.compose.ui.graphics.Color.Transparent).padding(horizontal = 14.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Box(Modifier.size(width = 24.dp, height = 4.dp).background(c(Tok.byName(m.colour) ?: Tok.route)))
                            Spacer(Modifier.size(10.dp))
                            Column(Modifier.weight(1f)) {
                                Text(if (m.robust) "${m.label} · holds up best" else m.label, fontSize = 15.sp, fontWeight = if (m.focused) FontWeight.SemiBold else FontWeight.Normal)
                                ChipsText(m.summary, size = 13.sp, maxLines = 2)
                            }
                        }
                    }
                }
                Text(card.verdict.joinToString(" "), fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp))
                TextButton(onClick = { Model.call<StatsTable>("router_cross")?.let { Model.sheet = TaskSheet.Stats(it) } }, modifier = Modifier.padding(horizontal = 8.dp)) { Text("Cross-check the routes") }
            }
            item {
                if (card.alternatives.isNotEmpty() && (card.models.isEmpty() || card.alternatives.size > 1)) {
                    BoxedList {
                        card.alternatives.forEachIndexed { i, a ->
                            if (i > 0) ListDivider()
                            AltLine(a, card.replay)
                        }
                    }
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilledTonalButton(onClick = { Model.call<StatsTable>("router_stats")?.let { Model.sheet = TaskSheet.Stats(it) } }, enabled = card.alternatives.isNotEmpty()) { Text("Statistics") }
            Spacer(Modifier.weight(1f))
            FilledTonalButton(onClick = { Model.send("router_discard") }) { Text(if (card.replay) "Close" else "Discard") }
            if (!card.replay) {
                Button(onClick = {
                    val sel = card.alternatives.firstOrNull { it.selected } ?: card.alternatives.firstOrNull()
                    if (sel != null) Model.send("router_use", Model.args("i" to sel.index))
                }, enabled = card.alternatives.isNotEmpty()) { Text("Use") }
            }
        }
    }
}

@Composable
private fun AltLine(a: AltRow, replay: Boolean) {
    Row(
        Modifier.fillMaxWidth().clickable { Model.send("router_select", Model.args("i" to a.index)) }.background(if (a.selected) c(Tok.hover) else androidx.compose.ui.graphics.Color.Transparent)
            .padding(start = 14.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        DashSwatch(a.colour, a.dash)
        Column(Modifier.weight(1f)) {
            Text(a.title, fontSize = 16.sp, fontWeight = if (a.selected) FontWeight.SemiBold else FontWeight.Normal)
            ChipsText(a.summary, size = 13.sp)
        }
        Checkbox(checked = a.selected, onCheckedChange = { Model.send("router_select", Model.args("i" to a.index)) })
        if (!replay) IconButton(onClick = { Model.send("router_remove", Model.args("i" to a.index)) }) { Icon(Icons.Outlined.Delete, "Remove this alternative") }
    }
}

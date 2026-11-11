package io.github.isomar.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ViewSidebar
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.filled.Navigation
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.AddRoad
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Remove
import androidx.compose.material.icons.outlined.DirectionsBoat
import androidx.compose.material.icons.outlined.Layers
import androidx.compose.material.icons.outlined.Map
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.Navigation
import androidx.compose.material.icons.outlined.Route
import androidx.compose.material.icons.outlined.Speed
import androidx.compose.material.icons.outlined.Straighten
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material.icons.outlined.WbCloudy
import androidx.compose.material.icons.outlined.WifiOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.RadioButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.isomar.Chip
import io.github.isomar.Detail
import io.github.isomar.Model
import io.github.isomar.Native
import io.github.isomar.Page
import io.github.isomar.TaskSheet
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.roundToInt

val Page.icon: ImageVector
    get() = when (this) {
        Page.Passage -> Icons.Outlined.Route
        Page.Weather -> Icons.Outlined.WbCloudy
        Page.Traffic -> Icons.Outlined.DirectionsBoat
        Page.Charts -> Icons.Outlined.Map
        Page.Instruments -> Icons.Outlined.Speed
    }

/** The three heights of the phone's sheet. */
enum class Stop { Peek, Half, Full }

/** The floating scale and planning row and the credits under it (`PhoneFooter`). */
private const val FOOTER = 56
/** The sheet's grabber and its tab bar: all of it that shows at its lowest. */
private const val GRABBER = 22
private const val TAB_BAR = 72

/**
 * The phone layout (design/ia.md, "Phones, foldables and tablets", compact; design/screens/android-phone-*):
 * the chart full screen and edge to edge; the status pill and the action capsule floating at the top; the
 * map controls on the right; the footer strip above a bottom sheet of three heights (peek, half, full) that
 * holds the five pages, what was tapped and the weather route. Under way, the sheet gives way to the
 * next-waypoint card at the top and the big readouts at the bottom.
 */
@Composable
fun PhoneScreen(chart: @Composable () -> Unit) {
    val density = LocalDensity.current
    val top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding()
    val bottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val underway = Model.window.underway
    BoxWithConstraints(Modifier.fillMaxSize().background(c(Tok.chartVoid))) {
        val h = maxHeight
        val landscape = maxWidth > maxHeight
        val stops = mapOf(
            Stop.Peek to (GRABBER.dp + TAB_BAR.dp + bottom),
            Stop.Half to h * 0.5f,
            Stop.Full to (h - top - 8.dp),
        )
        val start = when (Model.startSheet) { "medium" -> Stop.Half; "large" -> Stop.Full; else -> Stop.Peek }
        val sheet = remember { SheetState(start) }
        val visible = with(density) { sheet.height.value.toDp() }.coerceAtLeast(0.dp)
        LaunchedEffect(h, bottom) { sheet.place(with(density) { stops.mapValues { it.value.toPx() } }) }

        chart()
        if (Model.alerts.alarm?.dim == false) AlarmFrame()

        // What covers the chart: the floating bars at the top; the sheet, or the dashboard, and the footer.
        val dashboard = if (underway) (if (landscape) 0.dp else dashboardHeight(Model.underway.dashboard.size, h) + bottom) else 0.dp
        val covered = if (underway) dashboard else visible
        LaunchedEffect(top, covered, landscape, underway) {
            val left = if (underway && landscape) 300.0 else 0.0
            Native.setInsets((top + Size.floatingBarHeight + 8.dp).value.toDouble(), 0.0, (covered + FOOTER.dp).value.toDouble(), left)
        }

        // The floating chrome, over what the sheet leaves.
        Column(
            Modifier.fillMaxWidth().height((h - covered).coerceAtLeast(0.dp)).padding(top = top + 4.dp, start = if (underway && landscape) 304.dp else 12.dp, end = 12.dp, bottom = 6.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            val alarm = Model.alerts.alarm
            if (alarm != null && !alarm.dim) AlarmCard(alarm)
            else if (underway && Model.underway.card != null) NavigatingCard(Model.underway.card!!)
            else TopBar(onPages = { sheet.go(Stop.Full) })
            for (w in Model.alerts.warnings) WarningStrip(w)
            Model.alerts.hint?.let { HintCard(it) }
            Row(Modifier.fillMaxWidth()) {
                Spacer(Modifier.weight(1f))
                MapControls(big = underway)
            }
            Spacer(Modifier.weight(1f))
            Model.toast?.let { ToastCard(it) }
            Model.timeline?.let { TimelineCard(it) }
            PhoneFooter()
        }

        if (underway) {
            if (landscape) {
                InstrumentGrid(Model.underway.dashboard, Modifier.align(Alignment.TopStart).padding(top = top + 8.dp, start = 12.dp, bottom = bottom + 8.dp).width(280.dp).fillMaxHeight(), largest = 64.dp)
            } else {
                Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(dashboard).background(c(Tok.windowBg)).padding(start = 8.dp, end = 8.dp, top = 8.dp, bottom = bottom)) {
                    InstrumentGrid(Model.underway.dashboard, Modifier.fillMaxSize(), largest = 72.dp)
                }
            }
        } else {
            PhoneSheet(sheet, stops, h)
        }
        // What was tapped on the chart, over the pages' sheet (or the dashboard under way).
        if (Model.chartShown) ItemSheet(h, top, bottom)
    }
    // Picks open in the item sheet while this layout is on screen.
    androidx.compose.runtime.DisposableEffect(Unit) {
        Model.phoneLayout = true
        onDispose { Model.phoneLayout = false }
    }
    MenuSheet()
    Dialogs()
    TaskSheets()
}

/** Two readouts take a row, four two; a full set of instruments nearly half the screen (`InstrumentGrid`). */
private fun dashboardHeight(n: Int, h: Dp) = if (n <= 2) 140.dp else if (n <= 4) 280.dp else minOf(h * 0.46f, 460.dp)

/** The sheet's height, settling at one of three stops. */
class SheetState(initial: Stop) {
    val height = Animatable(0f)
    var stop by mutableStateOf(initial)
        private set
    private var stops: Map<Stop, Float> = emptyMap()
    private var scope: kotlinx.coroutines.CoroutineScope? = null

    suspend fun place(s: Map<Stop, Float>) {
        stops = s
        height.snapTo(s.getValue(stop))
    }

    fun attach(scope: kotlinx.coroutines.CoroutineScope) { this.scope = scope }

    fun go(to: Stop) {
        stop = to
        val target = stops[to] ?: return
        scope?.launch { height.animateTo(target, spring(stiffness = 600f, dampingRatio = 0.9f)) }
    }

    fun drag(delta: Float) {
        val lo = stops[Stop.Peek] ?: return
        val hi = stops[Stop.Full] ?: return
        scope?.launch { height.snapTo((height.value - delta).coerceIn(lo, hi)) }
    }

    /** Ends a drag: a fling goes on to the next stop that way, otherwise to the nearest. */
    fun settle(velocity: Float) {
        val now = height.value
        val ordered = stops.entries.sortedBy { it.value }
        val target = when {
            velocity < -800 -> ordered.firstOrNull { it.value > now + 1 } ?: ordered.last()
            velocity > 800 -> ordered.lastOrNull { it.value < now - 1 } ?: ordered.first()
            else -> ordered.minBy { abs(it.value - now) }
        }
        go(target.key)
    }
}

/**
 * The bottom sheet: the five pages behind a tab bar at its foot, as the system's own apps have them. At its
 * lowest only the tabs show, and any tab, the one selected too, opens it half way on its page (or the weather
 * route previewed). What is tapped on the chart opens in a sheet of its own (`ItemSheet`).
 */
@Composable
private fun PhoneSheet(sheet: SheetState, stops: Map<Stop, androidx.compose.ui.unit.Dp>, h: androidx.compose.ui.unit.Dp) {
    val scope = rememberCoroutineScope()
    sheet.attach(scope)
    val density = LocalDensity.current
    val bottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    LaunchedEffect(Model.routerCard != null) {
        Model.pagesOverRouter = false
        Model.previewOverDepartures = false
        // A new preview shows at half height: the chart and the time control above it.
        if (Model.routerCard != null && sheet.stop != Stop.Half) sheet.go(Stop.Half)
    }
    LaunchedEffect(Model.expandRequests) { if (Model.expandRequests > 0) sheet.go(Stop.Full) }
    // A script's `detent` (Script.kt).
    LaunchedEffect(Model.sheetRequest) {
        when (Model.sheetRequest) { "peek" -> sheet.go(Stop.Peek); "medium" -> sheet.go(Stop.Half); "large" -> sheet.go(Stop.Full) }
        Model.sheetRequest = null
    }
    val drag = rememberDraggableState { sheet.drag(it) }
    val visible = with(density) { sheet.height.value.toDp() }.coerceAtLeast(0.dp)
    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(visible)
                .clip(RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp))
                .background(c(Tok.sidebarBg))
                .border(1.dp, c(Tok.border), RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)),
        ) {
            Grabber(drag, sheet) { sheet.go(if (sheet.stop == Stop.Peek) Stop.Half else Stop.Peek) }
            Box(Modifier.fillMaxWidth().weight(1f)) {
                if (sheet.stop != Stop.Peek || sheet.height.isRunning) SheetContent()
            }
            androidx.compose.material3.NavigationBar(
                containerColor = c(Tok.sidebarBg),
                tonalElevation = 0.dp,
                windowInsets = WindowInsets(0, 0, 0, 0),
                modifier = Modifier.height(TAB_BAR.dp + bottom).padding(bottom = bottom)
                    .draggable(drag, Orientation.Vertical, onDragStopped = { sheet.settle(it) }),
            ) {
                for (p in Page.entries) {
                    NavigationBarItem(
                        selected = Model.page == p,
                        onClick = {
                            Model.showPage(p)
                            Model.pagesOverRouter = Model.routerCard != null
                            Model.previewOverDepartures = false
                            if (sheet.stop == Stop.Peek) sheet.go(Stop.Half)
                        },
                        icon = { Icon(p.icon, null, modifier = Modifier.size(22.dp)) },
                        label = { Text(p.title, fontSize = 11.sp, maxLines = 1) },
                    )
                }
            }
        }
    }
}

/** The handle at a sheet's top: drag it, or tap it to change the height. */
@Composable
private fun Grabber(drag: androidx.compose.foundation.gestures.DraggableState, sheet: SheetState, onTap: () -> Unit) {
    Box(
        Modifier.fillMaxWidth().height(GRABBER.dp)
            .draggable(drag, Orientation.Vertical, onDragStopped = { sheet.settle(it) })
            .clickable(onClickLabel = "Change the sheet's height", onClick = onTap),
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.width(32.dp).height(4.dp).clip(RoundedCornerShape(2.dp)).background(c(Tok.textDisabled)))
    }
}

/**
 * What was tapped on the chart, on a phone: a sheet of its own over the pages' and without their tabs, half
 * way up so the chart stays in sight and another tap there changes what it shows. A light's or a buoy's details
 * (`info`), or the page of a mark, a route or a vessel (`chartDetail`). Dragged down, it closes.
 */
@Composable
private fun ItemSheet(h: androidx.compose.ui.unit.Dp, top: androidx.compose.ui.unit.Dp, bottom: androidx.compose.ui.unit.Dp) {
    val density = LocalDensity.current
    val scope = rememberCoroutineScope()
    val sheet = remember { SheetState(Stop.Half) }
    sheet.attach(scope)
    LaunchedEffect(h) { sheet.place(with(density) { mapOf(Stop.Peek to 0f, Stop.Half to (h * 0.5f).toPx(), Stop.Full to (h - top - 8.dp).toPx()) }) }
    // Settled at the bottom: closed.
    LaunchedEffect(sheet.stop, sheet.height.isRunning) { if (sheet.stop == Stop.Peek && !sheet.height.isRunning) Model.closeChartItem() }
    // Going on to a page (Weather routing from a route) or to the chart (editing a route) leaves it.
    LaunchedEffect(Unit) {
        snapshotFlow { Model.detail }.drop(1).collect { if (it != null) Model.closeChartItem() }
    }
    LaunchedEffect(Model.alerts.hint != null) { if (Model.alerts.hint != null) Model.closeChartItem() }
    val drag = rememberDraggableState { sheet.drag(it) }
    val visible = with(density) { sheet.height.value.toDp() }.coerceAtLeast(0.dp)
    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(visible)
                .shadow(12.dp, RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp))
                .clip(RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp))
                .background(c(Tok.sidebarBg))
                .border(1.dp, c(Tok.border), RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp))
                .padding(bottom = bottom),
        ) {
            Grabber(drag, sheet) { sheet.go(if (sheet.stop == Stop.Full) Stop.Half else Stop.Full) }
            Box(Modifier.fillMaxWidth().weight(1f)) {
                val info = Model.info
                val d = Model.chartDetail
                if (info != null) InfoSheet(info)
                else if (d != null) androidx.compose.runtime.CompositionLocalProvider(LocalStandaloneDetail provides true) { DetailView(d) }
            }
        }
    }
}

@Composable
internal fun SheetContent() {
    val card = Model.routerCard
    val detail = Model.detail
    // Best departure's results stay in sight: picking one previews it on the chart (design/screens/*-best-departure).
    val departures = detail is Detail.Routing && detail.window && Model.departures?.routeId == detail.id
    when {
        card != null && !Model.pagesOverRouter && !departures -> RouterSheet(card)
        card != null && !Model.pagesOverRouter && Model.previewOverDepartures -> Column(Modifier.fillMaxSize()) {
            TextButton(onClick = { Model.previewOverDepartures = false }, modifier = Modifier.padding(start = 8.dp)) {
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowLeft, null)
                Text("Best departure")
            }
            Box(Modifier.weight(1f)) { RouterSheet(card) }
        }
        else -> Column(Modifier.fillMaxSize()) {
            // While a weather route is previewed, the way back to it.
            if (card != null) BackToRouter(card)
            Box(Modifier.weight(1f)) { if (detail != null) DetailView(detail) else PageView(Model.page) }
        }
    }
}

@Composable
private fun BackToRouter(card: io.github.isomar.RouterCard) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).clip(RoundedCornerShape(Radius.m)).background(c(Tok.accent).copy(alpha = 0.12f))
            .clickable { Model.pagesOverRouter = false; Model.previewOverDepartures = true }.padding(horizontal = 14.dp).height(Size.hitMinAndroid),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(Icons.Outlined.WbCloudy, null, tint = c(Tok.accent))
        Text(card.title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text("Show", fontSize = 15.sp, color = c(Tok.accent))
    }
}

// The floating chrome.

/** The status pill (pages button, title, speed and link) and the action capsule (weather, layers, more). */
@Composable
internal fun TopBar(onPages: (() -> Unit)?, newRoute: Boolean = false) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.SpaceBetween) {
        // One line: the light, and the speed (words only when something is wrong). It gives way to the capsule.
        FloatingCard(shape = CircleShape, modifier = Modifier.padding(end = 8.dp).height(Size.floatingBarHeight).weight(1f, fill = false)) {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(end = 14.dp)) {
                if (onPages != null) IconButton(onClick = onPages, modifier = Modifier.size(Size.hitMinAndroid)) { Icon(Icons.AutoMirrored.Outlined.ViewSidebar, "Show the pages") }
                else Spacer(Modifier.width(16.dp))
                val link = Model.window.link
                Box(Modifier.size(8.dp).clip(CircleShape).background(c(if (link == "ok") Tok.ok else if (link == "stale") Tok.warning else Tok.textDisabled)))
                Spacer(Modifier.width(6.dp))
                val said = if (link == "ok") Model.window.subtitle.filter { it.icon != null } else Model.window.subtitle
                Box(Modifier.weight(1f, fill = false)) {
                    if (said.isEmpty()) Text(Model.window.title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
                    else ChipsText(said, size = 15.sp, maxLines = 1)
                }
                ActivityIndicator()
            }
        }
        ActionCapsule(newRoute)
    }
}

/** A spinner while jobs run, with the jobs and their Cancel in a menu, opened too by a busy job button (`Model.job`). */
@Composable
private fun ActivityIndicator() {
    var pressed by remember { mutableStateOf(false) }
    val open = pressed || Model.activityShown
    val jobs = Model.window.jobs
    if (jobs.isNotEmpty()) {
        Box {
            Box(Modifier.size(Size.hitMinAndroid).clickable(onClickLabel = "Running tasks") { pressed = true }, contentAlignment = Alignment.Center) {
                CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
            }
            DropdownMenu(open, onDismissRequest = { pressed = false; Model.activityShown = false }) {
                for (j in jobs) {
                    DropdownMenuItem(
                        text = {
                            Column {
                                Text(j.title, fontWeight = FontWeight.SemiBold)
                                if (j.detail.isNotEmpty()) Text(j.detail, fontSize = 12.sp, color = c(Tok.textMuted))
                            }
                        },
                        trailingIcon = if (j.cancellable) ({ TextButton(onClick = { Model.command("job_cancel", j.id) }) { Text("Cancel") } }) else null,
                        onClick = {},
                    )
                }
            }
        }
    } else if (Model.window.offline) {
        Icon(Icons.Outlined.WifiOff, "Offline: no downloads", modifier = Modifier.padding(start = 8.dp).size(18.dp))
    }
}

@Composable
internal fun ActionCapsule(newRoute: Boolean = false) {
    var layers by remember { mutableStateOf(false) }
    var more by remember { mutableStateOf(false) }
    FloatingCard(shape = CircleShape, modifier = Modifier.height(Size.floatingBarHeight)) {
        Row(Modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            if (newRoute) IconButton(onClick = { Model.command("new_route") }) { Icon(Icons.Outlined.AddRoad, "New route") }
            IconButton(onClick = { Model.showPage(Page.Weather); Model.expandRequests++ }) { Icon(Icons.Outlined.WbCloudy, "Weather") }
            Box {
                IconButton(onClick = { layers = true }) { Icon(Icons.Outlined.Layers, "Layers") }
                LayersMenu(layers) { layers = false }
            }
            Box {
                IconButton(onClick = { more = true }) { Icon(Icons.Outlined.Menu, "More") }
                MoreMenu(more) { more = false }
            }
        }
    }
}

/** The layers to tick, grouped as the desktop's Layers popover. */
@Composable
fun LayersMenu(open: Boolean, close: () -> Unit) {
    val groups = listOf(
        "Chart data" to listOf("enc", "bsb", "depth", "soundings", "aids", "graticule"),
        "Base map" to listOf("land", "online-land", "online-seamarks", "towns"),
        "Overlays" to listOf("nav", "weather"),
        "Downloads" to listOf("online-depth"),
    )
    DropdownMenu(open, onDismissRequest = close) {
        groups.forEachIndexed { gi, (title, ids) ->
            if (gi > 0) HorizontalDivider()
            Text(title, fontSize = 12.sp, color = c(Tok.textMuted), modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
            for (id in ids) {
                val l = Model.layers.firstOrNull { it.id == id } ?: continue
                DropdownMenuItem(
                    text = { Text(l.label) },
                    leadingIcon = { androidx.compose.material3.Checkbox(checked = l.visible, onCheckedChange = null) },
                    onClick = { Model.setLayer(l.id, !l.visible) },
                )
            }
        }
    }
}

/** The desktop's primary menu: the look, the under-way layout, settings, and what the planning chart is. */
@Composable
private fun MoreMenu(open: Boolean, close: () -> Unit) {
    DropdownMenu(open, onDismissRequest = close) {
        Text("Look", fontSize = 12.sp, color = c(Tok.textMuted), modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
        for ((id, label) in listOf("auto" to "Automatic", "day" to "Day", "dusk" to "Dusk", "night" to "Night")) {
            DropdownMenuItem(text = { Text(label) }, leadingIcon = { RadioButton(selected = Model.mode == id, onClick = null) }, onClick = { Model.changeMode(id); close() })
        }
        HorizontalDivider()
        DropdownMenuItem(text = { Text(if (Model.window.underway) "Planning layout" else "Under way layout") }, onClick = {
            Model.send("underway_set", Model.args("on" to !Model.window.underway)); close()
        })
        DropdownMenuItem(text = { Text("Settings") }, onClick = { Model.sheet = TaskSheet.Settings; close() })
        DropdownMenuItem(text = { Text("Connections") }, onClick = { Model.sheet = TaskSheet.Connections; close() })
        DropdownMenuItem(text = { Text("Import GPX") }, onClick = { Model.pickFile("gpx"); close() })
        DropdownMenuItem(text = { Text("Export all as GPX") }, onClick = { Model.pickFile("gpx-export:0:isomar"); close() })
        DropdownMenuItem(text = { Text("About Isomar") }, onClick = { Model.sheet = TaskSheet.About; close() })
        DropdownMenuItem(text = { Text("Not for Navigation") }, leadingIcon = { Icon(Icons.Outlined.WarningAmber, null) }, onClick = { Model.sheet = TaskSheet.PlanningOnly; close() })
    }
}

/** The round map controls on the right: follow the boat, the layers, measure. Zooming is a pinch. */
@Composable
internal fun MapControls(big: Boolean, zoom: Boolean = false) {
    val size = if (big) Size.mapControlUnderway else Size.mapControlTouch
    var layers by remember { mutableStateOf(false) }
    FloatingCard(shape = RoundedCornerShape(Radius.l)) {
        Column(Modifier.padding(4.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            MapButton(if (Model.follow) Icons.Filled.Navigation else Icons.Outlined.Navigation, if (Model.follow) "Stop following the boat" else "Keep the boat centred", active = Model.follow, size = size) { Model.toggleFollow() }
            HorizontalDivider(Modifier.width(size * 0.6f).padding(vertical = 2.dp), color = c(Tok.border))
            Box {
                MapButton(Icons.Outlined.Layers, "Layers", size = size) { layers = true }
                LayersMenu(layers) { layers = false }
            }
            if (!big) MapButton(Icons.Outlined.Straighten, "Measure a distance and bearing", active = Model.measuring, size = size) { Model.toggleMeasure() }
            if (zoom) {
                HorizontalDivider(Modifier.width(size * 0.6f).padding(vertical = 2.dp), color = c(Tok.border))
                MapButton(Icons.Outlined.Add, "Zoom in", size = size) { Native.zoomBy(1.0) }
                MapButton(Icons.Outlined.Remove, "Zoom out", size = size) { Native.zoomBy(-1.0) }
            }
        }
    }
}

/**
 * Over the chart just above the sheet: the scale bar and the planning pill on small cards, the credits under
 * them (a tap lists them all). A strip along the sheet's edge was cramped and its credits too small to read.
 */
@Composable
internal fun PhoneFooter() {
    val f = Model.footer
    var credits by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.height(24.dp).clip(CircleShape).background(c(Tok.cardBg).copy(alpha = 0.9f)).padding(horizontal = 10.dp), contentAlignment = Alignment.Center) {
                ScaleLine(f.scale.px, f.scale.label)
            }
            Spacer(Modifier.weight(1f))
            if (f.planningOnly) {
                Pill(Icons.Outlined.WarningAmber, "Planning only", c(Tok.warning), Modifier.clip(CircleShape).background(c(Tok.cardBg).copy(alpha = 0.9f)).clickable(onClickLabel = f.planningText) { Model.sheet = TaskSheet.PlanningOnly })
            }
        }
        if (f.credits.isNotEmpty()) {
            Text(
                credit(f.credits), fontSize = 13.sp, color = c(Tok.text), maxLines = 1, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.clip(CircleShape).background(c(Tok.cardBg).copy(alpha = 0.8f)).clickable(onClickLabel = "Who the chart's data comes from") { credits = true }.padding(horizontal = 8.dp, vertical = 3.dp),
            )
        }
    }
    if (credits) AlertDialog(
        onDismissRequest = { credits = false },
        title = { Text("Credits") },
        text = { Text(f.credits.joinToString("\n")) },
        confirmButton = { TextButton(onClick = { credits = false }) { Text("OK") } },
    )
}

/** A strip under the chart: one scale bar, the credits and the planning pill (half an open foldable, a tablet). */
@Composable
internal fun FooterStrip() {
    val f = Model.footer
    Row(
        Modifier.fillMaxSize().background(c(Tok.windowBg).copy(alpha = 0.92f)).padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        ScaleLine(f.scale.px, f.scale.label)
        Text(credit(f.credits), fontSize = 12.sp, color = c(Tok.textMuted), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        if (f.planningOnly) {
            Pill(Icons.Outlined.WarningAmber, "Planning only", c(Tok.warning), Modifier.clickable(onClickLabel = f.planningText) { Model.sheet = TaskSheet.PlanningOnly })
        }
    }
}

private fun credit(c: List<String>): String {
    val first = c.firstOrNull()?.replace("© ", "") ?: return ""
    return if (c.size > 1) "© $first and ${c.size - 1} more" else "© $first"
}

/** The frame round the chart while an alarm sounds: 4 to 8 dp at 1 Hz; a steady 8 with reduced motion. */
@Composable
internal fun AlarmFrame() {
    val pulse = rememberInfiniteTransition(label = "alarm")
    val still = reduceMotion()
    val w by pulse.animateFloat(4f, 8f, infiniteRepeatable(tween((Timing.alarmPulse / 2).toInt()), RepeatMode.Reverse), label = "frame")
    Box(Modifier.fillMaxSize().border((if (still) 8f else w).dp, c(Tok.alarm)))
}

@Composable
fun reduceMotion(): Boolean {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    return remember { android.provider.Settings.Global.getFloat(ctx.contentResolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
}

@Composable
internal fun ToastCard(text: String) {
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Text(
            text, color = c(Tok.windowBg), fontSize = 14.sp,
            modifier = Modifier.clip(RoundedCornerShape(Radius.s)).background(c(Tok.text)).padding(horizontal = 16.dp, vertical = 10.dp),
        )
    }
}

// What was tapped, the long-press actions, and the engine's questions.

/** What was tapped, in the sheet: its title and kind, its rows, and the stepper through what lies under the tap. */
@Composable
internal fun InfoSheet(info: io.github.isomar.InfoRequest) {
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 4.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(info.title, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
                info.subtitle?.takeIf { it.isNotEmpty() }?.let { Text(it, fontSize = 14.sp, color = c(Tok.textMuted)) }
            }
            val ch = info.choice
            if (ch != null && ch.size == 2 && ch[1] > 1) {
                IconButton(onClick = { Model.command("info_cycle", -1) }) { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowLeft, "Previous") }
                Text("${ch[0]} of ${ch[1]}", fontSize = 13.sp, style = Numeric)
                IconButton(onClick = { Model.command("info_cycle", 1) }) { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, "Next") }
            }
            IconButton(onClick = { Model.closeInfo() }) { Icon(Icons.Outlined.Close, "Close") }
        }
        androidx.compose.foundation.lazy.LazyColumn(Modifier.fillMaxSize().padding(top = 8.dp)) {
            item {
                BoxedList {
                    info.rows.forEachIndexed { i, r ->
                        if (i > 0) ListDivider()
                        Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 12.dp)) {
                            Text(r.getOrElse(0) { "" }, fontSize = 14.sp, color = c(Tok.textMuted), modifier = Modifier.weight(0.4f))
                            androidx.compose.foundation.text.selection.SelectionContainer(Modifier.weight(0.6f)) {
                                Text(r.getOrElse(1) { "" }, fontSize = 14.sp, textAlign = androidx.compose.ui.text.style.TextAlign.End, modifier = Modifier.fillMaxWidth())
                            }
                        }
                    }
                }
                Spacer(Modifier.height(24.dp))
            }
        }
    }
}

/** The actions of a long press: a sheet headed by the position, one row per action. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun MenuSheet() {
    val menu = Model.menu ?: return
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(onDismissRequest = { Model.menu = null }, sheetState = state, containerColor = c(Tok.sidebarBg)) {
        Column(Modifier.padding(bottom = 24.dp)) {
            if (menu.title.isNotEmpty()) Text(menu.title, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, style = Numeric, modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp))
            BoxedList {
                val entries = menu.sections.flatten()
                entries.forEachIndexed { i, e ->
                    if (i > 0) ListDivider()
                    Text(
                        e.label, fontSize = 16.sp,
                        modifier = Modifier.fillMaxWidth().clickable { Model.menu = null; Native.menuActivate(e.i) }.padding(horizontal = 16.dp, vertical = 16.dp),
                    )
                }
            }
        }
    }
}

@Composable
internal fun Dialogs() {
    Model.confirm?.let { q ->
        val answer = { ok: Boolean -> Model.confirm = null; Native.confirmResult(q.id, ok) }
        AlertDialog(
            onDismissRequest = { answer(false) },
            title = { Text(q.heading) },
            text = if (q.body.isNotEmpty()) ({ Text(q.body) }) else null,
            confirmButton = { TextButton(onClick = { answer(true) }) { Text(q.ok, color = if (q.destructive) c(Tok.alarm) else Color.Unspecified) } },
            dismissButton = { TextButton(onClick = { answer(false) }) { Text("Cancel") } },
        )
    }
    Model.error?.let { e ->
        AlertDialog(
            onDismissRequest = { Model.error = null },
            title = { Text(e.heading) },
            text = { Text(e.details) },
            confirmButton = { TextButton(onClick = { Model.error = null }) { Text("OK") } },
        )
    }
}

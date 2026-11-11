package io.github.isomar.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.isomar.InfoRequest
import io.github.isomar.Model
import io.github.isomar.Native
import io.github.isomar.Page

/**
 * The layout follows the window, not the device (design/ia.md, "Phones, foldables and tablets"): under 600 dp
 * wide, or 480 tall (a phone on its side), the phone's sheet; otherwise the tablet's navigation rail and page
 * panel. A foldable open like a book puts the rail and the pages in the left half and the chart in the right,
 * nothing across the hinge.
 */
@Composable
fun IsomarScreen(chart: @Composable () -> Unit) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        if (maxWidth < 600.dp || maxHeight < 480.dp) PhoneScreen(chart) else TabletScreen(chart, maxWidth, maxHeight)
    }
}

@Composable
private fun TabletScreen(chart: @Composable () -> Unit, w: Dp, h: Dp) {
    val pad = WindowInsets.safeDrawing.asPaddingValues()
    val dir = LocalLayoutDirection.current
    val left = pad.calculateLeftPadding(dir)
    val hinge = Model.hinge
    val foldable = hinge != null
    val portrait = h > w && !foldable
    val underway = Model.window.underway
    // The rail and the panel: up to the hinge on a foldable, the design's width on a tablet.
    val panel = if (hinge != null) (hinge.dp - Size.navRailWidth - left).coerceAtLeast(240.dp) else 340.dp
    // Under way the instruments take the pages' place, beside the chart (`UnderwayPanel`).
    val show = Model.panelOpen && !underway
    val over = portrait && show
    val hingeGap = if (hinge != null) (Model.hingeWidth.dp) else 0.dp

    Box(Modifier.fillMaxSize().background(c(Tok.windowBg))) {
        Row(Modifier.fillMaxSize()) {
            Rail(left)
            if (underway) {
                UnderwayPanel(panel, pad.calculateTopPadding(), pad.calculateBottomPadding())
                VerticalDivider(color = c(Tok.border))
                if (hingeGap > 0.dp) Spacer(Modifier.width(hingeGap))
            } else if (show && !over) {
                Panel(panel, pad.calculateTopPadding())
                VerticalDivider(color = c(Tok.border))
                if (hingeGap > 0.dp) Spacer(Modifier.width(hingeGap))
            }
            Box(Modifier.weight(1f).fillMaxHeight()) {
                ChartArea(chart, over = if (over) panel else 0.dp, foldable = foldable, top = pad.calculateTopPadding(), bottom = pad.calculateBottomPadding())
            }
        }
        if (over) {
            Row(Modifier.padding(start = Size.navRailWidth + left).fillMaxHeight().shadow(12.dp)) {
                Panel(panel, pad.calculateTopPadding())
                VerticalDivider(color = c(Tok.border))
            }
        }
    }
    MenuSheet()
    Dialogs()
    TaskSheets()
}

/** The navigation rail: the panel's switch, and the five pages with their names. */
@Composable
private fun Rail(left: Dp) {
    NavigationRail(
        Modifier.padding(start = left).width(Size.navRailWidth),
        containerColor = c(Tok.sidebarBg),
        header = {
            IconButton(onClick = { Model.panelOpen = !Model.panelOpen }) { Icon(Icons.Outlined.Menu, if (Model.panelOpen) "Hide the pages" else "Show the pages") }
        },
    ) {
        Spacer(Modifier.height(12.dp))
        for (p in Page.entries) {
            NavigationRailItem(
                selected = Model.page == p,
                onClick = {
                    if (Model.page == p && Model.detail == null) Model.panelOpen = !Model.panelOpen
                    else { Model.showPage(p); Model.panelOpen = true }
                },
                icon = { Icon(p.icon, null) },
                label = { Text(p.title, fontSize = 12.sp) },
            )
        }
    }
}

/** The page panel: the page, the detail it pushed, the weather route. */
@Composable
private fun Panel(width: Dp, top: Dp) {
    Box(Modifier.width(width).fillMaxHeight().background(c(Tok.sidebarBg)).padding(top = top)) {
        val card = Model.routerCard
        val detail = Model.detail
        when {
            card != null -> RouterSheet(card)
            detail != null -> DetailView(detail)
            else -> PageView(Model.page)
        }
    }
}

@Composable
private fun ChartArea(chart: @Composable () -> Unit, over: Dp, foldable: Boolean, top: Dp, bottom: Dp) {
    val underway = Model.window.underway
    LaunchedEffect(over, top) { Native.setInsets((top + Size.floatingBarHeight + 16.dp).value.toDouble(), 0.0, 30.0 + bottom.value, over.value.toDouble()) }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val area = maxWidth
        // Half an open foldable: the alarm needs a row of its own.
        val narrow = area - over < 600.dp
        chart()
        if (Model.alerts.alarm?.dim == false) AlarmFrame()
        Column(
            Modifier.fillMaxSize().padding(start = over + 12.dp, end = 12.dp, top = top + 8.dp, bottom = 30.dp + bottom + 12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            val alarm = Model.alerts.alarm
            // The card on the left takes what the capsule leaves (a spacer sharing it took half, and on half a
            // foldable the alarm was a word wide).
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.SpaceBetween) {
                when {
                    alarm != null && !alarm.dim && narrow -> { Spacer(Modifier.weight(1f)); ActionCapsule() }
                    alarm != null && !alarm.dim -> {
                        Box(Modifier.padding(end = 8.dp).widthIn(max = 560.dp).weight(1f, fill = false)) { AlarmCard(alarm) }
                        ActionCapsule()
                    }
                    // The open foldable's left half shows the pages: the right half needs no status pill.
                    foldable -> { Spacer(Modifier.weight(1f)); ActionCapsule(newRoute = !underway) }
                    else -> TopBar(onPages = null, newRoute = !underway)
                }
            }
            if (alarm != null && !alarm.dim && narrow) AlarmCard(alarm)
            for (w in Model.alerts.warnings) Box(Modifier.widthIn(max = 560.dp)) { WarningStrip(w) }
            Model.alerts.hint?.let { Box(Modifier.widthIn(max = 560.dp)) { HintCard(it) } }
            Row(Modifier.fillMaxWidth()) {
                Spacer(Modifier.weight(1f))
                MapControls(big = underway, zoom = true)
            }
            Spacer(Modifier.weight(1f))
            Model.toast?.let { ToastCard(it) }
            Model.timeline?.let { Box(Modifier.widthIn(max = 520.dp)) { TimelineCard(it) } }
        }
        Model.info?.let { InfoCard(it, maxWidth, maxHeight) }
        Box(Modifier.align(Alignment.BottomStart).padding(start = over, bottom = bottom).fillMaxWidth().height(30.dp)) { FooterStrip() }
    }
}

/** What was tapped, on a tablet: a card by the place, as the desktop's popover. */
@Composable
private fun InfoCard(info: InfoRequest, w: Dp, h: Dp) {
    val cw = 340.dp
    val x = (info.x.dp + 16.dp).coerceIn(12.dp, (w - cw - 12.dp).coerceAtLeast(12.dp))
    val y = (info.y.dp - 40.dp).coerceIn(80.dp, (h - 380.dp).coerceAtLeast(80.dp))
    FloatingCard(Modifier.offset(x, y).width(cw).height(360.dp), shape = RoundedCornerShape(Radius.l)) {
        Column(Modifier.padding(bottom = 8.dp)) {
            Row(Modifier.padding(start = 16.dp, top = 8.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(info.title, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                    info.subtitle?.takeIf { it.isNotEmpty() }?.let { Text(it, fontSize = 13.sp, color = c(Tok.textMuted)) }
                }
                val ch = info.choice
                if (ch != null && ch.size == 2 && ch[1] > 1) {
                    androidx.compose.material3.TextButton(onClick = { Model.command("info_cycle", -1) }) { Text("‹") }
                    Text("${ch[0]} of ${ch[1]}", fontSize = 12.sp, style = Numeric)
                    androidx.compose.material3.TextButton(onClick = { Model.command("info_cycle", 1) }) { Text("›") }
                }
                IconButton(onClick = { Model.closeInfo() }) { Icon(Icons.Outlined.Close, "Close") }
            }
            Column(Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
                for (r in info.rows) {
                    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                        Text(r.getOrElse(0) { "" }, fontSize = 13.sp, color = c(Tok.textMuted), modifier = Modifier.width(110.dp))
                        Text(r.getOrElse(1) { "" }, fontSize = 13.sp)
                    }
                }
            }
        }
    }
}

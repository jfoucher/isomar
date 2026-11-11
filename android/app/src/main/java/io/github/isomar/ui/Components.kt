package io.github.isomar.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowForward
import androidx.compose.material.icons.automirrored.outlined.TrendingUp
import androidx.compose.material.icons.outlined.AccessTime
import androidx.compose.material.icons.outlined.Air
import androidx.compose.material.icons.outlined.Anchor
import androidx.compose.material.icons.outlined.ArrowDownward
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.CalendarToday
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material.icons.outlined.CloudDownload
import androidx.compose.material.icons.outlined.Explore
import androidx.compose.material.icons.outlined.Flag
import androidx.compose.material.icons.outlined.HourglassEmpty
import androidx.compose.material.icons.outlined.LocationOn
import androidx.compose.material.icons.outlined.NightsStay
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material.icons.outlined.Place
import androidx.compose.material.icons.outlined.SatelliteAlt
import androidx.compose.material.icons.outlined.SdStorage
import androidx.compose.material.icons.outlined.Speed
import androidx.compose.material.icons.outlined.Straighten
import androidx.compose.material.icons.outlined.SwapHoriz
import androidx.compose.material.icons.outlined.Thermostat
import androidx.compose.material.icons.outlined.Tornado
import androidx.compose.material.icons.outlined.Verified
import androidx.compose.material.icons.outlined.VerticalAlignBottom
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material.icons.outlined.Water
import androidx.compose.material.icons.outlined.WbCloudy
import androidx.compose.material.icons.outlined.ZoomIn
import androidx.compose.material.icons.outlined.Compress
import androidx.compose.material.icons.outlined.PrecisionManufacturing
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Icon
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.Placeholder
import androidx.compose.ui.text.PlaceholderVerticalAlign
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import io.github.isomar.Chip

/** The Material icon of each kind of value (the GTK shell maps the same names in `ui/chips.rs`). */
fun chipIcon(icon: String): ImageVector = when (icon) {
    "satellite" -> Icons.Outlined.SatelliteAlt
    "speed" -> Icons.Outlined.Speed
    "distance" -> Icons.Outlined.Straighten
    "points" -> Icons.Outlined.Place
    "time" -> Icons.Outlined.AccessTime
    "arrival" -> Icons.Outlined.Flag
    "date" -> Icons.Outlined.CalendarToday
    "wind" -> Icons.Outlined.Air
    "gusts" -> Icons.Outlined.Tornado
    "waves" -> Icons.Outlined.Water
    "current" -> Icons.AutoMirrored.Outlined.ArrowForward
    "pressure" -> Icons.Outlined.Compress
    "temperature" -> Icons.Outlined.Thermostat
    "engine" -> Icons.Outlined.PrecisionManufacturing
    "depth" -> Icons.Outlined.VerticalAlignBottom
    "compass" -> Icons.Outlined.Explore
    "position" -> Icons.Outlined.LocationOn
    "zoom" -> Icons.Outlined.ZoomIn
    "size" -> Icons.Outlined.SdStorage
    "licence" -> Icons.Outlined.Verified
    "warning" -> Icons.Outlined.WarningAmber
    "alarm" -> Icons.Outlined.NotificationsActive
    "upwind" -> Icons.AutoMirrored.Outlined.TrendingUp
    "night" -> Icons.Outlined.NightsStay
    "tack" -> Icons.Outlined.SwapHoriz
    "waiting" -> Icons.Outlined.HourglassEmpty
    "forecast" -> Icons.Outlined.WbCloudy
    "source" -> Icons.Outlined.CloudDownload
    "high" -> Icons.Outlined.ArrowUpward
    "low" -> Icons.Outlined.ArrowDownward
    "anchor" -> Icons.Outlined.Anchor
    else -> Icons.Outlined.Circle
}

/** The chips' words, for TalkBack. */
fun chipsPlain(chips: List<Chip>) = chips.map { it.label }.filter { it.isNotEmpty() }.joinToString(", ")

/**
 * The chips as one text, so a line of them wraps like words: each icon is held to its value by no-break
 * spaces, and the chips are apart by ordinary ones.
 */
@Composable
fun ChipsText(chips: List<Chip>, modifier: Modifier = Modifier, size: TextUnit = 13.sp, color: Color = c(Tok.textMuted), maxLines: Int = 2, style: TextStyle = TextStyle.Default) {
    val icons = chips.mapNotNull { it.icon }.distinct()
    val text: AnnotatedString = buildAnnotatedString {
        chips.forEachIndexed { i, ch ->
            if (i > 0) append("   ")
            if (ch.icon != null) {
                appendInlineContent(ch.icon, " ")
                append(' ')
            }
            append(ch.text.replace(' ', ' '))
        }
    }
    val inline = icons.associateWith { name ->
        InlineTextContent(Placeholder(1.1.em, 1.1.em, PlaceholderVerticalAlign.TextCenter)) {
            Icon(chipIcon(name), contentDescription = null, tint = color)
        }
    }
    Text(text, modifier = modifier, fontSize = size, color = color, maxLines = maxLines, overflow = TextOverflow.Ellipsis, inlineContent = inline, style = style)
}

/** Figures in tabular digits, so they do not dance as they change. */
val Numeric = TextStyle(fontFeatureSettings = "tnum", fontFamily = FontFamily.Default)

@Composable
fun SectionHeader(title: String, trailing: @Composable RowScope.() -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 14.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(title, fontSize = 14.sp, fontWeight = FontWeight.Medium, color = c(Tok.accent))
        Spacer(Modifier.weight(1f))
        trailing()
    }
}

/** A grouped list on a card, as in the design's pages. */
@Composable
fun BoxedList(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier.padding(horizontal = 16.dp).fillMaxWidth().clip(RoundedCornerShape(Radius.m))
            .background(c(Tok.viewBg)).border(1.dp, c(Tok.border), RoundedCornerShape(Radius.m)),
        content = content,
    )
}

@Composable
fun ListDivider() = HorizontalDivider(color = c(Tok.border), thickness = 1.dp)

/** One line of a list: an icon, a title and what it is, and what can be done with it. */
@Composable
fun ListRow(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String = "",
    chips: List<Chip> = emptyList(),
    more: List<Chip> = emptyList(),
    icon: ImageVector? = null,
    iconTint: Color? = null,
    selected: Boolean = false,
    dim: Boolean = false,
    /** Its job runs (`Model.busy`): a spinner in place of the icon, dimmed. */
    busy: Boolean = false,
    onClick: (() -> Unit)? = null,
    trailing: @Composable RowScope.() -> Unit = {},
) {
    val bg = if (selected) c(Tok.hover) else Color.Transparent
    Row(
        modifier.jobBusy(busy).fillMaxWidth().background(bg).let { if (onClick != null) it.clickable(onClick = onClick) else it }
            .padding(start = 16.dp, end = 4.dp).height(androidx.compose.ui.unit.max(Size.hitMinAndroid + 12.dp, 60.dp)),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        if (busy) androidx.compose.material3.CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
        else if (icon != null) Icon(icon, null, tint = iconTint ?: if (selected) c(Tok.route) else c(Tok.textMuted), modifier = Modifier.size(20.dp))
        Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
            val tc = if (dim) c(Tok.textDisabled) else c(Tok.text)
            Text(title, fontSize = 16.sp, color = tc, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (chips.isNotEmpty()) ChipsText(chips, size = 13.sp)
            else if (subtitle.isNotEmpty()) Text(subtitle, fontSize = 13.sp, color = c(Tok.textMuted), maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (more.isNotEmpty()) ChipsText(more, size = 13.sp)
        }
        trailing()
    }
}

/** What the design calls a card: the floating chrome over the chart. Night has no shadow (it glows). */
@Composable
fun FloatingCard(modifier: Modifier = Modifier, shape: androidx.compose.ui.graphics.Shape = RoundedCornerShape(Radius.l), content: @Composable () -> Unit) {
    val night = LocalTheme.current == ThemeId.Night
    Surface(
        modifier = modifier.shadow(if (night) 0.dp else 6.dp, shape, clip = false),
        shape = shape,
        color = c(Tok.cardBg),
        contentColor = c(Tok.text),
        border = androidx.compose.foundation.BorderStroke(1.dp, c(Tok.border)),
        content = content,
    )
}

/** A round map control at touch size; filled with the accent while it is on. */
@Composable
fun MapButton(icon: ImageVector, label: String, active: Boolean = false, size: Dp = Size.mapControlTouch, onClick: () -> Unit) {
    Box(
        Modifier.size(size).clip(CircleShape).background(if (active) c(Tok.accentBg) else Color.Transparent).clickable(onClickLabel = label, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, label, tint = if (active) c(Tok.onAccent) else c(Tok.text), modifier = Modifier.size(24.dp))
    }
}

/** The small outlined pill of the footer (Planning only). */
@Composable
fun Pill(icon: ImageVector, text: String, tint: Color, modifier: Modifier = Modifier) {
    Row(
        modifier.border(1.dp, tint, RoundedCornerShape(Radius.full)).background(tint.copy(alpha = 0.12f), RoundedCornerShape(Radius.full))
            .padding(horizontal = 8.dp, vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Icon(icon, null, tint = tint, modifier = Modifier.size(12.dp))
        Text(text.uppercase(), fontSize = 11.sp, fontWeight = FontWeight.Bold, color = tint, letterSpacing = 0.04.em)
    }
}

/** The scale bar: a bracket the length of a round distance, and the distance. */
@Composable
fun ScaleLine(px: Double, label: String) {
    val ink = c(Tok.text)
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Canvas(Modifier.width(px.dp).height(6.dp)) {
            val w = size.width
            val h = size.height
            drawLine(ink, Offset(0f, 0f), Offset(0f, h), 1.5.dp.toPx())
            drawLine(ink, Offset(0f, h), Offset(w, h), 1.5.dp.toPx())
            drawLine(ink, Offset(w, 0f), Offset(w, h), 1.5.dp.toPx())
        }
        Text(label, fontSize = 12.sp, fontWeight = FontWeight.Medium, color = ink)
    }
}

/** A short line in a route alternative's colour and dash pattern. */
@Composable
fun DashSwatch(token: String, dash: List<Double>) {
    val colour = c(Tok.byName(token) ?: Tok.route)
    Canvas(Modifier.width(28.dp).height(10.dp)) {
        val effect = if (dash.isEmpty()) null else PathEffect.dashPathEffect(dash.map { (it * density).toFloat() }.toFloatArray())
        drawLine(colour, Offset(0f, size.height / 2), Offset(size.width, size.height / 2), 3.dp.toPx(), cap = StrokeCap.Round, pathEffect = effect)
    }
}

/**
 * The forecast models as a segmented row, `selected` by id, with what the one picked is under it: `detail` (what
 * is loaded of it, then what it is) or only `about`.
 */
@Composable
fun ModelPicker(selected: String, detail: Boolean = false, pick: (String) -> Unit) {
    val models = io.github.isomar.Model.weather.models
    if (models.isEmpty()) return
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        models.forEachIndexed { i, m ->
            SegmentedButton(
                selected = m.id == selected, onClick = { pick(m.id) },
                shape = SegmentedButtonDefaults.itemShape(i, models.size),
            ) { Text(m.label.removePrefix("ECMWF ").removePrefix("DWD "), maxLines = 1) }
        }
    }
    models.firstOrNull { it.id == selected }?.let { EmptyText(if (detail) it.detail else it.about) }
}

/** A text that says there is nothing yet, and how to make some. */
@Composable
fun EmptyText(text: String) {
    Text(text, fontSize = 13.sp, color = c(Tok.textMuted), modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
}

/**
 * The filled button of the main action: `accent-bg` with `on-accent` (data/design/tokens.json), the pair that
 * reads in every palette. Material's own fills with `primary`, which is `accent`, the colour of links: at Night
 * its pale blue under the pale `on-accent` was unreadable. The ui package uses this one, not Material's.
 */
@Composable
fun Button(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: androidx.compose.material3.ButtonColors = androidx.compose.material3.ButtonDefaults.buttonColors(containerColor = c(Tok.accentBg), contentColor = c(Tok.onAccent)),
    contentPadding: androidx.compose.foundation.layout.PaddingValues = androidx.compose.material3.ButtonDefaults.ContentPadding,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.Button(
    onClick = onClick,
    modifier = modifier,
    enabled = enabled,
    colors = colors,
    contentPadding = contentPadding,
    content = content,
)

/** A job button's look while its job runs (`Model.job`): dimmed, as a disabled button, though it still takes a press. */
fun Modifier.jobBusy(busy: Boolean): Modifier = if (busy) alpha(0.5f) else this

/** A job button's content: `text` beside `icon`, or beside a spinner in its place while the job runs. */
@Composable
fun JobLabel(text: String, busy: Boolean, icon: ImageVector? = null) {
    if (busy) androidx.compose.material3.CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp, color = androidx.compose.material3.LocalContentColor.current)
    else if (icon != null) Icon(icon, null, Modifier.size(18.dp))
    if (busy || icon != null) Spacer(Modifier.width(8.dp))
    Text(text)
}

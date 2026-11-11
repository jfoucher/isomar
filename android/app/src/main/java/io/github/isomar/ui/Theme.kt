package io.github.isomar.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import io.github.isomar.Model

/** The look in force: Day, Dusk or Night (Tokens.kt, generated from data/design/tokens.json). */
val LocalTheme = staticCompositionLocalOf { ThemeId.Day }

/** A token's colour in the look in force. */
@Composable
@ReadOnlyComposable
fun c(t: Tok): Color = t.of(LocalTheme.current)

/** The look for the app's mode: "auto" follows the system (Dusk when it is dark), as on the Mac. */
fun themeFor(mode: String, systemDark: Boolean): ThemeId = when (mode) {
    "day" -> ThemeId.Day
    "dusk" -> ThemeId.Dusk
    "night" -> ThemeId.Night
    else -> if (systemDark) ThemeId.Dusk else ThemeId.Day
}

/**
 * Material 3 with the design's colours: the app's chrome is the design system's, not the wallpaper's
 * (no dynamic colour), so Night stays dim red-free blue-black on every phone.
 */
@Composable
fun IsomarTheme(content: @Composable () -> Unit) {
    val theme = themeFor(Model.mode, isSystemInDarkTheme())
    val t = { k: Tok -> k.of(theme) }
    val base = if (theme == ThemeId.Day) lightColorScheme() else darkColorScheme()
    val scheme = base.copy(
        primary = t(Tok.accent),
        onPrimary = t(Tok.onAccent),
        primaryContainer = t(Tok.accentBg),
        onPrimaryContainer = t(Tok.onAccent),
        secondary = t(Tok.accent),
        onSecondary = t(Tok.onAccent),
        secondaryContainer = t(Tok.hover),
        onSecondaryContainer = t(Tok.text),
        background = t(Tok.windowBg),
        onBackground = t(Tok.text),
        surface = t(Tok.sidebarBg),
        onSurface = t(Tok.text),
        surfaceVariant = t(Tok.viewBg),
        onSurfaceVariant = t(Tok.textMuted),
        surfaceContainerLowest = t(Tok.viewBg),
        surfaceContainerLow = t(Tok.viewBg),
        surfaceContainer = t(Tok.sidebarBg),
        surfaceContainerHigh = t(Tok.popoverBg),
        surfaceContainerHighest = t(Tok.popoverBg),
        outline = t(Tok.borderControl),
        outlineVariant = t(Tok.border),
        error = t(Tok.alarm),
        onError = t(Tok.onAlarm),
        errorContainer = t(Tok.alarmBg),
        onErrorContainer = t(Tok.onAlarm),
        scrim = t(Tok.scrim),
    )
    CompositionLocalProvider(LocalTheme provides theme) {
        MaterialTheme(colorScheme = scheme) {
            // Text outside a Surface (the sheet's pages and titles are columns on a background) takes the
            // content colour, which is black unless set: invisible at Dusk and Night.
            CompositionLocalProvider(androidx.compose.material3.LocalContentColor provides t(Tok.text), content = content)
        }
    }
}

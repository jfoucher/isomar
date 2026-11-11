import SwiftUI

/// A value after the icon that says what it is (`isomar_app::chips::Chip`): "[satellite] 8". `label` is
/// the whole phrase ("8 satellites in view"), for the tooltip and VoiceOver.
struct Chip: Decodable, Equatable, Hashable {
    let icon: String?
    let text: String
    let label: String
}

/// The SF Symbol of each kind of value (the GTK shell maps the same names in `ui/chips.rs`).
func chipSymbol(_ icon: String) -> String {
    switch icon {
    case "satellite": return "antenna.radiowaves.left.and.right"
    case "speed": return "speedometer"
    case "distance": return "ruler"
    case "points": return "mappin"
    case "time": return "clock"
    case "arrival": return "flag.checkered"
    case "date": return "calendar"
    case "wind": return "wind"
    case "gusts": return "tornado"
    case "waves": return "water.waves"
    case "current": return "arrow.forward"
    case "pressure": return "barometer"
    case "temperature": return "thermometer.medium"
    case "engine": return "engine.combustion"
    case "depth": return "arrow.down.to.line"
    case "compass": return "location.north.line"
    case "position": return "location"
    case "zoom": return "plus.magnifyingglass"
    case "size": return "internaldrive"
    case "licence": return "checkmark.seal"
    case "warning": return "exclamationmark.triangle"
    case "alarm": return "bell"
    case "upwind": return "arrow.up.left"
    case "night": return "moon"
    case "tack": return "arrow.triangle.swap"
    case "waiting": return "hourglass"
    case "forecast": return "cloud.sun"
    case "source": return "arrow.down.circle"
    case "high": return "arrow.up"
    case "low": return "arrow.down"
    default: return "circle"
    }
}

/// The chips as one `Text`, so a line of them wraps like words: each icon is held to its value by
/// no-break spaces, and the chips are apart by ordinary ones.
func chipsText(_ chips: [Chip]) -> Text {
    let nbsp = "\u{00A0}"
    var out = Text("")
    for (i, c) in chips.enumerated() {
        if i > 0 { out = out + Text("   ") }
        if let icon = c.icon { out = out + Text(Image(systemName: chipSymbol(icon))) + Text(nbsp) }
        out = out + Text(c.text.replacingOccurrences(of: " ", with: nbsp))
    }
    return out
}

/// The chips as words: their labels with commas, for tooltips and VoiceOver.
func chipsPlain(_ chips: [Chip]) -> String {
    chips.map(\.label).filter { !$0.isEmpty }.joined(separator: ", ")
}

/// Chips read from an api dictionary (`route_get`'s "chips", a plan's "summary").
func chips(from value: Any?) -> [Chip] {
    guard let value, JSONSerialization.isValidJSONObject(value), let data = try? JSONSerialization.data(withJSONObject: value) else { return [] }
    return (try? JSONDecoder().decode([Chip].self, from: data)) ?? []
}

extension View {
    /// The chips' words as the view's tooltip and accessibility label.
    func chipsHelp(_ chips: [Chip]) -> some View {
        let words = chipsPlain(chips)
        return self.help(words).accessibilityLabel(words)
    }
}

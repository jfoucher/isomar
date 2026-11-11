import SwiftUI

/// A card over the chart: solid, near-opaque, never a material.
struct CardStyle: ViewModifier {
    var radius: CGFloat = Radius.m
    func body(content: Content) -> some View {
        content
            .background(RoundedRectangle(cornerRadius: radius).fill(Color.isomar(.cardBg)))
            .overlay(RoundedRectangle(cornerRadius: radius).stroke(Color.isomar(.border), lineWidth: 1))
            .shadow(color: Theme.isNight ? .clear : .black.opacity(0.18), radius: 8, y: 2)
    }
}

extension View {
    func card(radius: CGFloat = Radius.m) -> some View { modifier(CardStyle(radius: radius)) }
}

/// A numeric readout: tabular figures always.
extension Font {
    static func numeric(_ size: CGFloat, weight: Font.Weight = .semibold) -> Font {
        .system(size: size, weight: weight).monospacedDigit()
    }
}

/// A small round button, as in the map's control cluster.
struct MapButton: View {
    let symbol: String
    var help: String
    var active = false
    var size: CGFloat = Size.mapControl
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: size * 0.42, weight: .regular))
                .frame(width: size, height: size)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(active ? Color.isomar(.onAccent) : Color.isomar(.text))
        .background(Circle().fill(active ? Color.isomar(.accentBg) : Color.clear))
        .help(help)
        .accessibilityLabel(help)
    }
}

/// The centred glyph and words of a status pill.
struct Pill: View {
    let symbol: String
    let text: String
    let tint: Tok

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: symbol)
            Text(text).font(.system(size: 10, weight: .semibold)).textCase(.uppercase)
        }
        .foregroundStyle(Color.isomar(tint))
        .padding(.horizontal, 8).padding(.vertical, 2)
        .overlay(Capsule().stroke(Color.isomar(tint), lineWidth: 1))
    }
}

/// Rows in one rounded card, like a boxed list on Linux.
struct BoxedList<Content: View>: View {
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(spacing: 0, content: content)
            .background(Color.isomar(.cardBg))
            .clipShape(RoundedRectangle(cornerRadius: Radius.m))
            .overlay(RoundedRectangle(cornerRadius: Radius.m).stroke(Color.isomar(.border), lineWidth: 1))
    }
}

/// A row of a boxed list that opens something: an icon, a title with what it does, and a chevron.
struct NavRow: View {
    let symbol: String
    let title: String
    let subtitle: String
    /// Its job runs (`IsomarModel.busy`): a spinner in place of the symbol, dimmed.
    var busy = false
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Group { if busy { ProgressView().controlSize(.small) } else { Image(systemName: symbol) } }.frame(width: 20)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(.system(size: 13))
                    Text(subtitle).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 4)
                Image(systemName: "chevron.right").foregroundStyle(Color.isomar(.textMuted))
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(hovering ? Color.isomar(.hover) : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(Color.isomar(.text))
        .onHover { hovering = $0 }
        .modifier(JobBusy(busy: busy))
    }
}

/// A text or icon button without a frame: borderless, and on a touch screen in the accent colour. The sheets
/// set the text's colour on everything in them (`Tok.text`, which changes with day, dusk and night), and a
/// borderless button takes that colour too, so on iOS it looked like plain text. The default style under
/// `touchStyle()`, and `.platformLink` where a view asks for a borderless button.
struct PlatformLinkStyle: PrimitiveButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        #if os(iOS)
        Button(configuration).buttonStyle(.borderless).foregroundStyle(Color.isomar(.accent))
        #else
        Button(configuration).buttonStyle(.borderless)
        #endif
    }
}

extension PrimitiveButtonStyle where Self == PlatformLinkStyle {
    static var platformLink: PlatformLinkStyle { PlatformLinkStyle() }
}

#if os(iOS)
/// A group on a touch screen: a white card with a hairline on the sheet's grey, its title above its rows,
/// as the boxed lists (`BoxedList`). The system's GroupBox is a grey on the sheet's grey.
struct CardGroupBoxStyle: GroupBoxStyle {
    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            configuration.label.font(.system(size: 15, weight: .semibold))
            configuration.content
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: Radius.m).fill(Color.isomar(.cardBg)))
        .overlay(RoundedRectangle(cornerRadius: Radius.m).stroke(Color.isomar(.border), lineWidth: 1))
    }
}

extension View {
    /// The touch screens' defaults for what the shared views leave unstyled: accent text buttons, card groups,
    /// the accent for controls. Applied at the root and again in each sheet, which is presented apart.
    func touchStyle() -> some View {
        // The tint is `accent`, the colour of text and glyphs: pickers and tab bars draw their labels in it
        // (`accentBg`, the filled accent, vanished at Night). Filled buttons take `accentBg` themselves.
        self.buttonStyle(.platformLink).groupBoxStyle(CardGroupBoxStyle()).tint(Color.isomar(.accent))
    }
}
#endif

extension View {
    /// `touchStyle()` on iOS; nothing on the Mac.
    @ViewBuilder func platformStyle() -> some View {
        #if os(iOS)
        self.touchStyle()
        #else
        self
        #endif
    }
}

/// The main action's button: the system's prominent button on the Mac; on a touch screen a capsule of
/// `accentBg` with its label in `onAccent`, the pair data/design/tokens.json gives a filled accent in every
/// palette (the system's white on Night's pale `accent` was unreadable).
struct PlatformProminentStyle: PrimitiveButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        #if os(iOS)
        Button(configuration).buttonStyle(AccentCapsuleStyle())
        #else
        Button(configuration).buttonStyle(.borderedProminent)
        #endif
    }
}

extension PrimitiveButtonStyle where Self == PlatformProminentStyle {
    static var platformProminent: PlatformProminentStyle { PlatformProminentStyle() }
}

#if os(iOS)
struct AccentCapsuleStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 16, weight: .semibold))
            .foregroundStyle(Color.isomar(.onAccent))
            .padding(.horizontal, 16).frame(minHeight: 40)
            .background(Capsule().fill(Color.isomar(.accentBg)))
            .contentShape(Capsule())
            .opacity(enabled ? (configuration.isPressed ? 0.75 : 1) : 0.4)
    }
}
#endif

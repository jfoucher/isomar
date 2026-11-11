import SwiftUI
import UIKit

/// The layout follows the window's width, not the device (design/ia.md, "Phones, foldables and tablets"):
/// under 600 pt (or 500 tall, a phone on its side) the phone layout; an open foldable (the iPhone Duo, about 890 × 626) the sidebar in the left
/// half; wider, the iPad's sidebar beside the chart. An iPad in Split View narrower than 600 is a phone.
struct TouchRoot: View {
    @ObservedObject var model: IsomarModel

    /// `ISOMAR_FRAME=890x626` lays the app out in a window of that size, top left, to see a layout the
    /// simulator has no device for (the open iPhone Duo) or cannot turn to (simctl does not rotate).
    private static let frame: CGSize? = {
        guard let v = ProcessInfo.processInfo.environment["ISOMAR_FRAME"]?.split(separator: "x"), v.count == 2,
              let w = Double(v[0]), let h = Double(v[1]) else { return nil }
        return CGSize(width: w, height: h)
    }()

    var body: some View {
        Group {
            if let f = Self.frame {
                layout.frame(width: f.width, height: f.height).clipped()
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    .background(Color.gray.ignoresSafeArea())
            } else {
                layout
            }
        }
        .touchStyle()
    }

    private var layout: some View {
        GeometryReader { geo in
            // A phone on its side (874 × 402) keeps the phone layout: its landscape is for a cockpit mount.
            if geo.size.width < 600 || geo.size.height < 500 {
                PhoneRoot(model: model)
            } else {
                TabletRoot(model: model, size: geo.size)
            }
        }
        .ignoresSafeArea(.keyboard)
    }
}

struct TabletRoot: View {
    @ObservedObject var model: IsomarModel
    let size: CGSize

    /// An open foldable: wide but short, and not as wide as a tablet. Its halves meet at the crease, so the
    /// sidebar takes the left one and nothing is drawn across the middle.
    private var foldable: Bool { size.width > size.height && size.width < 1000 && size.height < 700 }
    /// A tablet held upright: the sidebar opens over the chart rather than beside it.
    private var portrait: Bool { size.height > size.width && !foldable }
    private var sidebarWidth: CGFloat { foldable ? size.width / 2 : Size.sidebarWidth }
    private var underway: Bool { model.window.underway }
    /// The pages beside the chart. Under way the instruments take their place (`UnderwayColumn`).
    private var showSidebar: Bool { model.sidebarVisible && !underway }
    /// How much of the chart's left the sidebar covers, upright: the floating chrome keeps clear of it.
    private var overlaid: CGFloat { showSidebar && portrait ? sidebarWidth : 0 }
    /// The status bar and the home indicator the chart runs under: the chrome and the chart's centre keep
    /// clear of them.
    @State private var safe = EdgeInsets()

    var body: some View {
        ZStack(alignment: .leading) {
            HStack(spacing: 0) {
                if underway {
                    // Under way the instruments have the whole height beside the chart, upright too.
                    UnderwayColumn(model: model).frame(width: sidebarWidth)
                    Divider()
                } else if showSidebar && !portrait {
                    sidebar
                    Divider()
                }
                chartColumn
            }
            if showSidebar && portrait {
                HStack(spacing: 0) {
                    sidebar
                    Divider()
                }
                .shadow(color: .black.opacity(Theme.isNight ? 0 : 0.25), radius: 12)
                .transition(.move(edge: .leading))
            }
        }
        .animation(Platform.reduceMotion ? nil : .easeInOut(duration: Timing.panelSlide), value: showSidebar)
        .background(Color.isomar(.windowBg))
        .foregroundStyle(Color.isomar(.text))
        .sheet(item: $model.sheet) { s in SheetHost(sheet: s, model: model) }
        .confirmationDialog(model.menu?.title ?? "", isPresented: Binding(get: { model.menu != nil }, set: { if !$0 { model.menu = nil } }), titleVisibility: (model.menu?.title ?? "").isEmpty ? .hidden : .visible) {
            ForEach(model.menu?.sections.flatMap { $0 } ?? [], id: \.i) { e in Button(e.label) { isomar_menu_activate(Int32(e.i)) } }
        }
        .onAppear { model.sidebarVisible = true }
        .onChange(of: overlaid) { o in isomar_set_insets(safe.top + Size.floatingBarHeight + 16, safe.trailing, foldable ? safe.bottom + 60 : 0, o) }
        // A mark, route or vessel tapped on the chart opens in the sidebar (a phone gives it a sheet of its own).
        .onChange(of: model.window.picked) { p in if let p { model.open(p) } }
    }

    /// The pages, docked on the left (the open foldable's left half).
    private var sidebar: some View {
        SidebarView(model: model).frame(width: sidebarWidth)
    }

    /// The chart runs to the screen's edges, under the status bar (and the open Duo's status column on the
    /// right): a band of background there read as a margin round the chart.
    private var chartColumn: some View {
        VStack(spacing: 0) {
            ZStack {
                TouchChart().ignoresSafeArea(edges: foldable ? .all : [.top, .horizontal])
                if let a = model.alerts.alarm, !a.dim { AlarmFrame() }
                overlays.padding(.leading, overlaid)
                if let info = model.info { InfoCard(info: info, model: model) }
            }
            // Half a foldable is a phone's width: the phone's footer, on cards over the chart (in `overlays`).
            if !foldable { FooterView(model: model, live: model.live).padding(.leading, overlaid) }
        }
        .background(GeometryReader { g in Color.clear.onAppear { setInsets(g) }.onChange(of: g.size) { _ in setInsets(g) }.onChange(of: g.safeAreaInsets) { _ in setInsets(g) } })
    }

    private var overlays: some View {
        VStack(spacing: 8) {
            HStack(alignment: .top, spacing: 8) {
                // The open foldable's left half always shows the pages, so the right half needs no pill.
                if !foldable { StatusPill(model: model) { model.sidebarVisible.toggle() } }
                Spacer(minLength: 8)
                ActionCapsule(model: model, newRoute: !underway)
            }
            TouchAlerts(model: model).frame(maxWidth: 560)
            if let h = model.alerts.hint { HintCard(hint: h, model: model) }
            if let r = model.routerCard { HStack { RouterCardView(card: r, model: model); Spacer() } }
            HStack(alignment: .top) {
                Spacer()
                TabletControls(model: model)
            }
            Spacer()
            if let t = model.toast { ToastView(text: t) }
            if let t = model.timeline {
                HStack { TimelineBar(timeline: t, model: model).frame(maxWidth: 620); Spacer(minLength: 0) }
            }
            if foldable { PhoneFooter(model: model, live: model.live) }
        }
        .padding(.horizontal, Space.m)
        .padding(.top, 8)
        .padding(.bottom, Space.m)
    }

    /// The chart centres below the floating bars and clear of the status bar it runs under; the sidebar is
    /// beside it (or over it, upright).
    private func setInsets(_ g: GeometryProxy) {
        safe = g.safeAreaInsets
        let bottom = foldable ? safe.bottom + 60 : 0
        isomar_set_insets(safe.top + Size.floatingBarHeight + 16, safe.trailing, bottom, overlaid)
    }
}

/// The map controls of a tablet: follow, measure, and zoom buttons (a tablet on a stand is not always pinched).
struct TabletControls: View {
    @ObservedObject var model: IsomarModel

    private var size: CGFloat { model.window.underway ? Size.mapControlUnderway : Size.mapControlTouch }

    var body: some View {
        VStack(spacing: 2) {
            MapButton(symbol: model.follow > 0 ? "location.north.fill" : "location.north", help: model.follow > 0 ? "Stop following the boat" : "Keep the boat centred", active: model.follow > 0, size: size) { model.cycleFollow() }
            Divider().frame(width: size * 0.6)
            MapButton(symbol: "ruler", help: "Measure a distance and bearing", active: model.measuring, size: size) { model.toggleMeasure() }
            Divider().frame(width: size * 0.6)
            MapButton(symbol: "plus", help: "Zoom in", size: size) { model.zoom(by: 1) }
            MapButton(symbol: "minus", help: "Zoom out", size: size) { model.zoom(by: -1) }
        }
        .padding(4)
        .card(radius: Radius.l)
    }
}

/// What was tapped, on a tablet: a card by the place, as the desktop's popover.
struct InfoCard: View {
    let info: InfoRequest
    @ObservedObject var model: IsomarModel

    var body: some View {
        GeometryReader { g in
            let w: CGFloat = 320
            let x = min(max(info.x + 16, 12), g.size.width - w - 12)
            let y = min(max(info.y - 40, 70), max(70, g.size.height - 360))
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(info.title).font(.system(size: 17, weight: .semibold))
                        if let s = info.subtitle, !s.isEmpty { Text(s).font(.system(size: 13)).foregroundStyle(Color.isomar(.textMuted)) }
                    }
                    Spacer()
                    if let c = info.choice, c.count == 2, c[1] > 1 {
                        Button { isomar_command("info_cycle", -1) } label: { Image(systemName: "chevron.left").frame(width: 32, height: 32) }.buttonStyle(.plain)
                        Text("\(c[0]) of \(c[1])").font(.numeric(12, weight: .medium))
                        Button { isomar_command("info_cycle", 1) } label: { Image(systemName: "chevron.right").frame(width: 32, height: 32) }.buttonStyle(.plain)
                    }
                    Button { model.info = nil; isomar_command("info_closed", 0) } label: { Image(systemName: "xmark.circle.fill").foregroundStyle(Color.isomar(.textMuted)).frame(width: 32, height: 32) }
                        .buttonStyle(.plain).accessibilityLabel("Close")
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(info.rows.enumerated()), id: \.offset) { _, r in
                            HStack(alignment: .firstTextBaseline) {
                                Text(r.first ?? "").font(.system(size: 13)).foregroundStyle(Color.isomar(.textMuted)).frame(width: 100, alignment: .leading)
                                Text(r.dropFirst().first ?? "").font(.system(size: 13)).textSelection(.enabled)
                                Spacer(minLength: 0)
                            }
                        }
                    }
                }
                .frame(maxHeight: 260)
            }
            .padding(14)
            .frame(width: w)
            .card()
            .offset(x: x, y: y)
        }
    }
}

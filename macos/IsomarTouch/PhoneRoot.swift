import SwiftUI
import UIKit

/// The phone layout (design/ia.md, "Phones, foldables and tablets", compact): the chart full screen and
/// edge to edge; the status pill and the action capsule floating at the top; the map controls on the
/// right; the tab bar of the five pages at the bottom, and the footer strip above it. A tab opens the pages
/// in a bottom sheet (half, full) with the same bar at its foot; dragged away, the sheet is gone and no tab
/// is selected. What was tapped and the task sheets come in sheets too. Compare design/screens/iphone-*.png.
struct PhoneRoot: View {
    @ObservedObject var model: IsomarModel
    /// `ISOMAR_SHEET=medium|large` starts with the pages up at that height (for screenshots).
    private static let startSheet = ProcessInfo.processInfo.environment["ISOMAR_SHEET"]
    @State private var detent: PresentationDetent = startSheet == "large" ? .large : .medium
    /// The pages' sheet is presented. It follows `model.sidebarVisible` (the pages wanted: a tab, or the
    /// engine showing a page), except that a sheet hanging from here goes first (`syncPages`).
    @State private var pagesUp = false
    /// The sheet's height, as it reports it: the chart centres above it.
    @State private var sheetHeight: CGFloat = Size.sheetPeek
    /// The running jobs, shown under the status pill when its spinner is tapped. Not a popover: on a phone
    /// that becomes a sheet, which pushed the pages' sheet away for good.
    @State private var jobsOpen = false
    /// A weather route's preview fills the sheet; the pages can be looked at over it, and it comes back.
    @State private var pagesOverRouter = false
    /// Best departure's preview shown over its results, from the strip above them, until "Best departure".
    @State private var previewOverDepartures = false

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .top) {
                TouchChart().ignoresSafeArea()
                if model.alerts.alarm.map({ !$0.dim }) ?? false { AlarmFrame().ignoresSafeArea() }

                // What hangs from the top and what stands on the sheet are laid out apart: when they do not
                // both fit (an alarm, the time control and a sheet at half height), they overlap rather than
                // push the bars off the screen.
                ZStack {
                VStack(spacing: 8) {
                    if underway, let c = model.underway.card {
                        NavigatingCard(card: c, model: model)
                    } else {
                        // The capsule on the right keeps its size; the pill gives way and its words shorten.
                        HStack(alignment: .top, spacing: 8) {
                            StatusPill(model: model, expand: showPages) { withAnimation(.easeOut(duration: 0.15)) { jobsOpen.toggle() } }
                            Spacer(minLength: 8)
                            ActionCapsule(model: model).fixedSize().layoutPriority(1)
                        }
                    }
                    if jobsOpen && !model.window.jobs.isEmpty {
                        HStack { JobsCard(model: model); Spacer(minLength: 0) }
                    }
                    TouchAlerts(model: model)
                    if let h = model.alerts.hint { HintCard(hint: h, model: model) }
                    // Under way the sheet is away: the preview in one line, to use or put away.
                    if underway, let r = model.routerCard { CompactRouterCard(card: r, model: model) }
                    HStack(alignment: .top) {
                        Spacer()
                        TouchControls(model: model)
                    }
                    Spacer(minLength: 0)
                }
                .frame(maxHeight: .infinity, alignment: .top)
                VStack(spacing: 8) {
                    Spacer(minLength: 0)
                    if let t = model.toast { ToastView(text: t) }
                    if let t = model.timeline { PhoneTimeline(timeline: t, model: model) }
                    PhoneFooter(model: model, live: model.live)
                }
                .frame(maxHeight: .infinity, alignment: .bottom)
                }
                .padding(.horizontal, 12)
                .padding(.top, 4)
                .padding(.bottom, 6)
                .padding(.leading, landscapeUnderway(geo) ? 300 : 0)
                // Taller than the space (an alarm and a card under way), it runs down, not up past the top.
                .frame(height: max(0, geo.size.height - covered(geo)), alignment: .top)

                if underway {
                    UnderwayReadouts(model: model, landscape: landscapeUnderway(geo), height: covered(geo))
                } else {
                    // The pages' tabs on the chart, none selected while the sheet is away. The sheet comes up
                    // over this bar with the same bar at its foot, so the tabs seem to stay put.
                    PhoneTabBar(selected: pagesUp ? model.page : nil, pick: pick)
                        .frame(maxHeight: .infinity, alignment: .bottom)
                }
            }
            .sheet(isPresented: Binding(get: { pagesUp }, set: { if !$0 { pagesUp = false; model.sidebarVisible = false } })) {
                PhoneSheet(model: model, detent: $detent, height: $sheetHeight, pick: pick, pagesOverRouter: $pagesOverRouter, previewOverDepartures: $previewOverDepartures)
            }
            // While the pages' sheet is away (under way, or dragged down) what was tapped, the task sheets and
            // the long press's actions hang from here; the pages come up once they are gone.
            .background(EmptyView().sheet(isPresented: Binding(get: { !pagesUp && model.chartShown }, set: { if !$0 { model.closeChartItem() } }), onDismiss: syncPages) { ChartItemSheet(model: model) })
            .phonePresentations(model: model, active: !pagesUp && !model.chartShown, onDismiss: syncPages)
            .foregroundStyle(Color.isomar(.text))
            .onAppear {
                model.sidebarVisible = Self.startSheet == "medium" || Self.startSheet == "large"
                syncPages()
                setInsets(geo)
            }
            .onChange(of: model.sidebarVisible) { _ in syncPages() }
            // The long press's actions have no `onDismiss`: the pages wait for the dialog to have gone.
            .onChange(of: model.menu == nil) { gone in
                if gone { DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { syncPages() } }
            }
            .onChange(of: pagesUp) { _ in setInsets(geo) }
            .onChange(of: sheetHeight) { _ in setInsets(geo) }
            .onChange(of: underway) { _ in syncPages(); setInsets(geo) }
            .onChange(of: geo.size) { _ in setInsets(geo) }
            // A mark, a route or a vessel tapped on the chart: its page in a sheet of its own (a light's or a
            // buoy's details come as `info`, to the same sheet).
            .onChange(of: model.window.picked) { p in
                guard let p, let d = IsomarModel.detail(for: p) else { return }
                model.info = nil
                model.chartDetail = d
            }
            .onChange(of: model.info?.title) { t in if t != nil { model.chartDetail = nil } }
            // Gone on to a page from what was tapped (Weather routing from a route): the pages come up, full.
            .onChange(of: model.detail) { d in
                guard d != nil, !pagesUp else { return }
                detent = .large
                model.sidebarVisible = true
            }
            .onChange(of: model.window.jobs.isEmpty) { none in if none { jobsOpen = false } }
            // A busy job button was pressed: the running jobs show under the pill, the sheet lowered to show them.
            .onChange(of: model.activityShown) { on in
                guard on else { return }
                model.activityShown = false
                if detent == .large { detent = .medium }
                withAnimation(.easeOut(duration: 0.15)) { jobsOpen = true }
            }
            // ISOMAR_SCRIPT's `detent` and `jobs` (Script.swift).
            .onReceive(NotificationCenter.default.publisher(for: .init("isomar.script.detent"))) { n in
                switch n.object as? String {
                case "medium": detent = .medium; model.sidebarVisible = true
                case "large": detent = .large; model.sidebarVisible = true
                default: model.sidebarVisible = false
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: .init("isomar.script.jobs"))) { _ in jobsOpen = true }
            // A new preview shows in the sheet at half height, the chart and the time control above it.
            .onChange(of: model.routerCard != nil) { on in
                previewOverDepartures = false
                guard on else { pagesOverRouter = false; return }
                pagesOverRouter = false
                if detent != .medium { detent = .medium }
                model.sidebarVisible = true
            }
        }
    }

    /// The pill's leading button: the pages, full height; again, away.
    private func showPages() {
        model.info = nil
        if pagesUp && detent == .large {
            model.sidebarVisible = false
            return
        }
        if model.routerCard != nil { pagesOverRouter = true }
        detent = .large
        model.sidebarVisible = true
    }

    /// A tab picked, on the chart or on the sheet: its page, from the top. The sheet comes up half way to
    /// show it (or the weather route previewed); the tab already selected does the same.
    private func pick(_ p: Page) {
        model.info = nil
        pagesOverRouter = model.routerCard != nil
        previewOverDepartures = false
        if !pagesUp { detent = .medium }
        model.showPage(p)
    }

    /// Brings the pages' sheet in line with what is wanted. A sheet hanging from here (what was tapped, a
    /// task sheet) must be gone before another is presented from here: the pages wait for its `onDismiss`.
    private func syncPages() {
        let wanted = !underway && model.sidebarVisible
        if !wanted {
            if pagesUp { pagesUp = false }
        } else if !pagesUp {
            if model.chartShown {
                model.closeChartItem()
            } else if model.sheet == nil && model.menu == nil {
                pagesUp = true
            }
        }
    }

    /// The floating scale and planning row and the credits under it, for the chart's insets.
    private var footerHeight: CGFloat { 52 }
    private var underway: Bool { model.window.underway }

    /// Under way in landscape, for a cockpit mount: the readouts in a column on the left.
    private func landscapeUnderway(_ geo: GeometryProxy) -> Bool { underway && geo.size.width > geo.size.height }

    /// How much of the bottom the sheet or the tab bar, or under way the readouts, cover.
    private func covered(_ geo: GeometryProxy) -> CGFloat {
        guard underway else { return pagesUp ? sheetHeight : PhoneTabBar.height }
        if landscapeUnderway(geo) { return 0 }
        // Two readouts take a row, four two; a full set of instruments nearly half the screen, its figures
        // as large as fit (`InstrumentGrid`).
        let n = model.underway.dashboard.count
        if n <= 2 { return 128 + 8 }
        if n <= 4 { return 2 * 128 + 16 }
        return min(geo.size.height * 0.46, 460)
    }

    /// The chart centres in what the bars and the sheet leave visible. The chart is the whole screen; the
    /// sheet's height leaves out the home indicator's inset, which it covers too.
    private func setInsets(_ geo: GeometryProxy) {
        let top = geo.safeAreaInsets.top + (underway ? 140 : Size.floatingBarHeight + 8)
        isomar_set_insets(top, 0, covered(geo) + geo.safeAreaInsets.bottom + footerHeight, landscapeUnderway(geo) ? 300 : 0)
    }
}

/// The alarm on a touch screen (design/ia.md, "Alarms on touch devices"): a full-width banner with what it
/// is and Snooze and Acknowledge as buttons at least 52 tall. Warnings stay slim strips below it.
struct TouchAlerts: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(spacing: 6) {
            if let a = model.alerts.alarm, !a.dim {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: "exclamationmark.octagon.fill").font(.system(size: 22))
                        Text(a.title).font(.system(size: 16, weight: .bold)).fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 0)
                        if let l = a.countLabel { Text("\(a.count)").font(.numeric(15, weight: .bold)).accessibilityLabel(l) }
                    }
                    HStack(spacing: 10) {
                        if let s = a.snooze {
                            Button { model.command("alert_snooze", a.id) } label: {
                                Text(s).font(.system(size: 16, weight: .semibold)).frame(maxWidth: .infinity, minHeight: 52)
                                    .overlay(Capsule().stroke(Color.isomar(.onAlarm), lineWidth: 1.5))
                            }
                        }
                        if let s = a.acknowledge {
                            Button { model.command("alert_acknowledge", a.id) } label: {
                                Text(s).font(.system(size: 16, weight: .semibold)).foregroundStyle(Color.isomar(.alarmBg)).frame(maxWidth: .infinity, minHeight: 52)
                                    .background(Capsule().fill(Color.isomar(.onAlarm)))
                            }
                        }
                        if let s = a.action { Button(s) { model.command("alert_action", a.id) } }
                    }
                    .buttonStyle(.plain)
                }
                .padding(16)
                .foregroundStyle(Color.isomar(.onAlarm))
                .background(RoundedRectangle(cornerRadius: Radius.l).fill(Color.isomar(.alarmBg)))
            } else if let a = model.alerts.alarm {
                StripView(strip: a, model: model).clipShape(RoundedRectangle(cornerRadius: Radius.m))
            }
            ForEach(model.alerts.warnings) { w in StripView(strip: w, model: model).clipShape(RoundedRectangle(cornerRadius: Radius.m)) }
        }
    }
}

/// Under way on a phone: the instruments as big figures, filling the bottom of the screen held upright and a
/// column on the left in landscape, with Stop; the fewer they are, the bigger (`InstrumentGrid`).
struct UnderwayReadouts: View {
    @ObservedObject var model: IsomarModel
    let landscape: Bool
    let height: CGFloat

    var body: some View {
        if landscape {
            HStack {
                VStack(spacing: 8) {
                    InstrumentGrid(readouts: model.underway.dashboard, largest: 64)
                    stop
                }
                .frame(width: 280)
                .padding(.leading, 12)
                .padding(.vertical, 8)
                Spacer()
            }
        } else {
            VStack {
                Spacer()
                InstrumentGrid(readouts: model.underway.dashboard, largest: 72)
                    .padding(8)
                    .frame(height: height)
                    .background(Color.isomar(.windowBg).ignoresSafeArea(edges: .bottom))
            }
        }
    }

    private var stop: some View {
        Button(role: .destructive) { model.send("underway_set", ["on": false]) } label: {
            Label("Planning layout", systemImage: "rectangle.bottomhalf.inset.filled").frame(maxWidth: .infinity, minHeight: Size.mapControlUnderway)
        }
        .buttonStyle(.bordered)
    }
}

/// Over the chart just above the sheet: the scale bar and the planning pill on small cards, and the credits
/// under them, which a tap lists in full. The desktop's second scale bar and the pointer readout do not fit
/// a phone's width, and a finger has no pointer.
struct PhoneFooter: View {
    let model: IsomarModel
    @ObservedObject var live: LiveText

    var body: some View {
        let f = live.footer
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                ScaleLine(bar: f.scale).fixedSize()
                    .padding(.horizontal, 10).frame(height: 24)
                    .background(Capsule().fill(Color.isomar(.cardBg).opacity(0.9)))
                Spacer(minLength: 4)
                if f.planningOnly {
                    Button { model.sheet = .planningOnly } label: {
                        Pill(symbol: "exclamationmark.triangle.fill", text: "Planning only", tint: .warning).fixedSize()
                            .background(Capsule().fill(Color.isomar(.cardBg).opacity(0.9)))
                    }
                    .buttonStyle(.plain).accessibilityHint(f.planningText)
                }
            }
            if !f.credits.isEmpty {
                Button { Platform.alert("Credits", f.credits.joined(separator: "\n")) } label: {
                    Text(FooterStrip.credit(f.credits)).font(.system(size: 12)).foregroundStyle(Color.isomar(.text))
                        .lineLimit(1).truncationMode(.tail)
                        .padding(.horizontal, 8).padding(.vertical, 3)
                        .background(Capsule().fill(Color.isomar(.cardBg).opacity(0.8)))
                }
                .buttonStyle(.plain).accessibilityHint("Shows who the chart's data comes from")
            }
        }
    }
}

/// A strip under the chart: one scale bar, the credits and the planning pill, for half an open foldable.
struct FooterStrip: View {
    let model: IsomarModel
    @ObservedObject var live: LiveText

    var body: some View {
        let f = live.footer
        HStack(spacing: 10) {
            ScaleLine(bar: f.scale).fixedSize()
            Text(Self.credit(f.credits)).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).lineLimit(1).truncationMode(.tail)
            Spacer(minLength: 4)
            if f.planningOnly {
                Button { model.sheet = .planningOnly } label: { Pill(symbol: "exclamationmark.triangle.fill", text: "Planning only", tint: .warning).fixedSize() }
                    .buttonStyle(.plain).accessibilityHint(f.planningText)
            }
        }
        .padding(.horizontal, 12)
        .frame(maxHeight: .infinity)
        .background(Color.isomar(.windowBg).opacity(0.92))
        .overlay(alignment: .top) { Divider() }
    }

    static func credit(_ c: [String]) -> String {
        guard let first = c.first?.replacingOccurrences(of: "© ", with: "") else { return "" }
        return c.count > 1 ? "© \(first) and \(c.count - 1) more" : "© \(first)"
    }
}

/// The connection light and what the boat is doing (its speed, or what is wrong with the data), floating
/// at the top left, on one line that shortens rather than push the capsule beside it away. Its leading
/// button shows the pages; the spinner, while jobs run, lists them (`jobs`, or a popover without it).
struct StatusPill: View {
    @ObservedObject var model: IsomarModel
    let expand: () -> Void
    var jobs: (() -> Void)? = nil

    /// The speed, and words only when something is wrong ("No GPS fix"): a green light says the rest.
    private var said: [Chip] {
        model.window.link == "ok" ? model.window.subtitle.filter { $0.icon != nil } : model.window.subtitle
    }

    var body: some View {
        HStack(spacing: 8) {
            Button(action: expand) {
                Image(systemName: "sidebar.left").font(.system(size: 17)).frame(width: Size.hitMinTouch, height: Size.hitMinTouch)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Show the pages")
            HStack(spacing: 6) {
                Circle().fill(Color.isomar(model.window.link == "ok" ? .ok : (model.window.link == "stale" ? .warning : .textDisabled))).frame(width: 8, height: 8)
                if said.isEmpty {
                    Text(model.window.title).font(.system(size: 14, weight: .semibold))
                } else {
                    chipsText(said).font(.numeric(14, weight: .semibold)).lineLimit(1).truncationMode(.tail)
                }
            }
            .accessibilityElement(children: .combine)
            .layoutPriority(-1)
            ActivityButton(model: model, onTap: jobs).fixedSize()
        }
        .padding(.trailing, 14)
        .frame(height: Size.floatingBarHeight)
        .background(Capsule().fill(Color.isomar(.cardBg)))
        .overlay(Capsule().stroke(Color.isomar(.border), lineWidth: 1))
        .shadow(color: Theme.isNight ? .clear : .black.opacity(0.15), radius: 6, y: 2)
        .foregroundStyle(Color.isomar(.text))
    }
}

/// Weather, Layers and More, floating at the top right: the desktop's header buttons and primary menu.
struct ActionCapsule: View {
    @ObservedObject var model: IsomarModel
    /// A tablet's capsule starts with New route (a phone's is on the Passage page).
    var newRoute = false
    @State private var layers = false

    var body: some View {
        HStack(spacing: 0) {
            if newRoute { capsuleButton("point.topleft.down.to.point.bottomright.curvepath", "New route") { model.command("new_route") } }
            capsuleButton("cloud.sun", "Weather") { model.showPage(.weather) }
            capsuleButton("square.3.layers.3d", "Layers") { layers = true }
                .popover(isPresented: $layers) { LayersList(model: model).frame(minWidth: 280, minHeight: 360).presentationCompactAdaptation() }
            Menu {
                Picker("Look", selection: Binding(get: { model.mode }, set: { model.setMode($0) })) {
                    Text("Automatic").tag("auto")
                    Text("Day").tag("day")
                    Text("Dusk").tag("dusk")
                    Text("Night").tag("night")
                }
                Button { model.send("underway_set", ["on": !model.window.underway]) } label: {
                    Label(model.window.underway ? "Planning layout" : "Under way layout", systemImage: "gauge.with.dots.needle.67percent")
                }
                Button { model.sheet = .settings } label: { Label("Settings", systemImage: "gearshape") }
                Button { model.sheet = .connections } label: { Label("Connections", systemImage: "point.3.connected.trianglepath.dotted") }
                Button { model.importGPX() } label: { Label("Import GPX…", systemImage: "square.and.arrow.down") }
                Button { model.exportGPX() } label: { Label("Export all as GPX…", systemImage: "square.and.arrow.up") }
                Button { model.sheet = .about } label: { Label("About Isomar", systemImage: "info.circle") }
                Button { model.sheet = .planningOnly } label: { Label("Not for Navigation", systemImage: "exclamationmark.triangle") }
            } label: {
                Image(systemName: "ellipsis").font(.system(size: 17)).frame(width: Size.hitMinTouch, height: Size.hitMinTouch)
            }
            .accessibilityLabel("More")
        }
        .padding(.horizontal, 4)
        .frame(height: Size.floatingBarHeight)
        .background(Capsule().fill(Color.isomar(.cardBg)))
        .overlay(Capsule().stroke(Color.isomar(.border), lineWidth: 1))
        .shadow(color: Theme.isNight ? .clear : .black.opacity(0.15), radius: 6, y: 2)
        .foregroundStyle(Color.isomar(.text))
    }

    private func capsuleButton(_ symbol: String, _ label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol).font(.system(size: 17)).frame(width: Size.hitMinTouch, height: Size.hitMinTouch)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

extension View {
    /// A popover stays a popover on a phone (iOS 16.4 and later) instead of becoming a sheet over the sheet.
    @ViewBuilder func presentationCompactAdaptation() -> some View {
        if #available(iOS 16.4, *) { self.presentationCompactAdaptation(.popover) } else { self }
    }
}

/// The layers and downloads to tick, as the desktop's Layers popover.
struct LayersList: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        List(model.layers) { l in
            Toggle(l.label, isOn: Binding(get: { l.visible }, set: { model.setLayer(l.id, visible: $0) }))
        }
        .listStyle(.plain)
    }
}

/// The round map controls on the right, at touch size: follow the boat and measure. Zooming is a pinch.
struct TouchControls: View {
    @ObservedObject var model: IsomarModel

    private var size: CGFloat { model.window.underway ? Size.mapControlUnderway : Size.mapControlTouch }

    var body: some View {
        VStack(spacing: 2) {
            MapButton(symbol: model.follow > 0 ? "location.north.fill" : "location.north", help: model.follow > 0 ? "Stop following the boat" : "Keep the boat centred", active: model.follow > 0, size: size) { model.cycleFollow() }
            Divider().frame(width: size * 0.6)
            MapButton(symbol: "ruler", help: "Measure a distance and bearing", active: model.measuring, size: size) { model.toggleMeasure() }
        }
        .padding(4)
        .card(radius: Radius.l)
    }
}

/// The bottom sheet: the five pages with their tab bar at its foot, as Settings has (design/ia.md), half or
/// full height; dragged down, it goes (`PhoneRoot` keeps the bar on the chart). What is tapped on the chart
/// opens in a sheet of its own over this one (`ChartItemSheet`), without the tabs.
struct PhoneSheet: View {
    @ObservedObject var model: IsomarModel
    @Binding var detent: PresentationDetent
    @Binding var height: CGFloat
    let pick: (Page) -> Void
    @Binding var pagesOverRouter: Bool
    @Binding var previewOverDepartures: Bool

    var body: some View {
        page(model.page)
        .safeAreaInset(edge: .bottom, spacing: 0) { PhoneTabBar(selected: model.page, onSheet: true, pick: pick) }
        .background(GeometryReader { g in Color.clear.onAppear { height = g.size.height }.onChange(of: g.size.height) { height = $0 } })
        .background(Color.isomar(.sidebarBg).ignoresSafeArea())
        .foregroundStyle(Color.isomar(.text))
        .presentationDetents([.medium, .large], selection: $detent)
        .presentationDragIndicator(.visible)
        .sheetInteractsWithChart()
        // What was tapped on the chart, over this sheet; the task sheets and the long press's actions hang
        // from whichever sheet is on top.
        .sheet(isPresented: Binding(get: { model.chartShown }, set: { if !$0 { model.closeChartItem() } })) { ChartItemSheet(model: model) }
        .phonePresentations(model: model, active: !model.chartShown)
        .preferredColorScheme(IsomarTheme.current == .day ? .light : .dark)
        .touchStyle()
    }

    private func page(_ p: Page) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            if let r = model.routerCard, !pagesOverRouter, !departuresShown || previewOverDepartures {
                VStack(alignment: .leading, spacing: 0) {
                    if departuresShown {
                        Button { previewOverDepartures = false } label: { Label("Best departure", systemImage: "chevron.left").font(.system(size: 15)) }
                            .padding(.horizontal, 16).padding(.bottom, 4)
                    }
                    PhoneRouterSheet(card: r, model: model)
                }
                .padding(.top, 14)
            } else {
                VStack(alignment: .leading, spacing: 0) {
                    if let r = model.routerCard { BackToRouter(card: r) { pagesOverRouter = false; previewOverDepartures = true } }
                    if let d = model.detail {
                        DetailView(detail: d, model: model)
                    } else {
                        Text(p.title).font(.system(size: 22, weight: .bold)).padding(.horizontal, 16).padding(.vertical, 6)
                        switch p {
                        case .passage: PassagePage(model: model)
                        case .weather: WeatherPageView(model: model)
                        case .traffic: TrafficPage(model: model)
                        case .charts: ChartsPageView(model: model)
                        case .instruments: InstrumentsPage(model: model)
                        }
                    }
                }
                .padding(.top, 14)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color.isomar(.sidebarBg).ignoresSafeArea())
    }

    /// Best departure's results are on the page (design/screens/iphone-best-departure.png): picking one
    /// previews it on the chart, which must not hide the list; the preview is a tap away above it.
    private var departuresShown: Bool {
        guard case .routing(let id)? = model.detail else { return false }
        return model.routingMode == .window && model.departures?.routeId == id
    }
}

/// The five pages' tabs, floating at the foot of the chart or of the pages' sheet: one bar in both places, so
/// the sheet seems to rise from behind it. Unlike a TabView's it can have no tab selected (`selected` nil),
/// and the tab already selected still answers.
struct PhoneTabBar: View {
    /// What it covers above the bottom of the safe area.
    static let height: CGFloat = 64

    let selected: Page?
    /// On the sheet, which iOS 26 already sets in from the screen's edges: the bar stays where it was.
    var onSheet = false
    let pick: (Page) -> Void

    private var inset: CGFloat {
        if #available(iOS 26, *), onSheet { return 0 }
        return 12
    }

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Page.allCases) { p in
                let on = p == selected
                Button { pick(p) } label: {
                    VStack(spacing: 3) {
                        Image(systemName: p.symbol).font(.system(size: 19, weight: on ? .semibold : .regular)).frame(height: 24)
                        Text(p.title).font(.system(size: 10, weight: on ? .semibold : .medium)).lineLimit(1).minimumScaleFactor(0.8)
                    }
                    .foregroundStyle(Color.isomar(on ? .accent : .text))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Capsule().fill(Color.isomar(.accent).opacity(on ? 0.14 : 0)))
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(p.title)
                .accessibilityAddTraits(on ? [.isSelected] : [])
            }
        }
        .padding(4)
        .frame(height: Self.height - 6)
        .card(radius: (Self.height - 6) / 2)
        .padding(.horizontal, inset)
        .padding(.top, 6)
    }
}

/// What was tapped on the chart, on a phone: a sheet of its own over the pages' and without their tabs, at
/// half height so the chart stays in sight and another tap there changes what it shows. A light's or a buoy's
/// details (`info`), or the page of a mark, a route or a vessel (`chartDetail`).
struct ChartItemSheet: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        Group {
            if let info = model.info {
                InfoSheet(info: info, model: model)
            } else if let d = model.chartDetail {
                DetailView(detail: d, model: model).environment(\.standaloneDetail, true)
            }
        }
        .padding(.top, 16)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color.isomar(.sidebarBg).ignoresSafeArea())
        .foregroundStyle(Color.isomar(.text))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .sheetInteractsWithChart()
        .phonePresentations(model: model, active: true)
        // Going on to a page (Weather routing from a route) or to the chart (editing it) leaves this sheet.
        .onChange(of: model.detail) { d in if d != nil { model.closeChartItem() } }
        .onChange(of: model.alerts.hint != nil) { editing in if editing { model.closeChartItem() } }
        .preferredColorScheme(IsomarTheme.current == .day ? .light : .dark)
        .touchStyle()
    }
}

extension IsomarModel {
    /// Something tapped on the chart is shown in its own sheet (a phone).
    var chartShown: Bool { info != nil || chartDetail != nil }

    func closeChartItem() {
        if info != nil {
            info = nil
            isomar_command("info_closed", 0)
        }
        chartDetail = nil
    }
}

extension View {
    /// The task sheets and the long press's actions, from the sheet on top (`active`): a sheet already
    /// presenting another cannot present a third. `onDismiss` hears the task sheet gone.
    func phonePresentations(model: IsomarModel, active: Bool, onDismiss: (() -> Void)? = nil) -> some View {
        let entries = model.menu?.sections.flatMap { $0 } ?? []
        return self
            .sheet(item: active ? Binding(get: { model.sheet }, set: { model.sheet = $0 }) : .constant(nil), onDismiss: onDismiss) { s in SheetHost(sheet: s, model: model) }
            .confirmationDialog(model.menu?.title ?? "", isPresented: Binding(get: { active && model.menu != nil }, set: { if !$0 { model.menu = nil } }), titleVisibility: (model.menu?.title ?? "").isEmpty ? .hidden : .visible) {
                ForEach(entries, id: \.i) { e in Button(e.label) { isomar_menu_activate(Int32(e.i)) } }
            }
    }
}

extension View {
    /// The chart under a sheet at its lower heights stays usable (iOS 16.4 and later), and the sheet is
    /// solid: never a material over the chart.
    @ViewBuilder func sheetInteractsWithChart() -> some View {
        if #available(iOS 16.4, *) {
            self.presentationBackgroundInteraction(.enabled(upThrough: .medium))
                .presentationBackground(Color.isomar(.sidebarBg))
                .presentationCornerRadius(Radius.l * 2)
        } else {
            self
        }
    }
}

/// What was tapped, in the sheet: its title and kind, its rows, and the stepper through what lies under the
/// tap when there are several.
struct InfoSheet: View {
    let info: InfoRequest
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(info.title).font(.system(size: 20, weight: .bold))
                    if let s = info.subtitle, !s.isEmpty { Text(s).font(.system(size: 14)).foregroundStyle(Color.isomar(.textMuted)) }
                }
                Spacer()
                if let c = info.choice, c.count == 2, c[1] > 1 {
                    HStack(spacing: 4) {
                        Button { isomar_command("info_cycle", -1) } label: { Image(systemName: "chevron.left").frame(width: Size.hitMinTouch, height: Size.hitMinTouch) }
                        Text("\(c[0]) of \(c[1])").font(.numeric(13, weight: .medium))
                        Button { isomar_command("info_cycle", 1) } label: { Image(systemName: "chevron.right").frame(width: Size.hitMinTouch, height: Size.hitMinTouch) }
                    }
                    .buttonStyle(.plain)
                }
                Button { close() } label: { Image(systemName: "xmark.circle.fill").font(.system(size: 22)).foregroundStyle(Color.isomar(.textMuted)).frame(width: Size.hitMinTouch, height: Size.hitMinTouch) }
                    .buttonStyle(.plain).accessibilityLabel("Close")
            }
            ScrollView {
                BoxedList {
                    ForEach(Array(info.rows.enumerated()), id: \.offset) { i, r in
                        if i > 0 { Divider() }
                        HStack(alignment: .firstTextBaseline) {
                            Text(r.first ?? "").font(.system(size: 14)).foregroundStyle(Color.isomar(.textMuted))
                            Spacer(minLength: 12)
                            Text(r.dropFirst().first ?? "").font(.system(size: 14)).multilineTextAlignment(.trailing).textSelection(.enabled)
                        }
                        .padding(.horizontal, 12).padding(.vertical, 10)
                    }
                }
            }
        }
        .padding(.horizontal, 16).padding(.top, 4)
    }

    private func close() {
        model.info = nil
        isomar_command("info_closed", 0)
    }
}

/// The running jobs under the status pill, each with its progress and Cancel.
struct JobsCard: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        JobsList(model: model)
            .frame(maxWidth: 340)
            .card(radius: Radius.l)
            .transition(.opacity)
    }
}

/// A weather route's preview in the sheet (design/screens/iphone-weather-route.png): what it found, the
/// alternatives to tick, and Statistics, Discard and Use. The time control floats just above the sheet.
struct PhoneRouterSheet: View {
    let card: RouterCard
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 8) {
                        if card.failure { Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Color.isomar(.warning)) }
                        Text(card.title).font(.system(size: 22, weight: .bold))
                    }
                    Text(card.note).font(.system(size: 14)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                    if !card.alternatives.isEmpty {
                        BoxedList {
                            ForEach(Array(card.alternatives.enumerated()), id: \.element.id) { i, a in
                                if i > 0 { Divider().padding(.leading, 54) }
                                alternative(a)
                            }
                        }
                    }
                }
                .padding(.horizontal, 16).padding(.top, 4).padding(.bottom, 12)
            }
            // The buttons stay in sight below the list, at any height of the sheet.
            buttons
                .padding(.horizontal, 16).padding(.vertical, 10)
                .overlay(alignment: .top) { Divider() }
        }
    }

    private var buttons: some View {
        HStack(spacing: 10) {
            Button("Statistics") {
                if let t = model.call("router_stats", as: StatsTable.self) { model.sheet = .stats(t) }
            }
            .buttonStyle(.bordered).disabled(card.alternatives.isEmpty)
            Spacer()
            Button(card.replay ? "Close" : "Discard") { model.send("router_discard") }.buttonStyle(.bordered)
            if !card.replay {
                Button {
                    if let sel = card.alternatives.first(where: { $0.selected }) ?? card.alternatives.first { model.send("router_use", ["i": sel.index]) }
                } label: {
                    Text("Use").font(.system(size: 17, weight: .semibold)).foregroundStyle(Color.isomar(.onAccent))
                        .padding(.horizontal, 24).frame(minHeight: 50)
                        .background(Capsule().fill(Color.isomar(.accentBg)))
                        .contentShape(Capsule())
                }
                .buttonStyle(.plain).disabled(card.alternatives.isEmpty).opacity(card.alternatives.isEmpty ? 0.4 : 1)
            }
        }
        .controlSize(.large)
    }

    private func alternative(_ a: AltRow) -> some View {
        Button { model.send("router_select", ["i": a.index]) } label: {
            HStack(spacing: 12) {
                DashSwatch(token: a.colour, dash: a.dash)
                VStack(alignment: .leading, spacing: 2) {
                    Text(a.title).font(.system(size: 16, weight: a.selected ? .semibold : .regular))
                    chipsText(a.summary).font(.system(size: 13)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                Image(systemName: a.selected ? "checkmark.circle.fill" : "circle").font(.system(size: 22))
                    .foregroundStyle(Color.isomar(a.selected ? .accent : .borderControl))
            }
            .padding(.horizontal, 12).padding(.vertical, 10)
            .frame(minHeight: Size.hitMinTouch)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            if !card.replay {
                Button(role: .destructive) { model.send("router_remove", ["i": a.index]) } label: { Label("Remove this alternative", systemImage: "trash") }
            }
        }
    }
}

/// Over the pages while a weather route is previewed: the way back to it.
struct BackToRouter: View {
    let card: RouterCard
    let back: () -> Void

    var body: some View {
        Button(action: back) {
            HStack(spacing: 10) {
                Image(systemName: "cloud.sun").foregroundStyle(Color.isomar(.accent))
                Text(card.title).font(.system(size: 15, weight: .semibold))
                Spacer()
                Text("Show").font(.system(size: 15)).foregroundStyle(Color.isomar(.accent))
                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Color.isomar(.textMuted))
            }
            .padding(.horizontal, 12).frame(minHeight: Size.hitMinTouch)
            // A tint of the accent: `accentBg` is the solid fill of a selected row, for text in `onAccent`.
            .background(RoundedRectangle(cornerRadius: Radius.m).fill(Color.isomar(.accent).opacity(0.12)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 16).padding(.bottom, 6)
    }
}

/// The time control for the forecast and the router's clock, at a phone's width: play, the time and the
/// speed on one line, the slider under them at full width.
struct PhoneTimeline: View {
    let timeline: TimelineInfo
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 4) {
                button(timeline.playing ? "pause.fill" : "play.fill", timeline.playing ? "Pause" : "Play") { model.send("timeline_play", ["on": !timeline.playing]) }
                button("backward.frame.fill", "Back one hour") { model.send("timeline_step", ["hours": -1]) }
                button("forward.frame.fill", "Forward one hour") { model.send("timeline_step", ["hours": 1]) }
                Text(timeline.label).font(.numeric(15, weight: .semibold)).lineLimit(1).minimumScaleFactor(0.8).padding(.leading, 4)
                Spacer(minLength: 4)
                Menu {
                    ForEach([0.5, 1.0, 2.0, 4.0], id: \.self) { x in Button("\(x, specifier: "%g")×") { model.send("timeline_speed", ["x": x]) } }
                } label: {
                    Text("\(timeline.speed, specifier: "%g")×").font(.numeric(13, weight: .semibold))
                        .padding(.horizontal, 10).frame(height: 30)
                        .background(Capsule().fill(Color.isomar(.hover)))
                }
                .accessibilityLabel("Playback speed")
            }
            Slider(value: Binding(get: { Double(timeline.now) }, set: { model.send("timeline_set", ["t": Int($0)]) }), in: Double(timeline.start)...Double(max(timeline.end, timeline.start + 1)))
                .padding(.horizontal, 6)
            if let age = timeline.age {
                Text(age).font(.system(size: 12)).foregroundStyle(Color.isomar(timeline.stale ? .warning : .textMuted)).padding(.horizontal, 6)
            }
        }
        .padding(.horizontal, 6).padding(.vertical, 6)
        .card(radius: Radius.l)
    }

    private func button(_ symbol: String, _ label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol).font(.system(size: 17)).frame(width: 40, height: Size.hitMinTouch).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// A weather route's preview under way, when the sheet is away: what it is, and Discard and Use.
struct CompactRouterCard: View {
    let card: RouterCard
    @ObservedObject var model: IsomarModel

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "cloud.sun").foregroundStyle(Color.isomar(.accent))
            Text(card.title).font(.system(size: 15, weight: .semibold)).lineLimit(1)
            Spacer(minLength: 4)
            Button(card.replay ? "Close" : "Discard") { model.send("router_discard") }.buttonStyle(.bordered)
            if !card.replay, let sel = card.alternatives.first(where: { $0.selected }) ?? card.alternatives.first {
                Button("Use") { model.send("router_use", ["i": sel.index]) }.buttonStyle(.platformProminent)
            }
        }
        .padding(.leading, 14).padding(.trailing, 8).frame(minHeight: 52)
        .card(radius: Radius.l)
    }
}

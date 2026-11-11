import SwiftUI

#if os(macOS)

struct MainView: View {
    @ObservedObject var model = IsomarModel.shared

    var underway: Bool { model.window.underway }

    /// Under this width the sidebar floats over the chart instead of pushing it aside.
    private let narrowBelow: CGFloat = 1000

    private var chartColumn: some View {
        VStack(spacing: 0) {
            AlertStack(model: model)
            ZStack {
                ChartRepresentable()
                if let a = model.alerts.alarm, !a.dim { AlarmFrame() }
                ChartOverlays(model: model)
            }
            FooterView(model: model, live: model.live)
        }
    }

    var body: some View {
        GeometryReader { g in
            let narrow = g.size.width < narrowBelow
            let show = model.sidebarVisible && !underway
            Group {
                if underway {
                    // Under way the instruments take the sidebar's place, the whole height beside the chart.
                    HStack(spacing: 0) {
                        UnderwayColumn(model: model).frame(width: Size.sidebarWidth * 0.9)
                        Divider()
                        chartColumn
                    }
                } else if narrow {
                    ZStack(alignment: .leading) {
                        chartColumn
                        if show {
                            HStack(spacing: 0) {
                                SidebarView(model: model).frame(width: Size.sidebarWidth * 0.9)
                                Divider()
                            }
                            .shadow(color: .black.opacity(Theme.isNight ? 0 : 0.25), radius: 12)
                            .transition(.move(edge: .leading))
                        }
                    }
                } else {
                    HStack(spacing: 0) {
                        if show {
                            SidebarView(model: model).frame(width: Size.sidebarWidth * 0.9)
                            Divider()
                        }
                        chartColumn
                    }
                }
            }
            .animation(Platform.reduceMotion ? nil : .easeInOut(duration: Timing.panelSlide), value: show)
        }
        .background(Color.isomar(.windowBg))
        .frame(minWidth: 640, minHeight: 440)
        .sheet(item: $model.sheet) { SheetHost(sheet: $0, model: model) }
        .toolbar { MainToolbar(model: model) }
        .navigationTitle("")
    }
}

/// The toolbar: the sidebar switch, the title with its status light, and the actions.
struct MainToolbar: ToolbarContent {
    @ObservedObject var model: IsomarModel

    var body: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            if !model.window.underway {
                Button { model.sidebarVisible.toggle() } label: { Label("Sidebar", systemImage: "sidebar.left") }
                    .help("Show or hide the sidebar")
            }
        }
        ToolbarItem(placement: .principal) {
            TitleView(window: model.window)
        }
        ToolbarItemGroup(placement: .primaryAction) {
            if model.window.underway {
                Button(role: .destructive) { model.send("underway_stop") } label: { Label("Stop", systemImage: "stop.fill") }
                    .help("Stop navigating")
            } else {
                // Words beside the icons: a tester could not find New route among icons alone.
                Button { model.command("new_route") } label: { Label("New route", systemImage: "point.topleft.down.to.point.bottomright.curvepath").labelStyle(.titleAndIcon) }
                    .help("Draw a new route on the chart (⌘N)")
                Button { model.showPage(.weather) } label: { Label("Weather", systemImage: "cloud.sun").labelStyle(.titleAndIcon) }
                    .help("Get the forecast and choose what it shows")
            }
            ActivityButton(model: model)
            LayersMenu(model: model)
            if !model.window.underway {
                Button { model.toggleMeasure() } label: { Label("Measure", systemImage: "ruler").labelStyle(.titleAndIcon) }
                    .help("Measure a distance and bearing (M)")
            }
        }
    }
}

#endif

struct TitleView: View {
    let window: WindowInfo

    var body: some View {
        VStack(spacing: 0) {
            Text(window.title).font(.system(size: 13, weight: .semibold))
            HStack(spacing: 4) {
                Circle().fill(Color.isomar(window.link == "ok" ? .ok : (window.link == "stale" ? .warning : .textDisabled))).frame(width: 7, height: 7)
                chipsText(window.subtitle).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).lineLimit(1)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// The activity indicator: a spinner while jobs run, and what they are: in a popover, or where `onTap`
/// shows them (a phone, where a popover would become a sheet).
struct ActivityButton: View {
    @ObservedObject var model: IsomarModel
    var onTap: (() -> Void)? = nil
    @State private var open = false

    /// The popover, open when pressed or when a busy job button asks for it (`activityShown`); with
    /// `onTap`, the caller shows the jobs and watches `activityShown` itself.
    private var shown: Binding<Bool> {
        Binding(get: { onTap == nil && (open || model.activityShown) }, set: { open = $0; if !$0 { model.activityShown = false } })
    }

    var body: some View {
        Group {
            if !model.window.jobs.isEmpty {
                Button { if let onTap { onTap() } else { open.toggle() } } label: {
                    HStack(spacing: 4) {
                        ProgressView().controlSize(.small)
                        if model.window.jobs.count > 1 { Text("\(model.window.jobs.count)").font(.caption) }
                    }
                }
                .help("Running tasks")
                .popover(isPresented: shown) { JobsList(model: model) }
            } else if model.window.offline {
                Image(systemName: "wifi.slash").help("Offline: no downloads")
            }
        }
    }
}

struct JobsList: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        JobRows(model: model).padding(14).frame(width: 300)
    }
}

/// What is running, with its progress and a way to cancel it: in the activity popover, and under the
/// button that started it.
struct JobRows: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(model.window.jobs) { job in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text(job.title).font(.headline)
                        Spacer()
                        if job.cancellable {
                            Button("Cancel") { model.command("job_cancel", job.id) }.controlSize(.small)
                        }
                    }
                    if let f = job.fraction { ProgressView(value: f) } else { ProgressView().progressViewStyle(.linear) }
                    if !job.detail.isEmpty { Text(job.detail).font(.caption).foregroundStyle(.secondary) }
                }
            }
        }
    }
}

/// The Layers popover: what is on the chart, grouped by what it is.
struct LayersMenu: View {
    @ObservedObject var model: IsomarModel
    @State private var open = false

    private let groups: [(String, [String])] = [
        ("Chart data", ["enc", "bsb", "depth", "soundings", "aids", "graticule"]),
        ("Base map", ["land", "online-land", "online-seamarks", "towns"]),
        ("Overlays", ["nav", "weather"]),
        ("Downloads", ["online-depth"]),
    ]

    var body: some View {
        Button { open.toggle() } label: { Label("Layers", systemImage: "square.3.layers.3d").labelStyle(.titleAndIcon) }
            .help("Choose what the chart shows")
            .popover(isPresented: $open, arrowEdge: .bottom) {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(groups, id: \.0) { title, ids in
                        Text(title).font(.caption).foregroundStyle(Color.isomar(.textMuted)).textCase(.uppercase)
                        ForEach(ids, id: \.self) { id in
                            if let layer = model.layers.first(where: { $0.id == id }) {
                                Toggle(layer.label, isOn: Binding(get: { layer.visible }, set: { model.setLayer(id, visible: $0) }))
                                    .checkboxStyle()
                            }
                        }
                        if title == "Downloads" {
                            Text("Saved on this Mac, so they work offline.").font(.caption2).foregroundStyle(Color.isomar(.textMuted))
                        }
                        if title != "Downloads" { Divider() }
                    }
                }
                .padding(14).frame(width: 300)
            }
    }
}

// MARK: - Alerts

/// The alarm banner pinned under the toolbar, and warnings below it. Hints are cards, never banners.
struct AlertStack: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(spacing: 0) {
            if let a = model.alerts.alarm { StripView(strip: a, model: model) }
            ForEach(model.alerts.warnings) { w in StripView(strip: w, model: model) }
        }
    }
}

struct StripView: View {
    let strip: Strip
    @ObservedObject var model: IsomarModel
    @State private var pulse = false

    private var alarm: Bool { strip.level == "alarm" }

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: alarm ? "exclamationmark.octagon.fill" : "exclamationmark.triangle.fill").font(.system(size: 16))
            Text(strip.title).font(.system(size: 13, weight: .semibold)).lineLimit(2)
            if let l = strip.countLabel {
                // How many alarms there are: an icon and the number, the words in the tooltip.
                HStack(spacing: 4) {
                    Image(systemName: chipSymbol("alarm"))
                    Text("\(strip.count)").font(.numeric(12, weight: .semibold))
                }
                .padding(.horizontal, 8).padding(.vertical, 2)
                .overlay(Capsule().stroke(lineWidth: 1))
                .help(l).accessibilityLabel(l)
            }
            Spacer()
            if let s = strip.snooze { Button(s) { model.command("alert_snooze", strip.id) }.controlSize(.small) }
            if let s = strip.acknowledge { Button(s) { model.command("alert_acknowledge", strip.id) }.controlSize(.small) }
            if let s = strip.action { Button(s) { model.command("alert_action", strip.id) }.controlSize(.small) }
            if let s = strip.dismiss { Button(s) { model.command("alert_dismiss", strip.id) }.controlSize(.small) }
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .foregroundStyle(Color.isomar(strip.dim ? .alarm : (alarm ? .onAlarm : .onWarning)))
        .background(Color.isomar(strip.dim ? .cardBg : (alarm ? .alarmBg : .warningBg)))
        .overlay(alignment: .bottom) { Rectangle().fill(Color.isomar(alarm ? .alarm : .warning)).frame(height: alarm && !strip.dim ? (pulse ? 4 : 1) : 1) }
        .onAppear {
            guard alarm, !strip.dim, !Platform.reduceMotion else { return }
            withAnimation(.easeInOut(duration: Timing.alarmPulse / 2).repeatForever(autoreverses: true)) { pulse = true }
        }
        .accessibilityElement(children: .combine)
    }
}

/// The frame around the chart while an alarm sounds: 4 to 8 points at 1 Hz; a steady 8 with reduced motion.
struct AlarmFrame: View {
    @State private var big = false
    private var still: Bool { Platform.reduceMotion }

    var body: some View {
        RoundedRectangle(cornerRadius: 0)
            .strokeBorder(Color.isomar(.alarm), lineWidth: still ? 8 : (big ? 8 : 4))
            .allowsHitTesting(false)
            .onAppear {
                guard !still else { return }
                withAnimation(.easeInOut(duration: Timing.alarmPulse / 2).repeatForever(autoreverses: true)) { big = true }
            }
    }
}

// MARK: - Over the chart

struct ChartOverlays: View {
    @ObservedObject var model: IsomarModel

    private var underway: Bool { model.window.underway }
    private var controlSize: CGFloat { underway ? Size.mapControlUnderway : Size.mapControl }

    var body: some View {
        ZStack {
            // Top left: the navigating card, or the hint and the route preview.
            VStack(alignment: .leading, spacing: 10) {
                if let h = model.alerts.hint { HintCard(hint: h, model: model) }
                if let r = model.routerCard { RouterCardView(card: r, model: model) }
                Spacer()
            }
            .padding(Space.l)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)

            // Top right: what the forecast says under the pointer.
            if let r = model.weatherReadout {
                VStack { ReadoutCard(readout: r); Spacer() }
                    .padding(Space.l)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                    .allowsHitTesting(false)
            }

            // Bottom: the dashboard when under way, the timeline when there is a forecast, a toast.
            VStack(spacing: 10) {
                Spacer()
                if let t = model.toast { ToastView(text: t) }
                HStack(alignment: .bottom, spacing: 10) {
                    if let t = model.timeline { TimelineBar(timeline: t, model: model) }
                    Spacer(minLength: controlSize + 28)
                }
            }
            .padding(Space.l)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)

            // Right, low: the map controls.
            ControlCluster(model: model, size: controlSize)
                .padding(Space.l)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
        }
    }
}

struct ControlCluster: View {
    @ObservedObject var model: IsomarModel
    let size: CGFloat

    var body: some View {
        VStack(spacing: 2) {
            MapButton(symbol: model.follow > 0 ? "location.fill" : "location", help: model.follow > 0 ? "Stop following the boat" : "Keep the boat centred", active: model.follow > 0, size: size) { model.cycleFollow() }
            Divider().frame(width: size * 0.6)
            MapButton(symbol: "plus", help: "Zoom in", size: size) { model.zoom(by: 1) }
            MapButton(symbol: "minus", help: "Zoom out", size: size) { model.zoom(by: -1) }
        }
        .padding(4)
        .card(radius: Radius.l)
    }
}

struct ToastView: View {
    let text: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "info.circle.fill").foregroundStyle(Color.isomar(.accent))
            Text(text).font(.system(size: 13))
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
        .background(Capsule().fill(Color.isomar(.cardBg)))
        .overlay(Capsule().stroke(Color.isomar(.border)))
        .transition(.opacity)
        .allowsHitTesting(false)
    }
}

struct HintCard: View {
    let hint: Hint
    @ObservedObject var model: IsomarModel

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "pencil.and.outline").foregroundStyle(Color.isomar(.textMuted))
            Text(hint.title).font(.system(size: 13)).foregroundStyle(Color.isomar(.textMuted))
            if let u = hint.undo { Button { model.command("hint_undo") } label: { Image(systemName: "arrow.uturn.backward") }.help(u).buttonStyle(.platformLink) }
            if let f = hint.finish { Button(f) { model.command("hint_finish") }.controlSize(.small) }
            if let c = hint.cancel { Button(c) { model.command("hint_cancel") }.controlSize(.small) }
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .card()
    }
}

struct ReadoutCard: View {
    let readout: WeatherReadout

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(readout.chips, id: \.self) { c in chipsText([c]).font(.numeric(12, weight: .regular)).chipsHelp([c]) }
            Text(readout.time).font(.caption).foregroundStyle(Color.isomar(.textMuted))
        }
        .padding(10).card()
    }
}

// MARK: - Under way

struct NavigatingCard: View {
    let card: NavCard
    @ObservedObject var model: IsomarModel
    /// Over the chart it is a card of its own width; in the under-way column it takes the column's.
    var fill = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Text("\(card.number)").font(.numeric(14)).frame(width: 28, height: 28)
                    .background(Circle().fill(Color.isomar(.route))).foregroundStyle(Color.isomar(.onRoute))
                VStack(alignment: .leading, spacing: 0) {
                    Text("NEXT WAYPOINT").font(.system(size: 10, weight: .semibold)).foregroundStyle(Color.isomar(.textMuted))
                    Text(card.name).font(.system(size: 17, weight: .semibold))
                }
            }
            Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 6) {
                ForEach(0..<2, id: \.self) { row in
                    GridRow {
                        ForEach(card.cells[(row * 3)..<min(row * 3 + 3, card.cells.count)], id: \.label) { c in
                            VStack(alignment: .leading, spacing: 0) {
                                Text(c.label).font(.system(size: 10, weight: .semibold)).foregroundStyle(Color.isomar(.textMuted))
                                HStack(alignment: .firstTextBaseline, spacing: 3) {
                                    Text(c.value).font(.numeric(24))
                                    if !c.unit.isEmpty { Text(c.unit).font(.system(size: 13)).foregroundStyle(Color.isomar(.textMuted)) }
                                }
                            }
                        }
                    }
                }
            }
            HStack(spacing: 8) {
                Button { model.send("underway_skip") } label: { Label("Skip", systemImage: "forward.end.fill").frame(maxWidth: .infinity) }
                    .disabled(!card.canSkip).controlSize(.large)
                Button(role: .destructive) { model.send("underway_stop") } label: { Label("Stop", systemImage: "stop.fill").frame(maxWidth: .infinity) }
                    .controlSize(.large)
            }
        }
        .padding(14).frame(width: fill ? nil : 330).frame(maxWidth: fill ? .infinity : nil, alignment: .leading)
        .card()
    }
}

/// Under way on a screen wide enough for a column beside the chart (a tablet, the Mac, the open Duo): the
/// navigating card, the instruments filling what is left, and the sailing or motoring switch.
struct UnderwayColumn: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let c = model.underway.card { NavigatingCard(card: c, model: model, fill: true) }
            InstrumentGrid(readouts: model.underway.dashboard)
            if let s = model.underway.sail { SailSwitch(sail: s, model: model, compact: true) }
        }
        .padding(Space.m)
        .frame(maxHeight: .infinity)
        .background(Color.isomar(.sidebarBg).ignoresSafeArea())
    }
}

/// The under-way instruments filling the space they are given: the number of columns that makes the figures
/// largest, so a few readouts are big and a full set of instruments smaller. A last row that is not full
/// shares its width out rather than leave a hole.
struct InstrumentGrid: View {
    let readouts: [Readout]
    var gap: CGFloat = 8
    /// The figures stop growing here: four readouts in a tall column would otherwise be a foot high.
    var largest: CGFloat = 110

    /// The columns, the rows and the figures' size for `n` readouts in `size`.
    static func fit(_ n: Int, in size: CGSize, gap: CGFloat) -> (cols: Int, rows: Int, figure: CGFloat) {
        guard n > 0 else { return (1, 0, 0) }
        var best = (cols: 1, rows: n, figure: CGFloat(0))
        for c in 1...n {
            let r = (n + c - 1) / c
            let w = (size.width - gap * CGFloat(c - 1)) / CGFloat(c)
            let h = (size.height - gap * CGFloat(r - 1)) / CGFloat(r)
            // The figure, its label and its note in the height; four or five characters in the width.
            let f = min(h * 0.42, w * 0.27)
            if f > best.figure { best = (c, r, f) }
        }
        return best
    }

    var body: some View {
        GeometryReader { g in
            let fit = Self.fit(readouts.count, in: g.size, gap: gap)
            let figure = min(fit.figure, largest)
            VStack(spacing: gap) {
                ForEach(0..<fit.rows, id: \.self) { r in
                    HStack(spacing: gap) {
                        ForEach(readouts[(r * fit.cols)..<min(r * fit.cols + fit.cols, readouts.count)]) { x in
                            InstrumentTile(readout: x, figure: figure)
                        }
                    }
                }
            }
            .frame(width: g.size.width, height: g.size.height)
        }
    }
}

/// One readout of the under-way instruments, its figure `figure` points high, the rest in proportion.
struct InstrumentTile: View {
    let readout: Readout
    let figure: CGFloat

    var body: some View {
        let small = max(11, figure * 0.24)
        VStack(alignment: .leading, spacing: 0) {
            Text(readout.label.uppercased()).font(.system(size: small, weight: .semibold)).foregroundStyle(Color.isomar(.textMuted)).lineLimit(1)
            Spacer(minLength: 0)
            HStack(alignment: .firstTextBaseline, spacing: 3) {
                Text(readout.value).font(.numeric(figure)).lineLimit(1).minimumScaleFactor(0.4)
                if !readout.unit.isEmpty {
                    Text(readout.unit).font(.system(size: max(11, figure * 0.3))).foregroundStyle(Color.isomar(.textMuted)).lineLimit(1).fixedSize()
                }
            }
            Spacer(minLength: 0)
            if !readout.note.isEmpty {
                chipsText(readout.note).font(.system(size: small)).foregroundStyle(Color.isomar(.textMuted)).lineLimit(1)
            }
        }
        .padding(.horizontal, max(10, figure * 0.22)).padding(.vertical, max(8, figure * 0.16))
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .card(radius: Radius.m)
    }
}

// MARK: - Timeline and the router card

/// One time control for the forecast and for the router's clock.
struct TimelineBar: View {
    let timeline: TimelineInfo
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 10) {
                Button { model.send("timeline_play", ["on": !timeline.playing]) } label: { Image(systemName: timeline.playing ? "pause.fill" : "play.fill") }
                    .buttonStyle(.platformLink).help(Platform.isMac ? (timeline.playing ? "Pause (Space)" : "Play (Space)") : (timeline.playing ? "Pause" : "Play"))
                Button { model.send("timeline_step", ["hours": -1]) } label: { Image(systemName: "backward.frame.fill") }.buttonStyle(.platformLink).help("Back one hour")
                Slider(value: Binding(get: { Double(timeline.now) }, set: { model.send("timeline_set", ["t": Int($0)]) }), in: Double(timeline.start)...Double(max(timeline.end, timeline.start + 1)))
                    .frame(minWidth: 200)
                Button { model.send("timeline_step", ["hours": 1]) } label: { Image(systemName: "forward.frame.fill") }.buttonStyle(.platformLink).help("Forward one hour")
                Text(timeline.label).font(.numeric(12, weight: .regular)).frame(width: 118, alignment: .trailing)
                Menu("\(timeline.speed, specifier: "%g")×") {
                    ForEach([0.5, 1.0, 2.0, 4.0], id: \.self) { x in Button("\(x, specifier: "%g")×") { model.send("timeline_speed", ["x": x]) } }
                }
                .borderlessMenu().fixedSize()
            }
            if let age = timeline.age {
                Text(age).font(.caption).foregroundStyle(Color.isomar(timeline.stale ? .warning : .textMuted))
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .card()
    }
}

struct RouterCardView: View {
    let card: RouterCard
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                if card.failure { Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Color.isomar(.warning)) }
                Text(card.title).font(.system(size: 14, weight: .semibold))
            }
            if card.models.isEmpty {
                Text(card.note).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
            } else {
                // Routed on several forecasts: a row per model (tap one to show its search and wind).
                Divider()
                ForEach(card.models) { m in
                    HStack(spacing: 8) {
                        DashSwatch(token: m.colour, dash: [])
                        VStack(alignment: .leading, spacing: 0) {
                            Text(m.robust ? "\(m.label) · holds up best" : m.label).font(.system(size: 12, weight: m.focused ? .semibold : .regular))
                            chipsText(m.summary).chipsHelp(m.summary).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer()
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { model.send("router_focus_model", ["i": m.index]) }
                    .background(m.focused ? Color.isomar(.hover) : .clear)
                }
                Text(card.verdict.joined(separator: " ")).font(.system(size: 12)).fixedSize(horizontal: false, vertical: true)
                Button("Cross-check the routes…") {
                    if let t = model.call("router_cross", as: StatsTable.self) { model.sheet = .stats(t) }
                }
                .buttonStyle(.platformLink).help("Each model's route sailed through every model's forecast")
            }
            if !card.alternatives.isEmpty && (card.models.isEmpty || card.alternatives.count > 1) {
                Divider()
                ForEach(card.alternatives) { a in
                    HStack(spacing: 8) {
                        DashSwatch(token: a.colour, dash: a.dash)
                        VStack(alignment: .leading, spacing: 0) {
                            Text(a.title).font(.system(size: 12, weight: a.selected ? .semibold : .regular))
                            chipsText(a.summary).chipsHelp(a.summary).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer()
                        if !card.replay {
                            Button { model.send("router_remove", ["i": a.index]) } label: { Image(systemName: "trash") }
                                .buttonStyle(.platformLink).help("Remove this alternative")
                        }
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { model.send("router_select", ["i": a.index]) }
                    .background(a.selected ? Color.isomar(.hover) : .clear)
                }
            }
            HStack {
                Button("Statistics") {
                    if let t = model.call("router_stats", as: StatsTable.self) { model.sheet = .stats(t) }
                }.disabled(card.alternatives.isEmpty)
                Spacer()
                Button(card.replay ? "Close" : "Discard") { model.send("router_discard") }
                if !card.replay {
                    Button("Use this route") {
                        if let sel = card.alternatives.first(where: { $0.selected }) ?? card.alternatives.first { model.send("router_use", ["i": sel.index]) }
                    }
                    .buttonStyle(.platformProminent).disabled(card.alternatives.isEmpty)
                }
            }
            .controlSize(.small)
        }
        .padding(12).frame(width: 340)
        .card()
    }
}

/// A short line in an alternative's colour and dash pattern.
struct DashSwatch: View {
    let token: String
    let dash: [Double]

    var body: some View {
        Path { p in
            p.move(to: CGPoint(x: 0, y: 5))
            p.addLine(to: CGPoint(x: 30, y: 5))
        }
        .stroke(Color.isomar(Tok.byName(token) ?? .route), style: StrokeStyle(lineWidth: 3, lineCap: .round, dash: dash.map { CGFloat($0) }))
        .frame(width: 30, height: 10)
    }
}

// MARK: - Footer

/// The strip along the bottom: scale bars, credits, the planning pill and what is under the pointer.
struct FooterView: View {
    /// For what the footer does; it follows `live`, not the whole model.
    let model: IsomarModel
    @ObservedObject var live: LiveText

    var body: some View {
        let f = live.footer
        HStack(spacing: 14) {
            VStack(alignment: .leading, spacing: 1) {
                ScaleLine(bar: f.scale)
                ScaleLine(bar: f.metric)
            }
            Text(creditText(f.credits)).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).lineLimit(1).truncationMode(.tail)
            Spacer(minLength: 8)
            if f.planningOnly {
                Button { model.sheet = .planningOnly } label: { Pill(symbol: "exclamationmark.triangle.fill", text: "Planning only", tint: .warning) }
                    .buttonStyle(.plain).help(f.planningText)
            }
            chipsText(f.pointer?.chips ?? f.view)
                .font(.numeric(11, weight: .regular)).foregroundStyle(Color.isomar(.text))
                .onTapGesture { if let p = f.pointer { Platform.copy(p.position); model.show(toast: "Position copied") } }
        }
        .padding(.horizontal, 12).frame(height: 30)
        .background(Color.isomar(.windowBg))
        .overlay(alignment: .top) { Divider() }
    }

    private func creditText(_ c: [String]) -> String {
        guard !c.isEmpty else { return "" }
        let first = c[0].replacingOccurrences(of: "© ", with: "")
        return c.count > 1 ? "© \(first) and \(c.count - 1) more" : "© \(first)"
    }
}

struct ScaleLine: View {
    let bar: ScaleBar

    var body: some View {
        HStack(spacing: 4) {
            Path { p in
                p.move(to: CGPoint(x: 0, y: 0)); p.addLine(to: CGPoint(x: 0, y: 4))
                p.addLine(to: CGPoint(x: bar.px, y: 4)); p.addLine(to: CGPoint(x: bar.px, y: 0))
            }
            .stroke(Color.isomar(.ownShip), lineWidth: 1.5).frame(width: bar.px, height: 5)
            Text(bar.label).font(.system(size: 10, weight: .medium)).foregroundStyle(Color.isomar(.text))
        }
        .frame(height: 9)
    }
}

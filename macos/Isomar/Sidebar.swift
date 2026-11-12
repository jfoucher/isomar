import SwiftUI

/// The left sidebar: five pages, one at a time, and the detail a page pushes.
struct SidebarView: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(spacing: 0) {
            PageTabs(model: model)
                .padding(.horizontal, 12).padding(.top, 10).padding(.bottom, 6)
            Group {
                if let d = model.detail {
                    DetailView(detail: d, model: model)
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        Text(model.page.title).font(.system(size: 20, weight: .bold)).padding(.horizontal, 14).padding(.vertical, 6)
                        switch model.page {
                        case .passage: PassagePage(model: model)
                        case .weather: WeatherPageView(model: model)
                        case .traffic: TrafficPage(model: model)
                        case .charts: ChartsPageView(model: model)
                        case .instruments: InstrumentsPage(model: model)
                        }
                    }
                }
            }
        }
        .background(Color.isomar(.sidebarBg))
    }
}

/// The sidebar's pages as icons. On the Mac each is a button of its own, for its tooltip: the segments of a
/// segmented picker show none.
struct PageTabs: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        #if os(macOS)
        HStack(spacing: 2) {
            ForEach(Page.allCases) { p in
                let on = model.page == p
                Button { model.showPage(p) } label: {
                    Image(systemName: p.symbol)
                        .frame(maxWidth: .infinity, minHeight: 22)
                        .foregroundStyle(Color.isomar(on ? .text : .textMuted))
                        .background(RoundedRectangle(cornerRadius: 5).fill(on ? Color.isomar(.cardBg) : .clear).shadow(color: .black.opacity(on ? 0.12 : 0), radius: 1, y: 0.5))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(p.title)
                .accessibilityLabel(p.title)
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: 7).fill(Color.isomar(.hover)))
        #else
        Picker("", selection: Binding(get: { model.page }, set: { model.showPage($0) })) {
            ForEach(Page.allCases) { p in Image(systemName: p.symbol).tag(p) }
        }
        .pickerStyle(.segmented).labelsHidden()
        #endif
    }
}

struct SectionHeader<Trailing: View>: View {
    let title: String
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack {
            Text(title).font(.system(size: 11, weight: .semibold)).foregroundStyle(Color.isomar(.textMuted)).textCase(.uppercase)
            Spacer()
            trailing()
        }
        .padding(.horizontal, 14).padding(.top, 12).padding(.bottom, 4)
    }
}

struct Row<Leading: View, Trailing: View>: View {
    var selected = false
    var dim = false
    let title: String
    var subtitle = ""
    /// Values with icons in place of the subtitle.
    var chips: [Chip] = []
    /// A second line of values (a weather route's departure and arrival).
    var more: [Chip] = []
    @ViewBuilder var leading: () -> Leading
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(spacing: 10) {
            leading().frame(width: 20)
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.system(size: 13, weight: selected ? .semibold : .regular)).lineLimit(1)
                Group {
                    if chips.isEmpty { Text(subtitle) } else { chipsText(chips).chipsHelp(chips) }
                }
                .font(.system(size: 11)).foregroundStyle(selected ? Color.isomar(.onAccent).opacity(0.85) : Color.isomar(.textMuted)).lineLimit(chips.isEmpty ? 1 : 2)
                if !more.isEmpty {
                    chipsText(more).chipsHelp(more)
                        .font(.system(size: 11)).foregroundStyle(selected ? Color.isomar(.onAccent).opacity(0.85) : Color.isomar(.textMuted)).lineLimit(2)
                }
            }
            Spacer(minLength: 4)
            trailing()
        }
        .padding(.horizontal, 10).padding(.vertical, 6)
        .foregroundStyle(selected ? Color.isomar(.onAccent) : Color.isomar(.text))
        .background(RoundedRectangle(cornerRadius: Radius.s).fill(selected ? Color.isomar(.accentBg) : Color.clear))
        .opacity(dim ? 0.5 : 1)
        .padding(.horizontal, 6)
        .contentShape(Rectangle())
    }
}

// MARK: - Passage

struct PassagePage: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if let f = model.passage.following {
                    SectionHeader(title: "Following") { EmptyView() }
                    Row(selected: true, title: f.name, chips: f.detail, leading: { Image(systemName: "location.fill") }) {
                        Button { model.send("underway_stop") } label: { Image(systemName: "stop.fill") }.buttonStyle(.plain).help("Stop navigating")
                    }
                }
                SectionHeader(title: "Routes") {
                    Button { model.command("new_route") } label: { Label("New", systemImage: "point.topleft.down.to.point.bottomright.curvepath") }.controlSize(.small)
                }
                if model.passage.routes.isEmpty { Text(Platform.isMac ? "No routes. Use New, then click on the chart." : "No routes. Use New, then tap the chart.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14) }
                ForEach(model.passage.routes) { r in
                    Row(selected: r.active, dim: !r.visible, title: r.name, chips: r.detail, more: r.planned, leading: { Image(systemName: "point.topleft.down.to.point.bottomright.curvepath") }) {
                        HStack(spacing: 8) {
                            Button { model.send("start_route", ["id": r.id]) } label: { Image(systemName: "play.fill") }.buttonStyle(.plain).help("Follow this route")
                            Image(systemName: "chevron.right").foregroundStyle(.secondary)
                        }
                    }
                    .onTapGesture { model.command("zoom_route", r.id); model.detail = .route(r.id) }
                    .contextMenu {
                        Button("Activate") { model.send("start_route", ["id": r.id]) }
                        Button("Edit") { model.command("edit_route", r.id) }
                        Button("Reverse") { model.command("reverse_route", r.id) }
                        Button(r.visible ? "Hide" : "Show") { model.command("toggle_route", r.id) }
                        Divider()
                        Button("Delete…", role: .destructive) { model.command("delete_route", r.id) }
                    }
                }
                SectionHeader(title: "Marks") { EmptyView() }
                if model.passage.waypoints.isEmpty { Text(Platform.isMac ? "No marks. Right-click the chart and choose Drop waypoint here." : "No marks. Touch and hold the chart and choose Drop waypoint here.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14) }
                ForEach(model.passage.waypoints) { w in
                    Row(title: w.name, subtitle: w.detail, leading: { Image(systemName: "mappin") }) {
                        Menu {
                            Button("Navigate to") { model.command("goto_waypoint", w.id) }
                            Button("Properties") { model.detail = .waypoint(w.id) }
                            Divider()
                            Button("Delete…", role: .destructive) { model.command("delete_waypoint", w.id) }
                        } label: { Image(systemName: "ellipsis") }
                        .borderlessMenu().fixedSize()
                    }
                    .onTapGesture { model.command("zoom_waypoint", w.id) }
                }
                SectionHeader(title: "Tracks") { EmptyView() }
                if model.passage.tracks.isEmpty { Text("Recorded tracks appear here.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14) }
                ForEach(model.passage.tracks) { t in
                    Row(dim: !t.visible, title: t.name, chips: t.detail, leading: { Image(systemName: "point.3.connected.trianglepath.dotted") }) {
                        Menu {
                            Button(t.visible ? "Hide" : "Show") { model.command("toggle_track", t.id) }
                            Divider()
                            Button("Delete…", role: .destructive) { model.command("delete_track", t.id) }
                        } label: { Image(systemName: "ellipsis") }
                        .borderlessMenu().fixedSize()
                    }
                    .onTapGesture { model.command("zoom_track", t.id) }
                }
                HStack {
                    Button { model.command("record_track") } label: { Label(model.passage.recording ? "Stop recording" : "Record track", systemImage: model.passage.recording ? "stop.circle" : "record.circle") }
                    Button { importGPX() } label: { Label("Import GPX…", systemImage: "square.and.arrow.down") }
                }
                .controlSize(.small).padding(14)
            }
        }
    }

    private func importGPX() {
        model.choose(open: true, types: ["gpx"]) { path in
            if let e = model.send("gpx_import", ["path": path]) { IsomarModel.alert("Could not import the file", e) }
        }
    }
}

// MARK: - Details

extension EnvironmentValues {
    /// The detail is in a sheet of its own (a phone's `ChartItemSheet`), not pushed on a page: no way back to
    /// the page, a close button instead, and closing it is `chartDetail`'s, not `detail`'s.
    var standaloneDetail: Bool {
        get { self[StandaloneDetailKey.self] }
        set { self[StandaloneDetailKey.self] = newValue }
    }
}

private struct StandaloneDetailKey: EnvironmentKey { static let defaultValue = false }

extension IsomarModel {
    /// Closes a detail page: the one pushed on a page, or the one in a sheet of its own.
    func closeDetail(standalone: Bool) {
        if standalone { chartDetail = nil } else { detail = nil }
    }
}

struct BackBar: View {
    let title: String
    @ObservedObject var model: IsomarModel
    /// Where back goes, when it is not the page itself, and what to call it.
    var parent: Detail? = nil
    var parentTitle: String? = nil
    /// Makes the title a field that renames what the page shows.
    var rename: ((String) -> Void)? = nil
    @Environment(\.standaloneDetail) private var standalone

    @ViewBuilder private func heading(_ size: CGFloat) -> some View {
        if let rename { TitleField(title: title, size: size, rename: rename) } else { Text(title).font(.system(size: size, weight: .bold)).lineLimit(2) }
    }

    var body: some View {
        if standalone {
            HStack(alignment: .firstTextBaseline) {
                heading(22)
                Spacer()
                Button { model.chartDetail = nil } label: {
                    Image(systemName: "xmark.circle.fill").font(.system(size: 24)).foregroundStyle(Color.isomar(.textMuted)).frame(width: 44, height: 44)
                }
                .buttonStyle(.plain).accessibilityLabel("Close")
            }
            .padding(.leading, 16).padding(.trailing, 6)
        } else {
            HStack {
                Button { model.detail = parent } label: { Label(parentTitle ?? model.page.title, systemImage: "chevron.left") }.buttonStyle(.platformLink)
                Spacer()
            }
            .padding(.horizontal, 12).padding(.vertical, 6)
            heading(20).padding(.horizontal, 14)
        }
    }
}

/// A page's title that is also where to rename it: Return or leaving the field saves, Escape puts it back.
struct TitleField: View {
    let title: String
    let size: CGFloat
    let rename: (String) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            TextField("Name", text: $text)
                .textFieldStyle(.plain).font(.system(size: size, weight: .bold))
                .focused($focused)
                .onSubmit { commit() }
                #if os(macOS)
                .onExitCommand { text = title; focused = false }
                #endif
            if !focused {
                Image(systemName: "pencil").font(.system(size: size * 0.6)).foregroundStyle(Color.isomar(.textMuted))
                    .onTapGesture { focused = true }.accessibilityHidden(true)
            }
        }
        .help("Click to rename")
        .onAppear { text = title }
        .onChange(of: title) { t in if !focused { text = t } }
        .onChange(of: focused) { f in if !f { commit() } }
    }

    private func commit() {
        let t = text.trimmingCharacters(in: .whitespaces)
        if t.isEmpty { text = title } else if t != title { rename(t) }
    }
}

struct DetailView: View {
    let detail: Detail
    @ObservedObject var model: IsomarModel

    var body: some View {
        switch detail {
        case .route(let id): RouteDetail(id: id, model: model)
        case .waypoint(let id): WaypointDetail(id: id, model: model)
        case .vessel(let m): VesselDetail(mmsi: m, model: model)
        case .routing(let id): RoutingPage(id: id, model: model)
        case .performance: PerformancePage(model: model)
        }
    }
}

/// A form over a dictionary the engine described: fields are read and written by key.
struct FieldRow: View {
    let label: String
    @Binding var text: String

    var body: some View {
        HStack {
            Text(label).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted)).frame(width: 96, alignment: .leading)
            TextField("", text: $text).textFieldStyle(.roundedBorder)
        }
    }
}

/// A date and time that may be left unset: a picker with a Clear button, or "Not set" with Set, in
/// place of typing the engine's "YYYY-MM-DD HH:MM" (`Stamp`), which `text` holds ("" when unset).
struct OptionalDateRow: View {
    let label: String
    @Binding var text: String
    var unsetHelp = ""

    var body: some View {
        HStack {
            Text(label).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted)).frame(width: 96, alignment: .leading)
            if let date = Stamp.date(text) {
                DatePicker("", selection: Binding(get: { date }, set: { text = Stamp.text($0) }), displayedComponents: [.date, .hourAndMinute]).labelsHidden()
                Spacer()
                Button { text = "" } label: { Image(systemName: "xmark.circle.fill") }
                    .buttonStyle(.borderless).foregroundStyle(Color.isomar(.textMuted)).help("Clear").accessibilityLabel("Clear \(label)")
            } else {
                Text("Not set").font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted)).help(unsetHelp)
                Spacer()
                // The next whole hour, which the picker then moves.
                Button("Set") {
                    let next = Calendar.current.nextDate(after: Date(), matching: DateComponents(minute: 0), matchingPolicy: .nextTime) ?? Date()
                    text = Stamp.text(next)
                }.controlSize(.small)
            }
        }
    }
}

struct RouteDetail: View {
    let id: Int64
    @ObservedObject var model: IsomarModel
    @Environment(\.standaloneDetail) private var standalone
    /// The route's name: its title, where it is renamed.
    @State private var title = ""
    @State private var speed = ""
    @State private var departure = ""
    @State private var visible = true
    /// The form as last loaded or saved: Save is offered only once it differs.
    @State private var saved = ""
    @State private var legs: [[String: Any]] = []
    @State private var points: [[String: Any]] = []
    /// How many points and how long (`route_get`'s "chips").
    @State private var summary: [Chip] = []
    @State private var loaded = false
    /// What the route was planned with (`route_get`'s "plan"), when it was made by weather routing.
    @State private var plan: [String: Any]?
    /// Made by weather routing (`route_get`'s "weather_route"): not offered weather routing again.
    @State private var weatherRoute = false
    /// What the ensemble check said (`route_get`'s "ensemble").
    @State private var ensemble: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            BackBar(title: title, model: model, rename: { n in
                if let e = model.send("route_rename", ["id": id, "name": n]) { IsomarModel.alert("Could not rename the route", e) } else { title = n }
            })
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    chipsText(summary).chipsHelp(summary).font(.caption).foregroundStyle(Color.isomar(.textMuted))
                    HStack {
                        Button { model.send("start_route", ["id": id]) } label: { Label("Follow", systemImage: "play.fill") }.buttonStyle(.platformProminent)
                        Button { model.command("edit_route", id) } label: { Label("Edit", systemImage: "pencil") }
                        Menu("More") {
                            Button("Reverse") { model.command("reverse_route", id); reload() }
                            Button("Export GPX…") { exportGPX() }
                            Divider()
                            Button("Delete…", role: .destructive) { model.command("delete_route", id); model.closeDetail(standalone: standalone) }
                        }.fixedSize()
                    }
                    Text("Plan a passage").font(.system(size: 13)).padding(.leading, 4)
                    BoxedList {
                        // A route made by weather routing is not routed again.
                        if !weatherRoute {
                            NavRow(symbol: "cloud.sun", title: "Weather routing", subtitle: "The fastest way with the forecast and the boat's polar, and when to leave") { model.showRouting(id) }
                            Divider()
                        }
                        NavRow(symbol: "arrow.triangle.turn.up.right.diamond", title: "Auto route…", subtitle: "Adds waypoints to keep the route in safe water", busy: model.busy("autoroute:\(id)")) { model.job("autoroute:\(id)") { model.sheet = .autoRoute(id) } }
                        Divider()
                        NavRow(symbol: "cloud.sun.rain", title: "Ensemble check", subtitle: "Sails the route through 31 GEFS forecasts for the spread of its arrival and wind; a few minutes' download", busy: model.busy("ensemble:\(id)")) { model.job("ensemble:\(id)") { model.send("route_ensemble", ["id": id]) } }
                    }
                    if !ensemble.isEmpty {
                        GroupBox("Ensemble") {
                            Text(ensemble.joined(separator: "\n")).font(.system(size: 12)).fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    if let plan {
                        GroupBox("Weather routing") {
                            VStack(alignment: .leading, spacing: 6) {
                                let planned = chips(from: plan["summary"])
                                chipsText(planned).chipsHelp(planned).font(.system(size: 12)).fixedSize(horizontal: false, vertical: true)
                                Text(plan["forecast"] as? String ?? "").font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted))
                                if let b = plan["bracket"] as? String, !b.isEmpty {
                                    Text(b).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                                }
                                if plan["edited"] as? Bool == true {
                                    Text("The route has been edited since: this is the passage as it was planned.").font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted))
                                }
                                HStack {
                                    Spacer()
                                    Button("Statistics") {
                                        if let t = model.call("route_plan_stats", ["id": id], as: StatsTable.self) { model.sheet = .stats(t) }
                                    }
                                    Button {
                                        if let ok = model.dict("route_replay", ["id": id])?["ok"] as? Bool, !ok { IsomarModel.alert("Could not play the passage", "This route has no plan to play back.") }
                                    } label: { Label("Play", systemImage: "play.fill") }
                                    .buttonStyle(.platformProminent).help("Watch the boat sail the passage as planned, in the forecast it was planned with")
                                }.controlSize(.small)
                            }.padding(4).frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    GroupBox("Route") {
                        VStack(alignment: .leading, spacing: 8) {
                            FieldRow(label: "Speed (kn)", text: $speed)
                            OptionalDateRow(label: "Departure", text: $departure, unsetHelp: "Not set, weather routing and the ensemble check leave now")
                            Toggle("Show on the chart", isOn: $visible)
                            HStack { Spacer(); Button("Save") { save() }.buttonStyle(.platformProminent).disabled(form == saved) }
                        }.padding(4)
                    }
                    GroupBox("Legs") {
                        VStack(alignment: .leading, spacing: 3) {
                            ForEach(Array(legs.enumerated()), id: \.offset) { _, l in
                                HStack {
                                    Text("\(l["from"] as? String ?? "") → \(l["to"] as? String ?? "")").font(.system(size: 12))
                                    Spacer()
                                    Text(l["text"] as? String ?? "").font(.numeric(12, weight: .regular))
                                }
                            }
                        }.padding(4)
                    }
                }
                .padding(14)
            }
        }
        .onAppear { reload() }
    }

    private func reload() {
        guard let d = model.dict("route_get", ["id": id]) else { model.closeDetail(standalone: standalone); return }
        title = d["name"] as? String ?? ""
        speed = String(format: "%g", d["speed_kn"] as? Double ?? 5)
        departure = d["departure"] as? String ?? ""
        visible = d["visible"] as? Bool ?? true
        saved = Self.form(speed, departure, visible)
        legs = d["legs"] as? [[String: Any]] ?? []
        points = d["points"] as? [[String: Any]] ?? []
        summary = chips(from: d["chips"])
        plan = d["plan"] as? [String: Any]
        weatherRoute = d["weather_route"] as? Bool ?? false
        ensemble = d["ensemble"] as? [String] ?? []
    }

    private static func form(_ speed: String, _ departure: String, _ visible: Bool) -> String { "\(speed)|\(departure)|\(visible)" }
    private var form: String { Self.form(speed, departure, visible) }

    private func save() {
        let args: [String: Any] = ["id": id, "name": title, "speed_kn": Double(speed) ?? 5, "departure": departure, "visible": visible]
        if let e = model.send("route_save", args) { IsomarModel.alert("Could not save the route", e) } else { reload(); model.show(toast: "Route saved") }
    }

    private func exportGPX() {
        let stem = model.callText("route_file_name", ["name": title]).flatMap { $0.trimmingCharacters(in: CharacterSet(charactersIn: "\"")) } ?? "route"
        model.choose(open: false, types: ["gpx"], name: stem + ".gpx") { path in
            if let e = model.send("gpx_export", ["id": id, "path": path]) { IsomarModel.alert("Could not write the file", e) }
        }
    }
}

struct WaypointDetail: View {
    let id: Int64
    @ObservedObject var model: IsomarModel
    @Environment(\.standaloneDetail) private var standalone
    @State private var name = ""
    @State private var lat = ""
    @State private var lon = ""
    @State private var notes = ""
    @State private var radius = ""
    @State private var visible = true

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            BackBar(title: name, model: model)
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Button { model.command("goto_waypoint", id) } label: { Label("Navigate to", systemImage: "location.north.line") }.buttonStyle(.platformProminent)
                        Button("Delete…", role: .destructive) { model.command("delete_waypoint", id); model.closeDetail(standalone: standalone) }
                    }
                    GroupBox("Mark") {
                        VStack(alignment: .leading, spacing: 8) {
                            FieldRow(label: "Name", text: $name)
                            FieldRow(label: "Latitude", text: $lat)
                            FieldRow(label: "Longitude", text: $lon)
                            FieldRow(label: "Arrival (nm)", text: $radius)
                            FieldRow(label: "Notes", text: $notes)
                            Toggle("Show on the chart", isOn: $visible)
                            HStack { Spacer(); Button("Save") { save() }.buttonStyle(.platformProminent) }
                        }.padding(4)
                    }
                }
                .padding(14)
            }
        }
        .onAppear { load() }
    }

    private func load() {
        guard let d = model.dict("waypoint_get", ["id": id]) else { model.closeDetail(standalone: standalone); return }
        name = d["name"] as? String ?? ""
        lat = d["latitude"] as? String ?? ""
        lon = d["longitude"] as? String ?? ""
        notes = d["description"] as? String ?? ""
        radius = String(format: "%g", d["arrival_radius_nm"] as? Double ?? 0.05)
        visible = d["visible"] as? Bool ?? true
    }

    private func save() {
        let args: [String: Any] = ["id": id, "name": name, "latitude": lat, "longitude": lon, "description": notes, "arrival_radius_nm": Double(radius) ?? 0.05, "visible": visible]
        if let e = model.send("waypoint_save", args) { IsomarModel.alert("Could not save the mark", e) } else { load() }
    }
}

struct VesselDetail: View {
    let mmsi: UInt32
    @ObservedObject var model: IsomarModel
    @Environment(\.standaloneDetail) private var standalone
    @State private var title = ""
    @State private var groups: [[String: Any]] = []
    @State private var timer: Timer?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            BackBar(title: title, model: model)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(Array(groups.enumerated()), id: \.offset) { _, g in
                        GroupBox((g["title"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? "Vessel") {
                            VStack(alignment: .leading, spacing: 4) {
                                ForEach(Array((g["rows"] as? [[String]] ?? []).enumerated()), id: \.offset) { _, r in
                                    HStack(alignment: .top) {
                                        Text(r[0]).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted)).frame(width: 110, alignment: .leading)
                                        Text(r[1]).font(.system(size: 12)).textSelection(.enabled)
                                        Spacer()
                                    }
                                }
                            }.padding(4)
                        }
                    }
                }
                .padding(14)
            }
        }
        .onAppear { load(); timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { _ in load() } }
        .onDisappear { timer?.invalidate() }
    }

    private func load() {
        guard let d = model.dict("vessel_get", ["id": Int(mmsi)]) else { model.closeDetail(standalone: standalone); return }
        title = d["title"] as? String ?? ""
        groups = d["groups"] as? [[String: Any]] ?? []
    }
}

// MARK: - Weather, traffic, charts, instruments

struct WeatherPageView: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        let w = model.weather
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                // The model shown, and routed on: each keeps its own forecast.
                if !w.models.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text("Model").font(.system(size: 12)); Spacer()
                            Picker("Model", selection: Binding(get: { w.model }, set: { model.send("weather_show_model", ["model": $0]) })) {
                                ForEach(w.models) { m in Text(m.loaded ? "\(m.label) (loaded)" : m.label).tag(m.id) }
                            }
                            .labelsHidden().fixedSize()
                        }
                        if let m = w.models.first(where: { $0.shown }) {
                            Text(m.about).font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .padding(.horizontal, 14)
                }
                if w.loaded {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(w.modelLabel) loaded").font(.system(size: 13, weight: .semibold))
                        if let r = w.run { Text("Run \(r)").font(.caption).foregroundStyle(Color.isomar(.textMuted)) }
                        if let a = w.from, let b = w.to { Text("\(a) to \(b)").font(.caption).foregroundStyle(Color.isomar(.textMuted)) }
                    }
                    .padding(.horizontal, 14)
                } else {
                    Text("No forecast yet. Get weather for what is on screen, or draw an area on the chart.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
                }
                HStack {
                    // While a download runs: off, with a spinner, and pressing it shows the running jobs.
                    Button { model.job("weather") { if let a = model.call("weather_defaults", ["kind": "view"], as: WeatherArea.self) { model.sheet = .getWeather(a) } } } label: { JobLabel(title: "Get weather…", systemImage: "icloud.and.arrow.down", busy: model.busy("weather")) }
                        .buttonStyle(.platformProminent).modifier(JobBusy(busy: model.busy("weather")))
                    Button { model.send("weather_pick") } label: { Label("Draw an area", systemImage: "rectangle.dashed") }
                        .help("Drag a box on the chart")
                }
                .controlSize(.small).padding(.horizontal, 14)
                SectionHeader(title: "Show") { EmptyView() }
                VStack(alignment: .leading, spacing: 6) {
                    // One field colours the sea, picked like a radio; barbs, wave arrows and isobars go over it.
                    Text("Colour").font(.caption).foregroundStyle(Color.isomar(.textMuted))
                    Picker("Colour", selection: Binding(get: { w.fill }, set: { model.send("weather_fill", ["fill": $0]) })) {
                        Text("None").tag("none")
                        Text("Wind").tag("wind")
                        Text("Waves").tag("waves")
                        Text("Current").tag("current")
                    }
                    .pickerStyle(.segmented).labelsHidden()
                    Toggle("Wind barbs", isOn: Binding(get: { w.barbs }, set: { _ in model.send("weather_toggle", ["name": "barbs"]) }))
                    Toggle("Wave arrows", isOn: Binding(get: { w.waveArrows }, set: { _ in model.send("weather_toggle", ["name": "wave_arrows"]) }))
                    Toggle("Isobars", isOn: Binding(get: { w.pressure }, set: { _ in model.send("weather_toggle", ["name": "pressure"]) }))
                    if let n = w.noWaves, w.fill == "waves" || w.waveArrows {
                        Text(n).font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(.horizontal, 14)
                if let c = w.currents {
                    SectionHeader(title: "Currents") { EmptyView() }
                    VStack(alignment: .leading, spacing: 6) {
                        Text(c.note).font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                        ForEach(Array(c.tracking.enumerated()), id: \.offset) { _, t in
                            Text(t).font(.caption).fixedSize(horizontal: false, vertical: true)
                        }
                        ForEach(Array(c.sources.enumerated()), id: \.offset) { _, s in
                            VStack(alignment: .leading, spacing: 1) {
                                Label(s.label, systemImage: "arrow.forward").font(.system(size: 12))
                                Text(s.detail).font(.caption).foregroundStyle(Color.isomar(.textMuted))
                            }
                        }
                        Button { model.send("currents_fetch") } label: { JobLabel(title: "Get currents for this view", systemImage: "icloud.and.arrow.down", busy: model.busy("currents")) }
                            .controlSize(.small).modifier(JobBusy(busy: model.busy("currents")))
                        Toggle("Current arrows", isOn: Binding(get: { c.arrows }, set: { model.send("currents_arrows", ["on": $0]) }))
                        Toggle("Sail in the current", isOn: Binding(get: { c.routeWith }, set: { model.send("currents_route_with", ["on": $0]) }))
                        ForEach(c.atlases) { a in
                            HStack {
                                VStack(alignment: .leading, spacing: 1) {
                                    Text("Shom atlas: \(a.name)").font(.system(size: 12))
                                    Text(a.detail).font(.caption).foregroundStyle(Color.isomar(.textMuted))
                                }
                                Spacer()
                                if a.installed {
                                    Button { model.send("atlas_remove", ["id": a.id]) } label: { Image(systemName: "trash") }.buttonStyle(.plain).help("Remove")
                                } else {
                                    Button { model.send("atlas_install", ["id": a.id]) } label: { Image(systemName: "icloud.and.arrow.down") }.buttonStyle(.plain).help("Download")
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                }
                if w.loaded {
                    SectionHeader(title: "Forecast") { EmptyView() }
                    Button(role: .destructive) { model.send("weather_clear") } label: { Label("Clear the forecast", systemImage: "trash") }
                        .controlSize(.small).padding(.horizontal, 14)
                    Text(Platform.isMac ? "Move over the chart to read the forecast under the pointer. The time control at the bottom of the chart plays it." : "The time control at the bottom of the chart plays the forecast.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
                }
                if !w.tracking.isEmpty {
                    SectionHeader(title: "Forecasts against your wind") { EmptyView() }
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(w.tracking.enumerated()), id: \.offset) { _, t in
                            VStack(alignment: .leading, spacing: 1) {
                                Text(t.best ? "\(t.label) · closest" : t.label).font(.system(size: 12, weight: t.best ? .semibold : .regular))
                                Text(t.text).font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                }
                SectionHeader(title: "Tides") { EmptyView() }
                Text(Platform.isMac ? "Right-click the chart and choose Tides here." : "Touch and hold the chart and choose Tides here.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
            }
        }
    }
}

struct TrafficPage: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                SectionHeader(title: "\(model.vessels.count) vessels") { EmptyView() }
                if model.vessels.isEmpty { Text("AIS targets appear when a receiver is connected.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14) }
                ForEach(model.vessels) { v in
                    Row(title: v.name, chips: v.detail, leading: { Image(systemName: v.alert ? "exclamationmark.octagon.fill" : "ferry").foregroundStyle(v.alert ? Color.isomar(.alarm) : Color.isomar(.ais)) }) {
                        Button { model.detail = .vessel(v.mmsi) } label: { Image(systemName: "info.circle") }.buttonStyle(.plain).help("Vessel information")
                    }
                    .onTapGesture { model.command("show_vessel", Int64(v.mmsi)) }
                    .contextMenu {
                        if v.alert { Button("Acknowledge collision alert") { model.command("acknowledge", Int64(v.mmsi)) } }
                        Button("Vessel information") { model.detail = .vessel(v.mmsi) }
                    }
                }
            }
        }
    }
}

struct InstrumentsPage: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                SailSwitch(sail: model.sailSwitch, model: model)
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                    ForEach(model.tiles) { t in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(t.title).font(.system(size: 10, weight: .semibold)).foregroundStyle(Color.isomar(.textMuted))
                            Text(t.value).font(.numeric(20)).lineLimit(2).minimumScaleFactor(0.6)
                            Text(t.unit).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted))
                        }
                        .frame(maxWidth: .infinity, minHeight: Size.readoutTile * 0.6, alignment: .leading)
                        .padding(8)
                        .background(RoundedRectangle(cornerRadius: Radius.m).fill(Color.isomar(.viewBg)))
                    }
                }
                SectionHeader(title: "Performance") { EmptyView() }
                Row(title: "Performance and polar", subtitle: "What the boat does in each wind, learnt from the log", leading: { Image(systemName: "sailboat") }) {
                    Image(systemName: "chevron.right").foregroundStyle(Color.isomar(.textMuted))
                }
                .onTapGesture { model.detail = .performance }
                SectionHeader(title: "Anchor watch") { EmptyView() }
                HStack {
                    Button { model.command("anchor_here") } label: { Label("Set here", systemImage: "anchor") }
                    Button { model.command("stop_watching") } label: { Label("Clear", systemImage: "xmark") }
                }.controlSize(.small)
            }
            .padding(12)
        }
    }
}

struct ChartsPageView: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        let c = model.charts
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if c.planningOnly {
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Color.isomar(.warning))
                        Text("Planning only here: no official chart you added covers this area.").font(.caption)
                    }
                    .padding(.horizontal, 14).padding(.vertical, 6)
                }
                SectionHeader(title: "In use here") { EmptyView() }
                if c.inUse.isEmpty {
                    Text(c.vector + c.rasters == 0 ? "No charts loaded." : "None of the loaded charts is drawn here at this zoom.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
                }
                ForEach(c.inUse) { u in
                    VStack(alignment: .leading, spacing: 0) {
                        Text(u.name).font(.system(size: 12))
                        let line = [Chip(icon: nil, text: u.kind, label: u.kind)] + u.chips
                        chipsText(line).chipsHelp(line).font(.system(size: 10)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                    }.padding(.horizontal, 14).padding(.vertical, 2)
                }
                // Where a chart already covers the view, only what was downloaded is listed, to update or remove.
                if c.planningOnly || !c.here.isEmpty {
                    SectionHeader(title: c.planningOnly ? "For this area" : "Downloaded for this area") { EmptyView() }
                }
                if c.planningOnly && c.here.isEmpty { Text("No chart source is known for this region.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14) }
                ForEach(c.here) { s in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            Text(s.rating).font(.system(size: 10, weight: .bold)).padding(.horizontal, 5).overlay(Capsule().stroke(Color.isomar(.textMuted), lineWidth: 1)).help(s.ratingText)
                            Text(s.name).font(.system(size: 13, weight: .semibold))
                        }
                        let line = [Chip(icon: nil, text: s.ratingText, label: s.ratingText)] + s.chips
                        chipsText(line).chipsHelp(line).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                        if !s.note.isEmpty { Text(s.note).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true) }
                        HStack {
                            // While its install runs: off, with a spinner, and pressing it shows the running jobs.
                            Button { act(s) } label: { JobLabel(title: label(s.action), busy: model.busy("chart:\(s.id)")) }.controlSize(.small)
                                .modifier(JobBusy(busy: model.busy("chart:\(s.id)"))).disabled(["on", "installed"].contains(s.action))
                            if s.installed {
                                Button("Remove…") { model.send("charts_uninstall", ["id": s.id]) }.controlSize(.small)
                            }
                        }
                    }
                    .padding(.horizontal, 14).padding(.vertical, 6)
                }
                SectionHeader(title: "Your chart folders") {
                    Button { addFolder() } label: { Label("Add…", systemImage: "folder.badge.plus") }.controlSize(.small)
                }
                ForEach(c.folders, id: \.self) { f in
                    HStack {
                        Image(systemName: "folder")
                        Text(f).font(.system(size: 12)).lineLimit(1).truncationMode(.middle)
                        Spacer()
                        Button { model.send("charts_remove_folder", ["path": f]); model.showPage(.charts) } label: { Image(systemName: "trash") }.buttonStyle(.plain).help("Forget this folder")
                    }
                    .padding(.horizontal, 14).padding(.vertical, 2)
                }
                SectionHeader(title: "Loaded charts") { EmptyView() }
                Text("Vector charts (S-57): \(c.vector)").font(.caption).padding(.horizontal, 14)
                Text("Raster charts (BSB/KAP): \(c.rasters)").font(.caption).padding(.horizontal, 14)
            }
        }
        .onAppear { model.showPage(.charts) }
    }

    private func label(_ a: String) -> String {
        switch a { case "install": return "Install"; case "installing": return "Installing…"; case "installed": return "Installed"; case "update": return "Update"; case "on": return "On"; case "turn_on": return "Turn on"; default: return "Open website" }
    }

    private func act(_ s: SourceRow) {
        switch s.action {
        case "install", "update", "installing": model.job("chart:\(s.id)") { if s.action != "installing" { model.send("charts_install", ["id": s.id]) } }
        case "turn_on": model.send("charts_service_on"); model.showPage(.charts)
        case "website": if let u = URL(string: s.url) { Platform.open(u) }
        default: break
        }
    }

    private func addFolder() {
        model.choose(open: true, directory: true) { path in
            model.send("charts_add_folder", ["path": path])
            model.showPage(.charts)
        }
    }
}

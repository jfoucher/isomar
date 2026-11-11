import Charts
import SwiftUI

/// The sheet over the window that `model.sheet` names.
struct SheetHost: View {
    let sheet: Sheet
    @ObservedObject var model: IsomarModel

    var body: some View {
        content
            .platformStyle()
            #if os(iOS)
            .presentationDragIndicator(.visible)
            #endif
    }

    @ViewBuilder private var content: some View {
        switch sheet {
        case .getWeather(let a): GetWeatherSheet(area: a, model: model)
        case .autoRoute(let id): AutoRouteSheet(id: id, model: model)
        case .stats(let t): StatsSheet(table: t, model: model)
        case .tides(let la, let lo): TidesSheet(lat: la, lon: lo, model: model)
        case .planningOnly: PlanningOnlySheet(model: model)
        case .welcome: WelcomeSheet(model: model)
        case .settings:
            NavigationStack {
                SettingsView()
                    .navigationTitle("Settings")
                    #if os(iOS)
                    .navigationBarTitleDisplayMode(.inline)
                    #endif
                    .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { model.sheet = nil }.buttonStyle(.automatic) } }
            }
        case .connections, .about:
            NavigationStack {
                Group {
                    #if os(iOS)
                    if sheet == .connections { TouchConnections() } else { TouchAbout() }
                    #else
                    ConnectionsView()
                    #endif
                }
                .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { model.sheet = nil }.buttonStyle(.automatic) } }
            }
        }
    }
}

/// The frame of a task sheet: a title, what it does, its form, and the buttons.
struct TaskSheet<Content: View>: View {
    let title: String
    var intro: String = ""
    var action: String = "Done"
    var canAct = true
    var width: CGFloat = 460
    let onAct: () -> Void
    let onCancel: () -> Void
    @ViewBuilder var content: () -> Content

    var body: some View {
        #if os(iOS)
        // A phone's or tablet's sheet: the title and the two buttons in its bar, the form scrolling under them.
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if !intro.isEmpty { Text(intro).font(.system(size: 14)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true) }
                    content()
                }
                .padding(16)
            }
            .background(Color.isomar(.sidebarBg).ignoresSafeArea())
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                // The system's own buttons in the bar: `touchStyle()`'s default squeezed them ("Ca…").
                ToolbarItem(placement: .cancellationAction) { Button("Cancel", action: onCancel).buttonStyle(.automatic) }
                ToolbarItem(placement: .confirmationAction) { Button(action, action: onAct).fontWeight(.semibold).buttonStyle(.automatic).disabled(!canAct) }
            }
        }
        .foregroundStyle(Color.isomar(.text))
        #else
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.system(size: 17, weight: .semibold))
            if !intro.isEmpty { Text(intro).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true) }
            content()
            HStack {
                Spacer()
                Button("Cancel", action: onCancel).keyboardShortcut(.cancelAction)
                Button(action, action: onAct).keyboardShortcut(.defaultAction).buttonStyle(.platformProminent).disabled(!canAct)
            }
        }
        .padding(20).sheetWidth(width)
        .background(Color.isomar(.dialogBg))
        #endif
    }
}

struct LabeledField: View {
    let label: String
    @Binding var text: String
    var width: CGFloat? = nil

    var body: some View {
        HStack {
            Text(label).font(.system(size: 12)).frame(width: 150, alignment: .leading)
            TextField("", text: $text).textFieldStyle(.roundedBorder).frame(width: width)
        }
    }
}

// MARK: - Get weather

struct GetWeatherSheet: View {
    let area: WeatherArea
    @ObservedObject var model: IsomarModel
    @State private var north = ""
    @State private var south = ""
    @State private var west = ""
    @State private var east = ""
    @State private var horizon = 2
    @State private var step = 1
    @State private var groups: Set<String> = []
    @State private var forecastModel = ""
    @State private var estimate = WeatherEstimate(source: "—", run: "—", size: "—", files: "—", missing: "", ok: false)

    var body: some View {
        TaskSheet(title: "Get weather", intro: "Downloads the forecast for the area you choose.", action: "Download", canAct: estimate.ok, onAct: download, onCancel: { model.sheet = nil }) {
            GroupBox("Area") {
                VStack(spacing: 6) {
                    LabeledField(label: "North", text: $north)
                    LabeledField(label: "South", text: $south)
                    LabeledField(label: "West", text: $west)
                    LabeledField(label: "East", text: $east)
                }.padding(4)
            }
            GroupBox("Model") {
                VStack(alignment: .leading, spacing: 4) {
                    Picker("Model", selection: $forecastModel) { ForEach(model.weather.models) { m in Text(m.label).tag(m.id) } }.labelsHidden()
                    if let m = model.weather.models.first(where: { $0.id == forecastModel }) {
                        Text(m.about).font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                    }
                }.padding(4)
            }
            GroupBox("Time and data") {
                VStack(alignment: .leading, spacing: 6) {
                    HStack { Text("Forecast length"); Spacer(); Picker("Forecast length", selection: $horizon) { ForEach(Array(model.weather.horizons.enumerated()), id: \.offset) { Text($1).tag($0) } }.labelsHidden().fixedSize() }
                    HStack { Text("Time step"); Spacer(); Picker("Time step", selection: $step) { ForEach(Array(model.weather.steps.enumerated()), id: \.offset) { Text($1).tag($0) } }.labelsHidden().fixedSize() }
                    ForEach(model.weather.groups) { g in
                        Toggle(g.label, isOn: Binding(get: { groups.contains(g.id) }, set: { on in if on { groups.insert(g.id) } else { groups.remove(g.id) } }))
                    }
                }.padding(4)
            }
            GroupBox("What will be downloaded") {
                VStack(alignment: .leading, spacing: 3) {
                    infoRow("Source", estimate.source)
                    infoRow("Forecast run", estimate.run)
                    infoRow("Download size", estimate.size)
                    infoRow("Files", estimate.files)
                    if !estimate.missing.isEmpty { infoRow("Not available", estimate.missing) }
                }.padding(4)
            }
        }
        .onAppear {
            north = area.north; south = area.south; west = area.west; east = area.east
            forecastModel = model.weather.model
            groups = Set(model.weather.groups.filter { $0.default }.map { $0.id })
            refresh()
        }
        .onChange(of: args as NSDictionary) { _ in refresh() }
    }

    private func infoRow(_ l: String, _ v: String) -> some View {
        HStack { Text(l).foregroundStyle(Color.isomar(.textMuted)).frame(width: 110, alignment: .leading); Text(v).font(.numeric(12, weight: .regular)) }.font(.system(size: 12))
    }

    private var args: [String: Any] {
        ["north": north, "south": south, "west": west, "east": east, "horizon_index": horizon, "step_index": step, "groups": Array(groups).sorted(), "model": forecastModel]
    }

    private func refresh() { if let e = model.call("weather_estimate", args, as: WeatherEstimate.self) { estimate = e } }

    private func download() {
        if let e = model.send("weather_download", args) { IsomarModel.alert("Could not get the weather", e); return }
        model.sheet = nil
    }
}

// MARK: - Auto route

struct AutoRouteSheet: View {
    let id: Int64
    @ObservedObject var model: IsomarModel
    @State private var safety = "5"
    @State private var clearance = "0.15"
    @State private var download = true
    @State private var offline = false

    var body: some View {
        TaskSheet(title: "Auto route", intro: "Adds waypoints between the points of this route so that the legs keep clear of land and of water shallower than the safety depth. Your own points are kept. Depths are from the chart datum: the tide is not added, and this is not for navigation.", action: "Compute", onAct: go, onCancel: { model.sheet = nil }) {
            GroupBox("Safety") {
                VStack(spacing: 6) {
                    LabeledField(label: "Safety depth (m)", text: $safety, width: 80)
                    LabeledField(label: "Keep clear of land by (nm)", text: $clearance, width: 80)
                }.padding(4)
            }
            GroupBox("Data") {
                VStack(alignment: .leading, spacing: 2) {
                    Toggle("Download missing depth data", isOn: $download).disabled(offline)
                    Text("EMODnet, about half a megabyte for each half degree. What is already saved is always used.").font(.caption).foregroundStyle(Color.isomar(.textMuted))
                }.padding(4)
            }
        }
        .onAppear {
            guard let d = model.dict("auto_defaults") else { return }
            safety = String(format: "%g", d["safety_depth"] as? Double ?? 5)
            clearance = String(format: "%g", d["clearance_nm"] as? Double ?? 0.15)
            offline = d["offline"] as? Bool ?? false
            if offline { download = false }
        }
    }

    private func go() {
        model.send("auto_run", ["id": id, "safety_depth": Double(safety) ?? 5, "clearance_nm": Double(clearance) ?? 0.15, "download": download])
        model.sheet = nil
    }
}

// MARK: - Statistics

#if os(iOS)
/// The statistics on a phone or tablet: a sheet with a handle at half or full height, one alternative at a
/// time (picked at the top, the one ticked on the chart first), its sections as the app's boxed lists.
struct StatsSheet: View {
    let table: StatsTable
    @ObservedObject var model: IsomarModel
    @State private var column = 0

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if table.heads.count > 1 { picker }
                    ForEach(Array(table.sections.enumerated()), id: \.offset) { _, s in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(s.title).font(.system(size: 13, weight: .semibold)).foregroundStyle(Color.isomar(.textMuted)).textCase(.uppercase).padding(.leading, 4)
                            BoxedList {
                                ForEach(Array(s.rows.enumerated()), id: \.offset) { i, c in
                                    if i > 0 { Divider().padding(.leading, 12) }
                                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                                        Text(c.label).font(.system(size: 15)).foregroundStyle(Color.isomar(.text))
                                        Spacer(minLength: 8)
                                        Text(column < c.cells.count ? c.cells[column] : "").font(.numeric(15, weight: .medium)).multilineTextAlignment(.trailing)
                                    }
                                    .padding(.horizontal, 12).padding(.vertical, 11)
                                }
                            }
                        }
                    }
                }
                .padding(16)
            }
            .background(Color.isomar(.sidebarBg).ignoresSafeArea())
            .navigationTitle("Route statistics")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { model.sheet = nil }.buttonStyle(.automatic) } }
        }
        .foregroundStyle(Color.isomar(.text))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .onAppear { column = model.routerCard?.alternatives.firstIndex(where: { $0.selected }) ?? 0 }
    }

    /// The alternatives by their names on the router card, when the table is the preview's.
    private func name(_ i: Int) -> String {
        if let alts = model.routerCard?.alternatives, alts.count == table.heads.count { return alts[i].title }
        return table.heads[i]
    }

    /// The alternatives by number, and the one picked by its whole name and its line on the chart.
    private var picker: some View {
        VStack(alignment: .leading, spacing: 8) {
            Picker("Alternative", selection: $column) { ForEach(table.heads.indices, id: \.self) { i in Text(table.heads[i]).tag(i) } }
                .pickerStyle(.segmented)
            HStack(spacing: 10) {
                if let alts = model.routerCard?.alternatives, column < alts.count, alts.count == table.heads.count {
                    DashSwatch(token: alts[column].colour, dash: alts[column].dash)
                }
                Text(name(column)).font(.system(size: 15, weight: .semibold)).fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, 4)
        }
    }
}
#else
struct StatsSheet: View {
    let table: StatsTable
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Route statistics").font(.system(size: 17, weight: .semibold))
            ScrollView {
                Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 5) {
                    GridRow {
                        Text("")
                        ForEach(Array(table.heads.enumerated()), id: \.offset) { _, h in Text(h).font(.system(size: 12, weight: .semibold)) }
                    }
                    ForEach(Array(table.sections.enumerated()), id: \.offset) { _, s in
                        GridRow { Text(s.title).font(.system(size: 12, weight: .bold)).padding(.top, 8).gridCellColumns(table.heads.count + 1) }
                        ForEach(Array(s.rows.enumerated()), id: \.offset) { _, c in
                            GridRow {
                                Text(c.label).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted))
                                ForEach(Array(c.cells.enumerated()), id: \.offset) { _, v in Text(v).font(.numeric(12, weight: .regular)) }
                            }
                        }
                    }
                }
            }
            .frame(height: 440)
            HStack { Spacer(); Button("Close") { model.sheet = nil }.keyboardShortcut(.defaultAction) }
        }
        .padding(20).sheetWidth(220 + CGFloat(table.heads.count) * 150).background(Color.isomar(.dialogBg))
    }
}
#endif

// MARK: - Tides

struct TideCurvePoint: Identifiable { let t: Date; let h: Double; var id: Date { t } }

struct TidesSheet: View {
    let lat: Double
    let lon: Double
    @ObservedObject var model: IsomarModel
    @State private var info: [String: Any]?
    @State private var failed: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let info {
                Text(info["title"] as? String ?? "Tides").font(.system(size: 17, weight: .semibold))
                let pts = (info["curve"] as? [[Double]] ?? []).map { TideCurvePoint(t: Date(timeIntervalSince1970: $0[0]), h: $0[1]) }
                if !pts.isEmpty {
                    Chart {
                        ForEach(pts) { p in LineMark(x: .value("Time", p.t), y: .value("Height (m)", p.h)).foregroundStyle(Color.isomar(.ais)) }
                        RuleMark(x: .value("Now", Date(timeIntervalSince1970: info["now"] as? Double ?? 0))).foregroundStyle(Color.isomar(.route))
                    }
                    .chartYAxisLabel("m")
                    .frame(height: 150)
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 4) {
                        rows(info["station"] as? [[String]] ?? [])
                        let extremes = info["extremes"] as? [[String: Any]] ?? []
                        if !extremes.isEmpty {
                            Text(info["next_title"] as? String ?? "Next").font(.system(size: 12, weight: .bold)).padding(.top, 6)
                            // The next high and low waters: an arrow up or down, the time, the height on the right.
                            ForEach(Array(extremes.enumerated()), id: \.offset) { _, e in
                                let high = e["high"] as? Bool ?? false
                                let name = e["name"] as? String ?? ""
                                HStack {
                                    Image(systemName: chipSymbol(high ? "high" : "low")).frame(width: 16).help(name).accessibilityLabel(name)
                                    Text(e["time"] as? String ?? "").font(.numeric(12, weight: .regular))
                                    Spacer()
                                    Text(e["height"] as? String ?? "").font(.numeric(12, weight: .regular))
                                }
                                .frame(width: 200)
                            }
                        }
                        ForEach(Array((info["groups"] as? [[Any]] ?? []).enumerated()), id: \.offset) { _, g in
                            if let title = g.first as? String, let r = g.last as? [[String]] {
                                if !title.isEmpty { Text(title).font(.system(size: 12, weight: .bold)).padding(.top, 6) }
                                rows(r)
                            }
                        }
                    }
                }.frame(height: 220)
            } else if let failed {
                Text("No tide information").font(.system(size: 17, weight: .semibold))
                Text(failed).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted))
            } else {
                HStack { ProgressView().controlSize(.small); Text("Looking up the tides…") }
            }
            HStack { Spacer(); Button("Close") { model.sheet = nil }.keyboardShortcut(.defaultAction) }
        }
        .padding(20).sheetWidth(440).background(Color.isomar(.dialogBg))
        .onAppear {
            DispatchQueue.global().async {
                let text = String(cString: isomar_tide_info(lat, lon))
                let obj = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [String: Any]
                DispatchQueue.main.async {
                    if let e = obj?["error"] as? String { failed = e } else { info = obj }
                }
            }
        }
    }

    /// Labelled rows: the label muted on the left, the words on the right.
    @ViewBuilder private func rows(_ rows: [[String]]) -> some View {
        ForEach(Array(rows.enumerated()), id: \.offset) { _, r in
            HStack(alignment: .top) {
                Text(r[0]).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted)).frame(width: 90, alignment: .leading)
                Text(r[1]).font(.system(size: 12)).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

// MARK: - Planning only, and the first run

struct PlanningOnlySheet: View {
    @ObservedObject var model: IsomarModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("Planning only", systemImage: "exclamationmark.triangle.fill").font(.system(size: 17, weight: .semibold)).foregroundStyle(Color.isomar(.warning))
            Text("No official chart that you have added covers this part of the map. What you see here is built from open data (OpenStreetMap coastlines, seamarks and EMODnet depths). It is good for planning and not for navigation.")
                .font(.system(size: 13)).fixedSize(horizontal: false, vertical: true)
            Text("Add the electronic charts you have licensed, or install a free official source for your area, on the Charts page.").font(.system(size: 13)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
            HStack {
                Button("Open Charts") { model.sheet = nil; model.showPage(.charts) }
                Spacer()
                Button("Close") { model.sheet = nil }.keyboardShortcut(.defaultAction)
            }
        }
        #if os(iOS)
        // A short sheet of its own colour, not a box in the middle of a tall one.
        .padding(20).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color.isomar(.sidebarBg).ignoresSafeArea())
        .presentationDetents([.height(300), .medium])
        #else
        .padding(20).sheetWidth(440).background(Color.isomar(.dialogBg))
        #endif
    }
}

struct WelcomeSheet: View {
    @ObservedObject var model: IsomarModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Welcome to Isomar").font(.system(size: 22, weight: .bold))
            Text("A chart plotter for sailors. Start with what you have:").font(.system(size: 13)).foregroundStyle(Color.isomar(.textMuted))
            VStack(alignment: .leading, spacing: 10) {
                step("map", "Charts", "Add a folder of charts you own, or install a free source for your area.") { model.sheet = nil; model.showPage(.charts) }
                step("cable.connector", "A GPS or instruments", "Connect over the network, a serial port or a file.") { model.sheet = nil; openWindow(id: "connections") }
                step("cloud.sun", "The forecast", "Download the wind and waves for the area you sail.") { model.sheet = nil; model.showPage(.weather) }
            }
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Color.isomar(.warning))
                Text("Isomar helps you plan and follow a passage. It does not replace official charts, a lookout or your judgement. The open-data map is not for navigation.").font(.system(size: 12))
            }
            HStack { Spacer(); Button("Get started") { model.send("settings_patch", ["first_run_done": true]); model.sheet = nil }.keyboardShortcut(.defaultAction).buttonStyle(.platformProminent) }
        }
        .padding(24).sheetWidth(500).background(Color.isomar(.dialogBg))
    }

    private func step(_ symbol: String, _ title: String, _ text: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: symbol).font(.system(size: 20)).frame(width: 30).foregroundStyle(Color.isomar(.accent))
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(.system(size: 13, weight: .semibold))
                    Text(text).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted))
                }
                Spacer()
                Image(systemName: "chevron.right").foregroundStyle(.secondary)
            }
            .padding(10).card(radius: Radius.m)
        }
        .buttonStyle(.plain)
    }
}

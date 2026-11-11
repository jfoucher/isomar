import SwiftUI

// MARK: - Settings

/// A setting bound to the engine's settings: reads on appear, writes on change.
final class SettingsStore: ObservableObject {
    @Published var values: [String: Any] = [:]
    private let model = IsomarModel.shared

    init() { reload() }

    func reload() { values = model.settings }

    func double(_ key: String, _ fallback: Double = 0) -> Double { (values[key] as? NSNumber)?.doubleValue ?? fallback }
    func string(_ key: String, _ fallback: String = "") -> String { values[key] as? String ?? fallback }
    func bool(_ key: String) -> Bool { values[key] as? Bool ?? false }

    func set(_ key: String, _ v: Any) {
        values[key] = v
        model.send("settings_patch", [key: v])
    }

    func binding(_ key: String, _ fallback: Double = 0) -> Binding<Double> {
        Binding(get: { self.double(key, fallback) }, set: { self.set(key, $0) })
    }

    func binding(_ key: String, text fallback: String = "") -> Binding<String> {
        Binding(get: { self.string(key, fallback) }, set: { self.set(key, $0) })
    }

    func binding(_ key: String) -> Binding<Bool> {
        Binding(get: { self.bool(key) }, set: { self.set(key, $0) })
    }
}

struct SettingsView: View {
    @StateObject private var store = SettingsStore()
    @ObservedObject var model = IsomarModel.shared

    var body: some View {
        TabView {
            general.tabItem { Label("General", systemImage: "gearshape") }
            units.tabItem { Label("Units", systemImage: "ruler") }
            chart.tabItem { Label("Chart", systemImage: "map") }
            alarms.tabItem { Label("Alarms", systemImage: "bell") }
            boat.tabItem { Label("Boat", systemImage: "sailboat") }
            data.tabItem { Label("Data", systemImage: "externaldrive") }
        }
        #if os(macOS)
        .frame(width: 520, height: 520)
        #endif
        .onAppear { store.reload() }
    }

    /// The tiles of the dashboard: the user's choice, or the default four.
    private var dashboard: [String] {
        let d = store.values["dashboard"] as? [String] ?? []
        return d.isEmpty ? ["depth", "sog", "cog", "tws"] : d
    }

    private var general: some View {
        Form {
            Picker("Palette", selection: Binding(get: { model.mode }, set: { model.setMode($0) })) {
                Text("Follow system").tag("auto"); Text("Day").tag("day"); Text("Dusk").tag("dusk"); Text("Night").tag("night")
            }
            Toggle("Switch to the under-way layout when a route starts", isOn: store.binding("underway_auto"))
            #if os(iOS)
            Toggle("Dim the screen at Night", isOn: Binding(get: { Dimmer.enabled }, set: { Dimmer.enabled = $0 }))
            #endif
            Picker("Text size on the chart", selection: Binding(get: { Int(store.double("text_scale", 100)) }, set: { store.set("text_scale", $0) })) {
                ForEach([75, 100, 125, 150, 200], id: \.self) { Text("\($0) %").tag($0) }
            }
            Toggle("Show the dashboard on the chart", isOn: store.binding("dashboard_on_chart"))
            Section("Under-way dashboard") {
                ForEach([("depth", "Depth"), ("sog", "Speed over ground"), ("cog", "Course over ground"), ("heading", "Heading"), ("stw", "Speed through water"), ("awa", "Apparent wind angle"), ("aws", "Apparent wind speed"), ("twa", "True wind angle"), ("tws", "True wind speed"), ("twd", "True wind direction"), ("pilot", "Autopilot"), ("temp", "Water temperature"), ("set", "Set and drift"), ("pos", "Position")], id: \.0) { id, label in
                    Toggle(label, isOn: Binding(get: { dashboard.contains(id) }, set: { on in
                        var d = dashboard
                        if on { if !d.contains(id) { d.append(id) } } else { d.removeAll { $0 == id } }
                        store.set("dashboard", d)
                    }))
                }
            }
        }
        .formStyle(.grouped)
    }

    private var units: some View {
        Form {
            Picker("Speed", selection: store.binding("speed_unit", text: "kn")) { Text("Knots").tag("kn"); Text("km/h").tag("km/h"); Text("m/s").tag("m/s") }
            Picker("Distance", selection: store.binding("distance_unit", text: "nm")) { Text("Nautical miles").tag("nm"); Text("Kilometres").tag("km"); Text("Miles").tag("mi") }
            Picker("Depth", selection: store.binding("depth_unit", text: "m")) { Text("Metres").tag("m"); Text("Feet").tag("ft"); Text("Fathoms").tag("fa") }
            Picker("Bearings", selection: store.binding("bearing_magnetic")) { Text("True").tag(false); Text("Magnetic").tag(true) }
        }
        .formStyle(.grouped)
    }

    private var chart: some View {
        Form {
            Picker("Chart style", selection: store.binding("chart_style", text: "paper")) {
                Text("Paper (INT 1 colours)").tag("paper"); Text("ECDIS (S-52 colours)").tag("ecdis")
            }
            Toggle("Light sectors at their full range", isOn: store.binding("full_sectors"))
            Stepper(value: store.binding("safety_depth", 5), in: 1...50) { Text("Safety contour: \(Int(store.double("safety_depth", 5))) m") }
            Stepper(value: store.binding("vector_minutes", 6), in: 1...30) { Text("Course vector: \(Int(store.double("vector_minutes", 6))) minutes") }
        }
        .formStyle(.grouped)
    }

    private var alarms: some View {
        Form {
            Section("Collisions") {
                Stepper(value: store.binding("ais_cpa_nm", 0.5), in: 0.1...5, step: 0.1) { Text("Closest approach under \(store.double("ais_cpa_nm", 0.5), specifier: "%.1f") nm") }
                Stepper(value: store.binding("ais_tcpa_min", 20), in: 2...60) { Text("Within \(Int(store.double("ais_tcpa_min", 20))) minutes") }
            }
            Section("Anchor and course") {
                Stepper(value: store.binding("anchor_radius_nm", 0.05), in: 0.01...1, step: 0.01) { Text("Anchor circle: \(store.double("anchor_radius_nm", 0.05), specifier: "%.2f") nm") }
                Stepper(value: store.binding("xte_limit_nm", 0.1), in: 0.02...2, step: 0.02) { Text("Off course beyond \(store.double("xte_limit_nm", 0.1), specifier: "%.2f") nm") }
            }
            Section("Depth and wind (0 is off)") {
                Stepper(value: store.binding("shallow_alarm_m", 0), in: 0...30) { Text("Shallow-water alarm under \(Int(store.double("shallow_alarm_m"))) m") }
                Stepper(value: store.binding("wind_limit_kn", 0), in: 0...60) { Text("Wind warning above \(Int(store.double("wind_limit_kn"))) kn") }
            }
        }
        .formStyle(.grouped)
    }

    private var boat: some View {
        Form {
            Stepper(value: store.binding("boat_length_m", 10), in: 3...60, step: 0.5) { Text("Length: \(store.double("boat_length_m", 10), specifier: "%.1f") m") }
            Stepper(value: store.binding("boat_beam_m", 3.4), in: 1...15, step: 0.1) { Text("Beam: \(store.double("boat_beam_m", 3.4), specifier: "%.1f") m") }
            Button("Edit the polar…") { PolarWindow.open() }
            Text("The polar says how fast the boat sails at each wind angle and strength. Weather routing uses it.").font(.caption).foregroundStyle(.secondary)
            Toggle("Record performance while under way", isOn: store.binding("perf_recording"))
            Text("Learns what the boat really does: the instruments every 10 seconds, with the forecast's wind and waves. See Instruments, Performance.").font(.caption).foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
    }

    private var data: some View {
        Form {
            Stepper(value: store.binding("auto_clearance_nm", 0.15), in: 0.05...2, step: 0.05) { Text("Auto route keeps clear by \(store.double("auto_clearance_nm", 0.15), specifier: "%.2f") nm") }
            Stepper(value: store.binding("route_tolerance_nm", 0.1), in: 0.01...2, step: 0.01) { Text("Saved weather route stays within \(store.double("route_tolerance_nm", 0.1), specifier: "%.2f") nm of the path") }
            Text("A larger value gives fewer waypoints. Tacks and gybes are always kept, and legs never cross land the search avoided.").font(.caption).foregroundStyle(.secondary)
            Text("Chart data, tiles and forecasts are kept on this Mac in ~/Library/Application Support/isomar and ~/Library/Caches/isomar.").font(.caption).foregroundStyle(.secondary)
            Button("Show the data folder") { Platform.open(FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("isomar")) }
        }
        .formStyle(.grouped)
    }
}

// MARK: - Connections

struct Connection: Identifiable {
    var id: String
    var name: String
    var kind: String        // tcp, udp, serial, gpsd, signalk, file, simulator
    var host = "localhost"
    var port = "10110"
    var path = ""
    var baud = "4800"
    var url = "http://localhost:3000"
    var rate = "0"
    var enabled = true
    var filterMode = "off"
    var filterItems = ""
    var describe = ""
    var status = ""

    init(id: String = UUID().uuidString.prefix(8).lowercased(), name: String = "", kind: String = "tcp") {
        self.id = id; self.name = name; self.kind = kind
    }

    /// From a row of the engine's `connections` list.
    init?(row: [String: Any]) {
        guard let cfg = row["config"] as? [String: Any], let id = cfg["id"] as? String else { return nil }
        self.init(id: id, name: cfg["name"] as? String ?? "")
        enabled = cfg["enabled"] as? Bool ?? true
        filterMode = cfg["filter_mode"] as? String ?? "off"
        filterItems = cfg["filter_items"] as? String ?? ""
        describe = row["describe"] as? String ?? ""
        status = row["status"] as? String ?? ""
        // The engine's `Kind`, tagged by "type" (crates/isomar-core/src/comm/source.rs).
        let k = cfg["kind"] as? [String: Any] ?? [:]
        let num = { (key: String, d: Any) in "\(k[key] ?? d)" }
        switch k["type"] as? String {
        case "tcp": kind = "tcp"; host = k["host"] as? String ?? ""; port = num("port", 0)
        case "udp": kind = "udp"; port = num("port", 0)
        case "serial": kind = "serial"; path = k["path"] as? String ?? ""; baud = num("baud", 4800)
        case "gpsd": kind = "gpsd"; host = k["host"] as? String ?? ""; port = num("port", 2947)
        case "signal_k": kind = "signalk"; url = k["url"] as? String ?? ""
        case "file": kind = "file"; path = k["path"] as? String ?? ""; rate = num("rate_hz", 0)
        case "simulator": kind = "simulator"
        default: break
        }
    }

    var json: [String: Any] {
        var k: [String: Any]
        switch kind {
        case "udp": k = ["type": "udp", "port": Int(port) ?? 10110]
        case "serial": k = ["type": "serial", "path": path, "baud": Int(baud) ?? 4800]
        case "gpsd": k = ["type": "gpsd", "host": host, "port": Int(port) ?? 2947]
        case "signalk": k = ["type": "signal_k", "url": url]
        case "file": k = ["type": "file", "path": path, "rate_hz": Double(rate) ?? 0]
        case "simulator": k = ["type": "simulator", "lat": 48.0, "lon": -4.5, "cog": 90.0, "sog": 6.0]
        default: k = ["type": "tcp", "host": host, "port": Int(port) ?? 10110]
        }
        return ["id": id, "name": name.isEmpty ? id : name, "kind": k, "enabled": enabled, "filter_mode": filterMode, "filter_items": filterItems]
    }
}

struct ConnectionsView: View {
    @ObservedObject var model = IsomarModel.shared
    @State private var rows: [Connection] = []
    @State private var editing: Connection?
    @State private var timer: Timer?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Table(rows) {
                TableColumn("Name") { c in HStack { Circle().fill(color(c)).frame(width: 8, height: 8); Text(c.name) } }
                TableColumn("Source") { c in Text(c.describe).foregroundStyle(.secondary) }
                TableColumn("Status") { c in Text(c.status).foregroundStyle(.secondary) }
                TableColumn("") { c in
                    HStack {
                        Button("Edit") { editing = c }.controlSize(.small)
                        Button { model.send("connection_remove", ["id": c.id]); load() } label: { Image(systemName: "trash") }.buttonStyle(.platformLink)
                    }
                }.width(100)
            }
            Divider()
            HStack {
                Button { editing = Connection() } label: { Label("Add a connection", systemImage: "plus") }
                Spacer()
                Text("Instruments and GPS send NMEA 0183 over the network, a serial port or from a file.").font(.caption).foregroundStyle(.secondary)
            }
            .padding(10)
        }
        .frame(minWidth: 620, minHeight: 320)
        .sheet(item: $editing) { c in ConnectionEditor(connection: c) { saved in
            if let saved { if let e = model.send("connection_save", saved.json) { IsomarModel.alert("Could not save the connection", e) } }
            editing = nil
            load()
        } }
        .onAppear { load(); timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { _ in load() } }
        .onDisappear { timer?.invalidate() }
    }

    private func color(_ c: Connection) -> Color { c.status.hasPrefix("Connected") ? .green : (c.status.hasPrefix("Disc") ? .red : .gray) }
    private func load() { rows = model.list("connections").compactMap { Connection(row: $0) } }
}

struct ConnectionEditor: View {
    @State var connection: Connection
    let done: (Connection?) -> Void
    @State private var ports: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(connection.name.isEmpty ? "New connection" : connection.name).font(.system(size: 17, weight: .semibold))
            Form {
                TextField("Name", text: $connection.name)
                Picker("Kind", selection: $connection.kind) {
                    Text("TCP client").tag("tcp"); Text("UDP listener").tag("udp")
                    if Platform.isMac { Text("Serial port").tag("serial") }
                    Text("gpsd").tag("gpsd"); Text("Signal K").tag("signalk"); Text("NMEA file").tag("file"); Text("Simulator").tag("simulator")
                }
                switch connection.kind {
                case "tcp", "gpsd":
                    TextField("Host", text: $connection.host)
                    TextField("Port", text: $connection.port)
                case "udp": TextField("Port", text: $connection.port)
                case "serial":
                    Picker("Port", selection: $connection.path) { ForEach(ports, id: \.self) { Text($0).tag($0) } }
                    TextField("Baud rate", text: $connection.baud)
                case "signalk": TextField("URL", text: $connection.url)
                case "file":
                    HStack { TextField("File", text: $connection.path); Button("Choose…") { IsomarModel.shared.choose(open: true) { connection.path = $0 } } }
                    TextField("Lines per second (0 is 20)", text: $connection.rate)
                default: Text("A boat that sails east at 6 knots, with some traffic.").font(.caption).foregroundStyle(.secondary)
                }
                Toggle("Enabled", isOn: $connection.enabled)
                Picker("Filter", selection: $connection.filterMode) { Text("None").tag("off"); Text("Only these sentences").tag("allow"); Text("All but these").tag("deny") }
                if connection.filterMode != "off" { TextField("Sentence types", text: $connection.filterItems) }
            }
            .formStyle(.grouped)
            HStack {
                Spacer()
                Button("Cancel") { done(nil) }.keyboardShortcut(.cancelAction)
                Button("Save") { done(connection) }.keyboardShortcut(.defaultAction).buttonStyle(.platformProminent)
            }
        }
        .padding(20).sheetWidth(480)
        .onAppear {
            ports = (IsomarModel.shared.callText("serial_ports").flatMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String] }) ?? []
            if connection.kind == "serial", connection.path.isEmpty { connection.path = ports.first ?? "" }
        }
    }
}

// MARK: - Data monitor

struct MonitorLine: Identifiable { let id: Int; let time: String; let source: String; let text: String; let ok: Bool }

struct DataMonitorView: View {
    @ObservedObject var model = IsomarModel.shared
    @State private var lines: [MonitorLine] = []
    @State private var paused = false
    @State private var bad = 0
    @State private var seen = 0
    @State private var timer: Timer?

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(lines) { l in
                            HStack(spacing: 8) {
                                Text(l.time).foregroundStyle(.secondary)
                                Text(l.source).foregroundStyle(.secondary).frame(width: 70, alignment: .leading)
                                Text(l.text).foregroundStyle(l.ok ? Color.primary : Color.red)
                            }
                            .font(.system(size: 11, design: .monospaced)).id(l.id)
                        }
                    }
                    .padding(8)
                }
                .onChange(of: lines.count) { _ in if !paused, let last = lines.last { proxy.scrollTo(last.id, anchor: .bottom) } }
            }
            Divider()
            HStack {
                Toggle("Pause", isOn: $paused).toggleStyle(.button)
                Button("Clear") { lines = [] }
                Spacer()
                Text("\(bad) lines were not valid NMEA").font(.caption).foregroundStyle(.secondary)
            }.padding(8)
        }
        .frame(minWidth: 640, minHeight: 360)
        .onAppear { timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { _ in poll() } }
        .onDisappear { timer?.invalidate() }
    }

    private func poll() {
        guard !paused, let d = model.dict("monitor", ["since": 0]), let arr = d["lines"] as? [[String: Any]] else { return }
        bad = d["bad"] as? Int ?? 0
        let total = d["total"] as? Int ?? arr.count
        lines = arr.enumerated().map { MonitorLine(id: total - arr.count + $0.offset, time: $0.element["time"] as? String ?? "", source: $0.element["source"] as? String ?? "", text: $0.element["text"] as? String ?? "", ok: $0.element["ok"] as? Bool ?? true) }
    }
}

// MARK: - The polar editor

/// One polar of the library (`polar_list`).
struct PolarEntry: Decodable, Identifiable, Equatable { let id: String; let name: String; let builtin: Bool; let modified: Bool; let active: Bool; let samples: Int }
struct PolarList: Decodable { let active: String; let polars: [PolarEntry] }

/// The polar editor: which polar of the library (the built-in ones, averaged from published polars, and
/// the user's own), the one in use, and its table and plot. Only the polar in use learns from the log.
struct PolarEditor: View {
    @ObservedObject var model = IsomarModel.shared
    @State private var polars: [PolarEntry] = []
    /// The polar shown.
    @State private var selected = ""
    @State private var tws: [Double] = []
    @State private var twa: [Double] = []
    @State private var speeds: [[Double]] = []
    /// The speeds as loaded or last saved, to know when there are changes.
    @State private var savedSpeeds: [[Double]] = []
    @State private var column = 0
    @State private var status = ""
    /// What the performance log learnt, drawn under the polar.
    @State private var observed: PerfObserved?
    @State private var askName = false
    @State private var newName = ""
    @State private var confirmDelete = false
    @State private var pendingPick: String?

    private var current: PolarEntry? { polars.first { $0.id == selected } }
    private var dirty: Bool { speeds != savedSpeeds }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Picker("Polar", selection: Binding(get: { selected }, set: { pick($0) })) {
                    ForEach(polars) { p in Text(label(p)).tag(p.id) }
                }.frame(maxWidth: 360)
                if let c = current {
                    if c.active {
                        Label("In use", systemImage: "checkmark.circle").font(.caption).foregroundStyle(.secondary)
                            .help("Weather routing sails with it, and the performance log learns into it alone")
                    } else {
                        Button("Use this polar") { activate() }
                    }
                }
                Spacer()
                Menu("More") {
                    Button("Save as a New Polar…") { newName = ""; askName = true }
                    Menu("Set the Speeds From") {
                        ForEach(polars.filter { $0.builtin }) { p in Button(p.name) { setFrom(p.id) } }
                    }
                    Divider()
                    if current?.builtin ?? true {
                        Button("Back to the Published Speeds") { restore() }.disabled(!((current?.modified ?? false) || dirty))
                    } else {
                        Button("Delete This Polar…", role: .destructive) { confirmDelete = true }
                    }
                }.fixedSize()
            }
            HStack(spacing: 0) {
                VStack(alignment: .leading) {
                    Text("Boat speed (knots) by true wind angle and strength").font(.caption).foregroundStyle(.secondary)
                    ScrollView([.horizontal, .vertical]) {
                        Grid(horizontalSpacing: 4, verticalSpacing: 2) {
                            GridRow {
                                Text("TWA").font(.caption.bold())
                                ForEach(Array(tws.enumerated()), id: \.offset) { c, w in
                                    Button("\(w, specifier: "%g") kn") { column = c }.buttonStyle(.plain).font(.caption.bold()).foregroundStyle(c == column ? Color.accentColor : .primary)
                                }
                            }
                            ForEach(Array(twa.enumerated()), id: \.offset) { r, a in
                                GridRow {
                                    Text("\(a, specifier: "%g")°").font(.caption)
                                    ForEach(Array(tws.indices), id: \.self) { c in
                                        TextField("", value: Binding(get: { speeds[r][c] }, set: { speeds[r][c] = max(0, $0) }), format: .number.precision(.fractionLength(0...2)))
                                            .textFieldStyle(.roundedBorder).frame(width: 54).font(.system(size: 11).monospacedDigit())
                                    }
                                }
                            }
                        }.padding(4)
                    }
                    HStack {
                        Button("Import…") { importFile() }
                        if observed != nil { Button("Use the learnt speeds") { useLearnt() }.help("Where the performance log has enough data; the rest stays as it is") }
                        Spacer()
                        Text(status).font(.caption).foregroundStyle(.secondary)
                        Button("Save") { save() }.buttonStyle(.platformProminent)
                    }
                }
                .frame(minWidth: 420)
                Divider().padding(.horizontal, 12)
                VStack(alignment: .leading, spacing: 4) {
                    PolarPlot(tws: tws, twa: twa, speeds: $speeds, column: $column, observed: observed)
                        .frame(minWidth: 320, minHeight: 360)
                    if observed != nil {
                        Text("Dots: what the boat did, from the performance log. Dashed: the polar learnt from it.").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .padding(12)
        .frame(minWidth: 820, minHeight: 480)
        .onAppear {
            reloadList()
            load(polars.first { $0.active }?.id ?? "")
        }
        .alert("Save as a new polar", isPresented: $askName) {
            TextField("Name", text: $newName)
            Button("Save") { saveAs() }
            Button("Cancel", role: .cancel) {}
        } message: { Text("Your polars are listed after the built-in ones. Make it the one in use with “Use this polar”.") }
        .confirmationDialog("Delete \(current?.name ?? "")?", isPresented: $confirmDelete) {
            Button("Delete", role: .destructive) { delete() }
        } message: { Text(deleteMessage) }
        .confirmationDialog("Discard the changes to \(current?.name ?? "")?", isPresented: Binding(get: { pendingPick != nil }, set: { if !$0 { pendingPick = nil } })) {
            Button("Discard", role: .destructive) { if let id = pendingPick { load(id) }; pendingPick = nil }
        } message: { Text("The speeds you changed are not saved.") }
    }

    private func label(_ p: PolarEntry) -> String {
        if p.active { return "\(p.name) — in use" }
        return p.modified ? "\(p.name) — changed" : p.name
    }

    private var deleteMessage: String {
        var m = "The polar is deleted for good."
        if let c = current, c.samples > 0 { m += String(format: " So are the %.1f h of the performance log measured against it.", Double(c.samples) * 10 / 3600) }
        if current?.active == true { m += " The medium cruiser is used instead." }
        return m
    }

    private func reloadList() {
        polars = model.call("polar_list", as: PolarList.self)?.polars ?? []
    }

    /// The picker moved: show that polar, after asking when the one shown has changes.
    private func pick(_ id: String) {
        guard id != selected else { return }
        if dirty { pendingPick = id } else { load(id) }
    }

    private func load(_ id: String) {
        guard let d = model.dict("polar_get", id.isEmpty ? [:] : ["id": id]), d["error"] == nil else { return }
        selected = d["id"] as? String ?? id
        tws = d["tws"] as? [Double] ?? []
        twa = d["twa"] as? [Double] ?? []
        speeds = d["speeds"] as? [[Double]] ?? []
        savedSpeeds = speeds
        column = min(column, max(0, tws.count - 1))
        status = ""
        // The log learns into the polar in use only.
        observed = (d["active"] as? Bool ?? false) ? (model.call("polar_observed", as: PerfObserved?.self) ?? nil) : nil
    }

    private func setFrom(_ id: String) {
        guard let d = model.dict("polar_original", ["id": id]), d["error"] == nil else { return }
        tws = d["tws"] as? [Double] ?? []
        twa = d["twa"] as? [Double] ?? []
        speeds = d["speeds"] as? [[Double]] ?? []
        column = min(column, max(0, tws.count - 1))
        status = "Speeds of the \(polars.first { $0.id == id }?.name ?? id)"
    }

    private func activate() {
        if let e = model.send("polar_activate", ["id": selected]) { IsomarModel.alert("Could not use the polar", e); return }
        reloadList()
        model.toast = "\(current?.name ?? "") is in use: weather routing sails with it and the log learns into it."
        observed = model.call("polar_observed", as: PerfObserved?.self) ?? nil
    }

    private func useLearnt() {
        guard let d = model.dict("polar_with_learnt", ["tws": tws, "twa": twa, "speeds": speeds]) else { return }
        if let e = d["error"] as? String { status = e; return }
        speeds = d["speeds"] as? [[Double]] ?? speeds
        status = "\(d["changed"] as? Int ?? 0) speeds from the log"
    }

    private func importFile() {
        model.choose(open: true, types: ["csv", "pol", "txt"]) { p in
            if let d = model.dict("polar_import", ["path": p]), d["error"] == nil {
                tws = d["tws"] as? [Double] ?? []; twa = d["twa"] as? [Double] ?? []; speeds = d["speeds"] as? [[Double]] ?? []
            } else { IsomarModel.alert("Could not read the polar", model.dict("polar_import", ["path": p])?["error"] as? String ?? "") }
        }
    }

    private func save() {
        if let e = model.send("polar_save", ["id": selected, "tws": tws, "twa": twa, "speeds": speeds]) { IsomarModel.alert("Could not save the polar", e); return }
        savedSpeeds = speeds
        status = current?.active == true ? "Saved" : "Saved, not in use"
        reloadList()
        if current?.active == true { observed = model.call("polar_observed", as: PerfObserved?.self) ?? nil }
    }

    private func saveAs() {
        let name = newName.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        guard let d = model.dict("polar_save_as", ["name": name, "tws": tws, "twa": twa, "speeds": speeds]) else { return }
        if let e = d["error"] as? String { IsomarModel.alert("Could not save the polar", e); return }
        reloadList()
        load(d["id"] as? String ?? selected)
        status = "Saved as \(name)"
    }

    private func restore() {
        if let e = model.send("polar_remove", ["id": selected]) { IsomarModel.alert("Could not restore the polar", e); return }
        reloadList()
        load(selected)
    }

    private func delete() {
        if let e = model.send("polar_remove", ["id": selected]) { IsomarModel.alert("Could not delete the polar", e); return }
        reloadList()
        load(polars.first { $0.active }?.id ?? "")
    }
}

/// Half of a polar diagram: the wind axis is vertical, a curve for each wind strength; drag a point of
/// the selected curve to change the speed there.
struct PolarPlot: View {
    let tws: [Double]
    let twa: [Double]
    @Binding var speeds: [[Double]]
    @Binding var column: Int
    var observed: PerfObserved? = nil

    private var maxSpeed: Double { max(1, speeds.flatMap { $0 }.max() ?? 1) }

    var body: some View {
        GeometryReader { g in
            let c = CGPoint(x: 20, y: g.size.height / 2)
            let radius = min(g.size.width - 40, g.size.height / 2 - 20)
            let scale = radius / maxSpeed
            Canvas { ctx, _ in
                for k in stride(from: 2.0, through: maxSpeed.rounded(.up), by: 2.0) {
                    var ring = Path()
                    ring.addArc(center: c, radius: k * scale, startAngle: .degrees(-90), endAngle: .degrees(90), clockwise: false)
                    ctx.stroke(ring, with: .color(.gray.opacity(0.3)), lineWidth: 0.5)
                }
                for col in tws.indices {
                    var p = Path()
                    for (r, a) in twa.enumerated() {
                        let pt = point(c, a, speeds[r][col], scale)
                        r == 0 ? p.move(to: pt) : p.addLine(to: pt)
                    }
                    ctx.stroke(p, with: .color(col == column ? .accentColor : .gray.opacity(0.5)), lineWidth: col == column ? 2.5 : 1)
                }
                // What the performance log learnt in this wind: its dots, and the learnt polar dashed.
                if column < tws.count, let o = observed, let lc = o.tws.firstIndex(where: { abs($0 - tws[column]) < 0.5 }) {
                    if lc < o.cells.count {
                        for q in o.cells[lc] where q.count > 2 {
                            let pt = point(c, q[0], q[1], scale)
                            ctx.fill(Path(ellipseIn: CGRect(x: pt.x - 2.5, y: pt.y - 2.5, width: 5, height: 5)), with: .color(.primary.opacity(0.15 + 0.55 * q[2])))
                        }
                    }
                    var p = Path()
                    for (r, a) in o.twa.enumerated() where r < o.speeds.count && lc < o.speeds[r].count {
                        let pt = point(c, a, o.speeds[r][lc], scale)
                        r == 0 ? p.move(to: pt) : p.addLine(to: pt)
                    }
                    ctx.stroke(p, with: .color(.primary.opacity(0.7)), style: StrokeStyle(lineWidth: 1.5, dash: [5, 4]))
                }
                if column < tws.count {
                    for (r, a) in twa.enumerated() {
                        let pt = point(c, a, speeds[r][column], scale)
                        ctx.fill(Path(ellipseIn: CGRect(x: pt.x - 4, y: pt.y - 4, width: 8, height: 8)), with: .color(.accentColor))
                    }
                }
            }
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { v in drag(v.location, c, scale) })
        }
    }

    private func point(_ c: CGPoint, _ angle: Double, _ speed: Double, _ scale: CGFloat) -> CGPoint {
        let a = angle * .pi / 180
        return CGPoint(x: c.x + CGFloat(sin(a) * speed) * scale, y: c.y - CGFloat(cos(a) * speed) * scale)
    }

    private func drag(_ p: CGPoint, _ c: CGPoint, _ scale: CGFloat) {
        guard column < tws.count, !twa.isEmpty else { return }
        // The nearest point of the selected curve moves to the pointer's distance from the centre, along its own angle.
        let nearest = twa.indices.min { point(c, twa[$0], speeds[$0][column], scale).distance(to: p) < point(c, twa[$1], speeds[$1][column], scale).distance(to: p) }!
        let a = twa[nearest] * .pi / 180
        let along = ((p.x - c.x) * CGFloat(sin(a)) - (p.y - c.y) * CGFloat(cos(a))) / scale
        speeds[nearest][column] = max(0, (Double(along) * 20).rounded() / 20)
    }
}

extension CGPoint { func distance(to o: CGPoint) -> CGFloat { hypot(x - o.x, y - o.y) } }

enum PolarWindow {
    static func open() { NotificationCenter.default.post(name: .init("isomar.openPolar"), object: nil) }
}

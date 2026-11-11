import SwiftUI

/// Weather routing, pushed in Passage from a route: one place for "leave at this time" and "find the best
/// time", which share the passage, the forecast and the boat. A fixed departure runs the weather route and the
/// preview card on the chart follows. A window of departures runs the best-departure search; its results fill
/// the foot of the page, and picking one previews it on the chart.
struct RoutingPage: View {
    let id: Int64
    @ObservedObject var model: IsomarModel
    @State private var name = ""
    /// Where the route goes and how far: "Brest to Lorient  [distance] 68.2 nm".
    @State private var summary: [Chip] = []
    @State private var leave = Date()
    @State private var earliest = Date()
    @State private var days = 3.0
    @State private var stepHours = 3.0
    @State private var steps: [Double] = [1, 3, 6]
    @State private var weights: [String: Double] = [:]
    @State private var presets: [(String, [String: Double])] = []
    @State private var preset = -1
    @State private var maxWeight = 4.0
    @State private var perf = 100.0
    @State private var engine = false
    @State private var motor = 5.0
    @State private var maxWind = 35.0
    /// The polars of the library and the one in use, which routing sails with (`Settings::polar`).
    @State private var polars: [PolarEntry] = []
    @State private var polar = ""
    /// The forecast models to route on: several are each routed on and compared.
    @State private var forecasts: Set<String> = []
    /// Departures ranked by the worst forecast, not the average.
    @State private var cautious = false
    /// Sail in the current, and what covers the passage.
    @State private var currents = true
    @State private var currentsDetail = ""

    private let rows: [(String, String, String)] = [
        ("time", "Time at sea", "Shorter passages"),
        ("upwind", "Upwind", "Hours close-hauled"),
        ("engine", "Engine", "Hours motoring"),
        ("strong_wind", "Strong wind", "Hours in wind at or above the limit below"),
        ("gusts", "Gusts", "Hours with gusts at or above the limit below"),
        ("waves", "Rough sea", "Hours with waves at or above the height below"),
        ("night", "Night", "Hours in the dark"),
        ("manoeuvres", "Tacks and gybes", "Each counts as half an hour at the first mark"),
        ("waiting", "Leaving soon", "Hours waiting for the departure; later forecasts are less sure"),
        ("holding", "Waiting for the stream", "Hours stemming a foul stream off a headland until it turns"),
        ("wind_against_tide", "Wind against tide", "Hours of fresh wind against a strong stream: a short, steep sea"),
    ]

    private var mode: RoutingMode { model.routingMode }
    private var results: DeparturesInfo? { model.departures.flatMap { $0.routeId == id ? $0 : nil } }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            BackBar(title: "Weather routing", model: model, parent: .route(id), parentTitle: "Route")
            ScrollViewReader { scroll in
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 8) {
                        Image(systemName: "point.topleft.down.to.point.bottomright.curvepath").frame(width: 20)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(name).font(.system(size: 13, weight: .semibold)).lineLimit(1)
                            chipsText(summary).chipsHelp(summary).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).lineLimit(2)
                        }
                    }
                    Picker("", selection: $model.routingMode) {
                        Text("At a time").tag(RoutingMode.fixed)
                        Text("Best time").tag(RoutingMode.window)
                    }
                    .pickerStyle(.segmented).labelsHidden()

                    GroupBox {
                        VStack(alignment: .leading, spacing: 8) {
                            if mode == .fixed {
                                DateRow(label: "Leave at", date: $leave)
                                forecastRow
                            } else {
                                DateRow(label: "Earliest", date: $earliest)
                                HStack { Text("Over").font(.system(size: 12)); Spacer(); Stepper(value: $days, in: 1...14) { Text("\(Int(days)) days").font(.numeric(12, weight: .regular)) } }
                                HStack {
                                    Text("Try a departure").font(.system(size: 12)); Spacer()
                                    Picker("", selection: $stepHours) { ForEach(steps, id: \.self) { Text("Every \(Int($0)) h").tag($0) } }.labelsHidden().fixedSize()
                                }
                                forecastRow
                            }
                        }.padding(4)
                    }
                    if mode == .window { matters }
                    boat

                    HStack {
                        Spacer()
                        // While a route is computed: off, with a spinner, and pressing it shows the running jobs.
                        Button { model.job("route", go) } label: { JobLabel(title: mode == .fixed ? "Compute the route" : "Find the best departure", busy: model.busy("route")) }
                            .buttonStyle(.platformProminent).controlSize(.large)
                            .modifier(JobBusy(busy: model.busy("route")))
                        Spacer()
                    }
                    // The search and its download, here where they were asked for, not only in the toolbar.
                    if model.window.routing && !model.window.jobs.isEmpty {
                        JobRows(model: model)
                            .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                            .background(RoundedRectangle(cornerRadius: Radius.m).fill(Color.isomar(.viewBg)))
                    }
                    Text("The forecast for the passage is downloaded first if it is not already loaded. Planning only: not for navigation.")
                        .font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)

                    if mode == .window, let d = results { DepartureResults(d: d, model: model).id("results") }
                }
                .padding(14)
            }
            // New results are scrolled into view, once they are laid out: scrolled at once, a sidebar (the
            // Mac, the iPad) did not move, as the rows it should scroll to were not there yet.
            .onChange(of: results == nil) { none in
                if !none { DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { withAnimation { scroll.scrollTo("results", anchor: .top) } } }
            }
            }
        }
        .onAppear { load() }
    }

    // MARK: Forecast

    /// A toggle per model, with what is loaded of it or that it will be downloaded; with several, how the
    /// departures are ranked over them.
    private var forecastRow: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Forecasts").font(.system(size: 12, weight: .semibold))
            ForEach(model.weather.models) { m in
                VStack(alignment: .leading, spacing: 1) {
                    Toggle(m.label, isOn: Binding(get: { forecasts.contains(m.id) }, set: { on in if on { forecasts.insert(m.id) } else { forecasts.remove(m.id) } })).font(.system(size: 12))
                    Text(m.loaded ? m.detail : "None loaded: downloaded first. \(m.about)").font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                }
            }
            Toggle("Sail in the current", isOn: $currents).font(.system(size: 12))
            Text(currentsDetail).font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
            if mode == .window && forecasts.count > 1 {
                HStack {
                    Text("Rank departures by").font(.system(size: 12)); Spacer()
                    Picker("", selection: $cautious) { Text("The average").tag(false); Text("The worst forecast").tag(true) }.labelsHidden().fixedSize()
                }
            }
        }
    }

    private var modelList: [String] { model.weather.models.map(\.id).filter { forecasts.contains($0) } }

    // MARK: What matters

    private var matters: some View {
        GroupBox("What matters") {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text("Preset").font(.system(size: 12)); Spacer()
                    Picker("", selection: $preset) {
                        ForEach(Array(presets.enumerated()), id: \.offset) { Text($1.0).tag($0) }
                        Text("Custom").tag(-1)
                    }
                    .labelsHidden().fixedSize()
                    .onChange(of: preset) { p in if p >= 0 && p < presets.count { weights.merge(presets[p].1) { _, new in new } } }
                }
                DisclosureGroup("Adjust") {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("A passage costs its hours at sea, plus each hour of what you would rather avoid counted again as many times as its slider says (up to 4 more).")
                            .font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                        ForEach(rows, id: \.0) { r in
                            WeightSlider(label: r.1, hint: r.2, value: Binding(get: { weights[r.0] ?? 0 }, set: { weights[r.0] = $0; preset = -1 }), maxValue: maxWeight)
                        }
                        limit("Strong wind from (kn)", "strong_wind_kn")
                        limit("Strong gusts from (kn)", "gust_kn")
                        limit("Rough from (m of waves)", "wave_m")
                    }.padding(.top, 6)
                }
                .font(.system(size: 12))
            }.padding(4)
        }
    }

    private func limit(_ label: String, _ key: String) -> some View {
        HStack {
            Text(label).font(.system(size: 12)); Spacer()
            TextField("", value: Binding(get: { weights[key] ?? 0 }, set: { weights[key] = $0; preset = -1 }), formatter: NumberFormatter()).frame(width: 60).textFieldStyle(.roundedBorder)
        }
    }

    // MARK: Boat

    private var boat: some View {
        GroupBox("Boat") {
            VStack(alignment: .leading, spacing: 8) {
                if !polars.isEmpty {
                    HStack {
                        Text("Polar").font(.system(size: 12)); Spacer()
                        Picker("Polar", selection: Binding(get: { polar }, set: { usePolar($0) })) {
                            ForEach(polars) { p in Text(p.name).tag(p.id) }
                        }
                        .labelsHidden().fixedSize()
                    }
                    Text("The boat's speed at each wind angle and strength. The one picked here is also the one the performance log learns into.")
                        .font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                }
                Toggle("Use the engine in light wind", isOn: $engine)
                if engine { HStack { Text("Engine speed (kn)").font(.system(size: 12)); Spacer(); Stepper(value: $motor, in: 2...12, step: 0.5) { Text(String(format: "%.1f", motor)).font(.numeric(12, weight: .regular)) } } }
                HStack { Text("Avoid wind above (kn)").font(.system(size: 12)); Spacer(); Stepper(value: $maxWind, in: 15...60) { Text("\(Int(maxWind))").font(.numeric(12, weight: .regular)) } }
                DisclosureGroup("Advanced") {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack { Text("Performance (% of the polar)").font(.system(size: 12)); Spacer(); Stepper(value: $perf, in: 50...120, step: 5) { Text("\(Int(perf))").font(.numeric(12, weight: .regular)) } }
                        Text("Lower for a foul bottom, higher for a racing crew.").font(.caption).foregroundStyle(Color.isomar(.textMuted))
                    }.padding(.top, 6)
                }
                .font(.system(size: 12))
            }.padding(4)
        }
    }

    private var boatDict: [String: Any] { ["performance_pct": perf, "use_engine": engine, "motor_kn": motor, "max_wind_kn": maxWind] }

    // MARK: Actions

    private func usePolar(_ id: String) {
        guard id != polar else { return }
        if let e = model.send("polar_activate", ["id": id]) { IsomarModel.alert("Could not use the polar", e); return }
        polar = id
    }

    private func load() {
        if let l = model.call("polar_list", as: PolarList.self) { polars = l.polars; polar = l.active }
        let ends = model.dict("route_defaults", ["id": id])
        if let r = model.dict("route_get", ["id": id]) {
            name = r["name"] as? String ?? ""
            let way = "\(ends?["from"] as? String ?? "") to \(ends?["to"] as? String ?? "")"
            let nm = String(format: "%.1f nm", r["distance_nm"] as? Double ?? 0)
            summary = [Chip(icon: nil, text: way, label: way), Chip(icon: "distance", text: nm, label: nm)]
        }
        leave = Stamp.date(ends?["departure"] as? String) ?? Date()
        let chosen = ends?["models"] as? [String] ?? []
        forecasts = Set(chosen.isEmpty ? [ends?["model"] as? String ?? model.weather.model] : chosen)
        if let c = ends?["currents"] as? [String: Any] {
            currents = c["on"] as? Bool ?? true
            currentsDetail = c["detail"] as? String ?? ""
        }
        guard let d = model.dict("window_defaults", ["id": id]) else { return }
        earliest = Stamp.date(d["earliest"] as? String) ?? Date()
        days = d["days"] as? Double ?? 3
        stepHours = d["step_hours"] as? Double ?? 3
        steps = (d["steps"] as? [Double]) ?? [1, 3, 6]
        weights = (d["weights"] as? [String: Double]) ?? [:]
        maxWeight = d["max_weight"] as? Double ?? 4
        presets = (d["presets"] as? [[String: Any]] ?? []).map { ($0["name"] as? String ?? "", $0["weights"] as? [String: Double] ?? [:]) }
        preset = presets.firstIndex { $0.1 == weights } ?? -1
        cautious = d["cautious"] as? Bool ?? false
    }

    private func go() {
        switch mode {
        case .fixed:
            if let e = model.send("route_compute", ["id": id, "departure": Stamp.text(leave), "boat": boatDict, "models": modelList, "currents": currents]) { IsomarModel.alert("Weather route", e) }
        case .window:
            let args: [String: Any] = ["id": id, "earliest": Stamp.text(earliest), "days": days, "step_hours": stepHours, "weights": weights, "boat": boatDict, "models": modelList, "cautious": cautious, "currents": currents]
            if let e = model.send("window_run", args) { IsomarModel.alert("Best departure", e) }
        }
    }
}

/// The engine's departure text, "YYYY-MM-DD HH:MM" in local time, and the dates a picker works with.
enum Stamp {
    private static let format: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd HH:mm"
        return f
    }()

    static func date(_ text: String?) -> Date? { text.flatMap { format.date(from: $0) } }
    static func text(_ date: Date) -> String { format.string(from: date) }
}

struct DateRow: View {
    let label: String
    @Binding var date: Date

    var body: some View {
        HStack {
            Text(label).font(.system(size: 12))
            Spacer()
            DatePicker("", selection: $date, displayedComponents: [.date, .hourAndMinute]).labelsHidden()
        }
    }
}

/// A weight's title and what it counts, over a slider from "doesn't matter" to "matters a lot".
struct WeightSlider: View {
    let label: String
    let hint: String
    @Binding var value: Double
    let maxValue: Double

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.system(size: 12))
            Text(hint).font(.system(size: 10)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 6) {
                Text("No").font(.system(size: 10)).foregroundStyle(Color.isomar(.textMuted))
                Slider(value: $value, in: 0...maxValue, step: 0.25).accessibilityLabel(label)
                Text("A lot").font(.system(size: 10)).foregroundStyle(Color.isomar(.textMuted))
            }
        }
    }
}

/// The results of a best-departure search: what each departure costs on a timeline, and the best ten.
struct DepartureResults: View {
    let d: DeparturesInfo
    @ObservedObject var model: IsomarModel

    private func pick(_ i: Int) {
        model.send("departure_select", ["i": i])
        model.send("departure_preview")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Best departures").font(.system(size: 13, weight: .semibold))
                Spacer()
                Button("Compare") {
                    if let t = model.call("departure_compare", as: StatsTable.self) { model.sheet = .stats(t) }
                }
                .buttonStyle(.platformLink).disabled(!d.canCompare)
                .help("Compare the statistics of the best three and the selected departure")
            }
            Text(d.summary).font(.caption).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
            if !d.legend.isEmpty {
                HStack(spacing: 10) {
                    ForEach(Array(d.legend.enumerated()), id: \.offset) { _, l in
                        HStack(spacing: 4) { Circle().fill(Color.isomar(Tok.byName(l.colour) ?? .route)).frame(width: 8, height: 8); Text(l.label) }
                    }
                }
                .font(.caption)
            }
            DepartureTimeline(d: d, pick: pick).frame(height: 120)
            if !d.rows.isEmpty {
                BoxedList {
                    ForEach(Array(d.rows.enumerated()), id: \.element.id) { k, r in
                        if k > 0 { Divider() }
                        VStack(alignment: .leading, spacing: 2) {
                            Text(r.title).font(.system(size: 12, weight: .semibold))
                            Group {
                                chipsText(r.figures).chipsHelp(r.figures)
                                chipsText(r.why).chipsHelp(r.why)
                                if !r.models.isEmpty { chipsText(r.models).chipsHelp(r.models) }
                            }
                            .font(.system(size: 11)).foregroundStyle(r.selected ? Color.isomar(.onAccent).opacity(0.85) : Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12).padding(.vertical, 6)
                        .foregroundStyle(r.selected ? Color.isomar(.onAccent) : Color.isomar(.text))
                        .background(r.selected ? Color.isomar(.accentBg) : Color.clear)
                        .contentShape(Rectangle())
                        .onTapGesture { pick(r.index) }
                    }
                }
            }
        }
    }
}

/// The cost of each departure as a bar, over the night bands and the days, in the `cost-*` ramp.
struct DepartureTimeline: View {
    let d: DeparturesInfo
    let pick: (Int) -> Void
    private let axis: CGFloat = 26

    var body: some View {
        GeometryReader { g in
            let plotH = g.size.height - axis
            ZStack(alignment: .topLeading) {
                ForEach(Array(d.nights.enumerated()), id: \.offset) { _, n in
                    Rectangle().fill(Color.isomar(.costNightBand))
                        .frame(width: max(1, (n[1] - n[0]) * g.size.width), height: plotH)
                        .offset(x: n[0] * g.size.width)
                }
                ForEach(Array(d.days.enumerated()), id: \.offset) { _, day in
                    Rectangle().fill(Color.isomar(.border)).frame(width: 1, height: g.size.height).offset(x: (day.x * g.size.width).rounded())
                    if day.labelX * g.size.width > 20 && day.labelX * g.size.width < g.size.width - 20 {
                        Text(day.label).font(.system(size: 10)).foregroundStyle(Color.isomar(.textMuted)).fixedSize()
                            .position(x: day.labelX * g.size.width, y: plotH + 13)
                    }
                }
                ForEach(d.bars) { b in
                    let h = max(3, b.h * (plotH - 6))
                    let slot = b.w * g.size.width
                    RoundedRectangle(cornerRadius: min(4, slot / 2))
                        .fill(Color.isomar(!b.ok ? .costWorst : b.best ? .costBest : b.h < 0.6 ? .costMid : .costWorst))
                        .overlay(RoundedRectangle(cornerRadius: min(4, slot / 2)).stroke(Color.isomar(.text), lineWidth: b.selected ? 2 : 0))
                        .frame(width: max(1, slot - (slot > 6 ? 2 : 0)), height: h)
                        .offset(x: b.x * g.size.width + (slot > 6 ? 1 : 0), y: plotH - h)
                        .help(b.tooltip)
                    // Several forecasts: the spread of their costs, and a dot each in its colour.
                    let cx = b.x * g.size.width + slot / 2
                    if let r = b.range, r.count == 2 {
                        Rectangle().fill(Color.isomar(.text)).frame(width: 1.5, height: max(1, (r[1] - r[0]) * (plotH - 6)))
                            .offset(x: cx - 0.75, y: plotH - r[1] * (plotH - 6))
                    }
                    ForEach(Array(b.dots.enumerated()), id: \.offset) { _, dot in
                        let size = min(7, max(3, slot / 1.5))
                        Circle().fill(Color.isomar(Tok.byName(dot.colour) ?? .route)).overlay(Circle().stroke(Color.isomar(.viewBg), lineWidth: 1))
                            .frame(width: size, height: size)
                            .offset(x: cx - size / 2, y: plotH - dot.h * (plotH - 6) - size / 2)
                    }
                }
                Rectangle().fill(Color.isomar(.borderControl)).frame(height: 1).offset(y: plotH)
            }
            .contentShape(Rectangle())
            .onTapGesture { p in
                if let i = d.bars.firstIndex(where: { p.x / g.size.width >= $0.x && p.x / g.size.width < $0.x + $0.w }) { pick(i) }
            }
        }
    }
}

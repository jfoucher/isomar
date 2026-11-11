import Combine
import SwiftUI

// MARK: - What the engine reports (the JSON of crates/isomar-app/src/vm and api.rs)

struct LayerItem: Identifiable, Equatable {
    let id: String
    let label: String
    var visible: Bool
}

struct RouteRow: Decodable, Identifiable, Equatable { let id: Int64; let name: String; let detail: [Chip]; let planned: [Chip]; let visible: Bool; let active: Bool }
struct WaypointRow: Decodable, Identifiable, Equatable { let id: Int64; let name: String; let detail: String }
struct TrackRow: Decodable, Identifiable, Equatable { let id: Int64; let name: String; let detail: [Chip]; let visible: Bool }
struct FollowingRow: Decodable, Equatable { let name: String; let detail: [Chip] }
struct Passage: Decodable, Equatable {
    var routes: [RouteRow] = []
    var waypoints: [WaypointRow] = []
    var tracks: [TrackRow] = []
    var recording = false
    var following: FollowingRow?
}
struct Vessel: Decodable, Identifiable, Equatable { let mmsi: UInt32; let name: String; let detail: [Chip]; let alert: Bool; var id: UInt32 { mmsi } }
struct Tile: Decodable, Identifiable, Equatable { let title: String; let value: String; let unit: String; var id: String { title } }

/// `action` names what started it (`isomar_app::jobs`): its button shows it running (`IsomarModel.busy`).
struct JobRow: Decodable, Identifiable, Equatable { let id: Int64; let action: String; let title: String; let fraction: Double?; let detail: String; let cancellable: Bool }
struct WindowInfo: Decodable, Equatable {
    var title = "Isomar"
    var subtitle: [Chip] = []
    var link = "none"
    var jobs: [JobRow] = []
    var underway = false
    var offline = false
    var recording = false
    var keepAwake = false
    var watching = false
    var following = false
    var routing = false
    /// What was last selected on the chart, for a touch shell to open (`n` changes with each pick).
    var picked: Picked?
}
struct Picked: Decodable, Equatable { let kind: String; let id: Int64; let n: UInt64 }

struct Strip: Decodable, Identifiable, Equatable {
    let id: Int64
    let level: String
    let title: String
    let acknowledge: String?
    let snooze: String?
    let dismiss: String?
    let action: String?
    let count: Int
    let countLabel: String?
    let dim: Bool
}
struct Hint: Decodable, Equatable { let title: String; let finish: String?; let undo: String?; let cancel: String? }
struct AlertsInfo: Decodable, Equatable {
    var alarm: Strip?
    var warnings: [Strip] = []
    var hint: Hint?
}

struct ScaleBar: Decodable, Equatable { let px: Double; let label: String }
struct PointerInfo: Decodable, Equatable { let position: String; let chips: [Chip] }
struct FooterInfo: Decodable, Equatable {
    var scale = ScaleBar(px: 80, label: "")
    var metric = ScaleBar(px: 80, label: "")
    var credits: [String] = []
    var planningOnly = false
    var planningText = ""
    var pointer: PointerInfo?
    var view: [Chip] = []
}

struct UCell: Decodable, Equatable { let label: String; let value: String; let unit: String }
struct NavCard: Decodable, Equatable { let number: Int; let name: String; let cells: [UCell]; let canSkip: Bool }
struct Readout: Decodable, Identifiable, Equatable { let id: String; let label: String; let value: String; let unit: String; let note: [Chip] }
struct UnderwayInfo: Decodable, Equatable {
    var on = false
    var card: NavCard?
    var dashboard: [Readout] = []
    var sail: PerfSwitch?
}

// The performance log (`vm::performance`).
struct PerfSwitch: Decodable, Equatable {
    var mode = ""
    var byEngine = false
    var note: String?
    var hint: String?
}
struct PerfLine: Decodable, Equatable, Hashable { let title: String; let detail: String }
struct PerfCalibration: Decodable, Equatable { let findings: [PerfLine]; let evidence: String; let canSave: Bool; let saved: Bool }
struct PerfCurve: Decodable, Equatable { let tws: Double; let current: [[Double]]; let learned: [[Double]]; let cells: [[Double]]; let best: String? }
struct PerfLearned: Decodable, Equatable { let hours: Double; let summary: String; let curves: [PerfCurve]; let coverage: [[Double]]; let columns: [Double]; let changes: Int; let caution: String? }
struct PerfFactorRow: Decodable, Equatable, Hashable { let label: String; let pct: Double; let lo: Double; let hi: Double; let hours: Double; let text: String }
struct PerfFactorGroup: Decodable, Equatable, Hashable { let title: String; let note: String; let rows: [PerfFactorRow] }
struct PerfSuggestion: Decodable, Equatable { let pct: Double; let text: String; let button: String; let applied: Bool }
struct PerformanceInfo: Decodable, Equatable {
    let recording: Bool
    let analysing: Bool
    let backfilling: Bool
    let hindcastPending: Int
    let empty: Bool
    let summary: String
    let hours: [PerfLine]
    let calibration: PerfCalibration?
    let learned: PerfLearned?
    let factors: [PerfFactorGroup]
    let suggestion: PerfSuggestion?
    let forecastCheck: [PerfLine]
    let waves: [PerfLine]
    let passages: [PerfLine]
    let routingPct: Double?
    let canUndoPolar: Bool
    let polar: String
    let error: String?
}
/// What the polar editor draws from the log: the learnt polar and its confidence, and per wind speed the
/// bins the log holds as (angle, speed, confidence).
struct PerfObserved: Decodable, Equatable { let tws: [Double]; let twa: [Double]; let speeds: [[Double]]; let confidence: [[Double]]; let cells: [[[Double]]] }

struct TimelineInfo: Decodable, Equatable {
    let mode: String
    let start: Int64
    let end: Int64
    let now: Int64
    let step: Int64
    let playing: Bool
    let speed: Double
    let label: String
    let age: String?
    let stale: Bool
}

struct AltRow: Decodable, Identifiable, Equatable {
    let index: Int
    let title: String
    let summary: [Chip]
    let colour: String
    let dash: [Double]
    let selected: Bool
    let arrives: Bool
    var id: Int { index }
}
/// One forecast model's best way, when the passage was routed on several (`vm::router::ModelRow`).
struct ModelRowInfo: Decodable, Identifiable, Equatable {
    let index: Int
    let label: String
    let summary: [Chip]
    let colour: String
    let focused: Bool
    let arrives: Bool
    let robust: Bool
    var id: Int { index }
}
struct RouterCard: Decodable, Equatable {
    let title: String
    let note: String
    let failure: Bool
    let alternatives: [AltRow]
    let canAdd: Bool
    /// A saved route's plan played back: nothing to pick or use, only to close.
    let replay: Bool
    /// One row per model when routed on several, and what they say together.
    let models: [ModelRowInfo]
    let verdict: [String]
}

/// One forecast model's cost of a departure, on the bar's scale.
struct DepDot: Decodable, Equatable { let h: Double; let colour: String }
struct DepBar: Decodable, Identifiable, Equatable {
    let x: Double; let w: Double; let h: Double
    let ok: Bool; let best: Bool; let selected: Bool
    let tooltip: String
    /// With several models: each one's cost, and the least and most of them.
    let dots: [DepDot]
    let range: [Double]?
    var id: Double { x }
}
struct DepRow: Decodable, Identifiable, Equatable { let index: Int; let title: String; let figures: [Chip]; let why: [Chip]; let selected: Bool; let models: [Chip]; var id: Int { index } }
struct DepLegend: Decodable, Equatable { let label: String; let colour: String }
struct DepDay: Decodable, Equatable { let x: Double; let labelX: Double; let label: String }
struct DeparturesInfo: Decodable, Equatable {
    let routeId: Int64
    let summary: String
    let bars: [DepBar]
    let rows: [DepRow]
    let days: [DepDay]
    let nights: [[Double]]
    let canCompare: Bool
    let legend: [DepLegend]
    let cautious: Bool
}

struct StatSection: Decodable, Equatable { let title: String; let rows: [StatCell] }
struct StatCell: Decodable, Equatable {
    let label: String
    let cells: [String]
    init(from decoder: Decoder) throws {
        var c = try decoder.unkeyedContainer()
        label = try c.decode(String.self)
        cells = try c.decode([String].self)
    }
}
struct StatsTable: Decodable, Equatable { let heads: [String]; let sections: [StatSection] }

struct SourceRow: Decodable, Identifiable, Equatable { let id: String; let name: String; let rating: String; let ratingText: String; let chips: [Chip]; let note: String; let action: String; let url: String; var installed = false }
struct InUseRow: Decodable, Identifiable, Equatable { let name: String; let kind: String; let chips: [Chip]; var id: String { name } }
struct ChartsPage: Decodable, Equatable {
    var here: [SourceRow] = []
    var inUse: [InUseRow] = []
    var vector = 0
    var folders: [String] = []
    var rasters = 0
    var planningOnly = false
}

struct WeatherArea: Decodable, Equatable { let north: String; let south: String; let west: String; let east: String }
struct WeatherGroup: Decodable, Identifiable, Equatable { let id: String; let label: String; let `default`: Bool }
/// A forecast model to pick (`flows::weather::model_choices`): what it is and what is loaded of it.
struct ModelChoice: Decodable, Equatable, Identifiable { let id: String; let label: String; let about: String; let detail: String; let loaded: Bool; let shown: Bool }

/// One loaded model against the measured wind (`flows::perf::tracking`).
struct TrackingRow: Decodable, Equatable { let label: String; let text: String; let best: Bool }

struct WeatherPage: Decodable, Equatable {
    /// Under way, each loaded model against the wind the instruments measure.
    var tracking: [TrackingRow] = []
    /// The model shown, which weather routes use unless another is picked.
    var model = "gfs"
    var modelLabel = "GFS"
    var models: [ModelChoice] = []
    var loaded = false
    var fields = 0
    var run: String?
    var from: String?
    var to: String?
    /// What colours the sea: "none", "wind" or "waves"; barbs, wave arrows and isobars go over it.
    var fill = "wind"
    var barbs = true
    var pressure = true
    var waveArrows = true
    /// Why the wave colours and arrows show nothing: the forecast loaded has no waves.
    var noWaves: String?
    var picking = false
    var rect: WeatherArea?
    var horizons: [String] = []
    var steps: [String] = []
    var groups: [WeatherGroup] = []
    /// Tidal streams and ocean currents, shared by every model (`currents::page`).
    var currents: CurrentsPage?
}
struct CurrentSourceLine: Decodable, Equatable { let id: String; let label: String; let detail: String }
struct AtlasLine: Decodable, Equatable, Identifiable { let id: String; let name: String; let detail: String; let installed: Bool }
struct CurrentsPage: Decodable, Equatable {
    let loaded: Bool
    let arrows: Bool
    let routeWith: Bool
    let sources: [CurrentSourceLine]
    let atlases: [AtlasLine]
    let note: String
    /// Under way, each source against the stream measured.
    let tracking: [String]
}
struct WeatherReadout: Decodable, Equatable { let chips: [Chip]; let time: String }
struct WeatherEstimate: Decodable, Equatable { let source: String; let run: String; let size: String; let files: String; let missing: String; let ok: Bool }

struct MenuEntry: Decodable { let i: Int; let label: String }
struct MenuRequest: Decodable { let x: Double; let y: Double; var title: String? = nil; let sections: [[MenuEntry]] }
struct InfoRequest: Decodable { let x: Double; let y: Double; let title: String; let subtitle: String?; let rows: [[String]]; let choice: [Int]? }
struct CursorRequest: Decodable { let pointing: Bool }
struct ConfirmRequest: Decodable { let id: UInt64; let heading: String; let body: String; let ok: String; let destructive: Bool }
struct ErrorRequest: Decodable { let heading: String; let details: String }
struct ToastRequest: Decodable { let text: String }
struct SoundRequest: Decodable { let sound: String? }
struct NotifyRequest: Decodable { let title: String; let body: String }

struct DialogRequest: Decodable, Identifiable {
    let name: String
    let id64: Int64?
    let lat: Double?
    let lon: Double?
    let north: Double?
    let south: Double?
    let west: Double?
    let east: Double?
    var id: String { "\(name)-\(id64 ?? 0)" }
    enum CodingKeys: String, CodingKey { case name, id64 = "id", lat, lon, north, south, west, east }
}

/// A page of the sidebar.
enum Page: String, CaseIterable, Identifiable {
    case passage, weather, traffic, charts, instruments
    var id: String { rawValue }
    var title: String {
        switch self {
        case .passage: return "Passage"
        case .weather: return "Weather"
        case .traffic: return "Traffic"
        case .charts: return "Charts"
        case .instruments: return "Instruments"
        }
    }
    var symbol: String {
        switch self {
        case .passage: return "point.topleft.down.to.point.bottomright.curvepath"
        case .weather: return "cloud.sun"
        case .traffic: return "ferry"
        case .charts: return "map"
        case .instruments: return "gauge.with.dots.needle.33percent"
        }
    }
}

/// What a sidebar page pushed on top of itself shows.
enum Detail: Equatable {
    case route(Int64)
    case waypoint(Int64)
    case vessel(UInt32)
    /// Weather routing, pushed from a route.
    case routing(Int64)
    /// The performance log, pushed from Instruments.
    case performance
}

/// Whether Weather routing routes one departure or looks for the best one.
enum RoutingMode: String {
    case fixed, window
}

/// A sheet over the window.
enum Sheet: Identifiable, Equatable {
    case getWeather(WeatherArea)
    case autoRoute(Int64)
    case stats(StatsTable)
    case tides(Double, Double)
    case planningOnly
    case welcome
    /// The settings, as a sheet where there is no Settings window (iOS).
    case settings
    /// The connections and About, as sheets where there are no windows for them (iOS).
    case connections
    case about

    var id: String {
        switch self {
        case .getWeather: return "getWeather"
        case .autoRoute(let i): return "autoRoute\(i)"
        case .stats: return "stats"
        case .tides: return "tides"
        case .planningOnly: return "planningOnly"
        case .welcome: return "welcome"
        case .settings: return "settings"
        case .connections: return "connections"
        case .about: return "about"
        }
    }
}

// MARK: - The model

/// What changes with every move of the pointer or of the view: the footer's scale bars and positions.
/// Kept apart from `IsomarModel`, which every view of the window observes, so that only the footer is
/// worked out again sixty times a second while the chart pans.
final class LiveText: ObservableObject {
    @Published var pointerText = ""
    @Published var viewText = ""
    @Published var footer = FooterInfo()
}

/// The Swift side of the engine: owns the start-up, forwards the Rust library's callbacks, calls its JSON api
/// and holds what the window shows.
final class IsomarModel: ObservableObject {
    static let shared = IsomarModel()

    // Chart chrome.
    /// The footer's texts, observed by the footer alone.
    let live = LiveText()
    var pointerText: String { live.pointerText }
    var viewText: String { live.viewText }
    var footer: FooterInfo { live.footer }
    @Published var statusText = ""
    @Published var layers: [LayerItem] = []
    @Published var mode = "auto"
    @Published var window = WindowInfo()
    @Published var alerts = AlertsInfo()
    @Published var underway = UnderwayInfo()
    @Published var timeline: TimelineInfo?
    @Published var routerCard: RouterCard?
    @Published var departures: DeparturesInfo?
    @Published var weatherReadout: WeatherReadout?
    @Published var toast: String?
    @Published var follow = 0          // 0 off, 1 follow, 2 follow with course up (course up is not drawn yet)

    // Pages.
    @Published var passage = Passage()
    @Published var vessels: [Vessel] = []
    @Published var tiles: [Tile] = []
    @Published var sailSwitch = PerfSwitch()
    @Published var performance: PerformanceInfo?
    @Published var charts = ChartsPage()
    @Published var weather = WeatherPage()

    // Navigation of the window.
    @Published var page: Page = .passage
    @Published var sidebarVisible = true
    @Published var detail: Detail?
    /// The mode Weather routing opens in next: the one used last.
    @Published var routingMode = RoutingMode.fixed
    @Published var sheet: Sheet?
    /// The running jobs are asked to show where they always do (the activity indicator): a busy job
    /// button was pressed. The indicator takes it back to false.
    @Published var activityShown = false
    @Published var measuring = false
    @Published var sizeClass = SizeClass.regular
    @Published var systemDark = false

    enum SizeClass { case regular, narrow }

    weak var chartView: ChartSurface?
    /// On a touch screen, what was tapped (shown in the sheet) and the actions of a long press.
    @Published var info: InfoRequest?
    /// On a phone, the mark, route or vessel tapped on the chart: its page in a sheet of its own, apart from
    /// the pages' sheet and its tabs (`ChartItemSheet`). Tablets open it in the sidebar (`detail`).
    @Published var chartDetail: Detail?
    @Published var menu: MenuRequest?
    private var tick: Timer?
    private var toastTimer: Timer?
    /// Re-reads the Charts page once the view has settled, as the GTK page does.
    private var chartsTimer: Timer?
    /// A menu entry to choose at once instead of showing the menu (for `ISOMAR_SCRIPT`).
    var autoPick: String?
    var onDialog: ((DialogRequest) -> Void)?

    private let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    private init() {}

    // MARK: start

    func start() {
        let ctx = Unmanaged.passUnretained(self).toOpaque()
        let ok = isomar_start(ctx, { _, delay in
            // Any thread: a worker finishing a job wakes the pump from here.
            DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(Int(delay))) { isomar_pump() }
        }, { ctx in
            let model = Unmanaged<IsomarModel>.fromOpaque(ctx!).takeUnretainedValue()
            model.chartView?.setNeedsRedraw()
        }, { ctx, topic in
            let model = Unmanaged<IsomarModel>.fromOpaque(ctx!).takeUnretainedValue()
            model.changed(topic)
        }, { ctx, kind, json in
            let model = Unmanaged<IsomarModel>.fromOpaque(ctx!).takeUnretainedValue()
            model.ui(kind, String(cString: json!))
        })
        guard ok else { return }
        let env = ProcessInfo.processInfo.environment
        isomar_set_reduce_motion(Platform.reduceMotion)
        if let palette = env["ISOMAR_PALETTE"] { isomar_set_mode(palette) }
        if let p = env["ISOMAR_SIDEBAR"], let page = Page(rawValue: p) { self.page = page }
        // `ISOMAR_DIALOG=performance` opens the Performance page, as on Linux.
        if env["ISOMAR_DIALOG"] == "performance" { self.page = .instruments; self.detail = .performance }
        if env["ISOMAR_SIDEBAR"] == nil && env["ISOMAR_SCREENSHOT"] != nil && env["ISOMAR_DEMO"] == nil { sidebarVisible = true }
        mode = String(cString: isomar_mode())
        Theme.apply(mode)
        for topic in [ISOMAR_TOPIC_VIEW, ISOMAR_TOPIC_STATUS, ISOMAR_TOPIC_SESSION, ISOMAR_TOPIC_LAYERS, ISOMAR_TOPIC_DATA, ISOMAR_TOPIC_LIVE] {
            changed(Int32(topic))
        }
        tick = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { _ in isomar_tick() }
        RunLoop.main.add(tick!, forMode: .common)
        if !(settings["first_run_done"] as? Bool ?? true) && env["ISOMAR_SCREENSHOT"] == nil { DispatchQueue.main.async { self.sheet = .welcome } }
    }

    func shutdown() {
        tick?.invalidate()
        isomar_shutdown()
    }

    // MARK: calling the engine

    /// Calls `name` of the engine's api and decodes the answer.
    func call<T: Decodable>(_ name: String, _ args: [String: Any] = [:], as type: T.Type = T.self) -> T? {
        guard let text = callText(name, args), let data = text.data(using: .utf8), text != "null" else { return nil }
        return try? decoder.decode(T.self, from: data)
    }

    /// Calls `name` and returns the raw answer.
    func callText(_ name: String, _ args: [String: Any] = [:]) -> String? {
        let json = (try? JSONSerialization.data(withJSONObject: args)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
        guard let p = isomar_call(name, json) else { return nil }
        return String(cString: p)
    }

    /// Calls `name` for what it does; returns an error message if it said one.
    @discardableResult
    func send(_ name: String, _ args: [String: Any] = [:]) -> String? {
        guard let text = callText(name, args), let data = text.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let err = obj["error"] as? String else { return nil }
        return err
    }

    /// The settings as a dictionary.
    var settings: [String: Any] {
        guard let text = callText("settings_get"), let data = text.data(using: .utf8) else { return [:] }
        return (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }

    func command(_ name: String, _ arg: Int64 = 0) { isomar_command(name, arg) }

    // MARK: topics

    private func text(_ which: Int) -> String { String(cString: isomar_text(Int32(which))) }

    func changed(_ topic: Int32) {
        switch Int(topic) {
        case Int(ISOMAR_TOPIC_POINTER): refreshFooter()
        case Int(ISOMAR_TOPIC_VIEW): refreshFooter(); refreshChartsWhenSettled()
        case Int(ISOMAR_TOPIC_STATUS): statusText = text(Int(ISOMAR_TEXT_STATUS))
        case Int(ISOMAR_TOPIC_SESSION): break
        case Int(ISOMAR_TOPIC_LAYERS): reloadLayers()
        case Int(ISOMAR_TOPIC_DATA): reloadData()
        case Int(ISOMAR_TOPIC_LIVE): reloadLive()
        default: break
        }
    }

    private func update<T: Equatable>(_ keyPath: ReferenceWritableKeyPath<IsomarModel, T>, _ value: T?, or fallback: T) {
        let v = value ?? fallback
        if self[keyPath: keyPath] != v { self[keyPath: keyPath] = v }
    }

    /// "In use here" and the sources follow the centre of the chart: once the view stops moving, while the page shows.
    private func refreshChartsWhenSettled() {
        guard page == .charts else { return }
        chartsTimer?.invalidate()
        chartsTimer = Timer.scheduledTimer(withTimeInterval: 0.4, repeats: false) { [weak self] _ in
            guard let self, self.page == .charts else { return }
            self.update(\.charts, self.call("charts", as: ChartsPage.self), or: self.charts)
        }
    }

    /// The footer follows the pointer and the view at most ten times a second, and always ends on where
    /// they stopped: at every frame of a pan, SwiftUI laid the window out again for it.
    private func refreshFooter() {
        let since = Date().timeIntervalSince(footerAt)
        if since >= Self.footerEvery {
            footerTimer?.invalidate()
            footerTimer = nil
            footerAt = Date()
            set(\.pointerText, text(Int(ISOMAR_TEXT_POINTER)))
            set(\.viewText, text(Int(ISOMAR_TEXT_VIEW)))
            if let f = call("footer", as: FooterInfo.self) { set(\.footer, f) }
        } else if footerTimer == nil {
            footerTimer = Timer.scheduledTimer(withTimeInterval: Self.footerEvery - since, repeats: false) { [weak self] _ in
                self?.footerTimer = nil
                self?.refreshFooter()
            }
        }
    }

    private static let footerEvery = 0.1
    private var footerAt = Date.distantPast
    private var footerTimer: Timer?

    /// Sets one of the footer's texts, telling its observers only when it changed.
    private func set<T: Equatable>(_ keyPath: ReferenceWritableKeyPath<LiveText, T>, _ value: T) {
        if live[keyPath: keyPath] != value { live[keyPath: keyPath] = value }
    }

    /// Routes, waypoints, tracks, the charts and the weather changed.
    private func reloadData() {
        update(\.passage, call("passage", as: Passage.self), or: passage)
        update(\.weather, call("weather_page", as: WeatherPage.self), or: weather)
        update(\.routerCard, call("router_card", as: RouterCard?.self) ?? nil, or: nil)
        let before = departures
        update(\.departures, call("departures", as: DeparturesInfo?.self) ?? nil, or: nil)
        if page == .charts { update(\.charts, call("charts", as: ChartsPage.self), or: charts) }
        if detail == .performance { update(\.performance, call("perf_page", as: PerformanceInfo?.self) ?? nil, or: nil) }
        // New results of the route on show: its best departure goes on the chart at once.
        if let d = departures, before == nil, case .routing(let id)? = detail, d.routeId == id, d.rows.contains(where: { $0.selected }) {
            send("departure_preview")
        }
    }

    /// The 4 Hz clock: what changes with the boat.
    private func reloadLive() {
        update(\.window, call("window", as: WindowInfo.self), or: window)
        // Panning by hand ends following, in the engine.
        if !window.following && follow > 0 { follow = 0 }
        // Asked for while a job ran, and it has ended: the list must not open by itself with the next one.
        if activityShown && window.jobs.isEmpty { activityShown = false }
        update(\.alerts, call("alerts", as: AlertsInfo.self), or: alerts)
        update(\.underway, call("underway", as: UnderwayInfo.self), or: underway)
        update(\.timeline, call("timeline", as: TimelineInfo?.self) ?? nil, or: nil)
        update(\.weatherReadout, call("weather_readout", as: WeatherReadout?.self) ?? nil, or: nil)
        refreshFooter()
        switch page {
        case .traffic: update(\.vessels, call("traffic", as: [Vessel].self), or: vessels)
        case .instruments:
            update(\.tiles, call("instruments", as: [Tile].self), or: tiles)
            update(\.sailSwitch, call("perf_switch", as: PerfSwitch.self), or: sailSwitch)
        case .passage: update(\.passage, call("passage", as: Passage.self), or: passage)
        case .charts: break
        case .weather: break
        }
        Alerting.shared.update(alerts: alerts, window: window)
    }

    private func reloadLayers() {
        guard let data = String(cString: isomar_layers_json()).data(using: .utf8),
              let items = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return }
        layers = items.compactMap { d in
            guard let id = d["id"] as? String, let label = d["label"] as? String else { return nil }
            return LayerItem(id: id, label: label, visible: d["visible"] as? Bool ?? true)
        }
    }

    // MARK: commands the views use

    func setLayer(_ id: String, visible: Bool) { isomar_set_layer_visible(id, visible) }

    func setMode(_ m: String) {
        isomar_set_mode(m)
        mode = String(cString: isomar_mode())
        Theme.apply(mode)
    }

    func zoom(by delta: Double) { isomar_zoom_by(delta) }

    func cycleFollow() {
        follow = (follow + 1) % 2
        command("follow", follow > 0 ? 1 : 0)
        if follow > 0 { command("centre_boat") }
    }

    /// Weather routing for route `id`, in `mode` (or the one used last, or the best-departure one while the route has results).
    func showRouting(_ id: Int64, _ mode: RoutingMode? = nil) {
        routingMode = mode ?? (departures?.routeId == id ? .window : routingMode)
        page = .passage
        detail = .routing(id)
        sidebarVisible = true
    }

    /// Opens what was picked on the chart (a touch screen's sheet or sidebar): a mark, a route, a vessel.
    func open(_ p: Picked) {
        guard let d = Self.detail(for: p) else { return }
        info = nil
        if case .vessel = d { page = .traffic } else { page = .passage }
        detail = d
        sidebarVisible = true
    }

    /// The page of what was picked on the chart.
    static func detail(for p: Picked) -> Detail? {
        switch p.kind {
        case "waypoint": return .waypoint(p.id)
        case "route": return .route(p.id)
        case "vessel": return .vessel(UInt32(truncatingIfNeeded: p.id))
        default: return nil
        }
    }

    func showPage(_ p: Page) {
        page = p
        detail = nil
        sidebarVisible = true
        reloadData()
        reloadLive()
    }

    func toggleMeasure() {
        measuring.toggle()
        command(measuring ? "measure_begin" : "cancel")
    }

    // MARK: requests from the engine

    private func decode<T: Decodable>(_ s: String, as type: T.Type) -> T? {
        try? decoder.decode(T.self, from: Data(s.utf8))
    }

    func ui(_ kind: Int32, _ text: String) {
        switch Int(kind) {
        case Int(ISOMAR_UI_TOAST):
            if let t = decode(text, as: ToastRequest.self) { show(toast: t.text) }
        case Int(ISOMAR_UI_ERROR):
            if let e = decode(text, as: ErrorRequest.self) {
                DispatchQueue.main.async { Self.alert(e.heading, e.details) }
            }
        case Int(ISOMAR_UI_BEEP):
            let s = decode(text, as: SoundRequest.self)?.sound ?? "alarm"
            Sounds.play(s)
        case Int(ISOMAR_UI_NOTIFY):
            if let n = decode(text, as: NotifyRequest.self) { DispatchQueue.main.async { Alerting.shared.notify(n.title, n.body) } }
        case Int(ISOMAR_UI_CONFIRM):
            if let c = decode(text, as: ConfirmRequest.self) { DispatchQueue.main.async { Self.confirm(c) } }
        case Int(ISOMAR_UI_MENU):
            if let m = decode(text, as: MenuRequest.self) { DispatchQueue.main.async { [weak self] in self?.chartView?.showMenu(m) } }
        case Int(ISOMAR_UI_INFO):
            if let i = decode(text, as: InfoRequest.self) { DispatchQueue.main.async { [weak self] in self?.chartView?.showInfo(i) } }
        case Int(ISOMAR_UI_CLIPBOARD):
            if let t = decode(text, as: ToastRequest.self) {
                DispatchQueue.main.async { Platform.copy(t.text) }
            }
        case Int(ISOMAR_UI_CURSOR):
            if let c = decode(text, as: CursorRequest.self) { DispatchQueue.main.async { [weak self] in self?.chartView?.setPointing(c.pointing) } }
        case Int(ISOMAR_UI_DIALOG):
            if let d = decode(text, as: DialogRequest.self) { DispatchQueue.main.async { [weak self] in self?.open(d) } }
        default: break
        }
    }

    /// The engine asks for a dialog or window.
    func open(_ d: DialogRequest) {
        switch d.name {
        case "waypoint_properties": if let i = d.id64 { page = .passage; detail = .waypoint(i); sidebarVisible = true }
        case "route_properties": if let i = d.id64 { page = .passage; detail = .route(i); sidebarVisible = true }
        case "vessel": if let i = d.id64 { page = .traffic; detail = .vessel(UInt32(i)); sidebarVisible = true }
        case "weather_route": if let i = d.id64 { showRouting(i, .fixed) }
        case "best_departure": if let i = d.id64 { showRouting(i, .window) }
        case "auto_route": if let i = d.id64 { sheet = .autoRoute(i) }
        case "get_weather":
            if let w = call("weather_defaults", ["kind": "rect"], as: WeatherArea.self), d.north == nil { sheet = .getWeather(w) }
            else if let n = d.north, let s = d.south, let w = d.west, let e = d.east {
                sheet = .getWeather(WeatherArea(north: Self.fmt(n, lat: true), south: Self.fmt(s, lat: true), west: Self.fmt(w, lat: false), east: Self.fmt(e, lat: false)))
            }
        case "tides": if let la = d.lat, let lo = d.lon { sheet = .tides(la, lo) }
        case "charts_here": page = .charts; sidebarVisible = true; reloadData()
        default: show(toast: "\(d.name) is not available here")
        }
        onDialog?(d)
    }

    /// A coordinate the way the weather form takes it: decimal degrees are accepted.
    static func fmt(_ v: Double, lat: Bool) -> String { String(format: "%.4f", v) }

    static func alert(_ heading: String, _ details: String) { Platform.alert(heading, details) }

    private static func confirm(_ c: ConfirmRequest) {
        if ProcessInfo.processInfo.environment["ISOMAR_AUTO_ACCEPT"] != nil {
            isomar_confirm_result(c.id, true)
            return
        }
        Platform.confirm(c.heading, c.body, ok: c.ok, destructive: c.destructive) { isomar_confirm_result(c.id, $0) }
    }

    func show(toast text: String) {
        toast = text
        toastTimer?.invalidate()
        toastTimer = Timer.scheduledTimer(withTimeInterval: 4, repeats: false) { [weak self] _ in self?.toast = nil }
    }
}

extension IsomarModel {
    /// The answer of a call as a dictionary, for the forms that edit what it describes.
    func dict(_ name: String, _ args: [String: Any] = [:]) -> [String: Any]? {
        guard let text = callText(name, args), let data = text.data(using: .utf8) else { return nil }
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    }

    /// The answer of a call as an array of dictionaries.
    func list(_ name: String, _ args: [String: Any] = [:]) -> [[String: Any]] {
        guard let text = callText(name, args), let data = text.data(using: .utf8) else { return [] }
        return ((try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]) ?? []
    }

    /// Reads a GPX file into the routes, marks and tracks.
    func importGPX() {
        choose(open: true, types: ["gpx"]) { path in
            if let e = self.send("gpx_import", ["path": path]) { Self.alert("Could not import the file", e) }
        }
    }

    /// Writes every route, mark and track to a GPX file.
    func exportGPX() {
        choose(open: false, types: ["gpx"], name: "isomar.gpx") { path in
            if let e = self.send("gpx_export", ["id": 0, "path": path]) { Self.alert("Could not write the file", e) }
        }
    }

    /// Asks for a file and hands its path to `then`: one to read (`open`), a folder, or where to write one
    /// (`name`), which `then` writes at once. On the Mac a panel; on iOS the document picker, with what it
    /// gives copied into the app (an iOS app reads only its own container), and a file written to a
    /// temporary path, then exported where the user says.
    func choose(open: Bool, types: [String] = [], directory: Bool = false, name: String = "", then: @escaping (String) -> Void) {
        #if os(macOS)
        if open || directory {
            let p = NSOpenPanel()
            p.canChooseFiles = !directory
            p.canChooseDirectories = directory
            p.allowsMultipleSelection = false
            if !types.isEmpty { p.allowedFileTypes = types }
            if p.runModal() == .OK, let path = p.url?.path { then(path) }
            return
        }
        let p = NSSavePanel()
        p.nameFieldStringValue = name
        if !types.isEmpty { p.allowedFileTypes = types }
        if p.runModal() == .OK, let path = p.url?.path { then(path) }
        #else
        DocumentPicker.shared.choose(open: open, directory: directory, name: name, then: then)
        #endif
    }

}

/// Buttons that start a job (`isomar_app::jobs`, the GTK shell's `ui/busy.rs`): while their job runs they
/// look off, with a spinner in place of their icon, and a press shows the running jobs in the activity
/// indicator instead of starting the work again. `action` is what the engine's jobs name: `route`,
/// `weather`, `autoroute:<id>`, `ensemble:<id>`, `chart:<source id>`, `perf_analyse`, `perf_backfill`.
extension IsomarModel {
    func busy(_ action: String) -> Bool {
        !action.isEmpty && window.jobs.contains { $0.action == action }
    }

    /// What a job button does: `start` unless `action`'s job runs, else shows the running jobs.
    func job(_ action: String, _ start: () -> Void) {
        if busy(action) { activityShown = true } else { start() }
    }
}

/// A job button's look while its job runs: dimmed, as a disabled button, though it still takes a press.
struct JobBusy: ViewModifier {
    let busy: Bool
    func body(content: Content) -> some View { content.opacity(busy ? 0.5 : 1) }
}

/// A job button's label: `title` beside `systemImage`, or beside a spinner in its place while it runs.
struct JobLabel: View {
    let title: String
    var systemImage: String? = nil
    let busy: Bool

    var body: some View {
        HStack(spacing: 6) {
            if busy { ProgressView().controlSize(.small) } else if let i = systemImage { Image(systemName: i) }
            Text(title)
        }
    }
}

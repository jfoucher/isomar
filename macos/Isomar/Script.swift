import SwiftUI

extension IsomarModel {
    /// The route a menu command is about: the one open in the sidebar, the one being followed, or the first.
    var currentRoute: Int64? {
        if case .route(let id)? = detail { return id }
        if case .routing(let id)? = detail { return id }
        return passage.routes.first(where: { $0.active })?.id ?? passage.routes.first?.id
    }

    func getWeather() {
        if let a = call("weather_defaults", ["kind": "view"], as: WeatherArea.self) { sheet = .getWeather(a) }
    }
}

/// `ISOMAR_SCRIPT`, on the Mac and on iOS (the simulator cannot be touched over SSH).
enum Script {
    /// Drives the app the way a person would, for checks over SSH where nothing can be clicked:
    /// `click,<button>,<x>,<y>` `tap,<button>,<x>,<y>` `pick,<menu entry>` `pinch,<scale>[,<x>,<y>]` `key,<code>` `command,<name>,<id>` `page,<name>` `call,<name>,<json>`
    /// `sheet,<name>` `route,<id>` `compute` (the weather route of the current route, leaving in an hour)
    /// `stats` `waitfor,router|departures|idle` `wait,<seconds>`, separated by `;`. For screenshots of many states in one run:
    /// `shot,<name>` asks whoever drives the simulator for a screenshot (the file `isomar-shot` in the app's
    /// caches, which it deletes once taken) and goes on when it is gone; `end` says the script is over.
    /// `detent,medium|large|away` (the phone's pages sheet) `jobs` (the phone's list of jobs) `tapll,<lat>,<lon>`
    /// (a tap there; `tapll3` a long press) `departures` (best departure for the current route)
    /// `underway,on|off` `dismiss` (the menu, the tapped thing and the task sheet).
    static func run(_ script: String) {
        let steps = script.split(separator: ";").map { $0.split(separator: ",", maxSplits: 2, omittingEmptySubsequences: false).map(String.init) }
        var i = 0
        func next() {
            guard i < steps.count else { return }
            let s = steps[i]
            i += 1
            let m = IsomarModel.shared
            switch s[0] {
            case "click":
                let p = s[1...].joined(separator: ",").split(separator: ",").map(String.init)
                isomar_click(UInt32(p[0]) ?? 1, 1, Double(p[1]) ?? 0, Double(p[2]) ?? 0)
            case "tap":
                let p = s[1...].joined(separator: ",").split(separator: ",").map(String.init)
                isomar_tap(UInt32(p[0]) ?? 1, 1, Double(p[1]) ?? 0, Double(p[2]) ?? 0)
            case "dclick":
                let p = s[1...].joined(separator: ",").split(separator: ",").map(String.init)
                isomar_click(1, 2, Double(p[0]) ?? 0, Double(p[1]) ?? 0)
            case "pinch":
                // `pinch,<scale>,<x>,<y>`: two fingers from 1 to `scale` over a second, at 60 frames a second.
                let p = s[1...].joined(separator: ",").split(separator: ",").compactMap { Double($0) }
                let to = p.first ?? 0.01, x = p.count > 2 ? p[1] : 400, y = p.count > 2 ? p[2] : 300
                isomar_magnify_begin()
                for n in 1...60 {
                    DispatchQueue.main.asyncAfter(deadline: .now() + Double(n) / 60) { isomar_magnify(pow(to, Double(n) / 60), x, y) }
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) { next() }
                return
            case "pick": m.autoPick = s[1].replacingOccurrences(of: "_", with: " ")
            case "key": _ = isomar_key(Int32(s[1]) ?? 0)
            case "command": m.command(s[1], Int64(s.count > 2 ? s[2] : "0") ?? 0)
            case "page": if let p = Page(rawValue: s[1]) { m.showPage(p) }
            case "call":
                let args = (s.count > 2 ? (try? JSONSerialization.jsonObject(with: Data(s[2].utf8))) as? [String: Any] : nil) ?? [:]
                m.send(s[1], args)
            case "route": if let id = Int64(s[1]) { m.detail = .route(id) }
            case "shot", "end":
                let file = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("isomar-shot")
                try? (s[0] == "end" ? "END" : s[1]).write(to: file, atomically: true, encoding: .utf8)
                guard s[0] == "shot" else { return }
                var waited = 0.0
                func taken() {
                    if FileManager.default.fileExists(atPath: file.path), waited < 60 {
                        waited += 0.25
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { taken() }
                    } else {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { next() }
                    }
                }
                taken()
                return
            case "detent", "jobs":
                NotificationCenter.default.post(name: .init("isomar.script." + s[0]), object: s.count > 1 ? s[1] : nil)
            case "tapll", "tapll3":
                let p = s[1...].joined(separator: ",").split(separator: ",").compactMap { Double($0) }
                if p.count == 2, let at = m.dict("screen_of", ["lat": p[0], "lon": p[1]]), let x = at["x"] as? Double, let y = at["y"] as? Double {
                    isomar_tap(s[0] == "tapll3" ? 3 : 1, 1, x, y)
                }
            case "dismiss": m.menu = nil; m.info = nil; m.sheet = nil
            case "underway": m.send("underway_set", ["on": s[1] == "on"])
            case "departures":
                if let id = m.currentRoute, let d = m.dict("window_defaults", ["id": id]) {
                    m.showRouting(id, .window)
                    m.send("window_run", ["id": id, "earliest": d["earliest"] ?? "", "days": d["days"] ?? 2, "step_hours": d["step_hours"] ?? 3, "weights": d["weights"] ?? [:], "boat": [:] as [String: Any]])
                }
            case "stats":
                // Once the preview is there (`waitfor,router` first).
                if let t = m.call("router_stats", as: StatsTable.self) { m.sheet = .stats(t) }
            case "compute":
                if let id = m.currentRoute { m.send("route_compute", ["id": id, "departure": Stamp.text(Date().addingTimeInterval(3600))]) }
            case "sheet":
                switch s[1] {
                case "weather": m.getWeather()
                case "planning": m.sheet = .planningOnly
                case "welcome": m.sheet = .welcome
                case "weatherroute": if let r = m.currentRoute { m.showRouting(r, .fixed) }
                case "bestdeparture": if let r = m.currentRoute { m.showRouting(r, .window) }
                case "autoroute": if let r = m.currentRoute { m.sheet = .autoRoute(r) }
                case "tides": m.sheet = .tides(48.38, -4.49)
                case "settings": m.sheet = .settings
                case "connections": m.sheet = .connections
                case "about": m.sheet = .about
                default: break
                }
            case "sidebar": m.sidebarVisible = s[1] == "on"
            case "window": NotificationCenter.default.post(name: .init("isomar.open" + s[1].capitalized), object: nil)
            default: break
            }
            // `waitfor,router`: until the weather route's preview is up, checking every second.
            if s[0] == "waitfor", (s[1] == "router" && m.routerCard == nil) || (s[1] == "departures" && m.departures == nil) || (s[1] == "idle" && !m.window.jobs.isEmpty) {
                i -= 1
                DispatchQueue.main.asyncAfter(deadline: .now() + 1) { next() }
                return
            }
            let wait = s[0] == "wait" ? (Double(s[1]) ?? 1) : 0.4
            DispatchQueue.main.asyncAfter(deadline: .now() + wait) { next() }
        }
        next()
    }

}

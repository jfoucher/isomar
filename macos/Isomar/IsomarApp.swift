import AppKit
import CoreText
import SwiftUI

@main
struct IsomarApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate

    var body: some Scene {
        WindowGroup {
            RootView()
        }
        .commands { ChartCommands() }

        Settings { SettingsView() }

        Window("Connections", id: "connections") { ConnectionsView() }
        Window("Data Monitor", id: "monitor") { DataMonitorView() }
        Window("Boat Polar", id: "polar") { PolarEditor() }
    }
}

/// The main window, and what opens other windows from anywhere.
struct RootView: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        MainView()
            .onReceive(NotificationCenter.default.publisher(for: .init("isomar.openPolar"))) { _ in openWindow(id: "polar") }
            .onReceive(NotificationCenter.default.publisher(for: .init("isomar.openConnections"))) { _ in openWindow(id: "connections") }
            .onReceive(NotificationCenter.default.publisher(for: .init("isomar.openMonitor"))) { _ in openWindow(id: "monitor") }
            .onReceive(NotificationCenter.default.publisher(for: .init("isomar.openSettings"))) { _ in NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil) }
    }
}

extension IsomarModel {
    func copyPosition() {
        guard let p = footer.pointer?.position else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(p, forType: .string)
        show(toast: "Position copied")
    }

    func showAbout() {
        let credits = NSMutableAttributedString(string: """
        A chart plotter for sailors. GPL-3.0-or-later.

        Isomar is not for navigation unless an official chart you have licensed covers the area.

        Data: OpenStreetMap contributors (ODbL), Open Waters seamap tiles (CC BY 4.0), OpenFreeMap and OpenMapTiles, EMODnet Bathymetry, NOAA GFS forecasts, ECMWF open data forecasts (CC BY 4.0), SHOM open data (Licence Ouverte).
        """, attributes: [.font: NSFont.systemFont(ofSize: 11)])
        NSApp.orderFrontStandardAboutPanel(options: [.credits: credits])
    }
}

/// The menu bar, as `design/ia.md` lists it.
struct ChartCommands: Commands {
    @ObservedObject var model = IsomarModel.shared
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .appInfo) { Button("About Isomar") { model.showAbout() } }
        CommandGroup(replacing: .newItem) {
            Button("New Route") { model.command("new_route") }.keyboardShortcut("n")
            Button(model.window.recording ? "Stop Recording" : "Record Track") { model.command("record_track") }.keyboardShortcut("r", modifiers: [.command, .shift])
            Divider()
            Button("Import GPX…") { model.importGPX() }.keyboardShortcut("i")
            Button("Export GPX…") { model.exportGPX() }.keyboardShortcut("e")
            Divider()
            Button("Connections…") { openWindow(id: "connections") }.keyboardShortcut("k")
            Button("Data Monitor") { openWindow(id: "monitor") }
            Button("Boat Polar…") { openWindow(id: "polar") }
        }
        CommandGroup(after: .pasteboard) {
            Button("Copy Position") { model.copyPosition() }.keyboardShortcut("c", modifiers: [.command, .shift])
        }
        CommandGroup(replacing: .sidebar) {
            Button("Show Sidebar") { model.sidebarVisible.toggle() }.keyboardShortcut("s", modifiers: [.command, .option])
            Divider()
            ForEach(Array(Page.allCases.enumerated()), id: \.element) { i, p in
                Button(p.title) { model.showPage(p) }.keyboardShortcut(KeyEquivalent(Character("\(i + 1)")))
            }
            Divider()
            Menu("Layers") {
                ForEach(model.layers) { l in Toggle(l.label, isOn: Binding(get: { l.visible }, set: { model.setLayer(l.id, visible: $0) })) }
            }
            Menu("Palette") {
                Button("Follow System") { model.setMode("auto") }
                Button("Day") { model.setMode("day") }.keyboardShortcut("1", modifiers: [.command, .shift])
                Button("Dusk") { model.setMode("dusk") }.keyboardShortcut("2", modifiers: [.command, .shift])
                Button("Night") { model.setMode("night") }.keyboardShortcut("3", modifiers: [.command, .shift])
            }
            Button("Under Way Layout") { model.send("underway_set", ["on": !model.window.underway]) }.keyboardShortcut("u")
            Button(model.follow > 0 ? "Stop Following Boat" : "Follow Boat") { model.cycleFollow() }
            Divider()
            Button("Zoom In") { model.zoom(by: 1) }.keyboardShortcut("+")
            Button("Zoom Out") { model.zoom(by: -1) }.keyboardShortcut("-")
        }
        CommandMenu("Navigate") {
            Button("Activate Route") { if let r = model.currentRoute { model.send("start_route", ["id": r]) } }.disabled(model.currentRoute == nil)
            Button("Skip Waypoint") { model.send("underway_skip") }
            Button("Stop Navigation") { model.send("underway_stop") }
            Divider()
            Button("Set Anchor Watch") { model.command("anchor_here") }
            Button("Clear Anchor Watch") { model.command("stop_watching") }
            Divider()
            Button("Weather Routing…") { if let r = model.currentRoute { model.showRouting(r) } }.disabled(model.currentRoute == nil)
            Button("Auto Route…") { if let r = model.currentRoute { model.job("autoroute:\(r)") { model.sheet = .autoRoute(r) } } }.disabled(model.currentRoute == nil)
            Divider()
            Button("Measure") { model.toggleMeasure() }
        }
        CommandMenu("Weather") {
            Button("Get Weather…") { model.job("weather") { model.getWeather() } }.keyboardShortcut("w", modifiers: [.command, .shift])
            Button("Tides Here…") { if let v = model.dict("view"), let la = v["lat"] as? Double, let lo = v["lon"] as? Double { model.sheet = .tides(la, lo) } }
            Divider()
            Button("Next Time Step") { model.send("timeline_step", ["hours": 1]) }.keyboardShortcut(.rightArrow, modifiers: .command)
            Button("Previous Time Step") { model.send("timeline_step", ["hours": -1]) }.keyboardShortcut(.leftArrow, modifiers: .command)
        }
        CommandGroup(replacing: .help) {
            Button("Not for Navigation") { model.sheet = .planningOnly }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        // The chart's font is registered before the first draw, so that pango and cairo find it.
        for name in ["AdwaitaSans-Regular", "AdwaitaSans-Italic"] {
            if let url = Bundle.main.url(forResource: name, withExtension: "ttf") {
                CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
            }
        }
        // Text is translated through gettext: the catalogues ship in Resources/locale, in the user's language.
        if let locale = Bundle.main.resourceURL?.appendingPathComponent("locale").path, ProcessInfo.processInfo.environment["ISOMAR_LOCALEDIR"] == nil {
            setenv("ISOMAR_LOCALEDIR", locale, 1)
        }
        let languages = Locale.preferredLanguages.map { $0.replacingOccurrences(of: "-", with: "_") }
        if !languages.isEmpty, getenv("LANGUAGE") == nil { setenv("LANGUAGE", languages.joined(separator: ":"), 1) }
        IsomarModel.shared.start()
        Alerting.shared.setup()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let env = ProcessInfo.processInfo.environment
        if let size = env["ISOMAR_SIZE"]?.split(separator: "x").compactMap({ Double($0) }), size.count == 2 {
            DispatchQueue.main.async { NSApp.windows.first(where: { $0.isVisible })?.setContentSize(NSSize(width: size[0], height: size[1])) }
        }
        if let script = env["ISOMAR_SCRIPT"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { Script.run(script) }
        }
        if let path = env["ISOMAR_SCREENSHOT"] {
            let delay = Double(env["ISOMAR_SCREENSHOT_DELAY"] ?? "") ?? 2000
            DispatchQueue.main.asyncAfter(deadline: .now() + delay / 1000) {
                if let kind = env["ISOMAR_BENCH"] {
                    Self.bench(kind) { Self.screenshot(to: path) }
                } else {
                    Self.screenshot(to: path)
                }
            }
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func applicationWillTerminate(_ notification: Notification) {
        IsomarModel.shared.shutdown()
    }

    /// Moves the view 60 times at 60 Hz, as a drag or a zoom would, and logs how long frames took.
    private static func bench(_ kind: String, then: @escaping () -> Void) {
        let steps = Int(ProcessInfo.processInfo.environment["ISOMAR_BENCH_STEPS"] ?? "") ?? 60
        var n = 0
        let started = Date()
        guard let view = IsomarModel.shared.chartView as? ChartNSView else { return then() }
        let centre = (x: Double(view.bounds.width) / 2, y: Double(view.bounds.height) / 2)
        isomar_frames_start()
        let cpu = Self.cpuSeconds()
        let threads = Self.cpuByThread()
        if kind == "pan" { isomar_drag_begin(2, centre.x, centre.y) }
        Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) { timer in
            n += 1
            if n <= steps {
                if kind == "pan" {
                    isomar_drag_update(Double(n) * 20, Double(n) * 5)
                } else {
                    isomar_scroll_zoom(n <= steps / 2 ? -0.1 : 0.1, centre.x, centre.y)
                }
                return
            }
            if kind == "pan" && n == steps + 1 { isomar_drag_end(Double(steps) * 20, Double(steps) * 5) }
            // A last half second for the redraw that follows the gesture.
            if Date().timeIntervalSince(started) < Double(steps) / 60 + 0.5 { return }
            timer.invalidate()
            let now = Self.cpuByThread()
            let used = now.map { ($0.key, $0.value - (threads[$0.key] ?? 0)) }.filter { $0.1 >= 0.005 }.sorted { $0.1 > $1.1 }
            let report = String(cString: isomar_frames_report()) + String(format: ", cpu %.2f s", Self.cpuSeconds() - cpu)
                + " [" + used.map { String(format: "%@ %.2f", $0.0, $0.1) }.joined(separator: ", ") + "]"
            FileHandle.standardError.write(Data("isomar: bench \(kind) over \(String(format: "%.1f", Date().timeIntervalSince(started))) s: \(report)\n".utf8))
            try? "isomar: bench \(kind): \(report)\n".write(toFile: "/tmp/isomar-bench.txt", atomically: true, encoding: .utf8)
            then()
        }
    }

    /// CPU time each thread has used, in seconds, by thread name with its digits left out (`isomar-tile-`
    /// for all the tile workers; `main` for the main thread, unnamed ones as `other`). Threads that ended
    /// are not counted.
    private static func cpuByThread() -> [String: Double] {
        var out: [String: Double] = [:]
        var list: thread_act_array_t?
        var count: mach_msg_type_number_t = 0
        guard task_threads(mach_task_self_, &list, &count) == KERN_SUCCESS, let list else { return out }
        defer { vm_deallocate(mach_task_self_, vm_address_t(bitPattern: list), vm_size_t(Int(count) * MemoryLayout<thread_t>.size)) }
        for i in 0..<Int(count) {
            var info = thread_basic_info()
            var size = mach_msg_type_number_t(MemoryLayout<thread_basic_info>.size / MemoryLayout<integer_t>.size)
            let ok = withUnsafeMutablePointer(to: &info) { $0.withMemoryRebound(to: integer_t.self, capacity: Int(size)) { thread_info(list[i], thread_flavor_t(THREAD_BASIC_INFO), $0, &size) } }
            guard ok == KERN_SUCCESS else { continue }
            var name = "other"
            if let pt = pthread_from_mach_thread_np(list[i]) {
                var buf = [CChar](repeating: 0, count: 64)
                if pthread_getname_np(pt, &buf, 64) == 0, buf[0] != 0 { name = String(cString: buf).filter { !$0.isNumber } }
                if pthread_main_np() != 0 && pt == pthread_self() { name = "main" }
            }
            let t = Double(info.user_time.seconds) + Double(info.user_time.microseconds) / 1e6 + Double(info.system_time.seconds) + Double(info.system_time.microseconds) / 1e6
            out[name, default: 0] += t
            mach_port_deallocate(mach_task_self_, list[i])
        }
        return out
    }

    /// CPU time the process has used, in seconds.
    private static func cpuSeconds() -> Double {
        var u = rusage()
        getrusage(RUSAGE_SELF, &u)
        let t = { (v: timeval) in Double(v.tv_sec) + Double(v.tv_usec) / 1e6 }
        return t(u.ru_utime) + t(u.ru_stime)
    }

    /// Renders the window's content to a PNG and quits, so a build over SSH needs no screen recording permission.
    private static func screenshot(to path: String) {
        defer {
            if ProcessInfo.processInfo.environment["ISOMAR_KEEP_OPEN"] == nil {
                NSApp.terminate(nil)
                // A sheet over the window can hold termination back.
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    IsomarModel.shared.shutdown()
                    exit(0)
                }
            }
        }
        guard let window = NSApp.windows.first(where: { $0.isVisible && $0.canBecomeMain }), let frame = window.contentView?.superview ?? window.contentView else {
            FileHandle.standardError.write(Data("isomar: no window to capture\n".utf8))
            return
        }
        // The window as the window server shows it (an app may capture its own windows without asking).
        // `cacheDisplay` redraws the views instead, and left a scrolled sidebar at the top.
        // With the display asleep (a Mac over SSH) the window server has nothing to give: all black.
        func blank(_ rep: NSBitmapImageRep) -> Bool {
            let (w, h) = (rep.pixelsWide, rep.pixelsHigh)
            return stride(from: 1, to: 10, by: 1).allSatisfy { i in
                guard let c = rep.colorAt(x: w * i / 10, y: h * i / 10)?.usingColorSpace(.sRGB) else { return true }
                return c.alphaComponent < 0.01 || c.redComponent + c.greenComponent + c.blueComponent < 0.01
            }
        }
        if let cg = CGWindowListCreateImage(.null, .optionIncludingWindow, CGWindowID(window.windowNumber), [.boundsIgnoreFraming, .bestResolution]),
           case let grab = NSBitmapImageRep(cgImage: cg), !blank(grab),
           let png = grab.representation(using: .png, properties: [:]) {
            try? png.write(to: URL(fileURLWithPath: path))
        } else if let rep = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) {
            frame.cacheDisplay(in: frame.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
        }
        // Other windows (settings, connections) are saved as `<path>.window-N.png`.
        for (n, w) in NSApp.windows.filter({ $0.isVisible && $0 !== window && $0.contentView != nil && $0.frame.width > 200 }).enumerated() {
            guard let v = w.contentView?.superview ?? w.contentView, let r = v.bitmapImageRepForCachingDisplay(in: v.bounds) else { continue }
            v.cacheDisplay(in: v.bounds, to: r)
            try? r.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path + ".window-\(n).png"))
        }
        // A sheet is a window of its own: it is saved beside the first as `<path>.sheet.png`.
        if let sheet = window.attachedSheet?.contentView, let r = sheet.bitmapImageRepForCachingDisplay(in: sheet.bounds) {
            sheet.cacheDisplay(in: sheet.bounds, to: r)
            try? r.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path + ".sheet.png"))
        }
    }
}

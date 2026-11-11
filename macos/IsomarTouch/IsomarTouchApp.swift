import CoreLocation
import SwiftUI
import UIKit

/// The iPhone and iPad app: the Mac app's model, pages and sheets (macos/Isomar) in the touch layout of
/// design/ia.md, "Phones, foldables and tablets": the phone's sheet under 600 pt, the open iPhone Duo's halves,
/// the iPad's sidebar (TabletRoot.swift).
@main
struct IsomarTouchApp: App {
    @UIApplicationDelegateAdaptor(TouchAppDelegate.self) private var delegate

    var body: some Scene {
        WindowGroup {
            TouchRoot(model: .shared)
                .onAppear { Theme.apply(IsomarModel.shared.mode) }
                .onReceive(NotificationCenter.default.publisher(for: UIApplication.willResignActiveNotification)) { _ in
                    // iOS may end a suspended app without telling it: the view and the settings are saved now.
                    IsomarModel.shared.send("save_state")
                    Dimmer.update(active: false)
                }
                .onReceive(NotificationCenter.default.publisher(for: UIApplication.didBecomeActiveNotification)) { _ in Dimmer.update(active: true) }
        }
    }
}

final class TouchAppDelegate: NSObject, UIApplicationDelegate {
    /// hit-min-touch (data/design/tokens.json): 44 pt.
    static let touchHitMin = Size.hitMinTouch

    func application(_ app: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        Self.prepareEnvironment()
        IsomarModel.shared.start()
        Alerting.shared.setup()
        isomar_set_touch(Self.touchHitMin)
        // A phone's memory is shared with the GPU and jetsam ends apps that take much: less than a desktop's 192 MB.
        isomar_set_tile_cache_limit(UInt64(ProcessInfo.processInfo.physicalMemory <= 4 << 30 ? 64 << 20 : 96 << 20))
        NotificationCenter.default.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: .main) { _ in
            isomar_set_tile_cache_limit(32 << 20)
        }
        // Not for a screenshot: the system's question would cover it.
        if ProcessInfo.processInfo.environment["ISOMAR_SCREENSHOT"] == nil { DeviceLocation.shared.start() }
        if let script = ProcessInfo.processInfo.environment["ISOMAR_SCRIPT"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { Script.run(script) }
        }
        return true
    }

    /// Where everything lives in the app's container, and a configuration for fontconfig: pango finds its
    /// fonts through fontconfig on iOS (its CoreText backend is for macOS only): Adwaita Sans from the
    /// bundle, then the system's.
    private static func prepareEnvironment() {
        let fm = FileManager.default
        let support = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Isomar")
        let caches = fm.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("Isomar")
        func dir(_ base: URL, _ name: String) -> String {
            let u = base.appendingPathComponent(name)
            try? fm.createDirectory(at: u, withIntermediateDirectories: true)
            return u.path
        }
        let env = ProcessInfo.processInfo.environment
        for (key, path) in [("ISOMAR_CONFIG_DIR", dir(support, "config")), ("ISOMAR_STATE_DIR", dir(support, "state")), ("ISOMAR_DATA_DIR", dir(support, "data")), ("ISOMAR_CACHE_DIR", dir(caches, "isomar"))] where env[key] == nil {
            setenv(key, path, 1)
        }
        if let locale = Bundle.main.resourceURL?.appendingPathComponent("locale").path, env["ISOMAR_LOCALEDIR"] == nil {
            setenv("ISOMAR_LOCALEDIR", locale, 1)
        }
        let languages = Locale.preferredLanguages.map { $0.replacingOccurrences(of: "-", with: "_") }
        if !languages.isEmpty, env["LANGUAGE"] == nil { setenv("LANGUAGE", languages.joined(separator: ":"), 1) }
        let fonts = Bundle.main.resourceURL!.appendingPathComponent("Fonts").path
        let conf = """
        <?xml version="1.0"?>
        <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
        <fontconfig>
          <dir>\(fonts)</dir>
          <dir>/System/Library/Fonts</dir>
          <cachedir>\(dir(caches, "fontconfig"))</cachedir>
          <alias>
            <family>sans-serif</family>
            <prefer><family>Adwaita Sans</family><family>Helvetica Neue</family></prefer>
          </alias>
        </fontconfig>
        """
        let path = support.appendingPathComponent("fonts.conf")
        try? conf.write(to: path, atomically: true, encoding: .utf8)
        setenv("FONTCONFIG_FILE", path.path, 1)
    }
}

/// The phone's GPS as a source of fixes for the engine (source "device"), once the user allows it.
final class DeviceLocation: NSObject, CLLocationManagerDelegate {
    static let shared = DeviceLocation()
    private let manager = CLLocationManager()

    func start() {
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyBest
        manager.requestWhenInUseAuthorization()
    }

    /// While something is watched (an anchor watch, a route, a track) the fixes go on with the app out of
    /// sight, which the location background mode allows; the system shows that it does.
    func setWatching(_ on: Bool) {
        let modes = Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String] ?? []
        guard modes.contains("location") else { return }
        manager.allowsBackgroundLocationUpdates = on
        manager.showsBackgroundLocationIndicator = on
        manager.pausesLocationUpdatesAutomatically = !on
    }

    func locationManagerDidChangeAuthorization(_ m: CLLocationManager) {
        switch m.authorizationStatus {
        case .authorizedWhenInUse, .authorizedAlways: m.startUpdatingLocation()
        default: m.stopUpdatingLocation()
        }
    }

    func locationManager(_ m: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        for l in locations where l.horizontalAccuracy >= 0 {
            let sog = l.speed >= 0 ? l.speed * 3600 / 1852 : .nan
            let cog = l.course >= 0 ? l.course : .nan
            isomar_location(l.coordinate.latitude, l.coordinate.longitude, sog, cog, l.horizontalAccuracy, Int64(l.timestamp.timeIntervalSince1970 * 1000))
        }
    }
}

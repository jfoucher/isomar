import AppKit
import UserNotifications

/// The three looks, the system's appearance and the engine's palette move together.
enum Theme {
    private static var observer: NSKeyValueObservation?

    static func apply(_ mode: String) {
        guard NSApp != nil else { return }
        switch mode {
        case "day":
            IsomarTheme.current = .day
            NSApp.appearance = NSAppearance(named: .aqua)
        case "dusk":
            IsomarTheme.current = .dusk
            NSApp.appearance = NSAppearance(named: .darkAqua)
        case "night":
            IsomarTheme.current = .night
            NSApp.appearance = NSAppearance(named: .darkAqua)
        default:
            NSApp.appearance = nil
            IsomarTheme.current = systemIsDark ? .dusk : .day
        }
        if observer == nil {
            // Following the system: the look changes when the system's does.
            observer = NSApp.observe(\.effectiveAppearance) { _, _ in
                DispatchQueue.main.async {
                    IsomarModel.shared.systemDark = systemIsDark
                    if IsomarModel.shared.mode == "auto" { IsomarTheme.current = systemIsDark ? .dusk : .day }
                    IsomarModel.shared.chartView?.setNeedsRedraw()
                }
            }
        }
        IsomarModel.shared.systemDark = systemIsDark
        IsomarModel.shared.chartView?.setNeedsRedraw()
    }

    static var systemIsDark: Bool {
        NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
    }

    /// Night has solid chrome: no materials.
    static var isNight: Bool { IsomarTheme.current == .night }
}

/// The app's sounds ship with it, so they work offline.
enum Sounds {
    private static var cache: [String: NSSound] = [:]

    static func play(_ name: String) {
        if let s = cache[name] {
            s.stop()
            s.play()
            return
        }
        guard let url = Bundle.main.url(forResource: name, withExtension: "wav"), let s = NSSound(contentsOf: url, byReference: true) else {
            NSSound.beep()
            return
        }
        cache[name] = s
        s.play()
    }
}

/// What happens when an alarm arrives while the app is in the background, and keeping the machine awake.
final class Alerting: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Alerting()
    private var notified: Int64?
    private var activity: NSObjectProtocol?
    private var authorised = false
    private var asked = false

    private var available: Bool { Bundle.main.bundleIdentifier != nil && ProcessInfo.processInfo.environment["ISOMAR_SCREENSHOT"] == nil }

    func setup() {
        guard available else { return }
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        let ack = UNNotificationAction(identifier: "ACK", title: "Acknowledge", options: [])
        center.setNotificationCategories([UNNotificationCategory(identifier: "ALARM", actions: [ack], intentIdentifiers: [], options: [])])
    }

    func update(alerts: AlertsInfo, window: WindowInfo) {
        // The machine stays awake while data arrives or an anchor watch is set.
        if window.keepAwake, activity == nil {
            activity = ProcessInfo.processInfo.beginActivity(options: [.idleSystemSleepDisabled, .userInitiated], reason: "Isomar is following the boat")
        } else if !window.keepAwake, let a = activity {
            ProcessInfo.processInfo.endActivity(a)
            activity = nil
        }
        guard let alarm = alerts.alarm, !alarm.dim else {
            if notified != nil {
                notified = nil
                UNUserNotificationCenter.current().removeAllDeliveredNotifications()
            }
            return
        }
        guard !NSApp.isActive, notified != alarm.id else { return }
        notified = alarm.id
        NSApp.requestUserAttention(.criticalRequest)
        guard available else { return }
        whenAllowed {
            let c = UNMutableNotificationContent()
            c.title = "Isomar"
            c.body = alarm.title
            c.categoryIdentifier = "ALARM"
            c.interruptionLevel = .timeSensitive
            c.userInfo = ["id": alarm.id]
            UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "alarm", content: c, trigger: nil))
        }
    }

    /// A long calculation ended (`AppState::finished`; its sound has played): a notification and a
    /// bounce of the Dock icon, while the app is in the background.
    func notify(_ title: String, _ body: String) {
        guard !NSApp.isActive else { return }
        NSApp.requestUserAttention(.informationalRequest)
        guard available else { return }
        whenAllowed {
            let c = UNMutableNotificationContent()
            c.title = title
            c.body = body
            UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "finished", content: c, trigger: nil))
        }
    }

    /// Runs `post` once notifications are allowed, asking the first time.
    private func whenAllowed(_ post: @escaping () -> Void) {
        if authorised {
            post()
        } else if !asked {
            asked = true
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .timeSensitive]) { ok, _ in
                self.authorised = ok
                if ok { post() }
            }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler done: @escaping () -> Void) {
        if response.actionIdentifier == "ACK", let id = response.notification.request.content.userInfo["id"] as? Int64 {
            DispatchQueue.main.async { IsomarModel.shared.command("alert_acknowledge", id) }
        }
        done()
    }
}

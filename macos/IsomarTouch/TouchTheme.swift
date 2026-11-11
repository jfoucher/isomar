import AVFoundation
import SwiftUI
import UIKit
import UserNotifications

/// The three looks, the system's appearance and the engine's palette move together, as on the Mac
/// (macos/Isomar/Theme.swift): the windows are told Day is light and Dusk and Night dark.
enum Theme {
    static func apply(_ mode: String) {
        let style: UIUserInterfaceStyle
        switch mode {
        case "day": IsomarTheme.current = .day; style = .light
        case "dusk": IsomarTheme.current = .dusk; style = .dark
        case "night": IsomarTheme.current = .night; style = .dark
        default: IsomarTheme.current = systemIsDark ? .dusk : .day; style = .unspecified
        }
        for scene in UIApplication.shared.connectedScenes {
            for w in (scene as? UIWindowScene)?.windows ?? [] { w.overrideUserInterfaceStyle = style }
        }
        IsomarModel.shared.systemDark = systemIsDark
        IsomarModel.shared.chartView?.setNeedsRedraw()
        Dimmer.update()
    }

    /// The system's own appearance, whatever the app's windows were told.
    static var systemIsDark: Bool { UIScreen.main.traitCollection.userInterfaceStyle == .dark }

    /// Night has solid chrome: no materials.
    static var isNight: Bool { IsomarTheme.current == .night }

    /// The system's appearance changed: the look follows it while the mode is "auto".
    static func systemChanged() {
        IsomarModel.shared.systemDark = systemIsDark
        if IsomarModel.shared.mode == "auto" { IsomarTheme.current = systemIsDark ? .dusk : .day }
        IsomarModel.shared.chartView?.setNeedsRedraw()
    }
}

/// Night dims the screen to its lowest for eyes used to the dark (a phone preference, on by default), and
/// gives the brightness back on leaving Night or the app: iOS keeps an app's brightness system-wide.
enum Dimmer {
    static var enabled: Bool {
        get { UserDefaults.standard.object(forKey: "dimAtNight") as? Bool ?? true }
        set { UserDefaults.standard.set(newValue, forKey: "dimAtNight"); update() }
    }
    private static var saved: CGFloat?

    static func update(active: Bool = UIApplication.shared.applicationState != .background) {
        let want = active && enabled && Theme.isNight
        if want, saved == nil {
            saved = UIScreen.main.brightness
            UIScreen.main.brightness = 0.02
        } else if !want, let b = saved {
            UIScreen.main.brightness = b
            saved = nil
        }
    }
}

/// The app's sounds ship with it, so they work offline. They play through the silent switch: an alarm
/// that a phone set to silent does not sound is no alarm.
enum Sounds {
    private static var cache: [String: AVAudioPlayer] = [:]

    static func play(_ name: String) {
        try? AVAudioSession.sharedInstance().setCategory(.playback, options: [.mixWithOthers])
        if let p = cache[name] {
            p.currentTime = 0
            p.play()
            return
        }
        guard let url = Bundle.main.url(forResource: name, withExtension: "wav"), let p = try? AVAudioPlayer(contentsOf: url) else { return }
        cache[name] = p
        p.play()
    }
}

/// What happens when an alarm arrives while the app is in the background, and keeping the screen on.
final class Alerting: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Alerting()
    private var notified: Int64?
    private var authorised = false
    private var asked = false
    private var watching = false
    /// While a job runs: the time iOS lends an app that leaves the screen, so a calculation can end and say so.
    private var background: UIBackgroundTaskIdentifier = .invalid

    func setup() {
        guard available else { return }
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        let ack = UNNotificationAction(identifier: "ACK", title: "Acknowledge", options: [])
        center.setNotificationCategories([UNNotificationCategory(identifier: "ALARM", actions: [ack], intentIdentifiers: [], options: [])])
    }

    /// Not while taking a screenshot: the system's question would cover it.
    private var available: Bool { ProcessInfo.processInfo.environment["ISOMAR_SCREENSHOT"] == nil }

    func update(alerts: AlertsInfo, window: WindowInfo) {
        // The screen stays on while navigating; with the screen off, a watch goes on in the background.
        if UIApplication.shared.isIdleTimerDisabled != window.underway { UIApplication.shared.isIdleTimerDisabled = window.underway }
        if window.watching != watching {
            watching = window.watching
            DeviceLocation.shared.setWatching(watching)
        }
        if !window.jobs.isEmpty, background == .invalid {
            background = UIApplication.shared.beginBackgroundTask(withName: "Isomar calculation") { [weak self] in self?.endBackground() }
            // Asked now, while the user is looking: from the background the question could not show.
            if available, !asked { whenAllowed {} }
        } else if window.jobs.isEmpty, background != .invalid {
            endBackground()
        }
        guard available else { return }
        guard let alarm = alerts.alarm, !alarm.dim else {
            if notified != nil {
                notified = nil
                UNUserNotificationCenter.current().removeAllDeliveredNotifications()
            }
            return
        }
        guard UIApplication.shared.applicationState != .active, notified != alarm.id else { return }
        notified = alarm.id
        whenAllowed {
            let c = UNMutableNotificationContent()
            c.title = "Isomar"
            c.body = alarm.title
            c.categoryIdentifier = "ALARM"
            c.sound = UNNotificationSound(named: UNNotificationSoundName("alarm.wav"))
            if #available(iOS 15.0, *) { c.interruptionLevel = .timeSensitive }
            c.userInfo = ["id": alarm.id]
            UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "alarm", content: c, trigger: nil))
        }
    }

    /// A long calculation ended (`AppState::finished`): a notification, with the app's soft sound,
    /// while the app is out of sight (where its own sound may not play).
    func notify(_ title: String, _ body: String) {
        guard available, UIApplication.shared.applicationState != .active else { return }
        whenAllowed {
            let c = UNMutableNotificationContent()
            c.title = title
            c.body = body
            c.sound = UNNotificationSound(named: UNNotificationSoundName("tick.wav"))
            UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "finished", content: c, trigger: nil))
        }
    }

    /// Runs `post` once notifications are allowed, asking the first time.
    private func whenAllowed(_ post: @escaping () -> Void) {
        if authorised {
            post()
        } else if !asked {
            asked = true
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { ok, _ in
                self.authorised = ok
                if ok { post() }
            }
        }
    }

    private func endBackground() {
        UIApplication.shared.endBackgroundTask(background)
        background = .invalid
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler done: @escaping () -> Void) {
        if response.actionIdentifier == "ACK", let id = response.notification.request.content.userInfo["id"] as? Int64 {
            DispatchQueue.main.async { IsomarModel.shared.command("alert_acknowledge", id) }
        }
        done()
    }
}

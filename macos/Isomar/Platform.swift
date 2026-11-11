#if os(macOS)
import AppKit
#else
import UIKit
#endif
import SwiftUI

/// What the model asks of the chart view, the Mac's `ChartNSView` or the touch screens' `TouchChartView`.
protocol ChartSurface: AnyObject {
    func setNeedsRedraw()
    /// The actions for a place or object (a right click, a long press).
    func showMenu(_ request: MenuRequest)
    /// What was clicked or tapped (a popover on the Mac, the sheet on a phone).
    func showInfo(_ info: InfoRequest)
    /// The pointer is over something a click opens (the Mac's cursor; nothing on a touch screen).
    func setPointing(_ on: Bool)
}

/// The few things the shared views and the model do differently on the Mac and on iOS.
enum Platform {
    static var isMac: Bool {
        #if os(macOS)
        true
        #else
        false
        #endif
    }

    static var reduceMotion: Bool {
        #if os(macOS)
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        #else
        UIAccessibility.isReduceMotionEnabled
        #endif
    }

    static func copy(_ text: String) {
        #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        #else
        UIPasteboard.general.string = text
        #endif
    }

    static func open(_ url: URL) {
        #if os(macOS)
        NSWorkspace.shared.open(url)
        #else
        UIApplication.shared.open(url)
        #endif
    }

    #if !os(macOS)
    /// The controller to present from: the key window's, or what it already presents.
    static var presenter: UIViewController? {
        let root = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow?.rootViewController }.first
        var top = root
        while let p = top?.presentedViewController { top = p }
        return top
    }
    #endif

    static func alert(_ heading: String, _ details: String) {
        #if os(macOS)
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = heading
        alert.informativeText = details
        if let w = NSApp.keyWindow ?? NSApp.windows.first(where: { $0.isVisible }) {
            alert.beginSheetModal(for: w)
        } else {
            alert.runModal()
        }
        #else
        let a = UIAlertController(title: heading, message: details, preferredStyle: .alert)
        a.addAction(UIAlertAction(title: "OK", style: .default))
        presenter?.present(a, animated: true)
        #endif
    }

    /// Asks a question with an OK button of `ok` and Cancel; `done` hears the answer.
    static func confirm(_ heading: String, _ body: String, ok: String, destructive: Bool, done: @escaping (Bool) -> Void) {
        #if os(macOS)
        let alert = NSAlert()
        alert.messageText = heading
        if !body.isEmpty { alert.informativeText = body }
        let button = alert.addButton(withTitle: ok)
        if destructive { button.hasDestructiveAction = true }
        alert.addButton(withTitle: "Cancel")
        if let w = NSApp.keyWindow ?? NSApp.windows.first(where: { $0.isVisible }) {
            alert.beginSheetModal(for: w) { resp in done(resp == .alertFirstButtonReturn) }
        } else {
            done(alert.runModal() == .alertFirstButtonReturn)
        }
        #else
        let a = UIAlertController(title: heading, message: body.isEmpty ? nil : body, preferredStyle: .alert)
        a.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in done(false) })
        a.addAction(UIAlertAction(title: ok, style: destructive ? .destructive : .default) { _ in done(true) })
        guard let p = presenter else { return done(false) }
        p.present(a, animated: true)
        #endif
    }
}

extension View {
    /// A checkbox on the Mac, the system's switch elsewhere.
    @ViewBuilder func checkboxStyle() -> some View {
        #if os(macOS)
        self.toggleStyle(.checkbox)
        #else
        self
        #endif
    }

    /// A menu that looks like plain text on the Mac, the system's menu button elsewhere.
    @ViewBuilder func borderlessMenu() -> some View {
        #if os(macOS)
        self.menuStyle(.borderlessButton)
        #else
        self
        #endif
    }
}

extension View {
    /// A sheet's width: fixed on the Mac, where it is a window's sheet; the screen's on a phone or tablet.
    @ViewBuilder func sheetWidth(_ width: CGFloat) -> some View {
        #if os(macOS)
        self.frame(width: width)
        #else
        self.frame(maxWidth: .infinity, alignment: .leading)
        #endif
    }
}

import AppKit
import SwiftUI

/// The one Settings window, shown from the app menu (⌘,).
@MainActor
final class SettingsWindowController: NSWindowController {
    static let shared = SettingsWindowController(settings: .shared)

    init(settings: TutorSettings) {
        let host = NSHostingController(rootView: SettingsView(settings: settings))
        let window = NSWindow(contentViewController: host)
        window.title = "Settings"
        window.styleMask = [.titled, .closable]
        window.setFrameAutosaveName("RomlensSettings")
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    func show() {
        if window?.isVisible != true { window?.center() }
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }
}

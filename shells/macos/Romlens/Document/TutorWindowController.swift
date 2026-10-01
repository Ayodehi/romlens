import AppKit
import SwiftUI

/// The tutor's own window (docs/24, the user's choice: a window beside the
/// main one, not a pane in it).
@MainActor
final class TutorWindowController: NSWindowController, NSWindowDelegate {
    let tutor: TutorModel

    init(tutor: TutorModel, title: String) {
        self.tutor = tutor
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered, defer: false)
        window.minSize = NSSize(width: 380, height: 360)
        window.title = "Tutor — \(title)"
        window.tabbingMode = .disallowed
        window.setFrameAutosaveName("RomlensTutor")
        super.init(window: window)
        window.contentViewController = NSHostingController(rootView: TutorView(tutor: tutor))
        window.delegate = self
        shouldCascadeWindows = false
        if window.frame.origin == .zero { window.center() }
        showTitle()
    }

    /// The conversation's name under the window's title, kept up to date.
    private func showTitle() {
        let title = withObservationTracking { tutor.title } onChange: { [weak self] in
            Task { @MainActor in self?.showTitle() }
        }
        window?.subtitle = title ?? ""
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    /// The tutor window never closes the project.
    override var shouldCloseDocument: Bool {
        get { false }
        set {}
    }

    /// Closing the window stops the turn; the conversation stays for when
    /// it shows again.
    func windowWillClose(_ notification: Notification) {
        tutor.close()
    }
}

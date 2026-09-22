import AppKit
import RomlensKit
import os

/// The standard About panel, given a credits string that carries the core API
/// version the shell was built against (`docs/15` row 0.12, `docs/08` rule 6).
///
/// The shell and the core are separate artefacts: the app links whichever
/// XCFramework was built beside it, so the version in a bug report has to come
/// from the core at runtime and never from a constant compiled into Swift.
enum AboutPanel {
    static let log = Logger(subsystem: "io.github.ayodehi.Romlens", category: "app")

    /// Split out and pure so the test bundle can assert the version reaches the
    /// panel without opening a window.
    static func credits(coreVersion: String = apiVersion()) -> String {
        "Core API \(coreVersion)\nA study tool for SNES ROM images. Ships no ROM data."
    }

    /// Logged once at launch, so a version is recoverable from a console log
    /// even when the reporter never opened the About box.
    static func logVersions(coreVersion: String = apiVersion()) {
        let info = Bundle.main.infoDictionary
        let short = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        log.notice("Romlens \(short, privacy: .public) (\(build, privacy: .public)), core API \(coreVersion, privacy: .public)")
    }

    static func show() {
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
            .foregroundColor: NSColor.secondaryLabelColor,
        ]
        NSApp.orderFrontStandardAboutPanel(options: [
            .credits: NSAttributedString(string: credits(), attributes: attributes)
        ])
    }
}

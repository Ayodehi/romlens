import AppKit
import RomlensKit
import Testing
@testable import Romlens

/// Checklist row 0.12: the About box shows the core API version the shell was
/// built against. The panel itself cannot be inspected here (no screenshots on
/// this machine, `docs/15`), so the credits string is tested directly and the
/// menu is checked to route to it.
@MainActor
@Suite struct AboutPanelTests {
    @Test func creditsCarryTheRunningCoreVersion() {
        let credits = AboutPanel.credits()
        #expect(credits.contains(apiVersion()))
        // The version has to come from the core at runtime, not a literal.
        #expect(AboutPanel.credits(coreVersion: "9.9.9").contains("9.9.9"))
    }

    @Test func aboutItemRoutesToTheDelegate() throws {
        let app = try #require(MainMenu.build().items.first?.submenu)
        let about = try #require(app.items.first { $0.title.hasPrefix("About ") })
        #expect(about.action == #selector(AppDelegate.showAboutPanel(_:)))
        #expect(about.target == nil)  // walks the responder chain to the delegate
        #expect(AppDelegate.instancesRespond(to: about.action))
    }
}

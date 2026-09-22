import AppKit

/// Installed by `main.swift`. (`@main` on an `NSApplicationDelegate` only
/// instantiates the delegate from a main nib, and this app has none.)
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    // Instantiating the subclass first makes it the shared controller.
    private var documentController: ProjectDocumentController?

    func applicationWillFinishLaunching(_ notification: Notification) {
        documentController = ProjectDocumentController()
        NSApp.mainMenu = MainMenu.build()
        AboutPanel.logVersions()
    }

    /// The About item targets nothing, so it walks the responder chain to the
    /// application and on to this delegate; that is the only hook the standard
    /// panel offers for supplying credits.
    @objc func showAboutPanel(_ sender: Any?) {
        AboutPanel.show()
    }

    func applicationShouldOpenUntitledFile(_ sender: NSApplication) -> Bool {
        // There is no empty project: the controller turns this into an Open
        // panel instead.
        true
    }
}

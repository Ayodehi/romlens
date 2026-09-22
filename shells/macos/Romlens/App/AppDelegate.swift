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
    }

    func applicationShouldOpenUntitledFile(_ sender: NSApplication) -> Bool {
        // There is no empty project: the controller turns this into an Open
        // panel instead.
        true
    }
}

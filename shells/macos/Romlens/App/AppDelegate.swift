import AppKit

@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    // Instantiating the subclass first makes it the shared controller.
    private var documentController: RomDocumentController?

    func applicationWillFinishLaunching(_ notification: Notification) {
        documentController = RomDocumentController()
        NSApp.mainMenu = MainMenu.build()
    }

    func applicationShouldOpenUntitledFile(_ sender: NSApplication) -> Bool {
        // A ROM viewer has no "untitled" document; the controller turns this
        // into an Open panel instead.
        true
    }
}

/// Routes "new/untitled" requests (launch with no files, Dock click with no
/// windows) to the Open panel, since documents are always read from a file.
final class RomDocumentController: NSDocumentController {
    override func openUntitledDocumentAndDisplay(_ displayDocument: Bool) throws -> NSDocument {
        openDocument(nil)
        throw CocoaError(.userCancelled)
    }
}

import AppKit
import RomlensKit
import UniformTypeIdentifiers

/// File › Open Recording…: attach a `.romrec` to the document's graphics
/// views.
///
/// A recording holds VRAM, CGRAM and OAM — the game's assets — so it is read
/// where it is and never copied into the project (`12-content-policy.md`
/// rule 4). One made from a different ROM is refused with the core's
/// message, as a mismatched project package is.
@MainActor
enum RecordingController {
    static func open(model: RomViewModel, window: NSWindow?) {
        let panel = NSOpenPanel()
        panel.title = "Open Recording"
        panel.message = "A .romrec recording of this ROM, from the Mesen2 recorder, romlens rec import-raw or romlens testrec. It stays where it is; nothing is copied into the project."
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [UTType(filenameExtension: "romrec") ?? .data]
        panel.allowsOtherFileTypes = true
        let host = window ?? NSApp.keyWindow
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let url = panel.url else { return }
            attach(url: url, model: model, window: host)
        }
        if let host {
            panel.beginSheetModal(for: host, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }

    /// Attach, reporting the core's message on failure. Split out so tests
    /// can drive it without a panel.
    @discardableResult
    static func attach(url: URL, model: RomViewModel, window: NSWindow?) -> Bool {
        do {
            let session = try RecordingSession.open(path: url.path, recover: false)
            try model.graphics.attach(session, name: url.lastPathComponent)
            if model.graphicsTab == nil { model.graphicsTab = .tilemap }
            return true
        } catch {
            let alert = NSAlert()
            alert.messageText = "The recording could not be opened"
            alert.informativeText = (error as? RomlensError).map(message) ?? error.localizedDescription
            alert.alertStyle = .warning
            if let window {
                alert.beginSheetModal(for: window)
            } else {
                alert.runModal()
            }
            return false
        }
    }

    private static func message(_ e: RomlensError) -> String {
        switch e {
        case .Io(let msg), .InvalidRom(let msg), .BadAddress(let msg), .Project(let msg),
             .RomMismatch(let msg), .InvalidLabel(let msg), .Recording(let msg):
            msg
        case .Cancelled:
            "cancelled"
        }
    }
}

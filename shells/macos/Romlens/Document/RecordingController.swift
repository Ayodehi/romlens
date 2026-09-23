import AppKit
import RomlensKit
import UniformTypeIdentifiers

/// Recordings in the shell: File › Open Recording…, Import Snapshot… and
/// Export Frame Region…, and Help › Save Mesen Recorder Script….
///
/// A recording holds VRAM, CGRAM and OAM — the game's assets — so it is read
/// where it is and never copied into the project (`12-content-policy.md`
/// rule 4); the project keeps its path and fingerprint. One made from a
/// different ROM is refused with the core's message, as a mismatched project
/// package is, and one the validator finds broken is refused with the
/// validator's words.
@MainActor
enum RecordingController {
    static let recordingType = UTType(filenameExtension: "romrec") ?? .data

    static func open(model: RomViewModel, window: NSWindow?) {
        let panel = NSOpenPanel()
        panel.title = "Open Recording"
        panel.message = "A .romrec recording of this ROM, from the Mesen recorder (Help › Save Mesen Recorder Script…), Import Snapshot… or romlens testrec. It stays where it is; the project remembers where."
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [recordingType]
        panel.allowsOtherFileTypes = true
        run(panel, on: window) { url in attach(url: url, model: model, window: window) }
    }

    /// Open, validate and attach, reporting any refusal. Split out so tests
    /// can drive it without a panel.
    @discardableResult
    static func attach(url: URL, model: RomViewModel, window: NSWindow?, recover: Bool = false) -> Bool {
        let session: RecordingSession
        do {
            session = try RecordingSession.open(path: url.path, recover: recover)
        } catch {
            let text = message(error)
            if !recover, text.contains("no footer") {
                offerRecovery(url: url, model: model, window: window, reason: text)
            } else {
                show("The recording could not be opened", text, window: window)
            }
            return false
        }
        if let verdict = try? session.validate(sample: 8), verdict.errors > 0 || verdict.warnings > 0 {
            if verdict.errors > 0 {
                show("The recording is damaged", verdict.lines.joined(separator: "\n"), window: window)
                return false
            }
            show("The recording opened with warnings", verdict.lines.joined(separator: "\n"), window: window, style: .informational)
        }
        do {
            try model.graphics.attach(session, name: url.lastPathComponent)
        } catch {
            show("The recording could not be opened", message(error), window: window)
            return false
        }
        if let reference = session.reference() {
            model.workbench.attachRecording(reference: reference)
        }
        if model.graphicsTab == nil { model.graphicsTab = .tilemap }
        return true
    }

    /// Close the recording the views read, and stop referring to it.
    static func close(model: RomViewModel) {
        for r in model.workbench.recordings() { _ = model.workbench.detachRecording(path: r.path) }
        model.graphics.detach()
    }

    /// On opening a project: reattach the recording it refers to, if the file
    /// is still there and unchanged. Quietly does nothing otherwise, since a
    /// recording is a convenience, not part of the project's content.
    static func reattach(model: RomViewModel) {
        guard let r = model.workbench.recordings().last,
              let session = try? RecordingSession.open(path: r.path, recover: false),
              session.reference()?.fingerprint == r.fingerprint
        else { return }
        try? model.graphics.attach(session, name: URL(fileURLWithPath: r.path).lastPathComponent)
    }

    private static func offerRecovery(url: URL, model: RomViewModel, window: NSWindow?, reason: String) {
        let alert = NSAlert()
        alert.messageText = "The recording was not finished"
        alert.informativeText = reason + "\n\nThe frames that reached the disk can still be read."
        alert.addButton(withTitle: "Open What Was Recorded")
        alert.addButton(withTitle: "Cancel")
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            if response == .alertFirstButtonReturn {
                attach(url: url, model: model, window: window, recover: true)
            }
        }
        if let window {
            alert.beginSheetModal(for: window, completionHandler: finish)
        } else {
            finish(alert.runModal())
        }
    }

    // MARK: Help › Save Mesen Recorder Script…

    static func saveRecorderScript(window: NSWindow?) {
        let panel = NSSavePanel()
        panel.title = "Save Mesen Recorder Script"
        panel.nameFieldStringValue = "mesen_recorder.lua"
        panel.allowedContentTypes = [UTType(filenameExtension: "lua") ?? .plainText]
        run(panel, on: window) { url in
            do {
                try recorderScript().write(to: url, atomically: true, encoding: .utf8)
                show(
                    "Recorder script saved",
                    """
                    1. In Mesen, open Debug › Script Window and load \(url.lastPathComponent).
                    2. In the script window's settings, allow access to I/O and OS functions, then run it.
                    3. Play, then stop the script. Run romlens rec pack on the .rlstream it wrote (in Mesen's script data folder) to make a .romrec, and open that here.
                    """,
                    window: window,
                    style: .informational
                )
            } catch {
                show("The script could not be saved", error.localizedDescription, window: window)
            }
        }
    }

    // MARK: File › Import Snapshot…

    /// Loose VRAM, CGRAM and OAM dumps (a PPU register block too, if there is
    /// one) made into a one-frame recording, saved where the user says and
    /// attached. Which dump is which comes from its size, and for the two
    /// 512-byte kinds from its name.
    static func importSnapshot(model: RomViewModel, window: NSWindow?) {
        let panel = NSOpenPanel()
        panel.title = "Import Snapshot"
        panel.message = "Choose the dumps: VRAM (64 KB), CGRAM (512 bytes), OAM (544, or 512 named “oam” or “sprite”), and optionally a 256-byte PPU register block."
        panel.allowsMultipleSelection = true
        panel.allowsOtherFileTypes = true
        run(panel, on: window, urls: true) { urls in
            var vram: Data?, cgram: Data?, oam: Data?, ppu: Data?
            for url in urls {
                guard let data = try? Data(contentsOf: url) else { continue }
                let name = url.lastPathComponent.lowercased()
                switch data.count {
                case 0x10000: vram = data
                case 544: oam = data
                case 256: ppu = data
                case 512 where name.contains("oam") || name.contains("sprite"): oam = data
                case 512: cgram = data
                default: break
                }
            }
            guard vram != nil || cgram != nil || oam != nil else {
                show("No dumps recognised", "None of the files is the size of VRAM, CGRAM or OAM.", window: window)
                return
            }
            let save = NSSavePanel()
            save.title = "Save Snapshot Recording"
            save.nameFieldStringValue = "snapshot.romrec"
            save.allowedContentTypes = [recordingType]
            run(save, on: window) { out in
                do {
                    try writeSnapshotRecording(rom: model.rom, vram: vram, cgram: cgram, oam: oam, ppu: ppu, path: out.path)
                    attach(url: out, model: model, window: window)
                } catch {
                    show("The snapshot could not be imported", message(error), window: window)
                }
            }
        }
    }

    // MARK: File › Export Frame Region…

    static let exportable: [(StateRegion, String)] = [
        (.vram, "VRAM"), (.cgram, "CGRAM"), (.oam, "OAM"), (.wram, "WRAM"),
        (.ppu, "PPU registers"), (.cpu, "CPU registers"),
        (.io, "I/O registers"), (.timing, "Timing"),
    ]

    /// One region of the current frame, as raw bytes. It is the game's data,
    /// for the user's own tools; the panel says so.
    static func exportFrameRegion(model: RomViewModel, window: NSWindow?) {
        let graphics = model.graphics
        guard graphics.hasRecording else { return }
        let popup = NSPopUpButton(frame: .zero, pullsDown: false)
        let present = exportable.filter { graphics.recordingInfo?.regions.contains($0.0) ?? false }
        popup.addItems(withTitles: present.map(\.1))
        popup.sizeToFit()
        let panel = NSSavePanel()
        panel.title = "Export Frame Region"
        panel.message = "Frame \(graphics.frame): raw bytes from the recording, which are the game's own data. For your own tools; not for sharing."
        panel.accessoryView = popup
        panel.nameFieldStringValue = "frame\(graphics.frame).bin"
        run(panel, on: window) { url in
            let (region, _) = present[max(popup.indexOfSelectedItem, 0)]
            guard let bytes = graphics.regionBytes(region) else {
                show("Nothing to export", "The recording has no \(present[popup.indexOfSelectedItem].1) at this frame.", window: window)
                return
            }
            do {
                try bytes.write(to: url, options: .atomic)
            } catch {
                show("The region could not be exported", error.localizedDescription, window: window)
            }
        }
    }

    // MARK: Helpers

    private static func run(_ panel: NSSavePanel, on window: NSWindow?, then: @escaping (URL) -> Void) {
        let host = window ?? NSApp.keyWindow
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            if response == .OK, let url = panel.url { then(url) }
        }
        if let host {
            panel.beginSheetModal(for: host, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }

    private static func run(_ panel: NSOpenPanel, on window: NSWindow?, urls: Bool, then: @escaping ([URL]) -> Void) {
        let host = window ?? NSApp.keyWindow
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            if response == .OK, !panel.urls.isEmpty { then(panel.urls) }
        }
        if let host {
            panel.beginSheetModal(for: host, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }

    private static func show(_ title: String, _ text: String, window: NSWindow?, style: NSAlert.Style = .warning) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = text
        alert.alertStyle = style
        if let window {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
    }

    private static func message(_ error: Error) -> String {
        guard let e = error as? RomlensError else { return error.localizedDescription }
        switch e {
        case .Io(let msg), .InvalidRom(let msg), .BadAddress(let msg), .Project(let msg),
             .RomMismatch(let msg), .InvalidLabel(let msg), .Recording(let msg):
            return msg
        case .Cancelled:
            return "cancelled"
        }
    }
}

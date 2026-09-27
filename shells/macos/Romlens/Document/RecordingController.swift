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
    static let streamType = UTType(filenameExtension: "rlstream") ?? .data

    static func open(model: RomViewModel, window: NSWindow?) {
        let panel = NSOpenPanel()
        panel.title = "Open Recording"
        panel.message = "Open Recording"
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [streamType, recordingType]
        panel.allowsOtherFileTypes = true
        if let mesen = mesenScriptData, FileManager.default.fileExists(atPath: mesen.path) {
            panel.directoryURL = mesen
        }
        run(panel, on: window) { url in
            if url.pathExtension.lowercased() == "rlstream" {
                pack(stream: url, model: model, window: window)
            } else {
                attach(url: url, model: model, window: window)
            }
        }
    }

    /// Where Mesen's recorder script writes its streams: the real home's,
    /// not the sandbox container's.
    static var mesenScriptData: URL? {
        guard let pw = getpwuid(getuid()), let home = pw.pointee.pw_dir else { return nil }
        return URL(fileURLWithPath: String(cString: home))
            .appendingPathComponent("Library/Application Support/Mesen2/LuaScriptData", isDirectory: true)
    }

    /// Where packed recordings are kept: the app's own Recordings folder.
    static var packedFolder: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("Recordings", isDirectory: true)
    }

    /// Pack the recorder's stream into a recording in the app's own folder,
    /// off the main thread, then open it.
    static func pack(stream: URL, model: RomViewModel, window: NSWindow?) {
        let folder = packedFolder
        let name = stream.deletingPathExtension().lastPathComponent
        let stamp = Int(Date().timeIntervalSince1970)
        let out = folder.appendingPathComponent("\(name)-\(stamp).romrec")
        let rom = model.rom
        model.graphics.packing = stream.lastPathComponent
        Task {
            let result: Result<PackSummary, Error> = await Task.detached {
                do {
                    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                    // The SPC700's log beside the stream: the sandbox gave
                    // the app the stream alone, and lets it read the log
                    // as the stream's related item.
                    let log = RelatedFile.copy(extension: "spclog", beside: stream)
                    defer { if let log { try? FileManager.default.removeItem(at: log) } }
                    return .success(try packRecorderStream(rom: rom, stream: stream.path, out: out.path, spcLog: log?.path))
                } catch {
                    return .failure(error)
                }
            }.value
            model.graphics.packing = nil
            switch result {
            case .success(let summary):
                if attach(url: out, model: model, window: window), summary.truncated {
                    show("The recording was cut short", "Mesen closed before the recorder finished; every whole frame, \(summary.frames) of them, was kept.", window: window, style: .informational)
                }
            case .failure(let error):
                show("The recorder's stream could not be read", message(error), window: window)
            }
        }
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
                    3. Play, then stop the script, and open what it wrote with File › Open Recording…: the panel starts in Mesen's script data folder, and Romlens packs the stream itself.
                    Or choose File › Start Live Session first: while it runs, the views follow the game, sound included.
                    4. A Mesen with an execution log (the MesenCE fork) also gets a .mxlog beside the stream: import it with File › Import › Execution Trace… for the calls, jumps, reads and DMA the game made.
                    5. To watch the game live, also allow network access in the script settings, and choose File › Start Live Session here. The script connects within two seconds.
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
        panel.message = "VRAM, CGRAM and OAM dumps"
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
        panel.message = "Frame \(graphics.frame): game data, not for sharing"
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

/// A file beside one the user chose, with the same name and another
/// extension. A sandboxed app may read it when its Info.plist declares the
/// extension a related item (`NSIsRelatedItemType`) and it reads through
/// a file presenter whose primary item is the file chosen.
final class RelatedFile: NSObject, NSFilePresenter {
    let primaryPresentedItemURL: URL?
    let presentedItemURL: URL?
    let presentedItemOperationQueue = OperationQueue()

    private init(_ url: URL, primary: URL) {
        presentedItemURL = url
        primaryPresentedItemURL = primary
    }

    /// Copy `primary`'s related file with `extension` into the app's
    /// temporary folder: the copy, or nil when there is none or the system
    /// would not let the app read it.
    static func copy(extension ext: String, beside primary: URL) -> URL? {
        let url = primary.deletingPathExtension().appendingPathExtension(ext)
        let presenter = RelatedFile(url, primary: primary)
        NSFileCoordinator.addFilePresenter(presenter)
        defer { NSFileCoordinator.removeFilePresenter(presenter) }
        let to = FileManager.default.temporaryDirectory
            .appendingPathComponent("romlens-\(UUID().uuidString).\(ext)")
        var copied = false
        var error: NSError?
        NSFileCoordinator(filePresenter: presenter).coordinate(readingItemAt: url, options: [], error: &error) { u in
            copied = (try? FileManager.default.copyItem(at: u, to: to)) != nil
        }
        return copied ? to : nil
    }
}

import AppKit
import RomlensKit
import UniformTypeIdentifiers

/// File › Compare With… (docs/22, D2): another version of this ROM, a ROM
/// file or a saved project, opened beside this one for the Compare tab.
@MainActor
enum CompareController {
    static func open(model: RomViewModel, window: NSWindow?) {
        let panel = NSOpenPanel()
        panel.title = "Compare With"
        panel.message = "Another version of this ROM: a ROM file, or a saved project to bring its names. What changed is shown from it to this one."
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.treatsFilePackagesAsDirectories = false
        panel.allowedContentTypes = [
            UTType(ProjectDocument.projectType), UTType(ProjectDocument.romType),
        ].compactMap { $0 }
        panel.allowsOtherFileTypes = true
        let host = window ?? NSApp.keyWindow
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let url = panel.url else { return }
            start(url: url, model: model, window: window)
        }
        if let host {
            panel.beginSheetModal(for: host, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }

    /// Load `url` and compare. Split out so tests can drive it without a
    /// panel.
    static func start(url: URL, model: RomViewModel, window: NSWindow?) {
        let other: Workbench
        do {
            other = try load(url: url)
        } catch {
            let alert = NSAlert()
            alert.messageText = "\(url.lastPathComponent) could not be opened"
            alert.informativeText = "\(error)"
            if let window { alert.beginSheetModal(for: window) } else { alert.runModal() }
            return
        }
        compare(other: other, name: url.deletingPathExtension().lastPathComponent, model: model)
    }

    static func compare(other: Workbench, name: String, model: RomViewModel) {
        model.editorTab = .compare
        Task {
            await model.compare.start(
                this: model.workbench, other: other, name: name, generation: model.asmGeneration
            )
        }
    }

    /// A ROM file, or a `.romlens` package with its ROM found as opening it
    /// would find it.
    static func load(url: URL) throws -> Workbench {
        let isPackage = (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory ?? false
        guard isPackage else {
            let rom = try Rom.fromBytes(bytes: Data(contentsOf: url), name: url.lastPathComponent)
            return Workbench(rom: rom)
        }
        let wrapper = try FileWrapper(url: url)
        var files = ProjectDocument.packageFiles(wrapper)
        let local = files[ProjectDocument.localFileName]
            .flatMap { try? JSONDecoder().decode(LocalRecord.self, from: $0) }
        files[ProjectDocument.localFileName] = nil
        let identity = try projectIdentity(files: files)
        let located = try ProjectDocument.locatorFactory().locate(identity: identity, local: local, packageURL: url)
        let rom = try Rom.fromBytes(bytes: located.bytes, name: located.url.lastPathComponent)
        return try Workbench.withProjectFiles(rom: rom, files: files)
    }
}

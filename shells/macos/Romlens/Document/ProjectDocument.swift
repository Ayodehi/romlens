import AppKit
import RomlensKit
import UniformTypeIdentifiers

/// A `.romlens` package: the core's five JSON files plus a machine-local
/// `local.json` with the ROM's bookmark. Opening a plain `.sfc`/`.smc`
/// creates an untitled project; Save As chooses where it lives. The ROM is
/// never copied into the package.
final class ProjectDocument: NSDocument {
    // Constants and a pure comparison, so `nonisolated`: AppKit calls
    // `read(from:ofType:)` and `canConcurrentlyReadDocuments` off the main
    // actor, and isolating these to it only produced warnings at every such
    // call site.
    nonisolated static let projectType = "io.github.ayodehi.romlens.project"
    nonisolated static let romType = "io.github.ayodehi.romlens.sfc"
    nonisolated static let localFileName = "local.json"

    /// AppKit hands back the type name LaunchServices resolved, and
    /// LaunchServices lower-cases a declared UTI (`11-naming.md`). UTIs are
    /// case-insensitive, so type names are never compared with `==`.
    nonisolated static func isType(_ name: String, _ expected: String) -> Bool {
        name.compare(expected, options: .caseInsensitive) == .orderedSame
    }
    nonisolated static let coreFiles = [
        "project.json", "labels.json", "comments.json", "regions.json", "flags.json",
    ]

    /// Injected for tests; the app uses the bookmark-based default.
    nonisolated(unsafe) static var locatorFactory: @MainActor () -> RomLocator = { DefaultRomLocator() }

    private(set) var model: RomViewModel?
    private(set) var romURL: URL?
    private var romBookmark: Data?
    private var romSha256: String?
    /// The package as last read, so unknown files survive a save.
    ///
    /// `nonisolated(unsafe)` because `read(from:ofType:)` is a nonisolated
    /// override and `FileWrapper` is not `Sendable`, so the assignment cannot
    /// move inside the `assumeIsolated` block with the rest of the state. It
    /// is safe here for one specific reason: `canConcurrentlyReadDocuments`
    /// returns `false`, so AppKit never reads a document on two threads.
    private nonisolated(unsafe) var lastWrapper: FileWrapper?

    override init() {
        super.init()
        hasUndoManager = false
    }

    override class var autosavesInPlace: Bool { true }
    override class func canConcurrentlyReadDocuments(ofType typeName: String) -> Bool { false }
    override class var readableTypes: [String] { [projectType, romType] }
    override class var writableTypes: [String] { [projectType] }
    override class func isNativeType(_ type: String) -> Bool { isType(type, projectType) }

    /// Untitled projects are named after their ROM.
    var draftName: String {
        romURL?.deletingPathExtension().lastPathComponent ?? model?.info.title ?? "Untitled"
    }

    override var displayName: String! {
        get { fileURL == nil && model != nil ? draftName : super.displayName }
        set { super.displayName = newValue }
    }

    var workbench: Workbench? { model?.workbench }
    var session: WorkbenchSession? { model?.session }

    // MARK: Adopting a ROM

    /// Turn ROM bytes into an untitled project document.
    @MainActor
    func adoptRom(bytes: Data, name: String, url: URL?) throws {
        let rom = try Rom.fromBytes(bytes: bytes, name: name)
        let workbench = Workbench(rom: rom)
        install(rom: rom, workbench: workbench, url: url, bookmark: url.flatMap(DefaultRomLocator.bookmark(for:)))
        fileType = Self.projectType
        if let url {
            Self.locatorFactory().remember(url: url, sha256: rom.info().sha256)
        }
    }

    @MainActor
    private func install(rom: Rom, workbench: Workbench, url: URL?, bookmark: Data?) {
        romURL = url
        romBookmark = bookmark
        romSha256 = rom.info().sha256
        let model = RomViewModel(rom: rom, workbench: workbench)
        model.session.onCommand = { [weak self] _ in
            self?.updateChangeCount(.changeDone)
        }
        self.model = model
    }

    // MARK: Reading

    override func read(from data: Data, ofType typeName: String) throws {
        // The ROM path (a regular file): reading the bytes ourselves
        // sidesteps sandbox path questions.
        let url = fileURL
        let name = url?.lastPathComponent ?? "ROM"
        try MainActor.assumeIsolated {
            try adoptRom(bytes: data, name: name, url: url)
        }
        // An adopted ROM is a new, untitled project.
        fileURL = nil
        fileModificationDate = nil
    }

    override func read(from fileWrapper: FileWrapper, ofType typeName: String) throws {
        if Self.isType(typeName, Self.romType) || fileWrapper.isRegularFile {
            guard let data = fileWrapper.regularFileContents else {
                throw CocoaError(.fileReadCorruptFile)
            }
            try read(from: data, ofType: Self.romType)
            return
        }
        guard let children = fileWrapper.fileWrappers else { throw CocoaError(.fileReadCorruptFile) }
        var files = Self.packageFiles(fileWrapper)
        files[Self.localFileName] = nil
        let local: LocalRecord? = children[Self.localFileName]?.regularFileContents
            .flatMap { try? JSONDecoder().decode(LocalRecord.self, from: $0) }
        let identity = try projectIdentity(files: files)
        try MainActor.assumeIsolated {
            let located = try Self.locatorFactory().locate(identity: identity, local: local, packageURL: fileURL)
            let rom = try Rom.fromBytes(bytes: located.bytes, name: located.url.lastPathComponent)
            let workbench = try Workbench.withProjectFiles(rom: rom, files: files)
            // Creating the view model starts the analysis.
            install(rom: rom, workbench: workbench, url: located.url, bookmark: located.bookmark)
        }
        lastWrapper = fileWrapper
    }

    // MARK: Package paths

    /// Every regular file in a package, keyed by its `/`-separated path, as
    /// the core names them: `traces/coverage.cdl` is a file in a `traces`
    /// directory, not a file with a slash in its name.
    nonisolated static func packageFiles(_ wrapper: FileWrapper, prefix: String = "") -> [String: Data] {
        var files: [String: Data] = [:]
        for (name, child) in wrapper.fileWrappers ?? [:] {
            if child.isDirectory {
                files.merge(packageFiles(child, prefix: prefix + name + "/")) { a, _ in a }
            } else if child.isRegularFile, let data = child.regularFileContents {
                files[prefix + name] = data
            }
        }
        return files
    }

    /// Store `data` at a `/`-separated path under `root`, making the
    /// directories on the way and replacing whatever was there.
    nonisolated static func put(_ data: Data, at path: String, in root: FileWrapper) {
        var parts = path.split(separator: "/").map(String.init)
        guard let name = parts.popLast() else { return }
        var dir = root
        for part in parts {
            if let existing = dir.fileWrappers?[part], existing.isDirectory {
                dir = existing
            } else {
                if let old = dir.fileWrappers?[part] { dir.removeFileWrapper(old) }
                let made = FileWrapper(directoryWithFileWrappers: [:])
                made.preferredFilename = part
                dir.addFileWrapper(made)
                dir = made
            }
        }
        if let old = dir.fileWrappers?[name] { dir.removeFileWrapper(old) }
        let child = FileWrapper(regularFileWithContents: data)
        child.preferredFilename = name
        dir.addFileWrapper(child)
    }

    // MARK: Writing

    override func fileWrapper(ofType typeName: String) throws -> FileWrapper {
        guard let workbench else { throw CocoaError(.fileWriteUnknown) }
        let wrapper = lastWrapper ?? FileWrapper(directoryWithFileWrappers: [:])
        // An untitled project has no name yet; NSDocument assigns it on save.
        if wrapper.preferredFilename == nil, let name = fileURL?.lastPathComponent, !name.isEmpty {
            wrapper.preferredFilename = name
        }
        for (path, data) in workbench.projectFiles() {
            Self.put(data, at: path, in: wrapper)
        }
        let local = LocalRecord(bookmark: romBookmark, lastPath: romURL?.path)
        if let old = wrapper.fileWrappers?[Self.localFileName] { wrapper.removeFileWrapper(old) }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let localChild = FileWrapper(regularFileWithContents: try encoder.encode(local))
        localChild.preferredFilename = Self.localFileName
        wrapper.addFileWrapper(localChild)
        lastWrapper = wrapper
        return wrapper
    }

    // The completion-handler form, not the `async` one. An `async` override
    // resumes after `super` on the main actor, but closing the document (on
    // quit, say) blocks the main thread until any save in flight finishes:
    // an autosave still waiting to resume there never does, and the app
    // hangs on quit. AppKit calls this handler on the main thread itself.
    override func save(
        to url: URL,
        ofType typeName: String,
        for saveOperation: NSDocument.SaveOperationType,
        completionHandler: @escaping (Error?) -> Void
    ) {
        super.save(to: url, ofType: typeName, for: saveOperation) { [weak self] error in
            if error == nil { self?.workbench?.markSaved() }
            completionHandler(error)
        }
    }

    /// An untitled project is named after its ROM, so with the extension
    /// hidden the panel read as if it would overwrite `Game.sfc`. Show the
    /// extension and say what is being saved.
    override func prepareSavePanel(_ savePanel: NSSavePanel) -> Bool {
        savePanel.isExtensionHidden = false
        savePanel.canSelectHiddenExtension = false
        savePanel.message = "Save a Romlens project: your labels, comments, marks and imported traces. The ROM file is never changed."
        return true
    }

    override func makeWindowControllers() {
        guard let model else { return }
        addWindowController(RomWindowController(model: model))
    }

    /// The document's ROM hash, for de-duplication by the controller.
    var sha256: String? { romSha256 }
}

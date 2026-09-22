import AppKit
import RomlensKit
import UniformTypeIdentifiers

/// A `.romlens` package: the core's five JSON files plus a machine-local
/// `local.json` with the ROM's bookmark. Opening a plain `.sfc`/`.smc`
/// creates an untitled project; Save As chooses where it lives. The ROM is
/// never copied into the package.
final class ProjectDocument: NSDocument {
    static let projectType = "io.github.placeholder.romlens.project"
    static let romType = "io.github.placeholder.romlens.sfc"
    static let localFileName = "local.json"
    static let coreFiles = ["project.json", "labels.json", "comments.json", "regions.json", "flags.json"]

    /// Injected for tests; the app uses the bookmark-based default.
    nonisolated(unsafe) static var locatorFactory: @MainActor () -> RomLocator = { DefaultRomLocator() }

    private(set) var model: RomViewModel?
    private(set) var romURL: URL?
    private var romBookmark: Data?
    private var romSha256: String?
    /// The package as last read, so unknown files survive a save.
    private var lastWrapper: FileWrapper?

    override init() {
        super.init()
        hasUndoManager = false
    }

    override class var autosavesInPlace: Bool { true }
    override class func canConcurrentlyReadDocuments(ofType typeName: String) -> Bool { false }
    override class var readableTypes: [String] { [projectType, romType] }
    override class var writableTypes: [String] { [projectType] }
    override class func isNativeType(_ type: String) -> Bool { type == projectType }

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
        if typeName == Self.romType || fileWrapper.isRegularFile {
            guard let data = fileWrapper.regularFileContents else {
                throw CocoaError(.fileReadCorruptFile)
            }
            try read(from: data, ofType: Self.romType)
            return
        }
        guard let children = fileWrapper.fileWrappers else { throw CocoaError(.fileReadCorruptFile) }
        var files: [String: Data] = [:]
        for (name, child) in children where child.isRegularFile && name != Self.localFileName {
            if let data = child.regularFileContents { files[name] = data }
        }
        let local: LocalRecord? = children[Self.localFileName]?.regularFileContents
            .flatMap { try? JSONDecoder().decode(LocalRecord.self, from: $0) }
        let identity = try projectIdentity(files: files)
        try MainActor.assumeIsolated {
            let located = try Self.locatorFactory().locate(identity: identity, local: local, packageURL: fileURL)
            let rom = try Rom.fromBytes(bytes: located.bytes, name: located.url.lastPathComponent)
            let workbench = try Workbench.withProjectFiles(rom: rom, files: files)
            install(rom: rom, workbench: workbench, url: located.url, bookmark: located.bookmark)
            model?.session.startAnalysis()
        }
        lastWrapper = fileWrapper
    }

    // MARK: Writing

    override func fileWrapper(ofType typeName: String) throws -> FileWrapper {
        guard let workbench else { throw CocoaError(.fileWriteUnknown) }
        let wrapper = lastWrapper ?? FileWrapper(directoryWithFileWrappers: [:])
        if wrapper.preferredFilename == nil { wrapper.preferredFilename = fileURL?.lastPathComponent }
        let files = workbench.projectFiles()
        for (name, data) in files {
            if let old = wrapper.fileWrappers?[name] { wrapper.removeFileWrapper(old) }
            let child = FileWrapper(regularFileWithContents: data)
            child.preferredFilename = name
            wrapper.addFileWrapper(child)
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

    override func save(to url: URL, ofType typeName: String, for saveOperation: NSDocument.SaveOperationType) async throws {
        try await super.save(to: url, ofType: typeName, for: saveOperation)
        workbench?.markSaved()
    }

    override func makeWindowControllers() {
        guard let model else { return }
        addWindowController(RomWindowController(model: model))
    }

    /// The document's ROM hash, for de-duplication by the controller.
    var sha256: String? { romSha256 }
}

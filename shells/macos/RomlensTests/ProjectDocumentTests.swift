import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// A locator that answers from memory instead of bookmarks and panels.
@MainActor
final class StubLocator: RomLocator {
    var bytes: Data?
    var remembered: [(URL, String)] = []
    var decline = false

    func remember(url: URL, sha256: String) { remembered.append((url, sha256)) }

    func locate(identity: RomIdentityInfo, local: LocalRecord?, packageURL: URL?) throws -> LocatedRom {
        if decline { throw CocoaError(.userCancelled) }
        let bytes = try #require(self.bytes)
        return LocatedRom(url: URL(fileURLWithPath: local?.lastPath ?? "/stub/rom.sfc"), bytes: bytes, bookmark: nil)
    }
}

@MainActor
@Suite(.serialized) struct ProjectDocumentTests {
    private func withStub<T>(_ stub: StubLocator, _ body: () throws -> T) rethrows -> T {
        let previous = ProjectDocument.locatorFactory
        ProjectDocument.locatorFactory = { stub }
        defer { ProjectDocument.locatorFactory = previous }
        return try body()
    }

    @Test func romBytesBecomeAnUntitledProject() throws {
        let stub = StubLocator()
        let doc = try withStub(stub) {
            let doc = ProjectDocument()
            try doc.read(from: makeTestRom(mapping: .hiRom), ofType: Fixture.romType)
            return doc
        }
        let model = try #require(doc.model)
        #expect(model.info.mapping == .hiRom)
        #expect(doc.fileType == Fixture.projectType)
        #expect(doc.fileURL == nil)
        #expect(doc.displayName == "ROMLENS TEST")
        #expect(!doc.isDocumentEdited)
        #expect(doc.undoManager == nil)
        #expect(ProjectDocument.writableTypes == [Fixture.projectType])
        #expect(ProjectDocument.readableTypes.contains(Fixture.romType))
        #expect(ProjectDocument.autosavesInPlace)
        model.session.cancelAnalysis()
    }

    @Test func commandsMarkTheDocumentEdited() throws {
        let doc = ProjectDocument()
        try doc.read(from: makeTestRom(mapping: .loRom), ofType: Fixture.romType)
        let model = try #require(doc.model)
        model.session.cancelAnalysis()
        model.select(offset: 0)
        try model.setLabel(name: "Boot")
        #expect(doc.isDocumentEdited)
        #expect(model.session.canUndo)
    }

    /// Quit closes each edited document, which autosaves it while the main
    /// thread waits. A save override that needed the main thread to finish
    /// hung there with a beachball; this waited forever before the fix.
    @Test func closingAnEditedDocumentSavesWithoutHanging() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("romlens-close-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("t.romlens")
        let doc = ProjectDocument()
        try doc.read(from: makeTestRom(mapping: .loRom), ofType: Fixture.romType)
        let model = try #require(doc.model)
        model.session.cancelAnalysis()
        var saved: Error?? = .none
        doc.save(to: url, ofType: Fixture.projectType, for: .saveAsOperation) { saved = .some($0) }
        try await wait("Save As finishes") { saved != nil }
        #expect(saved! == nil)
        #expect(doc.fileURL == url)

        model.select(offset: 0)
        try model.setLabel(name: "Boot")
        #expect(doc.isDocumentEdited)
        let delegate = CloseDelegate()
        doc.canClose(withDelegate: delegate, shouldClose: #selector(CloseDelegate.document(_:shouldClose:contextInfo:)), contextInfo: nil)
        try await wait("the close finishes") { delegate.answer != nil }
        #expect(delegate.answer == true)
        #expect(!doc.isDocumentEdited, "the close saved the edit")
    }

    private func wait(_ what: String, until condition: @MainActor () -> Bool) async throws {
        do {
            try await Fixture.settle(until: condition)
        } catch {
            Issue.record("waited 5 s for \(what)")
            throw error
        }
    }

    /// An imported trace is stored as `traces/coverage.cdl`, in a directory
    /// inside the package. Saving one failed with "The file doesn't exist",
    /// and reading ignored the directory, so the trace was lost either way.
    @Test func aSavedProjectKeepsItsImportedTrace() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("romlens-trace-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("t.romlens")
        let doc = ProjectDocument()
        try doc.read(from: makeTestRom(mapping: .loRom), ofType: Fixture.romType)
        let model = try #require(doc.model)
        model.session.cancelAnalysis()
        var cdl = Array("CDLv2".utf8) + [0, 0, 0, 0]
        cdl += [UInt8](repeating: 0, count: Int(model.byteCount))
        cdl[9 + 0x0C] = 0x01
        _ = try model.session.importTrace(source: "play.cdl", bytes: Data(cdl))
        let stored = try #require(model.workbench.projectFiles()[ProjectDocumentTests.coverage])

        var saved: Error?? = .none
        doc.save(to: url, ofType: Fixture.projectType, for: .saveAsOperation) { saved = .some($0) }
        try await wait("Save As finishes") { saved != nil }
        #expect(saved! == nil)
        let onDisk = url.appendingPathComponent(ProjectDocumentTests.coverage)
        #expect(try Data(contentsOf: onDisk) == stored, "written in a traces directory")

        let stub = StubLocator()
        stub.bytes = makeTestRom(mapping: .loRom)
        let back = ProjectDocument()
        try withStub(stub) {
            try back.read(from: FileWrapper(url: url), ofType: Fixture.projectType)
        }
        let reopened = try #require(back.model)
        #expect(reopened.workbench.projectFiles()[ProjectDocumentTests.coverage] == stored, "read back from it")
        // Opening analyzes, and the progress bar goes away when it is done.
        #expect(reopened.session.analysis.isRunning)
        try await wait("the reopened project's analysis") {
            reopened.session.hasSnapshot && !reopened.session.analysis.isRunning
        }
        #expect(reopened.session.analysis == .idle)
    }

    static let coverage = "traces/coverage.cdl"

    @Test func packageRoundTripKeepsTheLabel() throws {
        let stub = StubLocator()
        stub.bytes = makeTestRom(mapping: .loRom)
        try withStub(stub) {
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("romlens-doc-\(UUID().uuidString).sfc")
            try makeTestRom(mapping: .loRom).write(to: url)
            defer { try? FileManager.default.removeItem(at: url) }
            let doc = ProjectDocument()
            try doc.read(from: makeTestRom(mapping: .loRom), ofType: Fixture.romType)
            doc.model?.session.cancelAnalysis()
            doc.model?.select(offset: 0)
            try doc.model?.setLabel(name: "Boot")
            let wrapper = try doc.fileWrapper(ofType: Fixture.projectType)
            let names = Set(wrapper.fileWrappers?.keys.map { $0 } ?? [])
            #expect(names == Set(ProjectDocument.coreFiles + [ProjectDocument.localFileName]))
            let local = try JSONDecoder().decode(LocalRecord.self, from: wrapper.fileWrappers![ProjectDocument.localFileName]!.regularFileContents!)
            #expect(local.lastPath == nil, "bytes read without a URL have no path")
            // Read the wrapper back into a fresh document through the stub locator.
            let back = ProjectDocument()
            try back.read(from: wrapper, ofType: Fixture.projectType)
            let model = try #require(back.model)
            model.session.cancelAnalysis()
            #expect(model.workbench.labelAt(snesAddress: 0x8000)?.name == "Boot")
            #expect(!back.isDocumentEdited)
            // A saved wrapper keeps unknown files.
            let extra = FileWrapper(regularFileWithContents: Data("hi".utf8))
            extra.preferredFilename = "notes.txt"
            wrapper.addFileWrapper(extra)
            let again = ProjectDocument()
            try again.read(from: wrapper, ofType: Fixture.projectType)
            again.model?.session.cancelAnalysis()
            let saved = try again.fileWrapper(ofType: Fixture.projectType)
            #expect(saved.fileWrappers?["notes.txt"] != nil)
        }
    }

    @Test func declinedLocatorCancels() throws {
        let stub = StubLocator()
        stub.decline = true
        try withStub(stub) {
            let doc = ProjectDocument()
            try doc.read(from: makeTestRom(mapping: .loRom), ofType: Fixture.romType)
            doc.model?.session.cancelAnalysis()
            let wrapper = try doc.fileWrapper(ofType: Fixture.projectType)
            let back = ProjectDocument()
            #expect(throws: CocoaError.self) { try back.read(from: wrapper, ofType: Fixture.projectType) }
            #expect(back.model == nil)
        }
    }

    @Test func controllerRoutesRomsAndDeduplicates() async throws {
        let controller = try #require(NSDocumentController.shared as? ProjectDocumentController)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("romlens-ctl-\(UUID().uuidString).sfc")
        try makeTestRom(mapping: .loRom).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        let (first, _) = try await controller.openDocument(withContentsOf: url, display: false)
        let doc = try #require(first as? ProjectDocument)
        doc.model?.session.cancelAnalysis()
        #expect(doc.fileURL == nil)
        #expect(doc.fileType == Fixture.projectType)
        let (second, _) = try await controller.openDocument(withContentsOf: url, display: false)
        #expect(second === first, "the same ROM (by hash) reuses the document")
        doc.close()
    }
}

@MainActor
@Suite struct MainMenuTests {
    @Test func menuHasTheDocumentedMenus() {
        let menu = MainMenu.build()
        let titles = menu.items.map(\.title)
        #expect(titles.dropFirst() == ["File", "Edit", "View", "Go", "Window", "Help"])
        func items(_ name: String) -> [NSMenuItem] { menu.items.first { $0.title == name }?.submenu?.items ?? [] }
        let jump = items("Go").first { $0.title == "Jump to Address…" }
        #expect(jump?.keyEquivalent == "l")
        #expect(jump?.action == #selector(RomWindowController.jumpToAddress(_:)))
        #expect(items("Go").contains { $0.title == "Forward" && $0.keyEquivalent == "]" })
        let undo = items("Edit").first { $0.title == "Undo" }
        #expect(undo?.action == #selector(RomWindowController.undo(_:)) && undo?.keyEquivalent == "z")
        #expect(items("Edit").contains { $0.title == "Redo" && $0.keyEquivalentModifierMask.contains(.shift) })
        #expect(items("Edit").contains { $0.title == "Rename Label…" && $0.keyEquivalent.isEmpty })
        let export = items("File").first { $0.title == "Export" }?.submenu?.items.map(\.title)
        #expect(export == ["Assembly Listing…", "Labels and Comments…", "Symbol File…"])
        #expect(items("File").contains { $0.title == "Save" && $0.keyEquivalent == "s" })
        let both = items("View").first { $0.title == "Both" }
        #expect(both?.keyEquivalent == "3" && both?.keyEquivalentModifierMask == [.command, .option])
    }

    /// The running test host is the real app, so its delegate must be installed.
    @Test func appDelegateIsInstalled() {
        #expect(NSApp.delegate is AppDelegate)
        #expect(NSApp.mainMenu?.items.contains { $0.title == "Go" } == true)
        #expect(NSDocumentController.shared is ProjectDocumentController)
    }
}

/// Receives `canClose(withDelegate:…)`'s answer.
@MainActor
final class CloseDelegate: NSObject {
    var answer: Bool?

    @objc func document(_ document: NSDocument, shouldClose: Bool, contextInfo: UnsafeMutableRawPointer?) {
        answer = shouldClose
    }
}

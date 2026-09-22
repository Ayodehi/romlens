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

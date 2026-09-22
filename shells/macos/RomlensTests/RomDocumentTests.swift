import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

@MainActor
@Suite struct RomDocumentTests {
    @Test func readsBytesIntoAModel() throws {
        let doc = RomDocument()
        try doc.read(from: makeTestRom(mapping: .hiRom), ofType: "io.github.placeholder.romlens.sfc")
        let model = try #require(doc.model)
        #expect(model.info.mapping == .hiRom)
        #expect(model.info.title == "ROMLENS TEST")
        #expect(model.rowCount == 4096)
    }

    @Test func readsFromAFileURL() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("romlens-doc-test.sfc")
        try makeTestRom(mapping: .loRom).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        let doc = try RomDocument(contentsOf: url, ofType: "io.github.placeholder.romlens.sfc")
        #expect(doc.model?.info.fileName == "romlens-doc-test.sfc")
        #expect(doc.fileURL == url)
    }

    @Test func rejectsGarbage() {
        let doc = RomDocument()
        #expect(throws: RomlensError.self) {
            try doc.read(from: Data(repeating: 0x5A, count: 0x8000), ofType: "io.github.placeholder.romlens.sfc")
        }
        #expect(doc.model == nil)
    }

    @Test func isReadOnly() {
        let doc = RomDocument()
        #expect(throws: (any Error).self) { try doc.data(ofType: "io.github.placeholder.romlens.sfc") }
        #expect(!RomDocument.autosavesInPlace)
    }
}

@MainActor
@Suite struct MainMenuTests {
    @Test func menuHasTheDocumentedMenus() {
        let menu = MainMenu.build()
        let titles = menu.items.map(\.title)
        #expect(titles.dropFirst() == ["File", "Edit", "View", "Go", "Window", "Help"])
        let go = menu.items.first { $0.title == "Go" }?.submenu
        let jump = go?.items.first { $0.title == "Jump to Address…" }
        #expect(jump?.keyEquivalent == "l")
        #expect(jump?.action == #selector(RomWindowController.jumpToAddress(_:)))
    }

    /// The running test host is the real app, so its delegate must be installed.
    @Test func appDelegateIsInstalled() {
        #expect(NSApp.delegate is AppDelegate)
        #expect(NSApp.mainMenu?.items.contains { $0.title == "Go" } == true)
    }
}

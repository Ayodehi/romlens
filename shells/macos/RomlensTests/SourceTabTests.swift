import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The Source tab (docs/22, S2): a ca65 program's `.dbg` imported, its
/// source shown, a line selecting its bytes and bytes their line.
@MainActor
@Suite struct SourceTabTests {
    /// The test program written to a folder of its own, as ld65 left it.
    private func program() throws -> (dir: URL, model: RomViewModel, dbg: String) {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("romlens-source-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        var rom = Data()
        var dbg = ""
        for f in makeCa65TestProgram() {
            try f.bytes.write(to: dir.appendingPathComponent(f.name))
            if f.name == "fixture.sfc" { rom = f.bytes }
            if f.name == "fixture.dbg" { dbg = String(decoding: f.bytes, as: UTF8.self) }
        }
        let model = RomViewModel(rom: try Rom.fromBytes(bytes: rom, name: "fixture.sfc"), startAnalysis: false)
        return (dir, model, dbg)
    }

    @Test func aDbgBringsTheSourceTab() async throws {
        let (dir, model, dbg) = try program()
        defer { try? FileManager.default.removeItem(at: dir) }
        #expect(!model.editorTabs.contains(.source), "no Source tab without sources")
        let result = try model.session.importDbg(source: "fixture.dbg", dir: dir.path, text: dbg)
        #expect(result.labelsAdded == 10)
        model.source.reload(workbench: model.workbench)
        #expect(model.editorTabs.contains(.source))
        #expect(model.source.files.map(\.name) == ["main.s", "macros.inc", "palette.s"])
        #expect(model.source.shownFile?.name == "main.s")
        let text = try #require(model.source.text)
        #expect(text[23].contains("brightness $80"))
        #expect(model.source.problem == nil)
        #expect(model.source.byLine[24]?.ranges.first?.start == 11)
        #expect(model.source.byLine[15] == nil, "`.proc` makes no bytes")
    }

    @Test func aLineSelectsItsBytesAndBytesShowTheirLine() async throws {
        let (dir, model, dbg) = try program()
        defer { try? FileManager.default.removeItem(at: dir) }
        _ = try model.session.importDbg(source: "fixture.dbg", dir: dir.path, text: dbg)
        model.source.reload(workbench: model.workbench)
        let pane = SourcePaneController(model: model)
        pane.update()
        #expect(pane.textView.string.contains("brightness $80"))

        // A click on `txs` (line 23) selects its one byte.
        let ns = pane.textView.string as NSString
        let txs = ns.range(of: "txs")
        pane.textView.setSelectedRange(NSRange(location: txs.location, length: 0))
        #expect(model.highlightedRange == 10..<11)

        // Bytes in the second module show that file.
        model.select(offset: 0x31)
        pane.update()
        #expect(model.source.shownFile?.name == "palette.s")

        // A macro's line made bytes twice: a second click goes to the next.
        model.source.show(model.source.files.firstIndex { $0.name == "macros.inc" }, workbench: model.workbench)
        pane.update()
        let lda = (pane.textView.string as NSString).range(of: "lda #level")
        pane.textView.setSelectedRange(NSRange(location: lda.location, length: 0))
        #expect(model.highlightedRange == 11..<13)
        pane.textView.setSelectedRange(NSRange(location: lda.location + 1, length: 0))
        #expect(model.highlightedRange == 22..<24)
    }

    @Test func theTabDrawsInAWindow() async throws {
        let (dir, _, dbg) = try program()
        defer { try? FileManager.default.removeItem(at: dir) }
        let rom = try Data(contentsOf: dir.appendingPathComponent("fixture.sfc"))
        let model = try await Fixture.analyzedModel(rom: try Rom.fromBytes(bytes: rom, name: "fixture.sfc"))
        _ = try model.session.importDbg(source: "fixture.dbg", dir: dir.path, text: dbg)
        model.source.reload(workbench: model.workbench)
        model.editorTab = .source
        let controller = RomWindowController(model: model)
        controller.window?.orderFront(nil)
        defer { controller.close() }
        let content = try #require(controller.window?.contentView)
        content.layoutSubtreeIfNeeded()
        Fixture.spin(0.2)
        #expect(content.bounds.width > 0)
    }
}

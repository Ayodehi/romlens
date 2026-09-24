import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The C tab (docs/18): the routine at the selection as pseudo-C, kept in
/// step with the disassembly, and Focus on Code.
@MainActor
@Suite struct DecompileTabTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    @Test func theTabFollowsTheSelectionToItsRoutine() async throws {
        let m = try await model()
        m.select(offset: 0x44)
        #expect(m.decompiler.state == .idle, "nothing is decompiled until the tab shows")
        m.editorTab = .c
        try await Fixture.settle(until: { m.decompiler.state == .ready })
        let d = try #require(m.decompiler.result)
        #expect(d.name == "SUB_008040")
        #expect(d.text.contains("if ((u8)A < ADDR_7E0021) {"))
        // The if came from the CMP and the BCS; both find it.
        let line = try #require(d.text.components(separatedBy: "\n").firstIndex { $0.contains("if (") })
        #expect(m.decompiler.lines(forInstructionAt: 0x44).contains(line))
        #expect(m.decompiler.lines(forInstructionAt: 0x42).contains(line))
        #expect(m.decompiler.offsets(forLine: line).contains(0x44))

        // Another routine: the text follows.
        m.select(offset: 0x22)
        try await Fixture.settle(until: { m.decompiler.result?.name == "SUB_008020" })
        #expect(m.decompiler.result?.text.contains("do {") == true)

        // The level picker.
        m.decompiler.level = .lift
        m.refreshDecompile()
        try await Fixture.settle(until: { m.decompiler.state == .ready && m.decompiler.result?.text.contains("lift level") == true })
    }

    @Test func outsideARoutineSaysSo() async throws {
        let m = try await model()
        m.editorTab = .c
        m.select(offset: 0x7FF0)
        #expect(m.decompiler.state == .notInRoutine)
    }

    @Test func aRenameShowsInTheC() async throws {
        let m = try await model()
        m.editorTab = .c
        m.select(offset: 0x40)
        try await Fixture.settle(until: { m.decompiler.result?.name == "SUB_008040" })
        try m.session.setLabel(address: 0x008040, name: "Larger")
        try await Fixture.settle(until: { m.decompiler.result?.name == "Larger" })
    }

    @Test func theSplitShowsTheText() async throws {
        let m = try await model()
        m.editorTab = .c
        m.select(offset: 0x40)
        let controller = RomWindowController(model: m)
        controller.window?.orderFront(nil)
        defer { controller.close() }
        try await Fixture.settle(until: { m.decompiler.state == .ready })
        let content = try #require(controller.window?.contentView)
        try await Fixture.settle(until: {
            content.layoutSubtreeIfNeeded()
            return Fixture.find(content, CTextView.self)?.string.contains("SUB_008040") == true
        })
        let text = try #require(Fixture.find(content, CTextView.self))
        // A click on the if line selects the instructions it came from.
        let ns = text.string as NSString
        let at = ns.range(of: "if (").location
        text.setSelectedRange(NSRange(location: at, length: 0))
        #expect(m.selectedOffset == 0x42)
    }

    @Test func focusHidesThePanelsAndPutsThemBack() async throws {
        let m = try await model()
        m.isInspectorVisible = false
        m.toggleFocus()
        #expect(m.isFocused)
        #expect(!m.isNavigatorVisible && !m.isInspectorVisible && !m.isStripVisible)
        m.toggleFocus()
        #expect(!m.isFocused)
        #expect(m.isNavigatorVisible && !m.isInspectorVisible && m.isStripVisible, "as they were")
    }
}

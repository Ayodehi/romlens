import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// Explanations (docs/20): hardware writes decoded in the inspector, idiom
/// notes in the listing and the C, and the toggle that hides them.
@MainActor
@Suite struct ExplanationTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeExplainTestRom(), name: "e.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    private func record(_ m: RomViewModel, line: UInt32) -> AsmLineRecord {
        m.asmBatch(containingLine: line).record(line: line)
    }

    @Test func theInspectorExplainsAWriteFieldByField() async throws {
        let m = try await model()
        m.select(offset: 0x53)
        let r = try #require(m.explanation?.register)
        #expect(r.store)
        #expect(r.short == "NMITIMEN = $81: NMI on, joypad auto-read on")
        #expect(r.parts[0].fields.map(\.name) == ["NMI", "Timer IRQ", "Joypad"])
        #expect(r.parts[0].fields[0].meaning == "NMI on")
        // A read of a status register: its fields, with no values.
        m.select(offset: 0x56)
        let read = try #require(m.explanation?.register)
        #expect(!read.store)
        #expect(read.parts[0].fields.allSatisfy { $0.raw == nil })
        #expect(m.explanation?.idioms.first?.title == "Wait for vertical blank")
    }

    @Test func aNoteLineSitsAboveItsIdiomAndSelectsIt() async throws {
        let m = try await model()
        let line = try #require(m.workbench.lineForOffset(fileOffset: 0x56))
        let note = record(m, line: line - 1)
        #expect(note.kind == .note)
        #expect(note.text.hasPrefix("; ▸ Wait for vertical blank"))
        #expect(note.tokens.first?.kind == .note)
        // The explained comment on a store.
        let store = try #require(m.workbench.lineForOffset(fileOffset: 0x53))
        #expect(record(m, line: store).text.hasSuffix("; NMITIMEN = $81: NMI on, joypad auto-read on"))

        m.selectIdiom(noteAt: 0x56)
        #expect(m.highlightedRange == 0x56..<0x5B)
    }

    @Test func turningExplanationsOffGivesThePlainListingAndC() async throws {
        let m = try await model()
        m.select(offset: 0x09)
        m.editorTab = .c
        try await Fixture.settle(until: { m.decompiler.state == .ready })
        #expect(m.decompiler.result?.text.contains("/* ▸ DMA transfer") == true)
        let lines = m.asmLineCount

        m.showExplanations = false
        try await Fixture.settle(until: { m.asmLineCount < lines })
        let all = (0..<m.asmLineCount).map { record(m, line: $0) }
        #expect(!all.contains { $0.kind == .note })
        let store = try #require(m.workbench.lineForOffset(fileOffset: 0x53))
        #expect(record(m, line: store).text.hasSuffix("; NMITIMEN"))
        try await Fixture.settle(until: {
            m.decompiler.state == .ready && m.decompiler.result?.text.contains("▸") == false
        })

        m.showExplanations = true
        try await Fixture.settle(until: { m.asmLineCount == lines })
    }

    @Test func theCPrintsNumbersInTheBaseAsked() async throws {
        let m = try await model()
        m.select(offset: 0x09)
        m.editorTab = .c
        try await Fixture.settle(until: { m.decompiler.state == .ready })
        #expect(m.decompiler.result?.text.contains("INIDISP = 0x8F;") == true)
        m.decompiler.numbers = .binary
        m.refreshDecompile()
        try await Fixture.settle(until: { m.decompiler.result?.text.contains("INIDISP = 0b10001111;") == true })
        m.decompiler.numbers = .decimal
        m.refreshDecompile()
        try await Fixture.settle(until: { m.decompiler.result?.text.contains("INIDISP = 143;") == true })
        #expect(CPaneController.value(of: "0b1000") == 8)
        #expect(CPaneController.bases(0x81) == "129 = 0x81 = 0b10000001")
    }

    @Test func theScreenSectionShowsTheSetupAndOpensItsViews() async throws {
        let m = try await model()
        m.select(offset: 0x56)
        #expect(m.screen == nil, "nothing is worked out until the section opens")
        m.showScreen = true
        try await Fixture.settle(until: { m.screen != nil })
        let irq = m.screen?.sections.first { $0.title == "Interrupts" }?.rows.first
        #expect(irq?.text == "NMI on, joypad auto-read on")
        #expect(irq?.setAt == [0x53])
        // Another instruction: the section follows.
        m.select(offset: 0x0C)
        try await Fixture.settle(until: {
            m.screen?.sections.first { $0.title == "Interrupts" }?.rows.first?.text == "not set yet"
        })
        m.open(screenLink: .tiles(rom: 0x1000, bpp: 2))
        #expect(m.graphicsTab == .tiles)
        #expect(m.graphics.format == .bpp2)
        #expect(m.graphics.romOffset == 0x1000)
    }
}

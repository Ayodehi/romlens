import Foundation
import RomlensKit
import Testing
@testable import Romlens

@MainActor
@Suite struct RomViewModelTests {
    private func model() throws -> RomViewModel {
        RomViewModel(rom: try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc"))
    }

    @Test func jumpSelectsScrollsAndRecordsHistory() throws {
        let m = try model()
        #expect(m.selectedOffset == nil)
        #expect(m.inspection == nil)
        let r = try m.jump(text: "$00:8000")
        #expect(r.fileOffset == 0)
        #expect(m.selectedOffset == 0)
        #expect(m.inspection?.valueU8 == 0x78)
        #expect(m.scrollRequest?.row == 0)
        #expect(!m.canGoBack)
        try m.jump(text: "0x7FFC")
        #expect(m.selectedOffset == 0x7FFC)
        #expect(m.inspection?.spanName == "Emulation RESET")
        #expect(m.scrollRequest?.row == 0x7FF)
        #expect(m.history == [0])
        m.goBack()
        #expect(m.selectedOffset == 0)
        #expect(!m.canGoBack)
    }

    @Test func jumpRejectsBadExpressions() throws {
        let m = try model()
        #expect(throws: RomlensError.self) { try m.jump(text: "$7E:0000") }
        #expect(throws: RomlensError.self) { try m.jump(text: "0x8000") }
        #expect(m.selectedOffset == nil)
        if case .failure(let error) = m.preview(text: "$7E:0000") {
            #expect(error.localizedDescription.contains("not mapped"))
        } else {
            Issue.record("expected a failure preview")
        }
        if case .success(let r) = m.preview(text: "80:8010") {
            #expect(r.fileOffset == 0x10)
        } else {
            Issue.record("expected a success preview")
        }
    }

    @Test func selectionMovesAndClamps() throws {
        let m = try model()
        m.moveSelection(by: -5)
        #expect(m.selectedOffset == 0)
        m.moveSelection(by: 16)
        #expect(m.selectedOffset == 16)
        m.moveSelection(by: 1 << 20)
        #expect(m.selectedOffset == 0x7FFF)
        m.select(offset: 0x9000)
        #expect(m.selectedOffset == 0x7FFF, "out-of-range selection is ignored")
        m.select(offset: nil)
        #expect(m.inspection == nil)
    }

    @Test func addressStyleRebuildsLayout() throws {
        let m = try model()
        let before = m.lineGeneration
        m.addressStyle = .snes
        #expect(m.lineGeneration == before + 1)
        #expect(m.layout.style == .snes)
        #expect(m.layout.prefixChars == 10)
        m.addressStyle = .snes
        #expect(m.lineGeneration == before + 1, "no-op change does not invalidate")
    }

    @Test func spansAndPalette() throws {
        let m = try model()
        #expect(m.spans.count == 22)
        #expect(m.palette.color(forSpanId: 1) != nil)
        #expect(m.palette.color(forSpanId: 0) == nil)
    }
}

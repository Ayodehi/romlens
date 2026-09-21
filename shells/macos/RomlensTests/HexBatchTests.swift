import CoreText
import Foundation
import RomlensKit
import Testing
@testable import Romlens

@Suite struct HexBatchTests {
    private func lorom() throws -> Rom {
        try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
    }

    @Test func decodesRecords() throws {
        let rom = try lorom()
        let batch = try HexBatch(startRow: 0, data: rom.hexRows(startRow: 0, count: 256))
        #expect(batch.rowCount == 256)
        let r0 = batch.record(row: 0)
        #expect(r0.fileOffset == 0)
        #expect(r0.snesAddress == 0x00_8000)
        #expect(r0.byteCount == 16)
        #expect(Array(r0.bytes[0..<3]) == [0x78, 0x18, 0xFB])
        #expect(r0.ascii[0] == UInt8(ascii: "x"))
        #expect(!r0.hasSpan)
        let r1 = batch.record(row: 1)
        #expect(r1.fileOffset == 16)
    }

    @Test func headerRowsCarrySpanIds() throws {
        let rom = try lorom()
        let start: UInt32 = 0x7FC0 / 16 - (0x7FC0 / 16) % 256
        let batch = try HexBatch(startRow: start, data: rom.hexRows(startRow: start, count: 256))
        let title = batch.record(row: 0x7FC0 / 16)
        #expect(title.hasSpan)
        #expect(Set(title.spanIds).count == 1)
        #expect(title.spanIds[0] != 0)
        let last = batch.record(row: 0x7FF0 / 16)
        #expect(last.isLastRow)
        #expect(last.spanIds[0] == 0 && last.spanIds[4] != 0)
    }

    @Test func rejectsMalformedBatches() {
        #expect(throws: HexBatch.DecodeError.tooShort) { try HexBatch(startRow: 0, data: Data([1, 0])) }
        var bad = Data([2, 0, 64, 0, 0, 0, 0, 0])
        #expect(throws: HexBatch.DecodeError.badVersion(2)) { try HexBatch(startRow: 0, data: bad) }
        bad = Data([1, 0, 32, 0, 0, 0, 0, 0])
        #expect(throws: HexBatch.DecodeError.badStride(32)) { try HexBatch(startRow: 0, data: bad) }
        bad = Data([1, 0, 64, 0, 2, 0, 0, 0]) + Data(repeating: 0, count: 64)
        #expect(throws: HexBatch.DecodeError.truncated) { try HexBatch(startRow: 0, data: bad) }
    }

    @Test func layoutFormatsRowsWithoutStringFormat() throws {
        let rom = try lorom()
        let batch = try HexBatch(startRow: 0, data: rom.hexRows(startRow: 0, count: 4))
        let both = HexRowLayout(style: .both)
        let text = String(decoding: both.text(for: batch.record(row: 0)), as: UTF8.self)
        #expect(text == "0x000000  $00:8000  78 18 FB E2 30 A9 80 8D  00 21 80 FE EA EA 40 00 |x...0....!....@.|")
        #expect(text.count == both.totalChars)
        let snes = String(decoding: HexRowLayout(style: .snes).text(for: batch.record(row: 1)), as: UTF8.self)
        #expect(snes.hasPrefix("$00:8010  00 00"))
        let file = String(decoding: HexRowLayout(style: .file).text(for: batch.record(row: 1)), as: UTF8.self)
        #expect(file.hasPrefix("0x000010  00 00"))
        // Hit testing maps hex and ASCII columns to bytes and the gaps to nil.
        #expect(both.byte(atX: both.x(ofChar: both.hexColumn(byte: 0)) + 1) == 0)
        #expect(both.byte(atX: both.x(ofChar: both.hexColumn(byte: 9)) + 1) == 9)
        #expect(both.byte(atX: both.x(ofChar: both.hexColumn(byte: 15)) + 1) == 15)
        #expect(both.byte(atX: both.x(ofChar: both.asciiColumn(byte: 3)) + 1) == 3)
        #expect(both.byte(atX: 2) == nil)
        #expect(both.byte(atX: both.x(ofChar: both.totalChars + 2)) == nil)
    }

    @Test func linesAreCachedPerGeneration() throws {
        let rom = try lorom()
        let batch = try HexBatch(startRow: 0, data: rom.hexRows(startRow: 0, count: 4))
        let layout = HexRowLayout(style: .both)
        let a = batch.line(row: 2, generation: 0, layout: layout)
        let b = batch.line(row: 2, generation: 0, layout: layout)
        #expect(a === b)
        let c = batch.line(row: 2, generation: 1, layout: HexRowLayout(style: .file))
        #expect(a !== c)
        #expect(CTLineGetGlyphCount(c) == HexRowLayout(style: .file).totalChars)
    }
}

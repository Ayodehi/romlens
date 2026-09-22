import CoreText
import Foundation
import RomlensKit
import Testing
@testable import Romlens

@Suite struct AsmBatchTests {
    /// Build a batch by hand: header + records + text area.
    private func handBuilt(records: [(kind: UInt8, text: String, tokens: [(UInt8, UInt16, UInt16)], offset: UInt32, snes: UInt32, bytes: [UInt8])]) -> Data {
        var recs = Data()
        var textArea = Data()
        let textAreaOff = 16 + records.count * 96
        for r in records {
            var rec = Data(repeating: 0, count: 96)
            func put<T: FixedWidthInteger>(_ v: T, at: Int) {
                withUnsafeBytes(of: v.littleEndian) { rec.replaceSubrange(at..<at + $0.count, with: $0) }
            }
            put(r.offset, at: 0)
            put(r.snes, at: 4)
            rec[8] = r.kind
            rec[9] = UInt8(r.bytes.count)
            rec[10] = 1
            rec[11] = 90
            rec[12] = 0b0001_1111
            rec[13] = 0b0000_1100
            rec[14] = UInt8(r.tokens.count)
            put(UInt16(r.text.utf8.count), at: 18)
            put(UInt32(textAreaOff + textArea.count), at: 20)
            put(UInt32(0xFFFF_FFFF), at: 24)
            put(UInt32(0xFFFF_FFFF), at: 28)
            for (i, b) in r.bytes.enumerated() { rec[36 + i] = b }
            for (t, tok) in r.tokens.enumerated() {
                rec[52 + t * 6] = tok.0
                put(tok.1, at: 52 + t * 6 + 2)
                put(tok.2, at: 52 + t * 6 + 4)
            }
            recs.append(rec)
            textArea.append(contentsOf: Array(r.text.utf8))
        }
        var data = Data()
        withUnsafeBytes(of: UInt16(1).littleEndian) { data.append(contentsOf: $0) }
        withUnsafeBytes(of: UInt16(96).littleEndian) { data.append(contentsOf: $0) }
        withUnsafeBytes(of: UInt32(records.count).littleEndian) { data.append(contentsOf: $0) }
        withUnsafeBytes(of: UInt32(textAreaOff).littleEndian) { data.append(contentsOf: $0) }
        withUnsafeBytes(of: UInt32(textArea.count).littleEndian) { data.append(contentsOf: $0) }
        data.append(recs)
        data.append(textArea)
        return data
    }

    @Test func decodesEveryKindAndToken() throws {
        let data = handBuilt(records: [
            (6, "; ==== bank $00 ====", [(14, 0, 20)], 0, 0x8000, []),
            (3, "Boot:", [(11, 0, 4), (2, 4, 1)], 0, 0x8000, []),
            (1, "SEI  ; disable IRQ", [(1, 0, 3), (8, 5, 13)], 0, 0x8000, [0x78]),
            (5, "; block", [(8, 0, 7)], 1, 0x8001, []),
            (2, "db $EA,$EA", [(12, 0, 2), (13, 3, 7)], 12, 0x800C, [0xEA, 0xEA]),
            (4, "", [], 14, 0xFFFF_FFFF, []),
            (1, "LDA #$01", [(1, 0, 3), (3, 4, 4), (99, 0, 1)], 14, 0x800E, [0xA9, 0x01]),
        ])
        let batch = try AsmBatch(startRow: 100, data: data)
        #expect(batch.lineCount == 7)
        #expect(batch.startLine == 100 && batch.endLine == 107)
        let section = batch.record(line: 100)
        #expect(section.kind == .section)
        #expect(section.text == "; ==== bank $00 ====")
        #expect(section.tokens.first?.kind == .section)
        let label = batch.record(line: 101)
        #expect(label.kind == .label)
        #expect(label.tokens.map(\.kind) == [.userLabelDef, .punct])
        let sei = batch.record(line: 102)
        #expect(sei.kind == .instruction)
        #expect(sei.bytes == [0x78])
        #expect(sei.range == 0..<1)
        #expect(sei.isLabelled && sei.isBlockEnd)
        #expect(sei.m && sei.x && sei.e && sei.dbrKnown && sei.dpKnown)
        #expect(sei.tokens[1] == AsmToken(kind: .comment, start: 5, len: 13))
        #expect(batch.record(line: 103).kind == .comment)
        let data2 = batch.record(line: 104)
        #expect(data2.kind == .data && data2.byteCount == 2 && data2.tokens.map(\.kind) == [.directive, .dataValue])
        let blank = batch.record(line: 105)
        #expect(blank.kind == .blank && blank.snesAddress == nil && blank.text.isEmpty)
        let lda = batch.record(line: 106)
        #expect(lda.tokens.map(\.kind) == [.mnemonic, .immediate, .other], "unknown kinds fall back")
        #expect(lda.target == nil && lda.targetFileOffset == nil)
    }

    @Test func rejectsMalformedBatches() {
        #expect(throws: AsmBatch.DecodeError.tooShort) { try AsmBatch(startRow: 0, data: Data([1, 0])) }
        var bad = Data([2, 0, 96, 0, 0, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0])
        #expect(throws: AsmBatch.DecodeError.badVersion(2)) { try AsmBatch(startRow: 0, data: bad) }
        bad = Data([1, 0, 64, 0, 0, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0])
        #expect(throws: AsmBatch.DecodeError.badStride(64)) { try AsmBatch(startRow: 0, data: bad) }
        bad = Data([1, 0, 96, 0, 2, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0]) + Data(repeating: 0, count: 96)
        #expect(throws: AsmBatch.DecodeError.truncated) { try AsmBatch(startRow: 0, data: bad) }
    }

    @MainActor
    @Test func realBatchFromAnalyzedWorkbench() async throws {
        let model = try await Fixture.analyzedModel(rom: try Fixture.smallRom())
        let batch = model.asmBatch(containingLine: 0)
        #expect(batch.lineCount > 4)
        // Section, label, then SEI with a mnemonic token.
        var line: UInt32 = 0
        while batch.record(line: line).kind != .instruction { line += 1 }
        let sei = batch.record(line: line)
        #expect(sei.text.hasPrefix("SEI"))
        #expect(sei.tokens.first?.kind == .mnemonic)
        #expect(sei.fileOffset == 0)
        let staLine = try #require(model.workbench.lineForOffset(fileOffset: 8))
        let sta = batch.record(line: staLine)
        #expect(sta.text.hasPrefix("STA $2100"))
        #expect(sta.fileOffset == 7)
        #expect(sta.tokens.contains { $0.kind == .hardwareRegister })
        let layout = AsmLineLayout(style: .both)
        let text = String(decoding: layout.text(for: sta), as: UTF8.self)
        #expect(text.hasPrefix("0x000007  $00:8007  8D 00 21     STA $2100"), "\(text)")
        let a = batch.line(line: staLine, generation: 0, layout: layout)
        let b = batch.line(line: staLine, generation: 0, layout: layout)
        #expect(a === b)
        #expect(batch.line(line: staLine, generation: 1, layout: layout) !== a)
    }
}

@Suite struct AsmLineLayoutTests {
    private func record(kind: AsmLineKind, text: String, tokens: [AsmToken] = [], offset: UInt32 = 0x41C, bytes: [UInt8] = []) -> AsmLineRecord {
        AsmLineRecord(
            fileOffset: offset, snesAddress: 0x80841C, kind: kind, byteCount: bytes.count, regionKind: .code,
            confidence: 90, flagsBefore: 0, lineFlags: 0, dbr: 0, dp: 0, text: text, target: nil,
            targetFileOffset: nil, xrefInCount: 0, bytes: bytes, tokens: tokens
        )
    }

    @Test func columnsAndText() {
        let both = AsmLineLayout(style: .both)
        #expect(both.addressChars == 20 && both.textColumn == 33)
        let insn = record(kind: .instruction, text: "JML $808423", tokens: [AsmToken(kind: .mnemonic, start: 0, len: 3), AsmToken(kind: .number, start: 4, len: 7)], bytes: [0x5C, 0x23, 0x84, 0x80])
        let text = String(decoding: both.text(for: insn), as: UTF8.self)
        #expect(text == "0x00041C  $80:841C  5C 23 84 80  JML $808423")
        let snes = String(decoding: AsmLineLayout(style: .snes).text(for: insn), as: UTF8.self)
        #expect(snes == "$80:841C  5C 23 84 80  JML $808423")
        let file = String(decoding: AsmLineLayout(style: .file).text(for: insn), as: UTF8.self)
        #expect(file == "0x00041C  5C 23 84 80  JML $808423")
        let label = record(kind: .label, text: "Boot:")
        #expect(String(decoding: both.text(for: label), as: UTF8.self) == "Boot:")
        let comment = record(kind: .comment, text: "; note")
        #expect(String(decoding: both.text(for: comment), as: UTF8.self) == String(repeating: " ", count: 33) + "; note")
        #expect(both.text(for: record(kind: .blank, text: "")).isEmpty)
        // Hit testing.
        #expect(both.column(atX: both.textStart - 20) == nil)
        #expect(both.column(atX: both.x(ofChar: 5) + 1) == 5)
        #expect(both.token(atColumn: both.textColumn + 1, in: insn)?.kind == .mnemonic)
        #expect(both.token(atColumn: both.textColumn + 6, in: insn)?.kind == .number)
        #expect(both.token(atColumn: both.textColumn + 3, in: insn) == nil, "the space between tokens")
        #expect(both.token(atColumn: 2, in: label) == nil)
        #expect(both.totalWidth > both.x(ofChar: both.totalChars))
        let line = both.makeLine(for: insn)
        #expect(CTLineGetGlyphCount(line) == text.count)
    }
}

@Suite struct BatchCacheTests {
    final class CountingBatch: RowBatch {
        static let rowsPerBatch: UInt32 = 16
        let startRow: UInt32
        init(startRow: UInt32, data: Data) throws {
            self.startRow = startRow
        }
    }

    @Test func fetchesOncePerBatchAndEvicts() {
        nonisolated(unsafe) var fetches: [UInt32] = []
        let cache = BatchCache<CountingBatch>(capacity: 2) { start, count in
            fetches.append(start)
            #expect(count == 16)
            return Data()
        }
        _ = cache.batch(containingRow: 3)
        _ = cache.batch(containingRow: 15)
        #expect(fetches == [0])
        _ = cache.batch(containingRow: 16)
        _ = cache.batch(containingRow: 0)
        _ = cache.batch(containingRow: 40) // evicts 16
        #expect(cache.count == 2)
        #expect(cache.isCached(row: 0) && !cache.isCached(row: 16) && cache.isCached(row: 32))
        #expect(cache.missCount == 3)
        cache.invalidateAll()
        #expect(cache.count == 0)
        _ = cache.batch(containingRow: 0)
        #expect(cache.missCount == 4)
    }
}

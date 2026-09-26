import CoreText
import Foundation

/// Line kinds (the core's `LineKind`).
enum AsmLineKind: UInt8 {
    case instruction = 1, data = 2, label = 3, blank = 4, comment = 5, section = 6
    /// An idiom's note (docs/20), above its first instruction.
    case note = 7

    var isContent: Bool { self == .instruction || self == .data }
}

/// Region kinds as the batch encodes them (record byte 10).
enum AsmRegionKind: UInt8 {
    case unknown = 0, code = 1, byte = 2, word = 3, long = 4, pointer = 5, table = 6
    case string = 7, graphics = 8, tilemap = 9, palette = 10, compressed = 11, structure = 12
    case sample = 13

    var isData: Bool { rawValue >= 2 }
}

/// Token kinds (the core's `TokenKind`); `other` catches future kinds.
enum AsmTokenKind: UInt8 {
    case mnemonic = 1, punct = 2, immediate = 3, number = 4, autoLabel = 5, userLabel = 6
    case hardwareRegister = 7, comment = 8, autoComment = 9, autoLabelDef = 10, userLabelDef = 11
    case directive = 12, dataValue = 13, section = 14, warning = 15, note = 16
    case other = 255

    init(raw: UInt8) { self = AsmTokenKind(rawValue: raw) ?? .other }
}

struct AsmToken: Equatable, Sendable {
    let kind: AsmTokenKind
    /// Byte range within the line text.
    let start: Int
    let len: Int

    var range: Range<Int> { start..<start + len }
}

/// One decoded 96-byte record (layout in the core's `viewmodel::asm_lines`).
struct AsmLineRecord: Sendable {
    static let noneAddress: UInt32 = 0xFFFF_FFFF

    let fileOffset: UInt32
    let snesAddress: UInt32?
    let kind: AsmLineKind
    let byteCount: Int
    let regionKind: AsmRegionKind
    /// 0...100
    let confidence: Int
    let flagsBefore: UInt8
    let lineFlags: UInt8
    let dbr: UInt8
    let dp: UInt16
    let text: String
    let target: UInt32?
    let targetFileOffset: UInt32?
    let xrefInCount: Int
    let bytes: [UInt8]
    let tokens: [AsmToken]

    var range: Range<UInt32> { fileOffset..<fileOffset + UInt32(byteCount) }
    var hasLineComment: Bool { lineFlags & 0x01 != 0 }
    var hasBlockComment: Bool { lineFlags & 0x02 != 0 }
    var isLabelled: Bool { lineFlags & 0x04 != 0 }
    var isBlockEnd: Bool { lineFlags & 0x08 != 0 }
    var hasIncomingXrefs: Bool { lineFlags & 0x10 != 0 }
    var isCallTarget: Bool { lineFlags & 0x20 != 0 }
    var isLowConfidence: Bool { lineFlags & 0x40 != 0 }
    var hasUserOverride: Bool { lineFlags & 0x80 != 0 }
    var hasComment: Bool { hasLineComment || hasBlockComment }
    var hasFlagOverride: Bool { flagsBefore & 0x20 != 0 }
    var hasWarning: Bool { flagsBefore & 0x40 != 0 }
    var hasAssumption: Bool { flagsBefore & 0x80 != 0 }
    var m: Bool { flagsBefore & 0x01 != 0 }
    var x: Bool { flagsBefore & 0x02 != 0 }
    var e: Bool { flagsBefore & 0x04 != 0 }
    var dbrKnown: Bool { flagsBefore & 0x08 != 0 }
    var dpKnown: Bool { flagsBefore & 0x10 != 0 }
}

/// A batch of lines as fetched from the core, decoded on demand, with one
/// cached `CTLine` per line for the current layout generation.
final class AsmBatch: RowBatch {
    static let rowsPerBatch: UInt32 = 256
    static let linesPerBatch: UInt32 = rowsPerBatch
    static let stride = 96
    static let headerLength = 16

    enum DecodeError: Error, Equatable {
        case tooShort, badVersion(UInt16), badStride(UInt16), truncated
    }

    let startRow: UInt32
    let lineCount: Int
    private let data: Data
    private var lines: [CTLine?]
    private var lineGeneration = -1

    init(startRow: UInt32, data: Data) throws {
        guard data.count >= Self.headerLength else { throw DecodeError.tooShort }
        let version = data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self) }
        let stride = data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 2, as: UInt16.self) }
        let count = data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 4, as: UInt32.self) }
        let textOff = data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 8, as: UInt32.self) }
        let textLen = data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 12, as: UInt32.self) }
        guard version == 1 else { throw DecodeError.badVersion(version) }
        guard Int(stride) == Self.stride else { throw DecodeError.badStride(stride) }
        guard data.count >= Self.headerLength + Int(count) * Self.stride,
              data.count >= Int(textOff) + Int(textLen)
        else { throw DecodeError.truncated }
        self.startRow = startRow
        self.lineCount = Int(count)
        self.data = data
        self.lines = Array(repeating: nil, count: Int(count))
    }

    var startLine: UInt32 { startRow }
    var endLine: UInt32 { startRow + UInt32(lineCount) }

    func contains(line: UInt32) -> Bool { line >= startRow && line < endLine }

    func record(line: UInt32) -> AsmLineRecord {
        precondition(contains(line: line), "line \(line) not in batch \(startRow)..<\(endLine)")
        let base = Self.headerLength + Int(line - startRow) * Self.stride
        return data.withUnsafeBytes { raw -> AsmLineRecord in
            let fileOffset = raw.loadUnaligned(fromByteOffset: base, as: UInt32.self)
            let snes = raw.loadUnaligned(fromByteOffset: base + 4, as: UInt32.self)
            let kind = AsmLineKind(rawValue: raw[base + 8]) ?? .blank
            let byteCount = Int(raw[base + 9])
            let region = AsmRegionKind(rawValue: raw[base + 10]) ?? .unknown
            let confidence = Int(raw[base + 11])
            let flagsBefore = raw[base + 12]
            let lineFlags = raw[base + 13]
            let tokenCount = Int(min(raw[base + 14], 6))
            let dbr = raw[base + 15]
            let dp = raw.loadUnaligned(fromByteOffset: base + 16, as: UInt16.self)
            let textLen = Int(raw.loadUnaligned(fromByteOffset: base + 18, as: UInt16.self))
            let textOff = Int(raw.loadUnaligned(fromByteOffset: base + 20, as: UInt32.self))
            let target = raw.loadUnaligned(fromByteOffset: base + 24, as: UInt32.self)
            let targetOff = raw.loadUnaligned(fromByteOffset: base + 28, as: UInt32.self)
            let xrefIn = Int(raw.loadUnaligned(fromByteOffset: base + 32, as: UInt16.self))
            let bytes = Array(raw[base + 36..<base + 36 + byteCount])
            var tokens: [AsmToken] = []
            tokens.reserveCapacity(tokenCount)
            for t in 0..<tokenCount {
                let at = base + 52 + t * 6
                tokens.append(AsmToken(
                    kind: AsmTokenKind(raw: raw[at]),
                    start: Int(raw.loadUnaligned(fromByteOffset: at + 2, as: UInt16.self)),
                    len: Int(raw.loadUnaligned(fromByteOffset: at + 4, as: UInt16.self))
                ))
            }
            let text = textLen > 0 ? String(decoding: raw[textOff..<textOff + textLen], as: UTF8.self) : ""
            return AsmLineRecord(
                fileOffset: fileOffset,
                snesAddress: snes == AsmLineRecord.noneAddress ? nil : snes,
                kind: kind, byteCount: byteCount, regionKind: region, confidence: confidence,
                flagsBefore: flagsBefore, lineFlags: lineFlags, dbr: dbr, dp: dp, text: text,
                target: target == AsmLineRecord.noneAddress ? nil : target,
                targetFileOffset: targetOff == AsmLineRecord.noneAddress ? nil : targetOff,
                xrefInCount: xrefIn, bytes: bytes, tokens: tokens
            )
        }
    }

    /// The line's text as a `CTLine`, cached until the layout generation
    /// changes (address style, font or appearance).
    func line(line: UInt32, generation: Int, layout: AsmLineLayout) -> CTLine {
        if lineGeneration != generation {
            lines = Array(repeating: nil, count: lineCount)
            lineGeneration = generation
        }
        let i = Int(line - startRow)
        if let cached = lines[i] { return cached }
        let made = layout.makeLine(for: record(line: line))
        lines[i] = made
        return made
    }
}

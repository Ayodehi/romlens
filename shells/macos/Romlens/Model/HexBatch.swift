import CoreText
import Foundation

/// One decoded 64-byte record from a hex-row batch (layout in the core's
/// `viewmodel::hex_rows`).
struct HexRowRecord: Sendable {
    static let bytesPerRow = 16
    static let unmappedAddress: UInt32 = 0xFFFF_FFFF

    let fileOffset: UInt32
    let snesAddress: UInt32?
    let byteCount: Int
    let flags: UInt8
    let bytes: [UInt8]
    let spanIds: [UInt8]
    let ascii: [UInt8]

    var hasSpan: Bool { flags & 0b01 != 0 }
    var isLastRow: Bool { flags & 0b10 != 0 }
}

/// A batch of rows as fetched from the core, decoded on demand, with one
/// cached `CTLine` per row for the current address style.
final class HexBatch {
    static let rowsPerBatch: UInt32 = 256
    static let stride = 64
    static let headerLength = 8

    enum DecodeError: Error, Equatable {
        case tooShort, badVersion(UInt16), badStride(UInt16), truncated
    }

    let startRow: UInt32
    let rowCount: Int
    private let data: Data
    private var lines: [CTLine?]
    private var lineGeneration = -1

    init(startRow: UInt32, data: Data) throws {
        guard data.count >= Self.headerLength else { throw DecodeError.tooShort }
        let version = data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self) }
        let stride = data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 2, as: UInt16.self) }
        let rows = data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 4, as: UInt32.self) }
        guard version == 1 else { throw DecodeError.badVersion(version) }
        guard Int(stride) == Self.stride else { throw DecodeError.badStride(stride) }
        guard data.count >= Self.headerLength + Int(rows) * Self.stride else { throw DecodeError.truncated }
        self.startRow = startRow
        self.rowCount = Int(rows)
        self.data = data
        self.lines = Array(repeating: nil, count: Int(rows))
    }

    var endRow: UInt32 { startRow + UInt32(rowCount) }

    func contains(row: UInt32) -> Bool { row >= startRow && row < endRow }

    /// Decode one row (absolute row index).
    func record(row: UInt32) -> HexRowRecord {
        precondition(contains(row: row), "row \(row) not in batch \(startRow)..<\(endRow)")
        let base = Self.headerLength + Int(row - startRow) * Self.stride
        return data.withUnsafeBytes { raw -> HexRowRecord in
            let fileOffset = raw.loadUnaligned(fromByteOffset: base, as: UInt32.self)
            let snes = raw.loadUnaligned(fromByteOffset: base + 4, as: UInt32.self)
            let count = Int(raw[base + 8])
            let flags = raw[base + 9]
            let bytes = Array(raw[base + 12..<base + 28])
            let spans = Array(raw[base + 28..<base + 44])
            let ascii = Array(raw[base + 44..<base + 60])
            return HexRowRecord(
                fileOffset: fileOffset,
                snesAddress: snes == HexRowRecord.unmappedAddress ? nil : snes,
                byteCount: count, flags: flags, bytes: bytes, spanIds: spans, ascii: ascii
            )
        }
    }

    /// The row's text as a `CTLine`, cached until the layout generation
    /// changes (address style, font or appearance).
    func line(row: UInt32, generation: Int, layout: HexRowLayout) -> CTLine {
        if lineGeneration != generation {
            lines = Array(repeating: nil, count: rowCount)
            lineGeneration = generation
        }
        let i = Int(row - startRow)
        if let line = lines[i] { return line }
        let line = layout.makeLine(for: record(row: row))
        lines[i] = line
        return line
    }
}

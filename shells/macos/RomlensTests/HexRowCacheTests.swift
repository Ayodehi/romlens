import Foundation
import RomlensKit
import Testing
@testable import Romlens

@Suite struct HexRowCacheTests {
    /// A 1 MB LoROM: the fixture header at 0x7FC0 still scores, and there
    /// are 256 batches to exercise eviction.
    private func bigRom() throws -> Rom {
        var bytes = makeTestRom(mapping: .loRom)
        bytes.append(Data(repeating: 0xEA, count: (1 << 20) - bytes.count))
        return try Rom.fromBytes(bytes: bytes, name: "big.sfc")
    }

    @Test func fetchesOncePerBatch() throws {
        let cache = HexRowCache(rom: try bigRom(), capacity: 4)
        _ = cache.record(row: 0)
        _ = cache.record(row: 255)
        _ = cache.record(row: 100)
        #expect(cache.missCount == 1)
        #expect(cache.count == 1)
        _ = cache.record(row: 256)
        #expect(cache.missCount == 2)
        #expect(cache.count == 2)
    }

    @Test func evictsLeastRecentlyUsed() throws {
        let cache = HexRowCache(rom: try bigRom(), capacity: 3)
        for batch in 0..<3 { _ = cache.record(row: UInt32(batch) * 256) }
        #expect(cache.count == 3)
        _ = cache.record(row: 0) // batch 0 is now the most recent
        _ = cache.record(row: 3 * 256) // evicts batch 1
        #expect(cache.count == 3)
        #expect(cache.isCached(row: 0))
        #expect(!cache.isCached(row: 256))
        #expect(cache.isCached(row: 512))
        #expect(cache.isCached(row: 768))
        _ = cache.record(row: 256)
        #expect(cache.missCount == 5)
    }

    @Test func recordsBeyondTheImageAreClipped() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let cache = HexRowCache(rom: rom)
        let batch = cache.batch(containingRow: 0x7FF)
        #expect(batch.startRow == 0x700)
        #expect(batch.rowCount == 256)
        #expect(batch.record(row: 0x7FF).isLastRow)
    }
}

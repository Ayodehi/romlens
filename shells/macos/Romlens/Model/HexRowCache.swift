import Foundation
import RomlensKit

/// LRU cache of decoded row batches. 64 batches × 256 rows × 64 bytes is
/// about 1 MB, and a miss costs ~10 µs per batch (docs/10).
final class HexRowCache {
    let rom: Rom
    let capacity: Int
    private var batches: [UInt32: HexBatch] = [:]
    private var order: [UInt32] = [] // least recently used first
    private(set) var missCount = 0

    init(rom: Rom, capacity: Int = 64) {
        self.rom = rom
        self.capacity = max(1, capacity)
    }

    var count: Int { batches.count }

    func batch(containingRow row: UInt32) -> HexBatch {
        let start = row - row % HexBatch.rowsPerBatch
        if let hit = batches[start] {
            touch(start)
            return hit
        }
        missCount += 1
        let data = rom.hexRows(startRow: start, count: HexBatch.rowsPerBatch)
        // The core always returns a well-formed batch; a decode failure here
        // is a build skew between the framework and the app.
        let batch = try! HexBatch(startRow: start, data: data)
        batches[start] = batch
        order.append(start)
        while batches.count > capacity, let victim = order.first {
            order.removeFirst()
            batches.removeValue(forKey: victim)
        }
        return batch
    }

    func record(row: UInt32) -> HexRowRecord {
        batch(containingRow: row).record(row: row)
    }

    func isCached(row: UInt32) -> Bool {
        batches[row - row % HexBatch.rowsPerBatch] != nil
    }

    private func touch(_ start: UInt32) {
        if let i = order.firstIndex(of: start) {
            order.remove(at: i)
            order.append(start)
        }
    }
}

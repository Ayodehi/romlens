import Foundation
import RomlensKit

/// A batch of consecutive rows (or lines) decoded from one flat buffer.
protocol RowBatch: AnyObject {
    static var rowsPerBatch: UInt32 { get }
    init(startRow: UInt32, data: Data) throws
    var startRow: UInt32 { get }
}

/// LRU cache of decoded batches keyed by their first row. 64 batches of 256
/// hex rows is about 1 MB; a miss costs ~10 µs per batch (docs/10).
final class BatchCache<Batch: RowBatch> {
    let capacity: Int
    private let fetch: (UInt32, UInt32) -> Data
    private var batches: [UInt32: Batch] = [:]
    private var order: [UInt32] = [] // least recently used first
    private(set) var missCount = 0

    init(capacity: Int = 64, fetch: @escaping (UInt32, UInt32) -> Data) {
        self.capacity = max(1, capacity)
        self.fetch = fetch
    }

    var count: Int { batches.count }

    func batch(containingRow row: UInt32) -> Batch {
        let start = row - row % Batch.rowsPerBatch
        if let hit = batches[start] {
            touch(start)
            return hit
        }
        missCount += 1
        let data = fetch(start, Batch.rowsPerBatch)
        // The core always returns a well-formed batch; a decode failure here
        // is a build skew between the framework and the app.
        let batch = try! Batch(startRow: start, data: data)
        batches[start] = batch
        order.append(start)
        while batches.count > capacity, let victim = order.first {
            order.removeFirst()
            batches.removeValue(forKey: victim)
        }
        return batch
    }

    func isCached(row: UInt32) -> Bool {
        batches[row - row % Batch.rowsPerBatch] != nil
    }

    /// Drop everything (the snapshot or labels changed).
    func invalidateAll() {
        batches.removeAll()
        order.removeAll()
    }

    private func touch(_ start: UInt32) {
        if let i = order.firstIndex(of: start) {
            order.remove(at: i)
            order.append(start)
        }
    }
}

typealias HexRowCache = BatchCache<HexBatch>
typealias AsmLineCache = BatchCache<AsmBatch>

extension BatchCache where Batch == HexBatch {
    /// Rows straight from the ROM (header spans only).
    convenience init(rom: Rom, capacity: Int = 64) {
        self.init(capacity: capacity) { rom.hexRows(startRow: $0, count: $1) }
    }

    /// Rows through the workbench (spans plus the region lane).
    convenience init(workbench: Workbench, capacity: Int = 64) {
        self.init(capacity: capacity) { workbench.hexRows(startRow: $0, count: $1) }
    }

    func record(row: UInt32) -> HexRowRecord {
        batch(containingRow: row).record(row: row)
    }
}

extension BatchCache where Batch == AsmBatch {
    convenience init(workbench: Workbench, capacity: Int = 64) {
        self.init(capacity: capacity) { workbench.asmLines(startLine: $0, count: $1) }
    }

    func record(line: UInt32) -> AsmLineRecord {
        batch(containingRow: line).record(line: line)
    }
}

import Foundation
import Observation
import RomlensKit

/// Find References' state: what was asked about, and every place that
/// refers to it.
///
/// The inspector lists the first fifty references to the selection; this is
/// the whole list, which stays put while the selection moves through it, so
/// a reader can visit each caller in turn.
@MainActor
@Observable
final class ReferencesModel {
    struct Row: Equatable {
        let fileOffset: UInt32
        let snesAddress: UInt32?
        let kindName: String
        let certain: Bool
        /// An emulator saw it happen (an execution log).
        let observed: Bool
        /// The referring instruction, labels applied; empty when the
        /// reference is from data (a pointer or table entry).
        let text: String
        /// The routine the reference is in, as `SUB_8B8000+1C`.
        let routine: String?
    }

    private(set) var target: UInt32?
    /// The target's label, or its address when it has none.
    private(set) var targetName = ""
    private(set) var rows: [Row] = []
    private(set) var current: Int?

    var hasResults: Bool { target != nil }

    var summary: String {
        guard target != nil else { return "" }
        if rows.isEmpty { return "No references" }
        let position = current.map { "\($0 + 1) of " } ?? ""
        return "\(position)\(rows.count) reference\(rows.count == 1 ? "" : "s")"
    }

    func find(to address: UInt32, in workbench: Workbench) {
        target = address
        targetName = workbench.labelAt(snesAddress: address)?.name ?? formatSnesAddress(address: address)
        let routines = Self.routineLabels(workbench.labels())
        rows = workbench.xrefsTo(snesAddress: address).map { x in
            Row(
                fileOffset: x.fromOffset,
                snesAddress: x.fromAddress,
                kindName: x.kindName,
                certain: x.certain,
                observed: x.observed,
                text: workbench.instructionAt(fileOffset: x.fromOffset)?.text ?? "",
                routine: x.fromAddress.flatMap { Self.routine(containing: $0, in: routines) }
            )
        }
        current = nil
    }

    func clear() {
        target = nil
        targetName = ""
        rows = []
        current = nil
    }

    func select(_ index: Int) -> Row? {
        guard rows.indices.contains(index) else { return nil }
        current = index
        return rows[index]
    }

    /// The labels that can name a routine, by address. The analyzer's
    /// `CODE_` labels mark branch and jump targets inside routines, and its
    /// `DATA_`, `PTR_` and `JTBL_` labels are not code, so the nearest label
    /// of any kind would usually name a loop rather than the routine.
    static func routineLabels(_ labels: [LabelInfo]) -> [LabelInfo] {
        let inner = ["CODE_", "DATA_", "PTR_", "JTBL_"]
        return labels
            .filter { l in l.source != .auto || !inner.contains { l.name.hasPrefix($0) } }
            .sorted { $0.address < $1.address }
    }

    /// The nearest routine label at or before `address` in the same bank,
    /// with the distance past it.
    static func routine(containing address: UInt32, in sorted: [LabelInfo]) -> String? {
        var lo = 0, hi = sorted.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if sorted[mid].address <= address { lo = mid + 1 } else { hi = mid }
        }
        guard lo > 0 else { return nil }
        let l = sorted[lo - 1]
        guard l.address >> 16 == address >> 16 else { return nil }
        let delta = address - l.address
        return delta == 0 ? l.name : String(format: "%@+%X", l.name, delta)
    }
}

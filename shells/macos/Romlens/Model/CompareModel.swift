import Foundation
import Observation
import RomlensKit

/// The Compare tab's state (docs/22, D2): another version of the ROM,
/// analysed beside this one, and what changed from it to this one. The
/// other version is `a` throughout, this one `b`.
@MainActor
@Observable
final class CompareModel {
    enum State: Equatable {
        case idle
        case loading(String)
        case ready
        case failed(String)
    }

    /// A row of the list.
    enum Item: Hashable {
        case routine(Int)
        case data(Int)
        case run(Int)
        case move(Int)
    }

    private(set) var state: State = .idle
    /// The other version's file name, without its extension.
    private(set) var otherName: String?
    private(set) var info: ComparisonInfo?
    var selected: Item?
    @ObservationIgnored private(set) var other: Workbench?
    @ObservationIgnored private var comparedGeneration = -1

    var isActive: Bool { state != .idle }

    /// Analyse `other` and compare it with `this`.
    func start(this: Workbench, other: Workbench, name: String, generation: Int) async {
        self.other = other
        otherName = name
        info = nil
        selected = nil
        state = .loading("Analysing \(name)…")
        do {
            _ = try await other.analyze()
            state = .loading("Comparing…")
            info = try await this.compareWith(other: other)
            comparedGeneration = generation
            state = .ready
        } catch {
            state = .failed("\(error)")
        }
    }

    /// Compare again when this version's analysis or names changed.
    func refresh(this: Workbench, generation: Int) async {
        guard let other, state == .ready, generation != comparedGeneration else { return }
        comparedGeneration = generation
        if let fresh = try? await this.compareWith(other: other) {
            info = fresh
        }
    }

    func close() {
        other = nil
        otherName = nil
        info = nil
        selected = nil
        state = .idle
    }

    // MARK: Rows

    var routines: [RoutinePairInfo] { info?.routines ?? [] }
    var data: [DataChangeInfo] { info?.data ?? [] }
    /// The first thousand stretches; the rest are counted.
    var runs: ArraySlice<ByteRunInfo> { (info?.runs ?? []).prefix(1000) }
    var moves: [MovedBlockInfo] { info?.moves ?? [] }

    static func title(_ p: RoutinePairing) -> String {
        switch p {
        case .same: "same"
        case .moved: "moved"
        case .changed: "changed"
        case .added: "added"
        case .removed: "removed"
        }
    }

    static func title(_ k: ByteRunKind) -> String {
        switch k {
        case .changed: "changed"
        case .inserted: "inserted"
        case .deleted: "deleted"
        }
    }
}

import Foundation
import Observation
import RomlensKit

/// The sidebar's data: labels, regions and banks, loaded off the main actor
/// and filtered with a short debounce.
@MainActor
@Observable
final class NavigatorModel {
    enum Tab: String, CaseIterable, Identifiable {
        case labels, variables, regions, banks
        var id: String { rawValue }
        var title: String {
            switch self {
            case .labels: "Labels"
            case .variables: "Variables"
            case .regions: "Regions"
            case .banks: "Banks"
            }
        }
    }

    struct Bank: Identifiable, Hashable, Sendable {
        let bank: UInt8
        let fileOffset: UInt32
        let length: UInt32
        var id: UInt8 { bank }
    }

    var tab: Tab = .labels
    var filter = "" {
        didSet { scheduleFilter() }
    }
    private(set) var labels: [LabelInfo] = []
    private(set) var variables: [VariableInfo] = []
    private(set) var filteredVariables: [VariableInfo] = []
    private(set) var regions: [RegionInfo] = []
    private(set) var banks: [Bank] = []
    private(set) var filteredLabels: [LabelInfo] = []
    private(set) var filteredRegions: [RegionInfo] = []
    private(set) var isLoading = false
    /// Whether the region list is the largest few rather than all of them.
    private(set) var regionsTruncated = false
    @ObservationIgnored private var filterTask: Task<Void, Never>?
    @ObservationIgnored var filterDelay: Duration = .milliseconds(150)

    /// The largest of each kind, per kind.
    ///
    /// The navigator used to ask for every region and filter shell-side, which
    /// was fine at a thousand and is not at the forty thousand a trace import
    /// produces — the whole list would cross the FFI as generated records on
    /// every analysis, to be mostly discarded. The largest blocks are what a
    /// person navigates by, and the list says when it is not showing
    /// everything rather than pretending it is.
    static let regionLimit: UInt32 = 500

    /// Load from the workbench off the main actor.
    func reload(workbench: Workbench, rom: Rom) async {
        isLoading = true
        let limit = Self.regionLimit
        let (labels, regions, banks, variables) = await Task.detached(priority: .userInitiated) {
            let labels = workbench.labels()
            let variables = workbench.variables()
            var regions = workbench.regionsOfKind(kind: .code, limit: limit)
            regions.append(contentsOf: workbench.regionsOfKind(kind: .data, limit: limit))
            regions.sort { $0.start < $1.start }
            let banks = Self.banks(rom: rom)
            return (labels, regions, banks, variables)
        }.value
        self.labels = labels
        self.variables = variables
        self.regions = regions
        regionsTruncated = regions.count >= Int(limit)
        self.banks = banks
        applyFilter()
        isLoading = false
    }

    /// Banks computed shell-side by stepping the canonical address every 32 KB.
    nonisolated static func banks(rom: Rom) -> [Bank] {
        let len = rom.byteLen()
        var out: [Bank] = []
        var offset: UInt32 = 0
        while offset < len {
            let step = min(UInt32(0x8000), len - offset)
            if let a = rom.snesAddressFor(fileOffset: offset) {
                let bank = UInt8(a >> 16)
                if let last = out.last, last.bank == bank {
                    out[out.count - 1] = Bank(bank: bank, fileOffset: last.fileOffset, length: last.length + step)
                } else {
                    out.append(Bank(bank: bank, fileOffset: offset, length: step))
                }
            }
            offset += step
        }
        return out
    }

    private func scheduleFilter() {
        filterTask?.cancel()
        let delay = filterDelay
        filterTask = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard let self, !Task.isCancelled else { return }
            self.applyFilter()
        }
    }

    func applyFilter() {
        filteredLabels = Self.filter(labels, query: filter)
        let q = filter.trimmingCharacters(in: .whitespaces).lowercased()
        filteredRegions = q.isEmpty ? regions : regions.filter { $0.name.lowercased().contains(q) }
        filteredVariables = q.isEmpty ? variables : variables.filter { $0.name.lowercased().contains(q) }
    }

    /// Case-insensitive contains, or an address prefix when the query starts
    /// with `$`; user labels first, then by address.
    nonisolated static func filter(_ labels: [LabelInfo], query: String) -> [LabelInfo] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        let matched: [LabelInfo]
        if q.isEmpty {
            matched = labels
        } else if q.hasPrefix("$") {
            let needle = q.dropFirst().replacingOccurrences(of: ":", with: "")
            matched = labels.filter { l in
                let hex = String(l.address, radix: 16).lowercased()
                let padded = String(repeating: "0", count: max(0, 6 - hex.count)) + hex
                return padded.hasPrefix(needle)
            }
        } else {
            matched = labels.filter { $0.name.lowercased().contains(q) }
        }
        return matched.sorted { a, b in
            let ua = a.source == .user || a.source == .imported
            let ub = b.source == .user || b.source == .imported
            if ua != ub { return ua }
            return a.address < b.address
        }
    }
}

import Foundation
import Observation
import RomlensKit

/// ⌘F's state: the query, the hits and which one is current.
///
/// Kept beside `RomViewModel` rather than inside it because it survives the
/// sheet closing — ⌘G steps through the results of a search made minutes ago,
/// which is the whole reason Find is not just a jump.
@MainActor
@Observable
final class SearchModel {
    enum Mode: String, CaseIterable, Identifiable {
        case bytes, text
        var id: String { rawValue }
        var title: String {
            switch self {
            case .bytes: "Bytes"
            case .text: "Text"
            }
        }
    }

    /// Enough to fill the results list without paying for a whole ROM of
    /// matches nobody scrolls to. The count says when it was reached.
    static let maxHits: UInt32 = 500
    /// Bytes either side of a hit, matching `romlens search`.
    static let context: UInt32 = 8

    var query = ""
    var mode: Mode = .bytes
    var ignoreCase = false
    private(set) var hits: [SearchHit] = []
    private(set) var current: Int?
    private(set) var error: String?
    /// The query the current hits came from, so the list is never stale.
    private(set) var searched = ""

    var hasResults: Bool { !hits.isEmpty }
    var reachedLimit: Bool { UInt32(hits.count) >= Self.maxHits }

    var summary: String {
        if let error { return error }
        if searched.isEmpty { return "" }
        if hits.isEmpty { return "No matches" }
        let position = current.map { "\($0 + 1) of " } ?? ""
        let limit = reachedLimit ? " (first \(Self.maxHits))" : ""
        return "\(position)\(hits.count) match\(hits.count == 1 ? "" : "es")\(limit)"
    }

    func search(in workbench: Workbench) {
        let trimmed = query.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else {
            clear()
            return
        }
        do {
            hits = try workbench.search(query: SearchQuery(
                pattern: trimmed,
                text: mode == .text,
                ignoreCase: mode == .text && ignoreCase,
                start: 0,
                len: UInt32.max,
                max: Self.maxHits,
                context: Self.context
            ))
            error = nil
            searched = trimmed
            current = hits.isEmpty ? nil : 0
        } catch {
            self.error = error.localizedDescription
            hits = []
            current = nil
            searched = trimmed
        }
    }

    func clear() {
        hits = []
        current = nil
        error = nil
        searched = ""
    }

    /// ⌘G and ⇧⌘G. Wraps, because a search that stops at the end of the image
    /// makes a reader wonder whether it found everything.
    @discardableResult
    func step(by delta: Int) -> SearchHit? {
        guard !hits.isEmpty else { return nil }
        let next = ((current ?? -1) + delta + hits.count) % hits.count
        current = next
        return hits[next]
    }

    func select(_ index: Int) -> SearchHit? {
        guard hits.indices.contains(index) else { return nil }
        current = index
        return hits[index]
    }
}

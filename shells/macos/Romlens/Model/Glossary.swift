import Foundation
import RomlensKit

/// The glossary (docs/27): the SNES's acronyms and initialisms from the
/// core, found in the tutor's text and linked the first time each appears.
/// A click shows the entry in a bubble (`GlossaryPopover`).
enum Glossary {
    /// Every spelling, the term's own and its others, to its entry.
    static let entries: [String: GlossaryEntryInfo] = {
        var d: [String: GlossaryEntryInfo] = [:]
        for e in glossary() {
            for spelling in [e.term] + e.also where d[spelling] == nil {
                d[spelling] = e
            }
        }
        return d
    }()

    /// A Markdown link or inline code, kept whole, or a term standing on
    /// its own: the longest spelling first, so `DSP-1` wins over `DSP`.
    static let pattern: NSRegularExpression = {
        let words = entries.keys.sorted { $0.count > $1.count }.map(NSRegularExpression.escapedPattern(for:))
        let term = #"(?<![\w$:/\-])(?:"# + words.joined(separator: "|") + #")(?![\w\-])"#
        return try! NSRegularExpression(pattern: #"\[[^\]]*\]\([^)]*\)|`[^`]*`|"# + term)
    }()

    static func entry(_ spelling: String) -> GlossaryEntryInfo? { entries[spelling] }

    /// The link a term's first appearance carries.
    static func url(_ term: String) -> URL {
        var c = URLComponents()
        c.scheme = "romlens"
        c.host = "g"
        c.path = "/"
        c.queryItems = [URLQueryItem(name: "t", value: term)]
        return c.url!
    }

    /// The entry a glossary link names, or nil for any other link.
    static func entry(for url: URL) -> GlossaryEntryInfo? {
        guard url.scheme == "romlens", url.host() == "g",
              let t = URLComponents(url: url, resolvingAgainstBaseURL: false)?
                  .queryItems?.first(where: { $0.name == "t" })?.value
        else { return nil }
        return entries[t]
    }

    /// A line of Markdown with each term not yet in `seen` made a link, and
    /// added to it. Links already there are left alone; inline code is
    /// linked only when it is a term and nothing else (`` `VMAIN` ``).
    static func link(_ line: String, seen: inout Set<String>) -> String {
        let ns = line as NSString
        var out = ""
        var last = 0
        for m in pattern.matches(in: line, range: NSRange(location: 0, length: ns.length)) {
            out += ns.substring(with: NSRange(location: last, length: m.range.location - last))
            last = m.range.location + m.range.length
            let found = ns.substring(with: m.range)
            let code = found.hasPrefix("`")
            let spelling = code ? String(found.dropFirst().dropLast()) : found
            guard !found.hasPrefix("["), let e = entries[spelling], !seen.contains(e.term) else {
                out += found
                continue
            }
            seen.insert(e.term)
            out += "[\(found)](\(url(e.term).absoluteString))"
        }
        out += ns.substring(from: last)
        return out
    }
}

import AppKit
import RomlensKit
import SwiftUI

/// An answer's text: Markdown, with each citation (`$BB:AAAA`, `frame N`)
/// a link into the editor, and code blocks coloured as Romlens colours
/// its own (docs/24, "The transcript").
struct MessageText: View {
    let tutor: TutorModel
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Self.rendered(text).enumerated()), id: \.offset) { _, s in
                switch s {
                case .prose(let p):
                    Text(p)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                case .code(let lang, let body):
                    CodeBlock(tutor: tutor, language: lang, code: body)
                case .table(let header, let rows):
                    TableBlock(header: header, rows: rows)
                case .rule:
                    Divider()
                }
            }
        }
    }

    /// A segment made ready to show: prose and table cells as attributed
    /// text, with each glossary term linked the first time the message
    /// uses it.
    enum Rendered {
        case prose(AttributedString)
        case code(language: String, body: String)
        case table(header: [AttributedString], rows: [[AttributedString]])
        case rule
    }

    static func rendered(_ text: String) -> [Rendered] {
        var seen = Set<String>()
        return segments(text).map { s in
            switch s {
            case .prose(let p): return .prose(prose(p, seen: &seen))
            case .code(let lang, let body): return .code(language: lang, body: body)
            case .table(let header, let rows):
                let h = header.map { prose($0, seen: &seen) }
                return .table(header: h, rows: rows.map { $0.map { prose($0, seen: &seen) } })
            case .rule: return .rule
            }
        }
    }

    enum Segment: Equatable {
        case prose(String)
        case code(language: String, body: String)
        /// A Markdown pipe table; each row has as many cells as the header.
        case table(header: [String], rows: [[String]])
        /// `---` on its own line.
        case rule
    }

    /// Prose, fenced code, tables and rules, apart. An unclosed fence
    /// (still streaming) is code to the end.
    static func segments(_ text: String) -> [Segment] {
        var out: [Segment] = []
        var prose: [Substring] = []
        var code: [Substring]?
        var lang = ""
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        func flush() {
            let p = prose.joined(separator: "\n").trimmingCharacters(in: .newlines)
            if !p.isEmpty { out.append(.prose(p)) }
            prose = []
        }
        var i = 0
        while i < lines.count {
            let line = lines[i]
            let t = line.trimmingCharacters(in: .whitespaces)
            i += 1
            if code == nil, t.hasPrefix("|"), i < lines.count, isTableRule(lines[i]) {
                let header = cells(t)
                var rows: [[String]] = []
                i += 1
                while i < lines.count {
                    let r = lines[i].trimmingCharacters(in: .whitespaces)
                    guard r.hasPrefix("|") else { break }
                    var c = cells(r)
                    if c.count < header.count { c += Array(repeating: "", count: header.count - c.count) }
                    rows.append(Array(c.prefix(header.count)))
                    i += 1
                }
                flush()
                out.append(.table(header: header, rows: rows))
                continue
            }
            if code == nil, t.range(of: #"^([-*_])( *\1){2,}$"#, options: .regularExpression) != nil {
                flush()
                out.append(.rule)
                continue
            }
            if t.hasPrefix("```") {
                if var c = code {
                    if c.last?.isEmpty == true { c.removeLast() }
                    out.append(.code(language: lang, body: c.joined(separator: "\n")))
                    code = nil
                } else {
                    flush()
                    lang = String(t.dropFirst(3)).lowercased()
                    code = []
                }
            } else if code != nil {
                code!.append(line)
            } else {
                prose.append(line)
            }
        }
        if let c = code { out.append(.code(language: lang, body: c.joined(separator: "\n"))) }
        let rest = prose.joined(separator: "\n").trimmingCharacters(in: .newlines)
        if !rest.isEmpty { out.append(.prose(rest)) }
        return out.filter { if case .prose(let p) = $0 { return !p.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } else { return true } }
    }

    /// `|---|:--:|`: the line under a table's header.
    static func isTableRule(_ line: Substring) -> Bool {
        let t = line.trimmingCharacters(in: .whitespaces)
        return t.contains("-") && t.contains("|")
            && t.range(of: #"^\|?( *:?-+:? *\|)+ *(:?-+:? *)?$"#, options: .regularExpression) != nil
    }

    /// A table row's cells, without the outer pipes; `\|` is a pipe in a
    /// cell, and so is one inside backticks.
    static func cells(_ row: String) -> [String] {
        var out: [String] = []
        var cell = ""
        var inCode = false
        var escaped = false
        for ch in row {
            if escaped {
                if ch != "|" { cell.append("\\") }
                cell.append(ch)
                escaped = false
                continue
            }
            switch ch {
            case "\\": escaped = true
            case "`": inCode.toggle(); cell.append(ch)
            case "|" where !inCode: out.append(cell); cell = ""
            default: cell.append(ch)
            }
        }
        out.append(cell)
        if row.hasPrefix("|") { out.removeFirst() }
        if row.hasSuffix("|"), !out.isEmpty { out.removeLast() }
        return out.map { $0.trimmingCharacters(in: .whitespaces) }
    }

    /// Markdown for a run of prose, with citations made links. Headings
    /// become bold lines and list items bullets, since the text keeps its
    /// line breaks.
    static func prose(_ text: String) -> AttributedString {
        var seen = Set<String>()
        return prose(text, seen: &seen)
    }

    /// As `prose(_:)`, linking only the glossary terms not in `seen`.
    static func prose(_ text: String, seen: inout Set<String>) -> AttributedString {
        var lines: [String] = []
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            var l = String(line)
            if let r = l.range(of: #"^#{1,6} +"#, options: .regularExpression) {
                l = "**" + l[r.upperBound...] + "**"
            } else if let r = l.range(of: #"^(\s*)[-*] +"#, options: .regularExpression) {
                let indent = l[l.startIndex..<r.upperBound].prefix { $0 == " " }
                l = indent + "• " + l[r.upperBound...]
            }
            lines.append(link(Glossary.link(l, seen: &seen)))
        }
        let md = lines.joined(separator: "\n")
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        guard var a = try? AttributedString(markdown: md, options: options) else { return AttributedString(text) }
        // A term's link is marked with a dotted underline, apart from the
        // links into the editor.
        for run in a.runs where run.link?.host() == "g" {
            a[run.range].underlineStyle = Text.LineStyle(pattern: .dot)
        }
        return a
    }

    /// `$80:8000` (bare or in backticks) and `frame 12` as Markdown links.
    static func link(_ line: String) -> String {
        var s = line
        s = s.replacingOccurrences(
            of: #"`\$([0-9A-Fa-f]{2}):([0-9A-Fa-f]{4})`"#,
            with: "[`\\$$1:$2`](romlens://a/$1$2)", options: .regularExpression)
        s = s.replacingOccurrences(
            of: #"(?<![\[`\w])\$([0-9A-Fa-f]{2}):([0-9A-Fa-f]{4})(?![\w`\]])"#,
            with: "[\\$$1:$2](romlens://a/$1$2)", options: .regularExpression)
        s = s.replacingOccurrences(
            of: #"(?<![\[\w])([Ff]rame) ([0-9]+)\b"#,
            with: "[$1 $2](romlens://f/$2)", options: .regularExpression)
        return s
    }
}

/// A Markdown table in an answer: a bold header, rows between rules, each
/// cell inline Markdown with its citations as links.
struct TableBlock: View {
    let header: [AttributedString]
    let rows: [[AttributedString]]

    var body: some View {
        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 6) {
            GridRow {
                ForEach(Array(header.enumerated()), id: \.offset) { _, h in
                    Text(h).bold()
                }
            }
            Divider()
            ForEach(Array(rows.enumerated()), id: \.offset) { n, row in
                GridRow {
                    ForEach(Array(row.enumerated()), id: \.offset) { _, c in
                        Text(c).fixedSize(horizontal: false, vertical: true)
                    }
                }
                if n < rows.count - 1 { Divider().opacity(0.5) }
            }
        }
        .textSelection(.enabled)
        .padding(8)
        .background(RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .textBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.2)))
    }
}

/// Code in an answer: C coloured by the core's lexer with the C view's
/// palette, 65816 and SPC700 assembly by a light pass.
struct CodeBlock: View {
    let tutor: TutorModel
    let language: String
    let code: String

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Text(coloured)
                .font(.system(.callout, design: .monospaced))
                .textSelection(.enabled)
                .padding(8)
        }
        .background(RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .textBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.2)))
        .overlay(alignment: .topTrailing) {
            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(code, forType: .string)
            } label: { Image(systemName: "doc.on.doc") }
                .buttonStyle(.borderless)
                .padding(4)
                .help("Copy")
        }
    }

    private var coloured: AttributedString {
        var a = AttributedString(code)
        let isC = ["c", "h", "cpp"].contains(language) || (language.isEmpty && code.contains(";") && code.contains("("))
        if isC {
            let ns = code as NSString
            for t in tutor.rom.session.workbench.lexC(text: code) {
                let r = NSRange(location: Int(t.start), length: Int(t.len))
                guard r.location + r.length <= ns.length, let range = Range(r, in: code),
                      let ar = Range(range, in: a) else { continue }
                a[ar].foregroundColor = Color(nsColor: CTokenPalette.color(for: t.kind))
            }
        } else {
            Self.asm(code, into: &a)
        }
        return a
    }

    /// A mnemonic after the address and bytes, `$` numbers, `;` comments.
    static func asm(_ code: String, into a: inout AttributedString) {
        let comment = try? NSRegularExpression(pattern: #";.*$"#, options: .anchorsMatchLines)
        let number = try? NSRegularExpression(pattern: #"#?\$[0-9A-Fa-f]+"#)
        let mnemonic = try? NSRegularExpression(pattern: #"(?<=\s|^)[A-Z]{3}(?=\s|$|\.)"#, options: .anchorsMatchLines)
        let ns = code as NSString
        func paint(_ re: NSRegularExpression?, _ color: NSColor) {
            re?.enumerateMatches(in: code, range: NSRange(location: 0, length: ns.length)) { m, _, _ in
                guard let m, let r = Range(m.range, in: code), let ar = Range(r, in: a) else { return }
                a[ar].foregroundColor = Color(nsColor: color)
            }
        }
        paint(mnemonic, .systemPurple)
        paint(number, .systemTeal)
        paint(comment, .systemGreen)
    }
}

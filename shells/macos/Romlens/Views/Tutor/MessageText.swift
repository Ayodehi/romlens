import AppKit
import RomlensKit
import SwiftUI

/// An answer's text: Markdown, with each citation (`$BB:AAAA`, `frame N`)
/// a link into the main window, and code blocks coloured as Romlens colours
/// its own (docs/24, "The transcript").
struct MessageText: View {
    let tutor: TutorModel
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Self.segments(text).enumerated()), id: \.offset) { _, s in
                switch s {
                case .prose(let p):
                    Text(Self.prose(p))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                case .code(let lang, let body):
                    CodeBlock(tutor: tutor, language: lang, code: body)
                }
            }
        }
    }

    enum Segment: Equatable {
        case prose(String)
        case code(language: String, body: String)
    }

    /// Prose and fenced code, apart. An unclosed fence (still streaming)
    /// is code to the end.
    static func segments(_ text: String) -> [Segment] {
        var out: [Segment] = []
        var prose: [Substring] = []
        var code: [Substring]?
        var lang = ""
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("```") {
                if var c = code {
                    if c.last?.isEmpty == true { c.removeLast() }
                    out.append(.code(language: lang, body: c.joined(separator: "\n")))
                    code = nil
                } else {
                    if !prose.isEmpty { out.append(.prose(prose.joined(separator: "\n"))) }
                    prose = []
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

    /// Markdown for a run of prose, with citations made links. Headings
    /// become bold lines and list items bullets, since the text keeps its
    /// line breaks.
    static func prose(_ text: String) -> AttributedString {
        var lines: [String] = []
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            var l = String(line)
            if let r = l.range(of: #"^#{1,6} +"#, options: .regularExpression) {
                l = "**" + l[r.upperBound...] + "**"
            } else if let r = l.range(of: #"^(\s*)[-*] +"#, options: .regularExpression) {
                let indent = l[l.startIndex..<r.upperBound].prefix { $0 == " " }
                l = indent + "• " + l[r.upperBound...]
            }
            lines.append(link(l))
        }
        let md = lines.joined(separator: "\n")
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: md, options: options)) ?? AttributedString(text)
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

import AppKit
import RomlensKit
import SwiftUI

/// A glossary entry in a bubble at the click (docs/27): the term, the words
/// it stands for, and a sentence or two. Clicking anywhere else closes it.
@MainActor
enum GlossaryPopover {
    private static var shown: NSPopover?

    /// Shows `entry` pointing at the mouse in the key window. `ask` puts a
    /// question about it in the composer.
    static func show(_ entry: GlossaryEntryInfo, ask: ((String) -> Void)? = nil) {
        guard let window = NSApp.keyWindow, let view = window.contentView else { return }
        let p = view.convert(window.convertPoint(fromScreen: NSEvent.mouseLocation), from: nil)
        shown?.close()
        let popover = NSPopover()
        popover.behavior = .transient
        let close = { [weak popover] in popover?.close() }
        let host = NSHostingController(rootView: GlossaryCard(
            entry: entry,
            ask: ask.map { a in { a(entry.term); close() } }))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        popover.show(
            relativeTo: NSRect(x: p.x - 2, y: p.y - 2, width: 4, height: 4),
            of: view,
            preferredEdge: view.isFlipped ? .minY : .maxY)
        shown = popover
    }
}

/// What the bubble shows.
struct GlossaryCard: View {
    let entry: GlossaryEntryInfo
    var ask: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(entry.term).font(.headline)
                Spacer()
                Text(kind).font(.caption).foregroundStyle(.secondary)
            }
            Text(entry.words).font(.subheadline).foregroundStyle(.secondary)
            Text(Self.inline(entry.about))
                .fixedSize(horizontal: false, vertical: true)
            if !entry.also.isEmpty {
                Text("Also written \(entry.also.joined(separator: ", "))")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let ask {
                Button("Ask the tutor about \(entry.term)", action: ask)
                    .buttonStyle(.link)
                    .font(.caption)
            }
        }
        .padding(12)
        .frame(width: 300, alignment: .leading)
    }

    private var kind: String {
        switch entry.kind {
        case .term: "Glossary"
        case .register: "Register"
        case .dspRegister: "S-DSP register"
        }
    }

    /// The entry's text, with its `code` shown as code.
    static func inline(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }
}

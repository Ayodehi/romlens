import AppKit
import RomlensKit
import UniformTypeIdentifiers

/// File › Import: an execution trace or a symbol file.
///
/// The report is not optional. An importer that rewrote forty names and did
/// not say so would be the exact failure `io::import::symbols` is written to
/// avoid, so whatever the core reports is shown before the sheet closes.
@MainActor
enum ImportController {
    enum Kind {
        case trace, symbols

        var title: String {
            switch self {
            case .trace: "Import Execution Trace"
            case .symbols: "Import Symbols"
            }
        }

        var message: String {
            switch self {
            case .trace:
                return "A Mesen2 .cdl or a bsnes-plus usage map. What an emulator saw execute outranks the disassembler's guesses, and the recorded M/X widths fix what static analysis cannot."
            case .symbols:
                return "A WLA-DX or bsnes-plus .sym, a no$sns .sym, or a VICE .lbl. Your own names are never overwritten."
            }
        }

        var extensions: [String] {
            switch self {
            case .trace: ["cdl", "map", "bin", "usage"]
            case .symbols: ["sym", "lbl", "txt"]
            }
        }
    }

    static func run(_ kind: Kind, document: ProjectDocument, window: NSWindow?) {
        guard let session = document.session else { return }
        let panel = NSOpenPanel()
        panel.title = kind.title
        panel.message = kind.message
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = kind.extensions.compactMap { UTType(filenameExtension: $0) }
        // A trace has no reserved extension, so the panel must not refuse one
        // that a person exported under some other name.
        panel.allowsOtherFileTypes = true
        let host = window ?? NSApp.keyWindow
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let url = panel.url else { return }
            apply(kind, url: url, session: session, document: document, window: host)
        }
        if let host {
            panel.beginSheetModal(for: host, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }

    private static func apply(
        _ kind: Kind,
        url: URL,
        session: WorkbenchSession,
        document: ProjectDocument,
        window: NSWindow?
    ) {
        let name = url.lastPathComponent
        do {
            let result: ImportResult
            switch kind {
            case .trace:
                result = try session.importTrace(source: name, bytes: try Data(contentsOf: url))
            case .symbols:
                result = try session.importSymbols(
                    source: name,
                    text: try String(contentsOf: url, encoding: .utf8)
                )
            }
            document.updateChangeCount(.changeDone)
            report(kind, result, window: window)
        } catch {
            present(
                style: .warning,
                title: "Could not import \(name)",
                text: error.localizedDescription,
                window: window
            )
        }
    }

    /// Everything the core reported, including what it could not use.
    static func summary(_ kind: Kind, _ r: ImportResult) -> String {
        var lines: [String] = []
        switch kind {
        case .trace:
            lines.append("\(r.executedBytes) bytes executed, \(r.readBytes) read, as \(r.format).")
            if r.hasWidths {
                lines.append("The recorded M/X widths now steer the disassembler.")
            }
        case .symbols:
            lines.append("\(r.labelsAdded) labels added, \(r.labelsReplaced) replaced, \(r.commentsAdded) comments added, as \(r.format).")
            if r.keptUser > 0 {
                lines.append("\(r.keptUser) kept: you had already named them.")
            }
            if !r.rewritten.isEmpty {
                let shown = r.rewritten.prefix(8).joined(separator: "\n  ")
                let more = r.rewritten.count > 8 ? "\n  … and \(r.rewritten.count - 8) more" : ""
                lines.append("\(r.rewritten.count) names rewritten to be usable:\n  \(shown)\(more)")
            }
            if !r.skipped.isEmpty {
                lines.append("\(r.skipped.count) lines not understood.")
            }
        }
        if !r.notice.isEmpty {
            lines.append("Notice kept with the project:\n\(r.notice)")
        }
        return lines.joined(separator: "\n\n")
    }

    private static func report(_ kind: Kind, _ result: ImportResult, window: NSWindow?) {
        present(
            style: .informational,
            title: "Imported \(result.source)",
            text: summary(kind, result),
            window: window
        )
    }

    private static func present(
        style: NSAlert.Style,
        title: String,
        text: String,
        window: NSWindow?
    ) {
        let alert = NSAlert()
        alert.alertStyle = style
        alert.messageText = title
        alert.informativeText = text
        if let window {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
    }
}

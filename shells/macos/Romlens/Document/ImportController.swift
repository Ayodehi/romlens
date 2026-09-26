import AppKit
import RomlensKit
import UniformTypeIdentifiers

/// File › Import: an execution trace, a symbol file or ca65's debug
/// information.
///
/// The report is not optional. An importer that rewrote forty names and did
/// not say so would be the exact failure `io::import::symbols` is written to
/// avoid, so whatever the core reports is shown before the sheet closes.
@MainActor
enum ImportController {
    enum Kind {
        case trace, symbols, dbg

        var title: String {
            switch self {
            case .trace: "Import Execution Trace"
            case .symbols: "Import Symbols"
            case .dbg: "Import ca65 Debug Information"
            }
        }

        var message: String {
            switch self {
            case .trace:
                // Short: the panel grows as wide as its message is long.
                return "A Mesen .cdl or execution log (.mxlog), or a bsnes-plus usage map."
            case .symbols:
                return "A WLA-DX or bsnes-plus .sym, a no$sns .sym, or a VICE .lbl. Your own names are never overwritten."
            case .dbg:
                return "The .dbg ld65 writes with --dbgfile: its labels, and which source line made which bytes."
            }
        }

        var extensions: [String] {
            switch self {
            case .trace: ["cdl", "map", "bin", "usage", "mxlog"]
            case .symbols: ["sym", "lbl", "txt"]
            case .dbg: ["dbg"]
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
            if kind == .dbg {
                withSources(of: url, window: host) {
                    apply(kind, url: url, session: session, document: document, window: host)
                }
            } else {
                apply(kind, url: url, session: session, document: document, window: host)
            }
        }
        if let host {
            panel.beginSheetModal(for: host, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }

    /// The sources are read beside the `.dbg`, which the sandbox allows
    /// only in a folder the person chose: ask once, unless it is remembered.
    /// Declining still imports; the Source tab asks again.
    private static func withSources(of dbg: URL, window: NSWindow?, then: @escaping @MainActor () -> Void) {
        let folder = dbg.deletingLastPathComponent()
        if SourceFolders.folder(holding: folder.appendingPathComponent("x").path) != nil {
            then()
            return
        }
        SourceFolders.ask(
            start: folder,
            message: "Romlens shows the sources \(dbg.lastPathComponent) names. Choose the folder that holds them (usually this one) to let it read them.",
            window: window
        ) { _ in
            // The sheet must be gone before the report's alert appears.
            DispatchQueue.main.async { then() }
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
            case .dbg:
                result = try session.importDbg(
                    source: name,
                    dir: url.deletingLastPathComponent().path,
                    text: try String(contentsOf: url, encoding: .utf8)
                )
                if let model = document.model {
                    model.source.reload(workbench: model.workbench)
                    model.editorTab = .source
                }
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
            if !r.detail.isEmpty {
                lines.append("Execution log: \(r.detail). Its calls, jumps and reads join the references, marked “seen”.")
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
        case .dbg:
            lines.append("\(r.labelsAdded) labels added, \(r.labelsReplaced) replaced. \(r.detail).")
            if r.keptUser > 0 {
                lines.append("\(r.keptUser) kept: you had already named them.")
            }
            if !r.rewritten.isEmpty {
                let shown = r.rewritten.prefix(8).joined(separator: "\n  ")
                let more = r.rewritten.count > 8 ? "\n  … and \(r.rewritten.count - 8) more" : ""
                lines.append("\(r.rewritten.count) names rewritten to be usable:\n  \(shown)\(more)")
            }
            if !r.skipped.isEmpty {
                lines.append("\(r.skipped.count) records not understood.")
            }
            lines.append("The Source tab shows each line beside the bytes it made.")
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

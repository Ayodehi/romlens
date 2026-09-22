import AppKit
import RomlensKit
import UniformTypeIdentifiers

/// File › Export: the assembly listing (with the docs/12 message), the
/// share-safe labels and comments, or the full symbol file.
@MainActor
enum ExportController {
    enum Kind {
        case assembly, annotations, symbols

        var title: String {
            switch self {
            case .assembly: "Export Assembly Listing"
            case .annotations: "Export Labels and Comments"
            case .symbols: "Export Symbol File"
            }
        }

        var fileExtension: String {
            switch self {
            case .assembly: "asm"
            case .annotations, .symbols: "sym"
            }
        }
    }

    static let assemblyMessage = "An assembly listing contains the ROM's bytes by nature. Keep it for local use; share labels and comments instead."

    static func run(_ kind: Kind, document: ProjectDocument, window: NSWindow?) {
        guard let workbench = document.workbench else { return }
        let panel = NSSavePanel()
        panel.title = kind.title
        panel.canCreateDirectories = true
        panel.allowedContentTypes = [UTType(filenameExtension: kind.fileExtension) ?? .plainText]
        let base = document.displayName ?? "Romlens"
        panel.nameFieldStringValue = "\(base).\(kind.fileExtension)"
        var includeBytes = false
        if kind == .assembly {
            panel.message = assemblyMessage
            let checkbox = NSButton(checkboxWithTitle: "Include byte columns", target: nil, action: nil)
            checkbox.state = .off
            panel.accessoryView = checkbox
            panel.beginSheetModal(for: window ?? NSApp.keyWindow ?? NSWindow()) { response in
                guard response == .OK, let url = panel.url else { return }
                includeBytes = checkbox.state == .on
                write(kind, includeBytes: includeBytes, workbench: workbench, to: url, document: document)
            }
            return
        }
        panel.beginSheetModal(for: window ?? NSApp.keyWindow ?? NSWindow()) { response in
            guard response == .OK, let url = panel.url else { return }
            write(kind, includeBytes: false, workbench: workbench, to: url, document: document)
        }
    }

    /// Generation off the main actor, the write on it.
    private static func write(_ kind: Kind, includeBytes: Bool, workbench: Workbench, to url: URL, document: ProjectDocument) {
        Task {
            let text = await Task.detached(priority: .userInitiated) {
                generate(kind, includeBytes: includeBytes, workbench: workbench)
            }.value
            do {
                try text.write(to: url, atomically: true, encoding: .utf8)
            } catch {
                document.presentError(error)
            }
        }
    }

    /// The text for a kind. Byte columns are added shell-side from the
    /// listing's own lines so the core's export stays byte-exact.
    nonisolated static func generate(_ kind: Kind, includeBytes: Bool, workbench: Workbench) -> String {
        switch kind {
        case .assembly:
            let listing = workbench.exportAsar(start: nil, len: nil)
            return includeBytes ? withByteColumns(listing, workbench: workbench) : listing
        case .annotations:
            return workbench.exportSymbols(includeAuto: false)
        case .symbols:
            return workbench.exportSymbols(includeAuto: true)
        }
    }

    /// Prefix each instruction line with its bytes as a comment column.
    nonisolated static func withByteColumns(_ listing: String, workbench: Workbench) -> String {
        var out = String()
        out.reserveCapacity(listing.count + listing.count / 4)
        var pc: UInt32? = nil
        for line in listing.split(separator: "\n", omittingEmptySubsequences: false) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("org $"), let v = UInt32(trimmed.dropFirst(5), radix: 16) {
                pc = workbench.rom().fileOffsetFor(snesAddress: v)
                out += line + "\n"
                continue
            }
            if line.hasPrefix("  "), let offset = pc, !trimmed.hasPrefix(";") {
                let head = trimmed.split(separator: " ").first.map(String.init) ?? ""
                var len: UInt32 = 0
                if head.hasPrefix("db") || head.hasPrefix("dw") || head.hasPrefix("dl") {
                    let width: UInt32 = head.hasPrefix("dw") ? 2 : (head.hasPrefix("dl") ? 3 : 1)
                    let body = trimmed.split(separator: ";").first.map(String.init) ?? trimmed
                    let values = body.dropFirst(head.count).split(separator: ",").count
                    len = UInt32(values) * width
                } else if let insn = workbench.instructionAt(fileOffset: offset), insn.fileOffset == offset {
                    len = UInt32(insn.len)
                }
                if len > 0 {
                    let bytes: [UInt8] = workbench.disassemble(fileOffset: offset, count: 1, flags: nil).first.map { Array($0.bytes) } ?? []
                    let hex = bytes.map { String(format: "%02X", $0) }.joined(separator: " ")
                    out += "  ; \(formatFileOffset(offset: offset))  \(hex.padding(toLength: 11, withPad: " ", startingAt: 0))\n"
                    pc = offset + len
                }
            }
            out += line + "\n"
        }
        return out
    }
}

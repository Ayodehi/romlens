import Foundation
import Observation
import RomlensKit

/// The Source tab's state (docs/22, S2): the source files of the project's
/// imported `.dbg` files, the one shown, its text, and which of its lines
/// made bytes.
@MainActor
@Observable
final class SourceModel {
    private(set) var files: [SourceFileInfo] = []
    /// An index into `files`.
    private(set) var shown: Int?
    /// The shown file's lines, or nil when it cannot be read.
    private(set) var text: [String]?
    /// Why `text` is nil.
    private(set) var problem: String?
    /// The shown file's lines that made bytes, by line number.
    private(set) var byLine: [UInt32: SourceLineInfo] = [:]
    /// Bumped whenever the shown file or its text changes.
    private(set) var generation = 0

    var hasFiles: Bool { !files.isEmpty }
    var shownFile: SourceFileInfo? { shown.map { files[$0] } }

    /// Read the files again, after an import or on opening a project.
    func reload(workbench: Workbench) {
        let fresh = workbench.sourceFiles()
        guard fresh != files else { return }
        let keep = shownFile
        files = fresh
        let again = keep.flatMap { k in files.firstIndex { $0.map == k.map && $0.file == k.file } }
        // The first file with lines of its own, usually the one with RESET.
        show(again ?? files.firstIndex { $0.lines > 0 } ?? (files.isEmpty ? nil : 0), workbench: workbench)
    }

    /// Show file `index`.
    func show(_ index: Int?, workbench: Workbench) {
        shown = index
        byLine = [:]
        text = nil
        problem = nil
        if let f = shownFile {
            for l in workbench.sourceFileLines(map: f.map, file: f.file) where byLine[l.line] == nil {
                byLine[l.line] = l
            }
            load(f)
        }
        generation += 1
    }

    /// Show the file of `line`, if it is not already.
    func show(fileOf line: SourceLineInfo, workbench: Workbench) {
        guard shownFile.map({ $0.map != line.map || $0.file != line.file }) ?? true,
              let i = files.firstIndex(where: { $0.map == line.map && $0.file == line.file })
        else { return }
        show(i, workbench: workbench)
    }

    /// Try reading the shown file again, after access was granted.
    func retry() {
        guard let f = shownFile else { return }
        load(f)
        generation += 1
    }

    private func load(_ f: SourceFileInfo) {
        guard SourceFolders.access(f.path) else {
            problem = FileManager.default.fileExists(atPath: f.path)
                ? "Romlens may not read \(f.name) yet."
                : "\(f.name) is not at \(f.path)."
            return
        }
        guard let data = FileManager.default.contents(atPath: f.path) else {
            problem = "\(f.name) could not be read."
            return
        }
        // ca65 reads bytes; most sources are ASCII or UTF-8, older ones Latin-1.
        let string = String(data: data, encoding: .utf8) ?? String(decoding: data, as: UTF8.self)
        var lines = string.components(separatedBy: "\n").map { $0.hasSuffix("\r") ? String($0.dropLast()) : $0 }
        if lines.last == "" { lines.removeLast() }
        text = lines
        if f.size != 0, data.count != Int(f.size) {
            problem = "\(f.name) has changed since it was assembled (\(data.count) bytes, was \(f.size)): lines may not match."
        }
    }
}

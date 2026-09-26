import AppKit
import Foundation

/// The folders a person let Romlens read sources from (docs/22, S2).
///
/// The app is sandboxed: choosing a `.dbg` in the open panel grants that
/// one file, not the sources beside it. So the import asks for the folder
/// once, and a security-scoped bookmark per folder, kept in user defaults
/// (they belong to this Mac, not to the project), opens it again later.
@MainActor
enum SourceFolders {
    static let defaultsKey = "sourceFolderBookmarks"

    /// Folders being accessed now, by path; each is started once.
    private static var open: [String: URL] = [:]

    private static var saved: [String: Data] {
        get { UserDefaults.standard.dictionary(forKey: defaultsKey) as? [String: Data] ?? [:] }
        set { UserDefaults.standard.set(newValue, forKey: defaultsKey) }
    }

    /// Keep access to `folder` from now on.
    static func remember(_ folder: URL) {
        let path = folder.standardizedFileURL.path
        if let data = try? folder.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil) {
            var all = saved
            all[path] = data
            saved = all
        }
        if open[path] == nil, folder.startAccessingSecurityScopedResource() {
            open[path] = folder
        }
    }

    /// The remembered folder holding `path`, if any.
    static func folder(holding path: String) -> String? {
        let file = URL(fileURLWithPath: path).standardizedFileURL.path
        return (Array(open.keys) + Array(saved.keys))
            .filter { file == $0 || file.hasPrefix($0.hasSuffix("/") ? $0 : $0 + "/") }
            .max(by: { $0.count < $1.count })
    }

    /// Start accessing the remembered folder that holds `path`; true when
    /// the file can be read.
    @discardableResult
    static func access(_ path: String) -> Bool {
        if FileManager.default.isReadableFile(atPath: path) { return true }
        guard let folder = folder(holding: path) else { return false }
        if open[folder] == nil, let data = saved[folder] {
            var stale = false
            if let url = try? URL(resolvingBookmarkData: data, options: .withSecurityScope, relativeTo: nil, bookmarkDataIsStale: &stale),
               url.startAccessingSecurityScopedResource() {
                open[folder] = url
                if stale { remember(url) }
            }
        }
        return FileManager.default.isReadableFile(atPath: path)
    }

    /// Ask for a folder, starting at `start`; the chosen one is remembered.
    static func ask(
        start: URL,
        message: String,
        window: NSWindow?,
        done: @escaping @MainActor (URL?) -> Void
    ) {
        let panel = NSOpenPanel()
        panel.title = "Allow Access to the Sources"
        panel.message = message
        panel.prompt = "Allow"
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.directoryURL = start
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let url = panel.url else {
                done(nil)
                return
            }
            remember(url)
            done(url)
        }
        if let window {
            panel.beginSheetModal(for: window, completionHandler: finish)
        } else {
            finish(panel.runModal())
        }
    }
}

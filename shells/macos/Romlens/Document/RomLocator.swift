import AppKit
import RomlensKit

/// Machine-local facts the shell keeps beside the core's project files.
struct LocalRecord: Codable, Equatable {
    var bookmark: Data?
    var lastPath: String?
}

/// Finds the ROM a project belongs to. Injected so tests can stub it.
@MainActor
protocol RomLocator {
    /// Remember where a ROM lives, keyed by its payload hash.
    func remember(url: URL, sha256: String)
    /// Locate the ROM, asking the user as a last resort; throws
    /// `CocoaError(.userCancelled)` when they decline.
    func locate(identity: RomIdentityInfo, local: LocalRecord?, packageURL: URL?) throws -> LocatedRom
}

/// A located ROM: the bytes and where they came from.
struct LocatedRom {
    let url: URL
    let bytes: Data
    let bookmark: Data?
}

/// Bookmarks by hash in user defaults, then the project's own bookmark and
/// path, then any `.sfc/.smc` beside the package, then an Open panel that
/// verifies the hash and asks again on a mismatch.
@MainActor
final class DefaultRomLocator: RomLocator {
    static let defaultsKey = "romBookmarks"

    func remember(url: URL, sha256: String) {
        var all = UserDefaults.standard.dictionary(forKey: Self.defaultsKey) as? [String: Data] ?? [:]
        if let bookmark = try? url.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil) {
            all[sha256] = bookmark
            UserDefaults.standard.set(all, forKey: Self.defaultsKey)
        }
    }

    static func bookmark(for url: URL) -> Data? {
        try? url.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil)
    }

    /// Resolve a bookmark, start accessing it, and read the bytes.
    private func read(bookmark: Data) -> LocatedRom? {
        var stale = false
        guard let url = try? URL(resolvingBookmarkData: bookmark, options: .withSecurityScope, relativeTo: nil, bookmarkDataIsStale: &stale) else {
            return nil
        }
        _ = url.startAccessingSecurityScopedResource()
        guard let bytes = try? Data(contentsOf: url) else { return nil }
        let fresh = stale ? Self.bookmark(for: url) ?? bookmark : bookmark
        return LocatedRom(url: url, bytes: bytes, bookmark: fresh)
    }

    private func read(path: String) -> LocatedRom? {
        let url = URL(fileURLWithPath: path)
        guard let bytes = try? Data(contentsOf: url) else { return nil }
        return LocatedRom(url: url, bytes: bytes, bookmark: Self.bookmark(for: url))
    }

    static func matches(_ candidate: LocatedRom, _ identity: RomIdentityInfo) -> Bool {
        guard let rom = try? Rom.fromBytes(bytes: candidate.bytes, name: candidate.url.lastPathComponent) else { return false }
        return rom.info().sha256 == identity.sha256
    }

    func locate(identity: RomIdentityInfo, local: LocalRecord?, packageURL: URL?) throws -> LocatedRom {
        var candidates: [LocatedRom?] = []
        let all = UserDefaults.standard.dictionary(forKey: Self.defaultsKey) as? [String: Data] ?? [:]
        if let bookmark = all[identity.sha256] { candidates.append(read(bookmark: bookmark)) }
        if let bookmark = local?.bookmark { candidates.append(read(bookmark: bookmark)) }
        if let path = local?.lastPath { candidates.append(read(path: path)) }
        if let dir = packageURL?.deletingLastPathComponent(),
           let siblings = try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil) {
            for url in siblings where ["sfc", "smc"].contains(url.pathExtension.lowercased()) {
                candidates.append(read(path: url.path))
            }
        }
        for candidate in candidates {
            if let candidate, Self.matches(candidate, identity) { return candidate }
        }
        // Ask, verifying the hash, until the user gives up.
        var mismatch: String?
        while true {
            let panel = NSOpenPanel()
            panel.title = "Locate ROM for \(identity.title)"
            panel.message = (mismatch.map { "\($0)\n\n" } ?? "")
                + "Choose the ROM image this project belongs to (SHA-256 \(identity.sha256.prefix(12))…)."
            panel.allowedContentTypes = [.data]
            panel.allowsMultipleSelection = false
            panel.canChooseDirectories = false
            guard panel.runModal() == .OK, let url = panel.url else {
                throw CocoaError(.userCancelled)
            }
            if let located = read(path: url.path), Self.matches(located, identity) {
                remember(url: url, sha256: identity.sha256)
                return located
            }
            mismatch = "\(url.lastPathComponent) is not that ROM (different SHA-256)."
        }
    }
}

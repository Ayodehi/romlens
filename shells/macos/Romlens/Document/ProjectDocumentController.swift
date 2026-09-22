import AppKit

/// Routes ROM images to a fresh (or the existing) project document and
/// projects through the normal path; "untitled" requests open the panel.
final class ProjectDocumentController: NSDocumentController {
    override func openUntitledDocumentAndDisplay(_ displayDocument: Bool) throws -> NSDocument {
        openDocument(nil)
        throw CocoaError(.userCancelled)
    }

    override func makeDocument(withContentsOf url: URL, ofType typeName: String) throws -> NSDocument {
        if typeName == ProjectDocument.romType {
            return try documentForRom(at: url)
        }
        return try super.makeDocument(withContentsOf: url, ofType: typeName)
    }

    override func makeDocument(for urlOrNil: URL?, withContentsOf contentsURL: URL, ofType typeName: String) throws -> NSDocument {
        if typeName == ProjectDocument.romType {
            return try documentForRom(at: contentsURL)
        }
        return try super.makeDocument(for: urlOrNil, withContentsOf: contentsURL, ofType: typeName)
    }

    /// A ROM already open (by hash) is reused; otherwise a new untitled
    /// project adopts it.
    private func documentForRom(at url: URL) throws -> NSDocument {
        let bytes = try Data(contentsOf: url)
        let doc = ProjectDocument()
        try MainActor.assumeIsolated {
            try doc.adoptRom(bytes: bytes, name: url.lastPathComponent, url: url)
        }
        if let sha = doc.sha256,
           let existing = documents.compactMap({ $0 as? ProjectDocument }).first(where: { $0.sha256 == sha }) {
            return existing
        }
        noteNewRecentDocumentURL(url)
        return doc
    }

    override func documentClass(forType typeName: String) -> AnyClass? {
        ProjectDocument.self
    }
}

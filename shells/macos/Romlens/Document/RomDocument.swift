import AppKit
import RomlensKit

/// A ROM opened as a read-only document. Phase 1 turns this into a
/// `.romlens` package that references the ROM by hash; Phase 0 just owns
/// the loaded `Rom` and its view model.
final class RomDocument: NSDocument {
    private(set) var model: RomViewModel?

    override class var autosavesInPlace: Bool { false }
    override class func canConcurrentlyReadDocuments(ofType typeName: String) -> Bool { false }

    override func read(from data: Data, ofType typeName: String) throws {
        // Reading the bytes ourselves sidesteps sandbox path questions and
        // costs well under the 200 ms open budget for a 4 MB image.
        let name = fileURL?.lastPathComponent ?? "ROM"
        let rom = try Rom.fromBytes(bytes: data, name: name)
        // `read` is declared nonisolated so subclasses may read concurrently;
        // `canConcurrentlyReadDocuments` is false, so AppKit calls it on main.
        MainActor.assumeIsolated {
            model = RomViewModel(rom: rom)
        }
    }

    override func data(ofType typeName: String) throws -> Data {
        throw NSError(
            domain: NSCocoaErrorDomain, code: NSFeatureUnsupportedError,
            userInfo: [NSLocalizedDescriptionKey: "Romlens opens ROM images read-only."]
        )
    }

    override func makeWindowControllers() {
        guard let model else { return }
        addWindowController(RomWindowController(model: model))
    }
}

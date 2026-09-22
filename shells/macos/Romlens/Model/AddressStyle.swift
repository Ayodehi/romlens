/// Which address column(s) the hex view shows. Both values are in every
/// row record, so switching never refetches from the core.
enum AddressStyle: String, CaseIterable, Identifiable, Sendable {
    case both, snes, file

    var id: String { rawValue }

    /// What the menu shows. Deliberately not "Both": the editor tabs already
    /// have a "Both", and two unrelated controls reading the same word in one
    /// toolbar is the kind of thing that makes a person click the wrong one.
    var label: String {
        switch self {
        case .both: "File + SNES"
        case .snes: "SNES only"
        case .file: "File only"
        }
    }

    /// The short form, for anywhere the full label does not fit.
    var shortLabel: String {
        switch self {
        case .both: "File + SNES"
        case .snes: "SNES"
        case .file: "File"
        }
    }
}

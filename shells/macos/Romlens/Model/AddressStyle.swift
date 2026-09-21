/// Which address column(s) the hex view shows. Both values are in every
/// row record, so switching never refetches from the core.
enum AddressStyle: String, CaseIterable, Identifiable, Sendable {
    case both, snes, file

    var id: String { rawValue }

    var label: String {
        switch self {
        case .both: "Both"
        case .snes: "SNES"
        case .file: "File"
        }
    }
}

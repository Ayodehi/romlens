import AppKit
import RomlensKit

/// Span kind → colour. System colours adapt to light and dark appearance
/// and to the accent colour without an asset catalogue.
struct SpanPalette {
    static func color(for kind: SpanKind) -> NSColor {
        switch kind {
        case .title: .systemBlue
        case .mapMode: .systemPurple
        case .cartridgeType: .systemIndigo
        case .romSize: .systemTeal
        case .ramSize: .systemCyan
        case .region: .systemGreen
        case .developerId: .systemMint
        case .version: .systemBrown
        case .checksumComplement: .systemOrange
        case .checksum: .systemRed
        case .nativeVector: .systemPink
        case .emulationVector: .systemYellow
        case .extendedHeader: .systemGray
        }
    }

    private let byId: [UInt8: NSColor]

    init(spans: [Span]) {
        var map: [UInt8: NSColor] = [:]
        for span in spans where span.id <= UInt32(UInt8.max) {
            map[UInt8(span.id)] = Self.color(for: span.kind)
        }
        byId = map
    }

    func color(forSpanId id: UInt8) -> NSColor? { byId[id] }
}

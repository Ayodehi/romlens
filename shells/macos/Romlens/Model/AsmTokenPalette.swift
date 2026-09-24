import AppKit

/// Token kind → colour. Semantic kinds from the core, styled here so every
/// shell can choose its own look (docs/08).
enum AsmTokenPalette {
    static func color(for kind: AsmTokenKind, dim: NSColor = .labelColor) -> NSColor {
        switch kind {
        case .mnemonic: .labelColor
        case .punct: .secondaryLabelColor
        case .immediate: .systemPurple
        case .number: .systemTeal
        case .autoLabel: .secondaryLabelColor
        case .userLabel: .controlAccentColor
        case .hardwareRegister: .systemPink
        case .comment: .systemGreen
        case .autoComment: .secondaryLabelColor
        case .autoLabelDef: .secondaryLabelColor
        case .userLabelDef: .controlAccentColor
        case .directive: .secondaryLabelColor
        case .dataValue: dim
        case .section: .tertiaryLabelColor
        case .warning: .systemOrange
        case .note: .systemIndigo
        case .other: .labelColor
        }
    }
}

/// Region kind and confidence → colour for gutters, tints and the hex lane.
enum RegionPalette {
    static let code = NSColor.systemBlue
    static let data = NSColor.systemOrange
    static let unknown = NSColor.systemGray

    /// Blended toward grey as confidence falls.
    static func color(kind: AsmRegionKind, confidence: Double) -> NSColor {
        let base: NSColor = kind == .code ? code : (kind.isData ? data : unknown)
        let c = max(0, min(1, confidence))
        return base.blended(withFraction: 1 - c, of: unknown) ?? base
    }

    /// Decode a hex-lane byte: `0x80 | kind << 4 | confidence4`.
    static func decodeLane(_ byte: UInt8) -> (kind: AsmRegionKind, confidence: Double)? {
        guard byte & 0x80 != 0 else { return nil }
        let kind: AsmRegionKind = (byte >> 4) & 0x7 == 1 ? .code : .byte
        return (kind, Double(byte & 0x0F) / 15.0)
    }
}

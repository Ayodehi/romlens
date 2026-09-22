import AppKit
import Foundation
import RomlensKit

/// One column of the overview strip, decoded from the core's flat batch.
struct StripColumn {
    let start: UInt32
    let len: UInt32
    let kindCode: UInt8
    let confidence: Double
    let share: Double
    let entropy: Double
    let executed: Double

    /// A column that blends kinds, which is hatched rather than painted flat:
    /// "this bank is mostly code" must not read as "this bank is code".
    var mixed: Bool { share < 0.9 }
    var end: UInt32 { start + len }
}

/// Decodes `Workbench.regionMap`.
///
/// A flat batch rather than generated records: the strip redraws on every
/// analysis and a few hundred records per redraw is measurable work for
/// nothing (`10-ffi-spike.md`). The layout is documented on
/// `viewmodel::region_summary::encode_summary`.
enum RegionStrip {
    static let headerLen = 8
    static let recordLen = 16
    static let version: UInt8 = 1

    static func decode(_ bytes: Data) -> [StripColumn] {
        guard bytes.count >= headerLen, bytes[0] == version else { return [] }
        let count = Int(UInt16(bytes[2]) | UInt16(bytes[3]) << 8)
        guard bytes.count >= headerLen + count * recordLen else { return [] }
        return (0..<count).map { i in
            let at = headerLen + i * recordLen
            func u32(_ o: Int) -> UInt32 {
                UInt32(bytes[at + o]) | UInt32(bytes[at + o + 1]) << 8
                    | UInt32(bytes[at + o + 2]) << 16 | UInt32(bytes[at + o + 3]) << 24
            }
            return StripColumn(
                start: u32(0),
                len: u32(4),
                kindCode: bytes[at + 8],
                confidence: Double(bytes[at + 9]) / 255,
                share: Double(bytes[at + 10]) / 255,
                entropy: Double(bytes[at + 11]) / 32,
                executed: Double(bytes[at + 12]) / 255
            )
        }
    }

    /// Kind code → colour. Code is the accent colour because it is what a
    /// reader is looking for; unknown is the control background, so a map full
    /// of holes looks like one rather than like a decision.
    static func color(forKindCode code: UInt8) -> NSColor {
        switch code {
        case 0: .controlBackgroundColor
        case 1: .controlAccentColor
        case 2: .systemGray          // byte
        case 3: .systemTeal          // word
        case 4: .systemCyan          // long
        case 5, 6: .systemPurple     // pointer, table
        case 7: .systemGreen         // string
        case 8: .systemOrange        // graphics
        case 9: .systemYellow        // tilemap
        case 10: .systemPink         // palette
        case 11: .systemRed          // compressed
        default: .systemBrown        // struct, and anything new
        }
    }
}

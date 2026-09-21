import AppKit
import CoreText

/// Column geometry and text formatting for one address style. Text is built
/// in a byte buffer with a 256-entry hex table, never `String(format:)`
/// (docs/10 measured that at 7 µs per row).
struct HexRowLayout {
    static let hexTable: [UInt8] = {
        var table = [UInt8](repeating: 0, count: 512)
        let digits = Array("0123456789ABCDEF".utf8)
        for b in 0..<256 {
            table[b * 2] = digits[b >> 4]
            table[b * 2 + 1] = digits[b & 0xF]
        }
        return table
    }()

    let style: AddressStyle
    let font: NSFont
    let charWidth: CGFloat
    let ascent: CGFloat
    let descent: CGFloat
    let rowHeight: CGFloat
    let leftPadding: CGFloat = 8

    init(style: AddressStyle, font: NSFont = .monospacedSystemFont(ofSize: 12, weight: .regular)) {
        self.style = style
        self.font = font
        let ctFont = font as CTFont
        var glyph: CGGlyph = 0
        var char: UniChar = 0x30 // "0"
        CTFontGetGlyphsForCharacters(ctFont, &char, &glyph, 1)
        var advance = CGSize.zero
        CTFontGetAdvancesForGlyphs(ctFont, .horizontal, &glyph, &advance, 1)
        charWidth = advance.width
        ascent = CTFontGetAscent(ctFont)
        descent = CTFontGetDescent(ctFont)
        rowHeight = ceil(ascent + descent + CTFontGetLeading(ctFont)) + 4
    }

    /// Characters before the first hex byte, including the trailing gap.
    var prefixChars: Int { style == .both ? 20 : 10 }
    /// `XX ` per byte plus one extra space after byte 8.
    var hexChars: Int { HexRowRecord.bytesPerRow * 3 + 1 }
    var asciiStart: Int { prefixChars + hexChars + 1 }
    var totalChars: Int { asciiStart + HexRowRecord.bytesPerRow + 1 }
    var totalWidth: CGFloat { leftPadding * 2 + CGFloat(totalChars) * charWidth }

    func hexColumn(byte i: Int) -> Int { prefixChars + i * 3 + (i >= 8 ? 1 : 0) }
    func asciiColumn(byte i: Int) -> Int { asciiStart + i }

    func x(ofChar c: Int) -> CGFloat { leftPadding + CGFloat(c) * charWidth }

    /// Rect of byte `i` in the hex area (two glyphs).
    func hexRect(byte i: Int, height: CGFloat) -> CGRect {
        CGRect(x: x(ofChar: hexColumn(byte: i)) - 1, y: 0, width: charWidth * 2 + 2, height: height)
    }

    func asciiRect(byte i: Int, height: CGFloat) -> CGRect {
        CGRect(x: x(ofChar: asciiColumn(byte: i)), y: 0, width: charWidth, height: height)
    }

    /// Which byte a horizontal position falls on, hex area or ASCII area.
    func byte(atX px: CGFloat) -> Int? {
        let c = Int(floor((px - leftPadding) / charWidth))
        guard c >= 0 else { return nil }
        if c >= prefixChars, c < prefixChars + hexChars {
            let rel = c - prefixChars
            let adjusted = rel >= 25 ? rel - 1 : rel // skip the mid gap
            let i = adjusted / 3
            return (0..<HexRowRecord.bytesPerRow).contains(i) ? i : nil
        }
        if c >= asciiStart, c < asciiStart + HexRowRecord.bytesPerRow {
            return c - asciiStart
        }
        return nil
    }

    /// The row text as ASCII bytes.
    func text(for r: HexRowRecord) -> [UInt8] {
        var out = [UInt8](repeating: 0x20, count: totalChars)
        var p = 0
        func put(_ s: [UInt8]) {
            out.replaceSubrange(p..<p + s.count, with: s)
            p += s.count
        }
        func hex32(_ v: UInt32, digits: Int) {
            for d in stride(from: digits - 1, through: 0, by: -1) {
                let nibble = Int((v >> (UInt32(d) * 4)) & 0xF)
                out[p] = Self.hexTable[nibble * 2 + 1]
                p += 1
            }
        }
        if style != .snes {
            put(Array("0x".utf8))
            hex32(r.fileOffset, digits: 6)
            p += 2
        }
        if style != .file {
            if let a = r.snesAddress {
                out[p] = UInt8(ascii: "$")
                p += 1
                hex32(a >> 16, digits: 2)
                out[p] = UInt8(ascii: ":")
                p += 1
                hex32(a & 0xFFFF, digits: 4)
            } else {
                put(Array("--:----".utf8))
            }
            p += 2
        }
        for i in 0..<HexRowRecord.bytesPerRow {
            if i == 8 { p += 1 }
            if i < r.byteCount {
                let b = Int(r.bytes[i])
                out[p] = Self.hexTable[b * 2]
                out[p + 1] = Self.hexTable[b * 2 + 1]
            }
            p += 3
        }
        out[p] = UInt8(ascii: "|")
        p += 1
        for i in 0..<HexRowRecord.bytesPerRow {
            out[p] = i < r.byteCount ? r.ascii[i] : 0x20
            p += 1
        }
        out[p] = UInt8(ascii: "|")
        return out
    }

    func makeLine(for r: HexRowRecord) -> CTLine {
        let string = String(decoding: text(for: r), as: UTF8.self)
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font,
            NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
        ]
        return CTLineCreateWithAttributedString(NSAttributedString(string: string, attributes: attributes))
    }
}

import AppKit
import CoreText

/// Glyph metrics of the monospaced editor font, shared by the hex and
/// disassembly layouts so both canvases use the same row height.
struct MonoMetrics {
    let font: NSFont
    let charWidth: CGFloat
    let ascent: CGFloat
    let descent: CGFloat
    let rowHeight: CGFloat

    init(font: NSFont = .monospacedSystemFont(ofSize: 12, weight: .regular)) {
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

    /// Baseline y for a row of `height` whose top is at `y` (flipped coordinates).
    func baseline(rowTop y: CGFloat, height: CGFloat) -> CGFloat {
        y + (height - (ascent + descent)) / 2 + ascent
    }

    static let hexTable: [UInt8] = {
        var table = [UInt8](repeating: 0, count: 512)
        let digits = Array("0123456789ABCDEF".utf8)
        for b in 0..<256 {
            table[b * 2] = digits[b >> 4]
            table[b * 2 + 1] = digits[b & 0xF]
        }
        return table
    }()
}

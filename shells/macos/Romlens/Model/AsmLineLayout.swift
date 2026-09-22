import AppKit
import CoreText

/// Column geometry and text for disassembly lines. Content lines carry the
/// address column(s), a 12-character bytes column and the text; label and
/// section lines start at column 0; comment lines start at the text column.
struct AsmLineLayout {
    let style: AddressStyle
    let metrics: MonoMetrics
    let leftPadding: CGFloat = 8
    /// The region stripe in the gutter.
    let gutterWidth: CGFloat = 6

    init(style: AddressStyle, metrics: MonoMetrics = MonoMetrics()) {
        self.style = style
        self.metrics = metrics
    }

    var font: NSFont { metrics.font }
    var charWidth: CGFloat { metrics.charWidth }
    var rowHeight: CGFloat { metrics.rowHeight }

    /// Characters used by the address column(s), including the trailing gap.
    var addressChars: Int { style == .both ? 20 : 10 }
    var bytesChars: Int { 12 }
    /// Column where the mnemonic (or `db`) starts.
    var textColumn: Int { addressChars + bytesChars + 1 }
    let maxTextChars = 80
    var totalChars: Int { textColumn + maxTextChars }
    var textStart: CGFloat { leftPadding + gutterWidth + 2 }
    var totalWidth: CGFloat { textStart + CGFloat(totalChars) * charWidth + leftPadding }

    func x(ofChar c: Int) -> CGFloat { textStart + CGFloat(c) * charWidth }

    /// Which character column a horizontal position falls on.
    func column(atX px: CGFloat) -> Int? {
        let c = Int(floor((px - textStart) / charWidth))
        return c >= 0 ? c : nil
    }

    /// Column at which a record's own text begins.
    func textOrigin(for r: AsmLineRecord) -> Int {
        switch r.kind {
        case .label, .section, .blank: 0
        case .comment, .instruction, .data: textColumn
        }
    }

    /// The token under a column, if any.
    func token(atColumn c: Int, in r: AsmLineRecord) -> AsmToken? {
        let rel = c - textOrigin(for: r)
        guard rel >= 0 else { return nil }
        return r.tokens.first { $0.range.contains(rel) }
    }

    /// The full display line as UTF-8 bytes.
    func text(for r: AsmLineRecord) -> [UInt8] {
        var out: [UInt8] = []
        out.reserveCapacity(textColumn + r.text.utf8.count)
        func hex32(_ v: UInt32, digits: Int) {
            for d in stride(from: digits - 1, through: 0, by: -1) {
                let nibble = Int((v >> (UInt32(d) * 4)) & 0xF)
                out.append(MonoMetrics.hexTable[nibble * 2 + 1])
            }
        }
        func pad(to column: Int) {
            while out.count < column { out.append(0x20) }
        }
        switch r.kind {
        case .blank:
            break
        case .label, .section:
            out.append(contentsOf: Array(r.text.utf8))
        case .comment:
            pad(to: textColumn)
            out.append(contentsOf: Array(r.text.utf8))
        case .instruction, .data:
            if style != .snes {
                out.append(contentsOf: Array("0x".utf8))
                hex32(r.fileOffset, digits: 6)
                out.append(0x20)
                out.append(0x20)
            }
            if style != .file {
                if let a = r.snesAddress {
                    out.append(UInt8(ascii: "$"))
                    hex32(a >> 16, digits: 2)
                    out.append(UInt8(ascii: ":"))
                    hex32(a & 0xFFFF, digits: 4)
                } else {
                    out.append(contentsOf: Array("--:----".utf8))
                }
                out.append(0x20)
                out.append(0x20)
            }
            for (i, b) in r.bytes.prefix(4).enumerated() {
                if i > 0 { out.append(0x20) }
                out.append(MonoMetrics.hexTable[Int(b) * 2])
                out.append(MonoMetrics.hexTable[Int(b) * 2 + 1])
            }
            pad(to: textColumn)
            out.append(contentsOf: Array(r.text.utf8))
        }
        return out
    }

    func makeLine(for r: AsmLineRecord) -> CTLine {
        let bytes = text(for: r)
        let string = NSMutableAttributedString(
            string: String(decoding: bytes, as: UTF8.self),
            attributes: [.font: font, .foregroundColor: NSColor.labelColor]
        )
        let origin = textOrigin(for: r)
        let dim = r.kind == .data ? NSColor.secondaryLabelColor : NSColor.labelColor
        if r.kind.isContent {
            string.addAttribute(.foregroundColor, value: NSColor.tertiaryLabelColor, range: NSRange(location: 0, length: min(addressChars, string.length)))
            let bytesRange = NSRange(location: min(addressChars, string.length), length: max(0, min(bytesChars, string.length - addressChars)))
            string.addAttribute(.foregroundColor, value: NSColor.secondaryLabelColor, range: bytesRange)
        }
        // Token offsets are byte offsets into the UTF-8 text; the text is
        // ASCII except inside comments, so map through the utf8 view.
        let utf8 = Array(r.text.utf8)
        for token in r.tokens {
            let color = AsmTokenPalette.color(for: token.kind, dim: dim)
            let start = String(decoding: utf8.prefix(min(token.start, utf8.count)), as: UTF8.self).utf16.count
            let len = String(decoding: utf8[min(token.start, utf8.count)..<min(token.start + token.len, utf8.count)], as: UTF8.self).utf16.count
            let location = origin + start
            guard location + len <= string.length else { continue }
            string.addAttribute(.foregroundColor, value: color, range: NSRange(location: location, length: len))
            if token.kind == .userLabel || token.kind == .userLabelDef {
                string.addAttribute(.font, value: NSFont.monospacedSystemFont(ofSize: font.pointSize, weight: .bold), range: NSRange(location: location, length: len))
            }
        }
        return CTLineCreateWithAttributedString(string)
    }
}

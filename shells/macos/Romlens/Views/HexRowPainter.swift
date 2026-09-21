import AppKit
import CoreText

/// Draws one hex row: span overlays and the selection rectangle underneath,
/// then the cached `CTLine`. Container-independent, so the canvas and any
/// future row container share it.
enum HexRowPainter {
    static func draw(
        record: HexRowRecord, line: CTLine, layout: HexRowLayout, palette: SpanPalette,
        selectedByte: Int?, in rect: NSRect, context: CGContext
    ) {
        let h = rect.height
        let y = rect.minY

        if record.hasSpan {
            var i = 0
            while i < record.byteCount {
                let id = record.spanIds[i]
                var j = i + 1
                while j < record.byteCount, record.spanIds[j] == id { j += 1 }
                if id != 0, let color = palette.color(forSpanId: id) {
                    context.setFillColor(color.withAlphaComponent(0.28).cgColor)
                    let first = layout.hexRect(byte: i, height: h)
                    let last = layout.hexRect(byte: j - 1, height: h)
                    context.fill(CGRect(x: first.minX, y: y + 2, width: last.maxX - first.minX, height: h - 4))
                    let a0 = layout.asciiRect(byte: i, height: h)
                    let a1 = layout.asciiRect(byte: j - 1, height: h)
                    context.fill(CGRect(x: a0.minX, y: y + 2, width: a1.maxX - a0.minX, height: h - 4))
                }
                i = j
            }
        }

        if let selectedByte, selectedByte < record.byteCount {
            let accent = NSColor.controlAccentColor
            context.setFillColor(accent.withAlphaComponent(0.35).cgColor)
            var r = layout.hexRect(byte: selectedByte, height: h)
            r.origin.y = y + 1
            r.size.height = h - 2
            context.fill(r)
            var a = layout.asciiRect(byte: selectedByte, height: h)
            a.origin.y = y + 1
            a.size.height = h - 2
            context.fill(a)
            context.setStrokeColor(accent.cgColor)
            context.stroke(r.insetBy(dx: 0.5, dy: 0.5), width: 1)
        }

        context.saveGState()
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        context.setFillColor(NSColor.labelColor.cgColor)
        let baseline = y + (h - (layout.ascent + layout.descent)) / 2 + layout.ascent
        context.textPosition = CGPoint(x: layout.leftPadding, y: baseline)
        CTLineDraw(line, context)
        context.restoreGState()
    }
}

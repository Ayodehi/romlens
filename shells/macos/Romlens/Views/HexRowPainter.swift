import AppKit
import CoreText

/// Draws one hex row: region tint and span overlays, the highlighted range
/// and the selected byte underneath, then the cached `CTLine`.
/// Container-independent, so the canvas and any future row container share it.
enum HexRowPainter {
    static func draw(
        record: HexRowRecord, line: CTLine, layout: HexRowLayout, palette: SpanPalette,
        highlighted: Range<Int>?, selectedByte: Int?, focused: Bool = true,
        in rect: NSRect, context: CGContext
    ) {
        let h = rect.height
        let y = rect.minY

        // Region lane (analysis) and header spans share the span-id lane.
        var i = 0
        while i < record.byteCount {
            let id = record.spanIds[i]
            var j = i + 1
            while j < record.byteCount, record.spanIds[j] == id { j += 1 }
            if id != 0 {
                let color: NSColor?
                if let lane = RegionPalette.decodeLane(id) {
                    color = RegionPalette.color(kind: lane.kind, confidence: lane.confidence)
                        .withAlphaComponent(0.08 + 0.14 * lane.confidence)
                } else {
                    color = palette.color(forSpanId: id)?.withAlphaComponent(0.28)
                }
                if let color {
                    context.setFillColor(color.cgColor)
                    let first = layout.hexRect(byte: i, height: h)
                    let last = layout.hexRect(byte: j - 1, height: h)
                    context.fill(CGRect(x: first.minX, y: y + 2, width: last.maxX - first.minX, height: h - 4))
                    let a0 = layout.asciiRect(byte: i, height: h)
                    let a1 = layout.asciiRect(byte: j - 1, height: h)
                    context.fill(CGRect(x: a0.minX, y: y + 2, width: a1.maxX - a0.minX, height: h - 4))
                }
            }
            i = j
        }

        let accent = NSColor.controlAccentColor
        if let highlighted, !highlighted.isEmpty {
            let lo = max(0, highlighted.lowerBound)
            let hi = min(record.byteCount, highlighted.upperBound)
            if lo < hi {
                context.setFillColor(accent.withAlphaComponent(focused ? 0.3 : 0.15).cgColor)
                let first = layout.hexRect(byte: lo, height: h)
                let last = layout.hexRect(byte: hi - 1, height: h)
                context.fill(CGRect(x: first.minX, y: y + 1, width: last.maxX - first.minX, height: h - 2))
                let a0 = layout.asciiRect(byte: lo, height: h)
                let a1 = layout.asciiRect(byte: hi - 1, height: h)
                context.fill(CGRect(x: a0.minX, y: y + 1, width: a1.maxX - a0.minX, height: h - 2))
            }
        }
        if let selectedByte, selectedByte < record.byteCount {
            var r = layout.hexRect(byte: selectedByte, height: h)
            r.origin.y = y + 1
            r.size.height = h - 2
            context.setStrokeColor(accent.cgColor)
            context.stroke(r.insetBy(dx: 0.5, dy: 0.5), width: 1)
        }

        context.saveGState()
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        context.setFillColor(NSColor.labelColor.cgColor)
        let baseline = layout.metrics.baseline(rowTop: y, height: h)
        context.textPosition = CGPoint(x: layout.leftPadding, y: baseline)
        CTLineDraw(line, context)
        context.restoreGState()
    }
}

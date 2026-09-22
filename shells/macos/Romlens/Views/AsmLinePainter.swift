import AppKit
import CoreText

/// Draws one disassembly line: background tint by region and confidence, a
/// gutter stripe, the selection fill, a separator above labels, the text.
enum AsmLinePainter {
    static func draw(
        record: AsmLineRecord, line: CTLine, layout: AsmLineLayout,
        selected: Bool, focused: Bool, in rect: NSRect, context: CGContext
    ) {
        let h = rect.height
        let y = rect.minY
        let confidence = Double(record.confidence) / 100
        if record.regionKind != .unknown {
            let color = RegionPalette.color(kind: record.regionKind, confidence: confidence)
            context.setFillColor(color.withAlphaComponent(0.05 + 0.10 * confidence).cgColor)
            context.fill(rect)
            context.setFillColor(color.cgColor)
            context.fill(CGRect(x: layout.leftPadding, y: y, width: layout.gutterWidth, height: h))
        }
        if record.kind == .label {
            context.setFillColor(NSColor.separatorColor.cgColor)
            context.fill(CGRect(x: rect.minX, y: y, width: rect.width, height: 1))
        }
        if selected {
            let accent = NSColor.controlAccentColor
            context.setFillColor(accent.withAlphaComponent(focused ? 0.25 : 0.12).cgColor)
            context.fill(rect)
            context.setStrokeColor(accent.cgColor)
            context.stroke(rect.insetBy(dx: 0.5, dy: 0.5), width: 1)
        }
        if record.hasWarning {
            context.setFillColor(NSColor.systemOrange.cgColor)
            context.fill(CGRect(x: layout.leftPadding + layout.gutterWidth + 1, y: y + h / 2 - 2, width: 3, height: 4))
        }
        context.saveGState()
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        context.textPosition = CGPoint(x: layout.textStart, y: layout.metrics.baseline(rowTop: y, height: h))
        CTLineDraw(line, context)
        context.restoreGState()
    }
}

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
            // A warning triangle in its own column, the size of a capital
            // letter, so it reads at a glance without covering the text.
            let side = min(layout.markWidth - 4, h - 4)
            let x = layout.markStart + (layout.markWidth - side) / 2 - 1
            let top = y + (h - side) / 2
            let triangle = CGMutablePath()
            triangle.move(to: CGPoint(x: x + side / 2, y: top))
            triangle.addLine(to: CGPoint(x: x + side, y: top + side))
            triangle.addLine(to: CGPoint(x: x, y: top + side))
            triangle.closeSubpath()
            context.setFillColor(NSColor.systemOrange.cgColor)
            context.addPath(triangle)
            context.fillPath()
            context.setFillColor(NSColor.black.withAlphaComponent(0.8).cgColor)
            let bar = max(1.5, side / 7)
            context.fill(CGRect(x: x + (side - bar) / 2, y: top + side * 0.35, width: bar, height: side * 0.33))
            context.fill(CGRect(x: x + (side - bar) / 2, y: top + side * 0.76, width: bar, height: bar))
        }
        context.saveGState()
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        context.textPosition = CGPoint(x: layout.textStart, y: layout.metrics.baseline(rowTop: y, height: h))
        CTLineDraw(line, context)
        context.restoreGState()
    }
}

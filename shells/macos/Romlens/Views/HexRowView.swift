import AppKit
import CoreText

/// One table row: address columns, sixteen hex bytes and ASCII, drawn with
/// CoreText from a cached `CTLine`, with span overlays and the selection
/// rectangle painted underneath. Container-independent: it only needs a
/// record, a line and a layout, so it can move to a single-canvas view.
final class HexRowView: NSView {
    static let identifier = NSUserInterfaceItemIdentifier("HexRow")

    var onSelectByte: ((UInt32) -> Void)?

    private var record: HexRowRecord?
    private var line: CTLine?
    private var layout: HexRowLayout?
    private var palette: SpanPalette?
    private var selectedByte: Int?

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }

    func configure(record: HexRowRecord, line: CTLine, layout: HexRowLayout, palette: SpanPalette, selectedByte: Int?) {
        self.record = record
        self.line = line
        self.layout = layout
        self.palette = palette
        self.selectedByte = selectedByte
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        NSColor.textBackgroundColor.setFill()
        context.fill(bounds)
        guard let record, let line, let layout else { return }
        let h = bounds.height

        if record.hasSpan, let palette {
            var i = 0
            while i < record.byteCount {
                let id = record.spanIds[i]
                var j = i + 1
                while j < record.byteCount, record.spanIds[j] == id { j += 1 }
                if id != 0, let color = palette.color(forSpanId: id) {
                    color.withAlphaComponent(0.28).setFill()
                    let first = layout.hexRect(byte: i, height: h)
                    let last = layout.hexRect(byte: j - 1, height: h)
                    context.fill(CGRect(x: first.minX, y: 2, width: last.maxX - first.minX, height: h - 4))
                    let a0 = layout.asciiRect(byte: i, height: h)
                    let a1 = layout.asciiRect(byte: j - 1, height: h)
                    context.fill(CGRect(x: a0.minX, y: 2, width: a1.maxX - a0.minX, height: h - 4))
                }
                i = j
            }
        }

        if let selectedByte, selectedByte < record.byteCount {
            NSColor.controlAccentColor.withAlphaComponent(0.35).setFill()
            var r = layout.hexRect(byte: selectedByte, height: h)
            r.origin.y = 1
            r.size.height = h - 2
            context.fill(r)
            var a = layout.asciiRect(byte: selectedByte, height: h)
            a.origin.y = 1
            a.size.height = h - 2
            context.fill(a)
            NSColor.controlAccentColor.setStroke()
            context.stroke(r.insetBy(dx: 0.5, dy: 0.5), width: 1)
        }

        context.saveGState()
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        context.setFillColor(NSColor.labelColor.cgColor)
        let baseline = (h - (layout.ascent + layout.descent)) / 2 + layout.ascent
        context.textPosition = CGPoint(x: layout.leftPadding, y: baseline)
        CTLineDraw(line, context)
        context.restoreGState()
    }

    override func mouseDown(with event: NSEvent) {
        guard let record, let layout else { return }
        let x = convert(event.locationInWindow, from: nil).x
        if let byte = layout.byte(atX: x), byte < record.byteCount {
            onSelectByte?(record.fileOffset + UInt32(byte))
        } else {
            super.mouseDown(with: event)
        }
    }
}

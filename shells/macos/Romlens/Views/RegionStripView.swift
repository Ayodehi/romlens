import AppKit
import RomlensKit
import SwiftUI

/// The whole-ROM overview: one column per pixel, the map at a glance.
///
/// A plain `NSView` outside any scroll view. It is an overview of the entire
/// image and never scrolls — scrolling an overview would defeat the point of
/// having one.
final class RegionStripNSView: NSView {
    var columns: [StripColumn] = [] { didSet { needsDisplay = true } }
    /// The byte the editor is showing, drawn as a caret.
    var marker: UInt32? { didSet { needsDisplay = true } }
    var romLength: UInt32 = 1
    var onScrub: ((UInt32) -> Void)?

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { false }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.controlBackgroundColor.setFill()
        bounds.fill()
        guard !columns.isEmpty else { return }
        let w = bounds.width / CGFloat(columns.count)
        for (i, c) in columns.enumerated() {
            let rect = NSRect(x: CGFloat(i) * w, y: 0, width: max(w, 1), height: bounds.height)
            let base = RegionStrip.color(forKindCode: c.kindCode)
            // Confidence is the alpha: a guess looks like a guess.
            base.withAlphaComponent(0.35 + 0.65 * c.confidence).setFill()
            rect.fill()
            if c.mixed {
                hatch(rect)
            }
            if c.executed > 0 {
                NSColor.controlAccentColor.withAlphaComponent(0.9).setFill()
                NSRect(x: rect.minX, y: bounds.height - 3, width: rect.width, height: 3 * c.executed)
                    .fill()
            }
        }
        if let marker, romLength > 0 {
            let x = bounds.width * CGFloat(marker) / CGFloat(romLength)
            NSColor.labelColor.setFill()
            NSRect(x: x - 0.5, y: 0, width: 1.5, height: bounds.height).fill()
        }
    }

    /// Diagonal lines over a column that is not all one kind.
    private func hatch(_ rect: NSRect) {
        NSGraphicsContext.saveGraphicsState()
        NSBezierPath(rect: rect).setClip()
        NSColor.controlBackgroundColor.withAlphaComponent(0.5).setStroke()
        let path = NSBezierPath()
        path.lineWidth = 1
        var x = rect.minX - rect.height
        while x < rect.maxX {
            path.move(to: NSPoint(x: x, y: rect.maxY))
            path.line(to: NSPoint(x: x + rect.height, y: rect.minY))
            x += 4
        }
        path.stroke()
        NSGraphicsContext.restoreGraphicsState()
    }

    override func mouseDown(with event: NSEvent) { scrub(event) }
    override func mouseDragged(with event: NSEvent) { scrub(event) }

    private func scrub(_ event: NSEvent) {
        guard bounds.width > 0, romLength > 0 else { return }
        let x = convert(event.locationInWindow, from: nil).x
        let fraction = (x / bounds.width).clamped(to: 0...1)
        onScrub?(UInt32(Double(romLength - 1) * fraction))
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .activeInKeyWindow, .inVisibleRect],
            owner: self
        ))
    }

    override func mouseMoved(with event: NSEvent) {
        guard bounds.width > 0, !columns.isEmpty else { return }
        let x = convert(event.locationInWindow, from: nil).x
        let i = Int((x / bounds.width * CGFloat(columns.count)).clamped(to: 0...CGFloat(columns.count - 1)))
        let c = columns[i]
        toolTip = String(
            format: "%@  %@%@  %.0f%% confident  %.1f bits/byte",
            formatFileOffset(offset: c.start),
            kindName(c.kindCode),
            c.mixed ? String(format: " (%.0f%%)", c.share * 100) : "",
            c.confidence * 100,
            c.entropy
        )
    }

    private func kindName(_ code: UInt8) -> String {
        switch code {
        case 0: "unknown"
        case 1: "code"
        case 2: "byte"
        case 3: "word"
        case 4: "long"
        case 5: "pointer"
        case 6: "table"
        case 7: "string"
        case 8: "graphics"
        case 9: "tilemap"
        case 10: "palette"
        case 11: "compressed"
        case 13: "sample"
        default: "struct"
        }
    }
}

private extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}

struct RegionStripView: NSViewRepresentable {
    let model: RomViewModel

    func makeNSView(context: Context) -> RegionStripNSView {
        let view = RegionStripNSView()
        view.onScrub = { offset in
            model.jump(to: offset)
        }
        return view
    }

    func updateNSView(_ view: RegionStripNSView, context: Context) {
        view.romLength = model.byteCount
        view.marker = model.selectedOffset
        // Re-reduce when the width changes, so one column really is one pixel.
        let wanted = max(64, UInt32(view.bounds.width.rounded()))
        if view.columns.count != Int(wanted) || model.stripGeneration != context.coordinator.seen {
            context.coordinator.seen = model.stripGeneration
            view.columns = RegionStrip.decode(model.workbench.regionMap(buckets: wanted))
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        var seen = -1
    }
}

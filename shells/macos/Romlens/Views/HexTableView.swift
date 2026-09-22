import AppKit
import SwiftUI

/// The virtualized hex view: an `NSScrollView` whose document is one canvas
/// view as tall as the image, which draws only the rows in the dirty rect.
/// No per-row views exist, so nothing accumulates during long scrolls and
/// there is no subtree for the SwiftUI host to walk on layout passes (the
/// `NSTableView` version leaked its row views during trackpad scrolls and
/// hung the app after ~160k rows).
struct HexTableView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator(model: model) }

    func makeNSView(context: Context) -> HexPaneView {
        let coordinator = context.coordinator
        let canvas = HexCanvasView(model: model)
        canvas.onKeyCommand = { [weak coordinator] command in coordinator?.handle(command) }

        let scroll = NSScrollView()
        scroll.documentView = canvas
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = true
        scroll.backgroundColor = .textBackgroundColor
        canvas.autoresizingMask = [.width]
        canvas.frame = NSRect(x: 0, y: 0, width: model.layout.totalWidth, height: canvas.documentHeight)
        coordinator.canvas = canvas
        coordinator.scrollView = scroll
        let header = HexColumnHeaderView(model: model)
        coordinator.header = header
        return HexPaneView(header: header, scrollView: scroll)
    }

    func updateNSView(_ pane: HexPaneView, context: Context) {
        let c = context.coordinator
        guard let canvas = c.canvas else { return }
        // Every property read here is observed; SwiftUI re-runs this update
        // when one changes.
        let generation = model.lineGeneration
        let selected = model.selectedOffset
        let scroll = model.scrollRequest

        if c.lineGeneration != generation {
            c.lineGeneration = generation
            canvas.layoutDidChange()
            pane.headerHeightDidChange()
        }
        if c.selectedOffset != selected {
            if let old = c.selectedOffset { canvas.invalidate(row: Int(old / 16)) }
            if let new = selected { canvas.invalidate(row: Int(new / 16)) }
            c.selectedOffset = selected
            c.header?.selectedByte = selected.map { Int($0 % 16) }
        }
        if let scroll, c.lastScrollId != scroll.id {
            c.lastScrollId = scroll.id
            c.scroll(toRow: Int(scroll.row))
        }
    }

    @MainActor
    final class Coordinator {
        let model: RomViewModel
        weak var canvas: HexCanvasView?
        weak var scrollView: NSScrollView?
        weak var header: HexColumnHeaderView?
        var lineGeneration = -1
        var selectedOffset: UInt32?
        var lastScrollId = 0

        init(model: RomViewModel) { self.model = model }

        /// Scroll so the row sits in the middle of the visible area.
        func scroll(toRow row: Int) {
            guard let canvas, let scrollView else { return }
            let rowRect = canvas.rect(ofRow: row)
            let clip = scrollView.contentView
            let visibleHeight = clip.bounds.height
            let y = max(0, min(rowRect.midY - visibleHeight / 2, canvas.bounds.height - visibleHeight))
            clip.scroll(to: NSPoint(x: clip.bounds.origin.x, y: y))
            scrollView.reflectScrolledClipView(clip)
        }

        func handle(_ command: HexCanvasView.KeyCommand) {
            switch command {
            case .left: model.moveSelection(by: -1)
            case .right: model.moveSelection(by: 1)
            case .up: model.moveSelection(by: -16)
            case .down: model.moveSelection(by: 16)
            case .pageUp, .pageDown:
                guard let canvas, let scrollView else { return }
                let visibleRows = max(1, Int(scrollView.contentView.bounds.height / canvas.rowHeight) - 1)
                model.moveSelection(by: (command == .pageUp ? -16 : 16) * visibleRows)
            case .home: model.jump(to: 0)
            case .end: model.jump(to: model.byteCount - 1)
            case .back: model.goBack()
            }
        }
    }
}

/// The document view: as tall as every row of the image, draws the rows in
/// the dirty rect from the model's batch cache, hit-tests clicks to bytes and
/// turns navigation keys into commands.
final class HexCanvasView: NSView {
    enum KeyCommand { case left, right, up, down, pageUp, pageDown, home, end, back }

    let model: RomViewModel
    var onKeyCommand: ((KeyCommand) -> Void)?

    init(model: RomViewModel) {
        self.model = model
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    var rowHeight: CGFloat { model.layout.rowHeight }
    var rowCount: Int { Int(model.rowCount) }
    var documentHeight: CGFloat { CGFloat(rowCount) * rowHeight }

    func rect(ofRow row: Int) -> NSRect {
        NSRect(x: 0, y: CGFloat(row) * rowHeight, width: bounds.width, height: rowHeight)
    }

    /// Rows intersecting `rect`, clamped to the image.
    func rows(in rect: NSRect) -> Range<Int> {
        let first = max(0, Int(floor(rect.minY / rowHeight)))
        let last = min(rowCount, Int(ceil(rect.maxY / rowHeight)))
        return first..<max(first, last)
    }

    var visibleRows: Range<Int> { rows(in: visibleRect) }

    /// Address style or font changed: resize and redraw everything.
    func layoutDidChange() {
        let width = max(model.layout.totalWidth, superview?.bounds.width ?? 0)
        setFrameSize(NSSize(width: width, height: documentHeight))
        needsDisplay = true
    }

    func invalidate(row: Int) {
        setNeedsDisplay(rect(ofRow: row))
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.setFillColor(NSColor.textBackgroundColor.cgColor)
        context.fill(dirtyRect)
        let layout = model.layout
        let generation = model.lineGeneration
        let selected = model.selectedOffset
        for row in rows(in: dirtyRect) {
            let row32 = UInt32(row)
            let batch = model.batch(containingRow: row32)
            let record = batch.record(row: row32)
            let line = batch.line(row: row32, generation: generation, layout: layout)
            var selectedByte: Int?
            if let s = selected, s / 16 == row32 { selectedByte = Int(s % 16) }
            HexRowPainter.draw(
                record: record, line: line, layout: layout, palette: model.palette,
                selectedByte: selectedByte, in: rect(ofRow: row), context: context
            )
        }
    }

    /// The byte under a point in the view's coordinates.
    func byte(at point: NSPoint) -> UInt32? {
        let row = Int(floor(point.y / rowHeight))
        guard row >= 0, row < rowCount, let byte = model.layout.byte(atX: point.x) else { return nil }
        let offset = UInt32(row) * 16 + UInt32(byte)
        return offset < model.byteCount ? offset : nil
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if let offset = byte(at: point) {
            window?.makeFirstResponder(self)
            model.select(offset: offset)
        } else {
            super.mouseDown(with: event)
        }
    }

    override func keyDown(with event: NSEvent) {
        let command: KeyCommand?
        switch event.specialKey {
        case .leftArrow: command = .left
        case .rightArrow: command = .right
        case .upArrow: command = .up
        case .downArrow: command = .down
        case .pageUp: command = .pageUp
        case .pageDown: command = .pageDown
        case .home: command = .home
        case .end: command = .end
        case .delete, .backspace: command = .back
        default: command = nil
        }
        if let command {
            onKeyCommand?(command)
        } else {
            super.keyDown(with: event)
        }
    }
}

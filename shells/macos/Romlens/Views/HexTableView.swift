import AppKit
import SwiftUI

/// The virtualized hex view: an `NSScrollView` whose document is one canvas
/// view as tall as the image, which draws only the rows in the dirty rect.
/// No per-row views exist, so nothing accumulates during long scrolls (the
/// `NSTableView` version leaked its row views during trackpad scrolls).
struct HexTableView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> HexPaneView {
        let controller = HexPaneController(model: model)
        context.coordinator.controller = controller
        return controller.pane
    }

    func updateNSView(_ pane: HexPaneView, context: Context) {
        context.coordinator.controller?.update()
    }

    @MainActor
    final class Coordinator {
        var controller: HexPaneController?
    }
}

/// Builds and updates one hex pane (header, scroll view, canvas). Shared by
/// the Hex tab and the Both tab.
@MainActor
final class HexPaneController {
    let model: RomViewModel
    let pane: HexPaneView
    let canvas: HexCanvasView
    let scrollView: NSScrollView
    let header: HexColumnHeaderView
    private var lineGeneration = -1
    private var highlighted: Range<UInt32>?
    private var lastScrollId = 0

    init(model: RomViewModel) {
        self.model = model
        canvas = HexCanvasView(model: model)
        scrollView = NSScrollView()
        scrollView.documentView = canvas
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor
        // Sideways rubber-banding when the rows already fit is just noise.
        scrollView.horizontalScrollElasticity = .none
        // Width is managed by HexPaneView.layout (an autoresizing mask would
        // add the clip view's growth to the canvas and make it scrollable).
        canvas.autoresizingMask = []
        canvas.frame = NSRect(x: 0, y: 0, width: model.layout.totalWidth, height: canvas.documentHeight)
        header = HexColumnHeaderView(model: model)
        pane = HexPaneView(header: header, scrollView: scrollView)
        canvas.onKeyCommand = { [weak self] command in self?.handle(command) }
    }

    /// Every property read here is observed by the hosting representable, so
    /// SwiftUI calls it again when one changes.
    func update() {
        let generation = model.lineGeneration
        let range = model.highlightedRange
        let scroll = model.scrollRequest

        if lineGeneration != generation {
            lineGeneration = generation
            canvas.layoutDidChange()
            pane.headerHeightDidChange()
        }
        if highlighted != range {
            invalidate(range: highlighted)
            invalidate(range: range)
            highlighted = range
            header.selectedByte = model.selectedOffset.map { Int($0 % 16) }
        }
        if let scroll, lastScrollId != scroll.id {
            lastScrollId = scroll.id
            self.scroll(toRow: Int(scroll.offset / 16))
        }
    }

    private func invalidate(range: Range<UInt32>?) {
        guard let range, !range.isEmpty else { return }
        for row in Int(range.lowerBound / 16)...Int((range.upperBound - 1) / 16) {
            canvas.invalidate(row: row)
        }
    }

    var visibleRows: Int {
        max(1, Int(scrollView.contentView.bounds.height / canvas.rowHeight) - 1)
    }

    /// Scroll so the row sits in the middle of the visible area.
    func scroll(toRow row: Int) {
        let rowRect = canvas.rect(ofRow: row)
        let clip = scrollView.contentView
        let visibleHeight = clip.bounds.height
        let y = max(0, min(rowRect.midY - visibleHeight / 2, canvas.bounds.height - visibleHeight))
        clip.scroll(to: NSPoint(x: clip.bounds.origin.x, y: y))
        scrollView.reflectScrolledClipView(clip)
    }

    func handle(_ command: EditorKeyCommand) {
        model.perform(command, from: .hex, visibleItems: visibleRows)
    }
}

/// The document view: as tall as every row of the image, draws the rows in
/// the dirty rect from the model's batch cache, hit-tests clicks to bytes and
/// turns navigation keys into commands.
final class HexCanvasView: NSView {
    let model: RomViewModel
    var onKeyCommand: ((EditorKeyCommand) -> Void)?

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
        fitWidth()
        needsDisplay = true
    }

    /// As wide as the text needs, or the viewport if that is wider, so the
    /// row background fills the pane and no horizontal scrolling appears
    /// unless the text genuinely does not fit.
    func fitWidth() {
        let width = max(model.layout.totalWidth, superview?.bounds.width ?? 0)
        if frame.size.width != width || frame.size.height != documentHeight {
            setFrameSize(NSSize(width: width, height: documentHeight))
        }
    }

    func invalidate(row: Int) {
        setNeedsDisplay(rect(ofRow: row))
    }

    override func becomeFirstResponder() -> Bool {
        needsDisplay = true
        return super.becomeFirstResponder()
    }

    override func resignFirstResponder() -> Bool {
        needsDisplay = true
        return super.resignFirstResponder()
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.setFillColor(NSColor.textBackgroundColor.cgColor)
        context.fill(dirtyRect)
        let layout = model.layout
        let generation = model.lineGeneration
        let selected = model.selectedOffset
        let highlighted = model.highlightedRange
        let focused = window?.firstResponder === self
        for row in rows(in: dirtyRect) {
            let row32 = UInt32(row)
            let batch = model.batch(containingRow: row32)
            let record = batch.record(row: row32)
            let line = batch.line(row: row32, generation: generation, layout: layout)
            var selectedByte: Int?
            if let s = selected, s / 16 == row32 { selectedByte = Int(s % 16) }
            var rowRange: Range<Int>?
            if let h = highlighted {
                let rowStart = row32 * 16
                let lo = max(h.lowerBound, rowStart)
                let hi = min(h.upperBound, rowStart + 16)
                if lo < hi { rowRange = Int(lo - rowStart)..<Int(hi - rowStart) }
            }
            HexRowPainter.draw(
                record: record, line: line, layout: layout, palette: model.palette,
                highlighted: rowRange, selectedByte: selectedByte, focused: focused,
                in: rect(ofRow: row), context: context
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
            if event.modifierFlags.contains(.shift) {
                model.extendSelection(to: offset)
            } else {
                model.select(offset: offset)
            }
            if event.clickCount == 2 { model.followReference() }
        } else {
            super.mouseDown(with: event)
        }
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let point = convert(event.locationInWindow, from: nil)
        if let offset = byte(at: point), model.highlightedRange?.contains(offset) != true {
            model.select(offset: offset)
        }
        return EditorContextMenu.build(hasSelection: model.selectedOffset != nil)
    }

    override func keyDown(with event: NSEvent) {
        if let command = EditorKeyCommand.from(event: event) {
            onKeyCommand?(command)
        } else {
            super.keyDown(with: event)
        }
    }
}

/// The right-click menu both canvases show; actions travel the responder
/// chain to `RomWindowController`.
enum EditorContextMenu {
    @MainActor
    static func build(hasSelection: Bool) -> NSMenu {
        let menu = NSMenu()
        func item(_ title: String, _ action: Selector) {
            let i = NSMenuItem(title: title, action: action, keyEquivalent: "")
            i.isEnabled = hasSelection
            menu.addItem(i)
        }
        item("Follow Reference", #selector(RomWindowController.followReference(_:)))
        // Titled and enabled by `RomWindowController.validateMenuItem`.
        item("Find References", #selector(RomWindowController.findReferences(_:)))
        item("Decompile Routine", #selector(RomWindowController.decompileRoutine(_:)))
        item("Show Graph", #selector(RomWindowController.showGraph(_:)))
        menu.addItem(.separator())
        item("Rename Label…", #selector(RomWindowController.renameLabel(_:)))
        item("Remove Label", #selector(RomWindowController.removeLabel(_:)))
        // Titled by `validateMenuItem` with the operand's address.
        item("Define Variable…", #selector(RomWindowController.defineVariable(_:)))
        item("Comment…", #selector(RomWindowController.editComment(_:)))
        menu.addItem(.separator())
        item("Mark as Code", #selector(RomWindowController.markAsCode(_:)))
        item("Mark as Data", #selector(RomWindowController.markAsData(_:)))
        item("Mark as Unknown", #selector(RomWindowController.markAsUnknown(_:)))
        item("Clear Mark", #selector(RomWindowController.clearMark(_:)))
        item("Set Flags…", #selector(RomWindowController.setFlags(_:)))
        menu.addItem(.separator())
        item("Copy Address", #selector(RomWindowController.copyAddress(_:)))
        item("Copy Line", #selector(RomWindowController.copyLine(_:)))
        return menu
    }
}

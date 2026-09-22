import AppKit
import SwiftUI

/// The disassembly view: one canvas as tall as the line index, drawing the
/// lines in the dirty rect from the asm batch cache.
struct AsmTableView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSScrollView {
        let controller = AsmPaneController(model: model)
        context.coordinator.controller = controller
        return controller.scrollView
    }

    func updateNSView(_ view: NSScrollView, context: Context) {
        context.coordinator.controller?.update()
    }

    @MainActor
    final class Coordinator {
        var controller: AsmPaneController?
    }
}

/// Builds and updates one disassembly pane. Shared by the Disassembly tab
/// and the Both tab.
@MainActor
final class AsmPaneController {
    let model: RomViewModel
    let canvas: AsmCanvasView
    let scrollView: NSScrollView
    private var asmGeneration = -1
    private var selectedOffset: UInt32?
    private var lastScrollId = 0

    init(model: RomViewModel) {
        self.model = model
        canvas = AsmCanvasView(model: model)
        scrollView = AsmScrollView()
        scrollView.documentView = canvas
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor
        scrollView.horizontalScrollElasticity = .none
        canvas.autoresizingMask = []
        canvas.frame = NSRect(x: 0, y: 0, width: model.asmLayout.totalWidth, height: canvas.documentHeight)
        (scrollView as? AsmScrollView)?.onLayout = { [weak canvas] in canvas?.fitWidth() }
        canvas.onKeyCommand = { [weak self] command in self?.handle(command) }
    }

    func update() {
        let generation = model.asmGeneration
        let selected = model.selectedOffset
        let scroll = model.scrollRequest
        if asmGeneration != generation {
            asmGeneration = generation
            canvas.layoutDidChange()
        }
        if selectedOffset != selected {
            if let old = selectedOffset, let line = model.workbench.lineForOffset(fileOffset: old) {
                canvas.invalidate(line: Int(line))
            }
            if let new = selected, let line = model.workbench.lineForOffset(fileOffset: new) {
                canvas.invalidate(line: Int(line))
            }
            selectedOffset = selected
        }
        if let scroll, lastScrollId != scroll.id {
            lastScrollId = scroll.id
            if let line = model.workbench.lineForOffset(fileOffset: scroll.offset) {
                self.scroll(toLine: Int(line))
            }
        }
    }

    var visibleLines: Int {
        max(1, Int(scrollView.contentView.bounds.height / canvas.rowHeight) - 1)
    }

    func scroll(toLine line: Int) {
        let rect = canvas.rect(ofLine: line)
        let clip = scrollView.contentView
        let visibleHeight = clip.bounds.height
        let y = max(0, min(rect.midY - visibleHeight / 2, canvas.bounds.height - visibleHeight))
        clip.scroll(to: NSPoint(x: clip.bounds.origin.x, y: y))
        scrollView.reflectScrolledClipView(clip)
    }

    func handle(_ command: EditorKeyCommand) {
        model.perform(command, from: .asm, visibleItems: visibleLines)
    }
}

/// A scroll view that tells its canvas to refit when the viewport resizes.
final class AsmScrollView: NSScrollView {
    var onLayout: (() -> Void)?

    override func layout() {
        super.layout()
        onLayout?()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        (documentView as? AsmCanvasView)?.appearanceDidChange()
    }
}

/// The disassembly document view.
final class AsmCanvasView: NSView {
    let model: RomViewModel
    var onKeyCommand: ((EditorKeyCommand) -> Void)?
    /// Token colours are baked into the cached lines; bump on appearance change.
    private var appearanceGeneration = 0

    init(model: RomViewModel) {
        self.model = model
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    var rowHeight: CGFloat { model.asmLayout.rowHeight }
    var lineCount: Int { Int(model.asmLineCount) }
    var documentHeight: CGFloat { CGFloat(lineCount) * rowHeight }
    private var generation: Int { model.asmGeneration * 1000 + appearanceGeneration }

    func rect(ofLine line: Int) -> NSRect {
        NSRect(x: 0, y: CGFloat(line) * rowHeight, width: bounds.width, height: rowHeight)
    }

    func lines(in rect: NSRect) -> Range<Int> {
        let first = max(0, Int(floor(rect.minY / rowHeight)))
        let last = min(lineCount, Int(ceil(rect.maxY / rowHeight)))
        return first..<max(first, last)
    }

    var visibleLines: Range<Int> { lines(in: visibleRect) }

    func layoutDidChange() {
        fitWidth()
        needsDisplay = true
    }

    func appearanceDidChange() {
        appearanceGeneration += 1
        needsDisplay = true
    }

    func fitWidth() {
        let width = max(model.asmLayout.totalWidth, superview?.bounds.width ?? 0)
        if frame.size.width != width || frame.size.height != documentHeight {
            setFrameSize(NSSize(width: width, height: documentHeight))
        }
    }

    func invalidate(line: Int) {
        setNeedsDisplay(rect(ofLine: line))
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
        let layout = model.asmLayout
        let gen = generation
        let selectedLine = model.selectedOffset.flatMap { model.workbench.lineForOffset(fileOffset: $0) }
        let focused = window?.firstResponder === self
        for line in lines(in: dirtyRect) {
            let line32 = UInt32(line)
            let batch = model.asmBatch(containingLine: line32)
            guard batch.contains(line: line32) else { continue }
            let record = batch.record(line: line32)
            let ctLine = batch.line(line: line32, generation: gen, layout: layout)
            AsmLinePainter.draw(
                record: record, line: ctLine, layout: layout,
                selected: selectedLine == line32, focused: focused,
                in: rect(ofLine: line), context: context
            )
        }
    }

    /// The line under a point.
    func line(at point: NSPoint) -> UInt32? {
        let line = Int(floor(point.y / rowHeight))
        guard line >= 0, line < lineCount else { return nil }
        return UInt32(line)
    }

    func record(atLine line: UInt32) -> AsmLineRecord? {
        let batch = model.asmBatch(containingLine: line)
        return batch.contains(line: line) ? batch.record(line: line) : nil
    }

    /// The token under a point, with its record.
    func token(at point: NSPoint) -> (AsmLineRecord, AsmToken)? {
        guard let line = line(at: point), let record = record(atLine: line),
              let column = model.asmLayout.column(atX: point.x),
              let token = model.asmLayout.token(atColumn: column, in: record)
        else { return nil }
        return (record, token)
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        guard let line = line(at: point), let record = record(atLine: line) else {
            super.mouseDown(with: event)
            return
        }
        window?.makeFirstResponder(self)
        if event.clickCount == 2 {
            if let target = record.targetFileOffset {
                model.jump(to: target)
            } else {
                model.followReference()
            }
            return
        }
        if event.modifierFlags.contains(.shift) {
            model.extendSelection(to: record.fileOffset)
        } else {
            model.select(offset: record.fileOffset)
        }
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let point = convert(event.locationInWindow, from: nil)
        if let line = line(at: point), let record = record(atLine: line),
           model.highlightedRange?.contains(record.fileOffset) != true {
            model.select(offset: record.fileOffset)
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

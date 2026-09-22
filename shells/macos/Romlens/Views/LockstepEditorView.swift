import AppKit
import SwiftUI

/// The Both tab: hex on the left, disassembly on the right, scrolled in
/// lockstep, with a bracket joining the highlighted bytes to their line.
struct LockstepEditorView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> LockstepPaneView {
        let hex = HexPaneController(model: model)
        let asm = AsmPaneController(model: model)
        let pane = LockstepPaneView(hex: hex, asm: asm)
        let controller = LockstepController(model: model, hex: hex, asm: asm, pane: pane)
        context.coordinator.controller = controller
        return pane
    }

    func updateNSView(_ pane: LockstepPaneView, context: Context) {
        context.coordinator.controller?.update()
    }

    @MainActor
    final class Coordinator {
        var controller: LockstepController?
    }
}

/// A vertical split of the two panes with the bracket overlay on top.
final class LockstepPaneView: NSView {
    let split: NSSplitView
    let overlay: BracketOverlayView
    let hex: HexPaneController
    let asm: AsmPaneController

    init(hex: HexPaneController, asm: AsmPaneController) {
        self.hex = hex
        self.asm = asm
        split = NSSplitView()
        split.isVertical = true
        split.dividerStyle = .thin
        split.autosaveName = "lockstep"
        split.addArrangedSubview(hex.pane)
        split.addArrangedSubview(asm.scrollView)
        overlay = BracketOverlayView()
        super.init(frame: .zero)
        addSubview(split)
        addSubview(overlay)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var isFlipped: Bool { true }

    override func layout() {
        super.layout()
        split.frame = bounds
        overlay.frame = bounds
        if split.arrangedSubviews.count == 2, hex.pane.frame.width < 200, bounds.width > 400 {
            split.setPosition(bounds.width * 0.55, ofDividerAt: 0)
        }
    }
}

/// Keeps the two clip views aligned and the bracket current.
@MainActor
final class LockstepController: NSObject {
    let model: RomViewModel
    let hex: HexPaneController
    let asm: AsmPaneController
    let pane: LockstepPaneView
    private var syncing = false
    private var lastHighlighted: Range<UInt32>?

    init(model: RomViewModel, hex: HexPaneController, asm: AsmPaneController, pane: LockstepPaneView) {
        self.model = model
        self.hex = hex
        self.asm = asm
        self.pane = pane
        super.init()
        for clip in [hex.scrollView.contentView, asm.scrollView.contentView] {
            clip.postsBoundsChangedNotifications = true
        }
        // Selector-based observers are unregistered automatically on dealloc.
        NotificationCenter.default.addObserver(
            self, selector: #selector(hexClipChanged(_:)),
            name: NSView.boundsDidChangeNotification, object: hex.scrollView.contentView
        )
        NotificationCenter.default.addObserver(
            self, selector: #selector(asmClipChanged(_:)),
            name: NSView.boundsDidChangeNotification, object: asm.scrollView.contentView
        )
    }

    @objc private func hexClipChanged(_ note: Notification) { hexScrolled() }
    @objc private func asmClipChanged(_ note: Notification) { asmScrolled() }

    func update() {
        hex.update()
        asm.update()
        if lastHighlighted != model.highlightedRange {
            lastHighlighted = model.highlightedRange
        }
        refreshBracket()
    }

    /// The asm line that should sit at the top when hex row `row` is at the top.
    func asmLine(forTopRow row: Int) -> Int? {
        model.workbench.lineForOffset(fileOffset: UInt32(row) * 16).map(Int.init)
    }

    /// The hex row that should sit at the top when asm line `line` is at the top.
    func hexRow(forTopLine line: Int) -> Int? {
        model.workbench.offsetForLine(line: UInt32(line)).map { Int($0 / 16) }
    }

    private func hexScrolled() {
        guard !syncing else { return }
        let hexTop = hex.scrollView.contentView.bounds.origin.y
        let row = Int(floor(hexTop / hex.canvas.rowHeight))
        if let line = asmLine(forTopRow: row) {
            let y = CGFloat(line) * asm.canvas.rowHeight
            setTop(of: asm.scrollView, y: y)
        }
        refreshBracket()
    }

    private func asmScrolled() {
        guard !syncing else { return }
        let asmTop = asm.scrollView.contentView.bounds.origin.y
        let line = Int(floor(asmTop / asm.canvas.rowHeight))
        if let row = hexRow(forTopLine: line) {
            let y = CGFloat(row) * hex.canvas.rowHeight
            setTop(of: hex.scrollView, y: y)
        }
        refreshBracket()
    }

    private func setTop(of scrollView: NSScrollView, y: CGFloat) {
        let clip = scrollView.contentView
        let maxY = max(0, (scrollView.documentView?.bounds.height ?? 0) - clip.bounds.height)
        let target = max(0, min(y, maxY))
        // A dead band stops the two panes nudging each other forever.
        guard abs(clip.bounds.origin.y - target) > 0.5 else { return }
        syncing = true
        clip.scroll(to: NSPoint(x: clip.bounds.origin.x, y: target))
        scrollView.reflectScrolledClipView(clip)
        syncing = false
    }

    /// Anchors: the highlighted rows' right edge in the hex pane and the
    /// selected line's left edge in the asm pane, converted into the
    /// overlay and clipped to their scroll frames.
    func refreshBracket() {
        guard let range = model.highlightedRange, !range.isEmpty,
              let line = model.workbench.lineForOffset(fileOffset: range.lowerBound)
        else {
            pane.overlay.bracket = nil
            return
        }
        let firstRow = Int(range.lowerBound / 16)
        let lastRow = Int((range.upperBound - 1) / 16)
        let hexLayout = model.layout
        let xRight = hexLayout.hexRect(byte: 15, height: 1).maxX
        let hexTop = hex.canvas.convert(NSPoint(x: xRight, y: hex.canvas.rect(ofRow: firstRow).minY), to: pane.overlay)
        let hexBottom = hex.canvas.convert(NSPoint(x: xRight, y: hex.canvas.rect(ofRow: lastRow).maxY), to: pane.overlay)
        let asmRect = asm.canvas.rect(ofLine: Int(line))
        let asmTop = asm.canvas.convert(NSPoint(x: 0, y: asmRect.minY), to: pane.overlay)
        let asmBottom = asm.canvas.convert(NSPoint(x: 0, y: asmRect.maxY), to: pane.overlay)
        let hexFrame = hex.scrollView.convert(hex.scrollView.contentView.frame, to: pane.overlay)
        let asmFrame = asm.scrollView.convert(asm.scrollView.contentView.frame, to: pane.overlay)
        func clip(_ p: NSPoint, to frame: NSRect) -> NSPoint {
            NSPoint(x: min(max(p.x, frame.minX), frame.maxX), y: min(max(p.y, frame.minY), frame.maxY))
        }
        pane.overlay.bracket = BracketOverlayView.Bracket(
            hexTop: clip(hexTop, to: hexFrame), hexBottom: clip(hexBottom, to: hexFrame),
            asmTop: clip(asmTop, to: asmFrame), asmBottom: clip(asmBottom, to: asmFrame),
            hexFrame: hexFrame, asmFrame: asmFrame
        )
    }
}

/// Draws the bracket; never intercepts events.
final class BracketOverlayView: NSView {
    struct Bracket: Equatable {
        var hexTop: NSPoint
        var hexBottom: NSPoint
        var asmTop: NSPoint
        var asmBottom: NSPoint
        var hexFrame: NSRect
        var asmFrame: NSRect
    }

    var bracket: Bracket? {
        didSet { if bracket != oldValue { needsDisplay = true } }
    }

    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func draw(_ dirtyRect: NSRect) {
        guard let b = bracket, let context = NSGraphicsContext.current?.cgContext else { return }
        let accent = NSColor.controlAccentColor
        context.setStrokeColor(accent.withAlphaComponent(0.8).cgColor)
        context.setLineWidth(1.5)
        let path = CGMutablePath()
        // Hex side: a bracket along the highlighted rows' right edge.
        path.move(to: CGPoint(x: b.hexTop.x - 4, y: b.hexTop.y))
        path.addLine(to: CGPoint(x: b.hexTop.x + 4, y: b.hexTop.y))
        path.addLine(to: CGPoint(x: b.hexBottom.x + 4, y: b.hexBottom.y))
        path.addLine(to: CGPoint(x: b.hexBottom.x - 4, y: b.hexBottom.y))
        // The join across the divider to the line's left edge.
        let hexMid = CGPoint(x: b.hexTop.x + 4, y: (b.hexTop.y + b.hexBottom.y) / 2)
        let asmMid = CGPoint(x: b.asmTop.x + 6, y: (b.asmTop.y + b.asmBottom.y) / 2)
        path.move(to: hexMid)
        path.addCurve(
            to: asmMid,
            control1: CGPoint(x: (hexMid.x + asmMid.x) / 2, y: hexMid.y),
            control2: CGPoint(x: (hexMid.x + asmMid.x) / 2, y: asmMid.y)
        )
        // Asm side: a bracket along the selected line's left edge.
        path.move(to: CGPoint(x: b.asmTop.x + 12, y: b.asmTop.y))
        path.addLine(to: CGPoint(x: b.asmTop.x + 6, y: b.asmTop.y))
        path.addLine(to: CGPoint(x: b.asmBottom.x + 6, y: b.asmBottom.y))
        path.addLine(to: CGPoint(x: b.asmBottom.x + 12, y: b.asmBottom.y))
        context.addPath(path)
        context.strokePath()
    }
}

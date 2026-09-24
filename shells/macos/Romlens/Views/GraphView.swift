import AppKit
import CoreText
import RomlensKit
import SwiftUI

/// The Graph tab (docs/19): the routine at the cursor as a control-flow
/// graph of the listing's own lines, or with its callers and callees.
struct GraphView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let pane = GraphPaneController(model: model)
        context.coordinator.pane = pane
        return pane.view
    }

    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.pane?.update()
    }

    /// Whatever the editor area offers: a wide graph must never widen the
    /// window's columns.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSView, context: Context) -> CGSize? {
        proposal.replacingUnspecifiedDimensions(by: CGSize(width: 400, height: 300))
    }

    @MainActor
    final class Coordinator {
        var pane: GraphPaneController?
    }
}

/// The header (the routine, what the graph holds, the mode and zoom) over
/// the zoomable canvas.
@MainActor
final class GraphPaneController: NSObject {
    let model: RomViewModel
    let view = NSView()
    let canvas = GraphCanvasView()
    let scrollView = NSScrollView()
    private let title = NSTextField(labelWithString: "")
    private let status = NSTextField(labelWithString: "")
    private let modes = NSSegmentedControl(
        labels: GraphModel.Mode.allCases.map(\.title), trackingMode: .selectOne, target: nil, action: nil
    )
    private let zoom = NSSegmentedControl(
        labels: ["−", "Fit", "+"], trackingMode: .momentary, target: nil, action: nil
    )
    private var shownGeneration = -1
    private var shownEntry: UInt32?
    private var shownMode: GraphModel.Mode?
    private var selected: UInt32?
    /// A click in the canvas moved the selection: it is in view already.
    private var selectingFromCanvas = false
    private var zoomId = 0

    init(model: RomViewModel) {
        self.model = model
        super.init()
        canvas.metrics = model.metrics
        canvas.setAccessibilityIdentifier("graph-canvas")
        canvas.onSelect = { [weak self] offset in
            guard let self else { return }
            self.selectingFromCanvas = true
            self.model.select(offset: offset)
            self.model.requestScroll(toOffset: offset)
        }
        canvas.onOpen = { [weak self] address in self?.model.jump(toSnesAddress: address) }

        scrollView.documentView = canvas
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .underPageBackgroundColor
        scrollView.allowsMagnification = true
        scrollView.minMagnification = 0.1
        scrollView.maxMagnification = 3

        title.font = .boldSystemFont(ofSize: NSFont.systemFontSize)
        title.lineBreakMode = .byTruncatingTail
        status.textColor = .secondaryLabelColor
        status.lineBreakMode = .byTruncatingTail
        modes.target = self
        modes.action = #selector(modeChanged(_:))
        modes.controlSize = .small
        modes.setAccessibilityIdentifier("graph-mode")
        modes.toolTip = "Blocks: the routine's control flow. Calls: who calls it and what it calls."
        zoom.target = self
        zoom.action = #selector(zoomed(_:))
        zoom.controlSize = .small
        zoom.setToolTip("Zoom out (⌘−)", forSegment: 0)
        zoom.setToolTip("Fit the graph in the window", forSegment: 1)
        zoom.setToolTip("Zoom in (⌘=)", forSegment: 2)

        let header = NSStackView(views: [title, status, NSView(), modes, zoom])
        header.orientation = .horizontal
        header.spacing = 8
        header.edgeInsets = NSEdgeInsets(top: 4, left: 8, bottom: 4, right: 8)
        header.setHuggingPriority(.defaultHigh, for: .vertical)
        status.setContentCompressionResistancePriority(.init(1), for: .horizontal)
        title.setContentCompressionResistancePriority(.init(2), for: .horizontal)
        let rule = NSBox()
        rule.boxType = .separator
        let stack = NSStackView(views: [header, rule, scrollView])
        stack.orientation = .vertical
        stack.spacing = 0
        stack.alignment = .leading
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            stack.topAnchor.constraint(equalTo: view.topAnchor),
            stack.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            header.widthAnchor.constraint(equalTo: stack.widthAnchor),
            rule.widthAnchor.constraint(equalTo: stack.widthAnchor),
            scrollView.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ])
    }

    func update() {
        let g = model.graph
        modes.selectedSegment = GraphModel.Mode.allCases.firstIndex(of: g.mode) ?? 0
        switch g.state {
        case .idle:
            title.stringValue = "No routine"
            status.stringValue = "Select an instruction to see its routine as a graph."
        case .loading:
            status.stringValue = g.mode == .blocks ? "Building the graph…" : "Finding callers and callees…"
        case .ready:
            if let b = g.blocks {
                title.stringValue = "\(b.name)  \(formatSnesAddress(address: b.entry))"
                status.stringValue = GraphSceneBuilder.summary(b)
            } else if let c = g.calls {
                title.stringValue = "\(c.name)  \(formatSnesAddress(address: c.entry))"
                status.stringValue = GraphSceneBuilder.summary(c)
            }
        case .notInRoutine:
            title.stringValue = "No routine"
            status.stringValue = "The selection is not inside a routine the analysis found."
        case .failed(let message):
            status.stringValue = message
        }

        var routineChanged = false
        if shownGeneration != g.resultGeneration {
            shownGeneration = g.resultGeneration
            let entry = g.blocks?.entry ?? g.calls?.entry
            routineChanged = entry != shownEntry || g.mode != shownMode
            let origin = scrollView.contentView.bounds.origin
            if let b = g.blocks {
                canvas.scene = GraphSceneBuilder.blocks(b, model: model)
            } else if let c = g.calls {
                canvas.scene = GraphSceneBuilder.calls(c, model: model)
            } else {
                canvas.scene = .empty
            }
            shownEntry = entry
            shownMode = g.mode
            if routineChanged {
                showTop()
            } else {
                scrollView.contentView.scroll(to: origin)
                scrollView.reflectScrolledClipView(scrollView.contentView)
            }
        }

        if let z = g.zoomRequest, z.id != zoomId {
            zoomId = z.id
            switch z.kind {
            case .zoomIn: zoomIn()
            case .zoomOut: zoomOut()
            case .fit: fit()
            }
        }

        let sel = model.instruction?.fileOffset ?? model.selectedOffset
        if sel != selected || routineChanged {
            selected = sel
            canvas.selectedOffset = sel
            if !selectingFromCanvas, let sel, let rect = canvas.scene.rect(forOffset: sel) {
                reveal(rect)
            }
        }
        selectingFromCanvas = false
    }

    /// The routine's first box at the top, centred across.
    private func showTop() {
        guard let first = canvas.scene.boxes.first else { return }
        let clip = scrollView.contentView
        let visible = clip.bounds.size
        let x = max(0, min(first.rect.midX - visible.width / 2, canvas.bounds.width - visible.width))
        clip.scroll(to: NSPoint(x: x, y: 0))
        scrollView.reflectScrolledClipView(clip)
    }

    private func reveal(_ rect: CGRect) {
        let visible = scrollView.contentView.documentVisibleRect
        guard !visible.contains(rect) else { return }
        let clip = scrollView.contentView
        let size = clip.bounds.size
        let x = max(0, min(rect.midX - size.width / 2, canvas.bounds.width - size.width))
        let y = max(0, min(rect.midY - size.height / 2, canvas.bounds.height - size.height))
        clip.scroll(to: NSPoint(x: x, y: y))
        scrollView.reflectScrolledClipView(clip)
    }

    @objc private func modeChanged(_ sender: NSSegmentedControl) {
        model.graph.mode = GraphModel.Mode.allCases[max(0, sender.selectedSegment)]
        model.refreshGraph()
    }

    @objc private func zoomed(_ sender: NSSegmentedControl) {
        switch sender.selectedSegment {
        case 0: zoomOut()
        case 1: fit()
        default: zoomIn()
        }
    }

    func zoomIn() { scrollView.animator().magnification = min(scrollView.maxMagnification, scrollView.magnification * 1.25) }
    func zoomOut() { scrollView.animator().magnification = max(scrollView.minMagnification, scrollView.magnification / 1.25) }

    func fit() {
        let size = scrollView.contentView.frame.size
        guard canvas.bounds.width > 0, canvas.bounds.height > 0 else { return }
        let m = min(size.width / canvas.bounds.width, size.height / canvas.bounds.height, 1)
        scrollView.magnification = max(scrollView.minMagnification, m)
        showTop()
    }
}

/// What the canvas draws: boxes of lines and the edges between them, in the
/// canvas's coordinates (top-left origin).
struct GraphScene {
    struct Line {
        let text: CTLine
        /// The instruction (or data) a click selects.
        let offset: UInt32?
    }

    struct Box {
        let rect: CGRect
        let lines: [Line]
        /// Loops it is in, for the tint.
        let loopDepth: Int
        let loopHeader: Bool
        /// A tail call or an unknown destination: no instructions.
        let stub: Bool
        /// The routine in the middle of the Calls view, or the entry block.
        let emphasis: Bool
        /// The recording never reached it.
        let dim: Bool
        /// Above the box's top-right corner, as `768×`.
        let badge: String?
        let toolTip: String?
        /// Where a double-click goes.
        let target: UInt32?
    }

    struct Edge {
        let points: [CGPoint]
        let color: NSColor
        let width: CGFloat
        let dashed: Bool
        let label: String?
        let dim: Bool
    }

    var boxes: [Box]
    var edges: [Edge]
    var size: CGSize
    let rowHeight: CGFloat
    let padding: CGSize

    static var empty: GraphScene { GraphScene(boxes: [], edges: [], size: .zero, rowHeight: 16, padding: .zero) }

    /// The box and line holding the instruction at `offset`.
    func line(forOffset offset: UInt32) -> (box: Int, line: Int)? {
        for (b, box) in boxes.enumerated() {
            if let l = box.lines.firstIndex(where: { $0.offset == offset }) {
                return (b, l)
            }
        }
        return nil
    }

    func rect(forOffset offset: UInt32) -> CGRect? {
        guard let (b, l) = line(forOffset: offset) else { return nil }
        return lineRect(box: b, line: l)
    }

    func lineRect(box b: Int, line l: Int) -> CGRect {
        let r = boxes[b].rect
        return CGRect(x: r.minX, y: r.minY + padding.height + CGFloat(l) * rowHeight, width: r.width, height: rowHeight)
    }

    /// The box and line under a point.
    func hit(_ p: CGPoint) -> (box: Int, line: Int?)? {
        guard let b = boxes.lastIndex(where: { $0.rect.contains(p) }) else { return nil }
        let r = boxes[b].rect
        let l = Int(floor((p.y - r.minY - padding.height) / rowHeight))
        return (b, boxes[b].lines.indices.contains(l) ? l : nil)
    }
}

/// Draws a `GraphScene`; reports clicks as the instruction they select and
/// double-clicks as the address they open.
final class GraphCanvasView: NSView {
    var metrics = MonoMetrics()
    var scene = GraphScene.empty {
        didSet {
            let margin: CGFloat = 24
            setFrameSize(NSSize(width: scene.size.width + margin, height: scene.size.height + margin))
            removeAllToolTips()
            for box in scene.boxes {
                if let tip = box.toolTip {
                    addToolTip(box.rect, owner: tip as NSString, userData: nil)
                }
            }
            needsDisplay = true
        }
    }
    var selectedOffset: UInt32? {
        didSet { if selectedOffset != oldValue { needsDisplay = true } }
    }
    var onSelect: ((UInt32) -> Void)?
    var onOpen: ((UInt32) -> Void)?

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        NSColor.underPageBackgroundColor.setFill()
        dirtyRect.fill()
        let selected = selectedOffset.flatMap { scene.line(forOffset: $0) }
        for (i, box) in scene.boxes.enumerated() where box.rect.insetBy(dx: -60, dy: -20).intersects(dirtyRect) {
            draw(box, index: i, selectedLine: selected?.box == i ? selected?.line : nil, in: context)
        }
        for edge in scene.edges {
            draw(edge, in: context)
        }
    }

    private func draw(_ box: GraphScene.Box, index: Int, selectedLine: Int?, in context: CGContext) {
        context.saveGState()
        if box.dim { context.setAlpha(0.4) }
        let r = box.rect
        let shape = NSBezierPath(roundedRect: r, xRadius: 4, yRadius: 4)
        var fill = NSColor.textBackgroundColor
        if box.loopDepth > 0 {
            fill = fill.blended(withFraction: min(0.08 * CGFloat(box.loopDepth), 0.24), of: .systemBlue) ?? fill
        }
        (box.stub ? NSColor.clear : fill).setFill()
        shape.fill()
        if let l = selectedLine {
            let lr = scene.lineRect(box: index, line: l)
            NSColor.controlAccentColor.withAlphaComponent(0.25).setFill()
            lr.fill()
        }
        let border: NSColor = box.loopHeader ? .systemBlue : (box.emphasis ? .labelColor : .separatorColor)
        border.setStroke()
        shape.lineWidth = box.emphasis || box.loopHeader ? 1.5 : 1
        if box.stub { shape.setLineDash([4, 3], count: 2, phase: 0) }
        shape.stroke()

        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        let h = scene.rowHeight
        for (i, line) in box.lines.enumerated() {
            let top = r.minY + scene.padding.height + CGFloat(i) * h
            context.textPosition = CGPoint(x: r.minX + scene.padding.width, y: metrics.baseline(rowTop: top, height: h))
            CTLineDraw(line.text, context)
        }
        if let badge = box.badge {
            let s = NSAttributedString(string: badge, attributes: [
                .font: NSFont.monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular),
                .foregroundColor: NSColor.secondaryLabelColor,
            ])
            let size = s.size()
            s.draw(at: NSPoint(x: r.maxX - size.width, y: r.minY - size.height - 1))
        }
        context.restoreGState()
    }

    private func draw(_ edge: GraphScene.Edge, in context: CGContext) {
        guard edge.points.count >= 2 else { return }
        context.saveGState()
        if edge.dim { context.setAlpha(0.35) }
        let path = CGMutablePath()
        let p = edge.points
        path.move(to: p[0])
        for i in 1..<p.count - 1 {
            // Round each corner by up to 8 points.
            let a = p[i - 1], b = p[i], c = p[i + 1]
            let r = min(8, hypot(b.x - a.x, b.y - a.y) / 2, hypot(c.x - b.x, c.y - b.y) / 2)
            path.addArc(tangent1End: b, tangent2End: c, radius: r)
        }
        // Stop short of the tip so the line does not poke through it.
        let end = p[p.count - 1], before = p[p.count - 2]
        let len = max(hypot(end.x - before.x, end.y - before.y), 0.001)
        let ux = (end.x - before.x) / len, uy = (end.y - before.y) / len
        let arrow: CGFloat = 7
        path.addLine(to: CGPoint(x: end.x - ux * arrow * 0.8, y: end.y - uy * arrow * 0.8))
        context.addPath(path)
        context.setStrokeColor(edge.color.cgColor)
        context.setLineWidth(edge.width)
        context.setLineJoin(.round)
        if edge.dashed { context.setLineDash(phase: 0, lengths: [5, 3]) }
        context.strokePath()
        context.setLineDash(phase: 0, lengths: [])
        let head = CGMutablePath()
        head.move(to: end)
        head.addLine(to: CGPoint(x: end.x - ux * arrow - uy * arrow * 0.5, y: end.y - uy * arrow + ux * arrow * 0.5))
        head.addLine(to: CGPoint(x: end.x - ux * arrow + uy * arrow * 0.5, y: end.y - uy * arrow - ux * arrow * 0.5))
        head.closeSubpath()
        context.addPath(head)
        context.setFillColor(edge.color.cgColor)
        context.fillPath()
        if let label = edge.label {
            let s = NSAttributedString(string: label, attributes: [
                .font: NSFont.monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular),
                .foregroundColor: edge.color,
            ])
            s.draw(at: NSPoint(x: p[0].x + 4, y: p[0].y + 1))
        }
        context.restoreGState()
    }

    // MARK: Mouse

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        let p = convert(event.locationInWindow, from: nil)
        guard let (b, l) = scene.hit(p) else { return }
        let box = scene.boxes[b]
        if event.clickCount == 2, let target = box.target {
            onOpen?(target)
            return
        }
        if let l, let offset = box.lines[l].offset {
            onSelect?(offset)
        } else if let offset = box.lines.compactMap(\.offset).first {
            onSelect?(offset)
        }
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let p = convert(event.locationInWindow, from: nil)
        guard let (b, l) = scene.hit(p), let l, let offset = scene.boxes[b].lines[l].offset else { return nil }
        onSelect?(offset)
        return EditorContextMenu.build(hasSelection: true)
    }

    /// Select the line at a point, as a click there would (tests).
    func click(at p: CGPoint) {
        guard let (b, l) = scene.hit(p), let l, let offset = scene.boxes[b].lines[l].offset else { return }
        onSelect?(offset)
    }
}

/// Builds the scene for each mode from the core's graph, the listing's
/// records and the core's layout.
@MainActor
enum GraphSceneBuilder {
    static let padding = CGSize(width: 8, height: 4)
    static let options = GraphLayoutOptions(nodeGap: 28, layerGap: 44, edgeGap: 14)
    static let margin: CGFloat = 12

    static func summary(_ g: RoutineGraphInfo) -> String {
        var parts = [
            plural(g.blocks.count, "block"),
            plural(g.edges.count, "edge"),
            plural(g.loops.count, "loop"),
        ]
        if g.counted { parts.append("counts from the execution log") }
        if g.irreducible { parts.append("a loop with more than one way in") }
        if g.truncated { parts.append("cut short: too long to follow") }
        return parts.joined(separator: ", ")
    }

    static func summary(_ n: CallNeighbourhoodInfo) -> String {
        "\(plural(n.callers.count, "caller")), \(plural(n.callees.count, "callee"))"
            + (n.counted ? ", counts from the execution log" : "")
    }

    static func plural(_ n: Int, _ what: String) -> String {
        "\(n) \(what)\(n == 1 ? "" : "s")"
    }

    static func count(_ n: UInt64) -> String {
        "\(n.formatted(.number.grouping(.automatic)))×"
    }

    // MARK: Blocks

    static func blocks(_ g: RoutineGraphInfo, model: RomViewModel) -> GraphScene {
        let m = model.metrics
        var contents: [(lines: [GraphScene.Line], width: CGFloat)] = []
        for b in g.blocks {
            var lines: [GraphScene.Line] = []
            if let first = b.firstLine {
                for n in first..<first + b.lineCount {
                    let r = model.asmBatch(containingLine: n).record(line: n)
                    if r.kind == .blank || r.kind == .section { continue }
                    let offset = r.kind.isContent ? r.fileOffset : b.offsets.first
                    lines.append(.init(text: CTLineCreateWithAttributedString(text(for: r, metrics: m)), offset: offset))
                }
            } else {
                let name = b.exitText.isEmpty ? "?" : (b.exit == .tail ? b.exitText : "?")
                let s = NSAttributedString(string: "→ \(name)", attributes: [
                    .font: m.font, .foregroundColor: NSColor.secondaryLabelColor,
                ])
                lines.append(.init(text: CTLineCreateWithAttributedString(s), offset: nil))
            }
            let width = lines.map { CGFloat(CTLineGetTypographicBounds($0.text, nil, nil, nil)) }.max() ?? 0
            contents.append((lines, width))
        }
        let sizes = contents.map {
            GraphNodeSize(
                width: Double($0.width + padding.width * 2),
                height: Double(CGFloat(max($0.lines.count, 1)) * m.rowHeight + padding.height * 2)
            )
        }
        let specs = g.edges.map { GraphEdgeSpec(from: $0.from, to: $0.to, back: $0.back) }
        let layout = layoutGraph(nodes: sizes, edges: specs, options: options)

        var boxes: [GraphScene.Box] = []
        for (i, b) in g.blocks.enumerated() {
            let rect = CGRect(
                x: layout.nodes[i].x + margin, y: layout.nodes[i].y + margin + 14,
                width: sizes[i].width, height: sizes[i].height
            )
            let stub = b.offsets.isEmpty
            var tip: String?
            switch b.exit {
            case .tail: tip = "Continues in \(b.exitText); double-click to go there"
            case .unknown: tip = "Goes where the analysis cannot follow: \(b.exitText)"
            default: if let r = b.runs { tip = "Its first instruction ran \(r.formatted()) times in the execution log" }
            }
            boxes.append(.init(
                rect: rect, lines: contents[i].lines, loopDepth: Int(b.loopDepth), loopHeader: b.loopHeader,
                stub: stub, emphasis: i == 0, dim: g.counted && !stub && b.runs == 0,
                badge: stub ? nil : b.runs.map(count), toolTip: tip,
                target: b.exit == .tail ? b.exitTarget : nil
            ))
        }
        var edges: [GraphScene.Edge] = []
        for (i, e) in g.edges.enumerated() {
            let color: NSColor = switch e.kind {
            case _ where e.back: .systemBlue
            case .taken: .systemGreen
            case .notTaken: .systemRed
            case .case: .systemOrange
            case .jump, .fall: .secondaryLabelColor
            }
            var label: [String] = []
            if e.kind == .case { label.append(e.cases.map(String.init).joined(separator: ",")) }
            if let c = e.count { label.append(count(c)) }
            edges.append(.init(
                points: points(layout.edges[i].points, dy: margin + 14),
                color: color, width: e.back ? 1.6 : 1.2, dashed: false,
                label: label.isEmpty ? nil : label.joined(separator: " "),
                dim: g.counted && e.count == 0
            ))
        }
        return GraphScene(
            boxes: boxes, edges: edges,
            size: CGSize(width: layout.width + margin * 2, height: layout.height + margin * 2 + 14),
            rowHeight: m.rowHeight, padding: padding
        )
    }

    /// A listing record as the graph shows it: the SNES address and the
    /// text, coloured as the listing colours it.
    static func text(for r: AsmLineRecord, metrics: MonoMetrics) -> NSAttributedString {
        let s = NSMutableAttributedString()
        func add(_ t: String, _ c: NSColor) {
            s.append(NSAttributedString(string: t, attributes: [.font: metrics.font, .foregroundColor: c]))
        }
        if r.kind.isContent {
            let at = r.snesAddress.map { formatSnesAddress(address: $0) } ?? formatFileOffset(offset: r.fileOffset)
            add(at + "  ", .tertiaryLabelColor)
        }
        let dim: NSColor = r.kind == .data ? .secondaryLabelColor : .labelColor
        let bytes = Array(r.text.utf8)
        var at = 0
        for t in r.tokens.sorted(by: { $0.start < $1.start }) where t.start >= at && t.start + t.len <= bytes.count {
            if t.start > at { add(String(decoding: bytes[at..<t.start], as: UTF8.self), dim) }
            add(String(decoding: bytes[t.start..<t.start + t.len], as: UTF8.self), AsmTokenPalette.color(for: t.kind, dim: dim))
            at = t.start + t.len
        }
        if at < bytes.count { add(String(decoding: bytes[at...], as: UTF8.self), r.kind == .comment ? .systemGreen : dim) }
        return s
    }

    static func points(_ p: [GraphPoint], dy: CGFloat) -> [CGPoint] {
        p.map { CGPoint(x: $0.x + margin, y: $0.y + dy) }
    }

    // MARK: Calls

    static func calls(_ n: CallNeighbourhoodInfo, model: RomViewModel) -> GraphScene {
        let m = model.metrics
        let bold = NSFontManager.shared.convert(m.font, toHaveTrait: .boldFontMask)
        func line(_ t: String, _ font: NSFont, _ color: NSColor) -> GraphScene.Line {
            let s = NSAttributedString(string: t, attributes: [.font: font, .foregroundColor: color])
            return .init(text: CTLineCreateWithAttributedString(s), offset: nil)
        }
        func sitesText(_ c: CallLinkInfo) -> String {
            var hows: [String] = []
            for s in c.sites {
                let h = switch s.how {
                case .call: "call"
                case .table: "table"
                case .tail: "tail call"
                case .observed: "seen"
                }
                if !hows.contains(h) { hows.append(h) }
            }
            var t = "\(plural(c.sites.count, "site")): \(hows.joined(separator: ", "))"
            let total = c.sites.compactMap(\.count).reduce(0, +)
            if n.counted { t += ", ran \(total.formatted())×" }
            return t
        }
        struct Node {
            let lines: [GraphScene.Line]
            let entry: UInt32
            let how: CallHowKind?
            let centre: Bool
        }
        var nodes = [Node(
            lines: [
                line(n.name, bold, .labelColor),
                line(formatSnesAddress(address: n.entry), m.font, .secondaryLabelColor),
                line("\(plural(n.callers.count, "caller")), \(plural(n.callees.count, "callee"))", m.font, .secondaryLabelColor),
            ],
            entry: n.entry, how: nil, centre: true
        )]
        var specs: [GraphEdgeSpec] = []
        var hows: [CallHowKind] = []
        func add(_ c: CallLinkInfo, caller: Bool) {
            let how = c.sites.first?.how ?? .call
            if c.entry == n.entry {
                // Calls itself: once, as a loop on the middle box.
                if !caller {
                    specs.append(GraphEdgeSpec(from: 0, to: 0, back: true))
                    hows.append(how)
                }
                return
            }
            nodes.append(Node(
                lines: [
                    line(c.name, bold, .labelColor),
                    line("\(formatSnesAddress(address: c.entry))  \(sitesText(c))", m.font, .secondaryLabelColor),
                ],
                entry: c.entry, how: how, centre: false
            ))
            let i = UInt32(nodes.count - 1)
            specs.append(caller ? GraphEdgeSpec(from: i, to: 0, back: false) : GraphEdgeSpec(from: 0, to: i, back: false))
            hows.append(how)
        }
        n.callers.forEach { add($0, caller: true) }
        n.callees.forEach { add($0, caller: false) }

        let widths = nodes.map { node in
            node.lines.map { CGFloat(CTLineGetTypographicBounds($0.text, nil, nil, nil)) }.max() ?? 0
        }
        let sizes = nodes.enumerated().map { i, node in
            GraphNodeSize(
                width: Double(widths[i] + padding.width * 2),
                height: Double(CGFloat(node.lines.count) * m.rowHeight + padding.height * 2)
            )
        }
        // Sideways: callers in a column on the left, callees on the right,
        // so many of either stack down the page instead of across it. The
        // core lays it out top to bottom with the axes swapped.
        let swapped = sizes.map { GraphNodeSize(width: $0.height, height: $0.width) }
        let layout = layoutGraph(
            nodes: swapped, edges: specs,
            options: GraphLayoutOptions(nodeGap: 10, layerGap: 96, edgeGap: 10)
        )
        let boxes = nodes.enumerated().map { i, node in
            GraphScene.Box(
                rect: CGRect(x: layout.nodes[i].y + margin, y: layout.nodes[i].x + margin, width: sizes[i].width, height: sizes[i].height),
                lines: node.lines, loopDepth: 0, loopHeader: false, stub: false, emphasis: node.centre,
                dim: false, badge: nil,
                toolTip: node.centre ? nil : "Double-click to put this routine in the middle",
                target: node.centre ? nil : node.entry
            )
        }
        let edges = specs.enumerated().map { i, _ in
            let color: NSColor = switch hows[i] {
            case .call: .secondaryLabelColor
            case .table: .systemOrange
            case .tail: .systemPurple
            case .observed: .systemTeal
            }
            return GraphScene.Edge(
                points: layout.edges[i].points.map { CGPoint(x: $0.y + margin, y: $0.x + margin) },
                color: color, width: 1.2,
                dashed: hows[i] == .observed, label: nil, dim: false
            )
        }
        return GraphScene(
            boxes: boxes, edges: edges,
            size: CGSize(width: layout.height + margin * 2, height: layout.width + margin * 2),
            rowHeight: m.rowHeight, padding: padding
        )
    }
}

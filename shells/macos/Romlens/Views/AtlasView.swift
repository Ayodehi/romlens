import AppKit
import RomlensKit
import SwiftUI

/// The Atlas tab (docs/22, A2): the whole ROM as a map, one bank a row,
/// zoomed continuously from all of it down to single instructions.
struct AtlasView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let pane = AtlasPaneController(model: model)
        context.coordinator.pane = pane
        return pane.view
    }

    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.pane?.update()
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSView, context: Context) -> CGSize? {
        proposal.replacingUnspecifiedDimensions(by: CGSize(width: 400, height: 300))
    }

    @MainActor
    final class Coordinator {
        var pane: AtlasPaneController?
    }
}

/// The header (what is under the pointer, the overlay, the zoom) over the
/// canvas.
@MainActor
final class AtlasPaneController: NSObject {
    let model: RomViewModel
    let view = NSView()
    let canvas: AtlasCanvasView
    private let status = NSTextField(labelWithString: "")
    private let overlays = NSSegmentedControl(
        labels: AtlasModel.Overlay.allCases.map(\.title), trackingMode: .selectOne, target: nil, action: nil
    )
    private let zoom = NSSegmentedControl(
        labels: ["−", "Fit", "+"], trackingMode: .momentary, target: nil, action: nil
    )
    private var zoomId = 0

    init(model: RomViewModel) {
        self.model = model
        canvas = AtlasCanvasView(model: model)
        super.init()
        canvas.setAccessibilityIdentifier("atlas-canvas")
        canvas.onStatus = { [weak self] text in self?.status.stringValue = text }

        let title = NSTextField(labelWithString: "Atlas")
        title.font = .boldSystemFont(ofSize: NSFont.systemFontSize)
        status.textColor = .secondaryLabelColor
        status.lineBreakMode = .byTruncatingTail
        status.setContentCompressionResistancePriority(.init(1), for: .horizontal)
        overlays.target = self
        overlays.action = #selector(overlayChanged(_:))
        overlays.controlSize = .small
        overlays.setAccessibilityIdentifier("atlas-overlay")
        for (i, o) in AtlasModel.Overlay.allCases.enumerated() {
            overlays.setToolTip(o.help, forSegment: i)
        }
        zoom.target = self
        zoom.action = #selector(zoomed(_:))
        zoom.controlSize = .small
        zoom.setToolTip("Zoom out (⌘−)", forSegment: 0)
        zoom.setToolTip("The whole ROM, one bank a row (⌥⇧⌘0)", forSegment: 1)
        zoom.setToolTip("Zoom in (⌘=); pinch or ⌘-scroll to zoom where the pointer is", forSegment: 2)

        let header = NSStackView(views: [title, status, NSView(), overlays, zoom])
        header.orientation = .horizontal
        header.spacing = 8
        header.edgeInsets = NSEdgeInsets(top: 4, left: 8, bottom: 4, right: 8)
        header.setHuggingPriority(.defaultHigh, for: .vertical)
        let rule = NSBox()
        rule.boxType = .separator
        let stack = NSStackView(views: [header, rule, canvas])
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
            canvas.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ])
        canvas.showStatus()
    }

    func update() {
        let a = model.atlas
        overlays.selectedSegment = AtlasModel.Overlay.allCases.firstIndex(of: a.overlay) ?? 0
        canvas.overlay = a.overlay
        canvas.generation = model.stripGeneration
        canvas.selection = model.selectedOffset
        if let z = a.zoomRequest, z.id != zoomId {
            zoomId = z.id
            switch z.kind {
            case .zoomIn: canvas.zoom(by: 1.5, at: nil)
            case .zoomOut: canvas.zoom(by: 1 / 1.5, at: nil)
            case .fit: canvas.fit()
            }
        }
    }

    @objc private func overlayChanged(_ sender: NSSegmentedControl) {
        model.atlas.overlay = AtlasModel.Overlay.allCases[max(0, sender.selectedSegment)]
    }

    @objc private func zoomed(_ sender: NSSegmentedControl) {
        switch sender.selectedSegment {
        case 0: canvas.zoom(by: 1 / 1.5, at: nil)
        case 1: canvas.fit()
        default: canvas.zoom(by: 1.5, at: nil)
        }
    }
}

/// Draws the map. Each bank is a row; `ppb` pixels a byte across, fitted
/// to the width when zoomed out. Rows grow taller as it zooms in, up to a
/// limit, so a deep zoom is a long strip of one bank rather than a wall.
/// Columns come from the core a window at a time: the visible bytes of
/// each visible row, a column a pixel.
final class AtlasCanvasView: NSView {
    private let model: RomViewModel
    var overlay: AtlasModel.Overlay = .kind {
        didSet { if overlay != oldValue { needsDisplay = true } }
    }
    /// The analysis generation: a new one drops what was fetched.
    var generation = -1 {
        didSet {
            if generation != oldValue {
                columns.removeAll()
                items.removeAll()
                arcs = nil
                needsDisplay = true
            }
        }
    }
    var selection: UInt32? {
        didSet {
            defer { selecting = false }
            guard selection != oldValue else { return }
            if !selecting, let s = selection { reveal(s) }
            needsDisplay = true
        }
    }
    var onStatus: ((String) -> Void)?
    /// What the header's status line says.
    private(set) var statusText = "" {
        didSet { onStatus?(statusText) }
    }

    private let romLength: UInt32
    private let bankSize: UInt32
    private var rows: Int { Int((romLength + bankSize - 1) / bankSize) }
    /// Pixels a byte across.
    private(set) var ppb: CGFloat = 0
    private var atFit = true
    /// The top-left of the view in the map's coordinates.
    private var origin = CGPoint.zero
    private var hover: CGPoint?
    private var selecting = false

    private var columns: [String: [StripColumn]] = [:]
    private var items: [String: AtlasItemsInfo] = [:]
    /// Every call in the ROM between columns of `bytes` bytes.
    private var arcs: (bytes: UInt32, buckets: UInt32, list: [CallArcInfo])?

    private let gutter: CGFloat = 44
    private let pad: CGFloat = 8
    private let gap: CGFloat = 2
    private let maxRowHeight: CGFloat = 96
    /// Items (instructions, data rows) show from here.
    private let itemZoom: CGFloat = 4
    private let maxZoom: CGFloat = 48

    init(model: RomViewModel) {
        self.model = model
        romLength = max(1, model.byteCount)
        bankSize = model.info.mapping == .loRom ? 0x8000 : 0x1_0000
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    // MARK: Geometry

    private var fitPpb: CGFloat { max(bounds.width - gutter - pad, 1) / CGFloat(bankSize) }

    private var fitRowHeight: CGFloat {
        min(28, max(4, (bounds.height - 2 * pad) / CGFloat(max(rows, 1)) - gap))
    }

    private var rowHeight: CGFloat {
        guard fitPpb > 0 else { return fitRowHeight }
        return min(maxRowHeight, max(fitRowHeight, fitRowHeight * ppb / fitPpb))
    }

    private var pitch: CGFloat { rowHeight + gap }

    private var contentSize: CGSize {
        CGSize(width: CGFloat(bankSize) * ppb + pad, height: 2 * pad + CGFloat(rows) * pitch)
    }

    private var mapWidth: CGFloat { max(bounds.width - gutter, 1) }

    private func clampOrigin() {
        let size = contentSize
        origin.x = min(max(0, origin.x), max(0, size.width - mapWidth))
        origin.y = min(max(0, origin.y), max(0, size.height - bounds.height))
    }

    private func rowY(_ row: Int) -> CGFloat { pad + CGFloat(row) * pitch - origin.y }

    private func x(ofByte b: UInt32) -> CGFloat { gutter + CGFloat(b) * ppb - origin.x }

    /// The byte under a point in the view, if any.
    func offset(at p: CGPoint) -> UInt32? {
        guard p.x >= gutter else { return nil }
        let row = Int(floor((p.y + origin.y - pad) / pitch))
        guard row >= 0, row < rows, (p.y + origin.y - pad) - CGFloat(row) * pitch <= rowHeight else { return nil }
        let b = (p.x - gutter + origin.x) / ppb
        guard b >= 0, b < CGFloat(bankSize) else { return nil }
        let off = UInt32(row) * bankSize + UInt32(b)
        return off < romLength ? off : nil
    }

    /// Where a byte sits: the middle of its column, its row's middle.
    func point(ofOffset off: UInt32) -> CGPoint {
        let row = Int(off / bankSize)
        return CGPoint(x: x(ofByte: off % bankSize) + max(ppb, 1) / 2, y: rowY(row) + rowHeight / 2)
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        if atFit || ppb == 0 {
            ppb = fitPpb
            origin = .zero
        }
        ppb = max(ppb, fitPpb)
        clampOrigin()
        arcs = nil
    }

    func fit() {
        atFit = true
        ppb = fitPpb
        origin = .zero
        arcs = nil
        needsDisplay = true
        showStatus()
    }

    /// Zoom by `factor` keeping the byte under `anchor` (the view's centre
    /// when nil) where it is.
    func zoom(by factor: CGFloat, at anchor: CGPoint?) {
        let a = anchor ?? CGPoint(x: gutter + mapWidth / 2, y: bounds.height / 2)
        let byteX = (a.x - gutter + origin.x) / ppb
        let rowF = (a.y + origin.y - pad) / pitch
        let next = min(maxZoom, max(fitPpb, ppb * factor))
        guard next != ppb else { return }
        ppb = next
        atFit = abs(ppb - fitPpb) < 1e-9
        origin.x = byteX * ppb - (a.x - gutter)
        origin.y = pad + rowF * pitch - a.y
        clampOrigin()
        arcs = nil
        needsDisplay = true
        showStatus()
    }

    /// Scroll so `off` is in view, unless it is already.
    private func reveal(_ off: UInt32) {
        guard off < romLength, bounds.width > 0 else { return }
        let p = point(ofOffset: off)
        let inside = p.x >= gutter && p.x <= bounds.width && p.y >= 0 && p.y <= bounds.height
        guard !inside else { return }
        origin.x += p.x - (gutter + mapWidth / 2)
        origin.y += p.y - bounds.height / 2
        clampOrigin()
    }

    // MARK: Data

    private func fetchColumns(start: UInt32, len: UInt32, buckets: UInt32) -> [StripColumn] {
        let key = "\(start):\(len):\(buckets)"
        if let c = columns[key] { return c }
        if columns.count > 600 { columns.removeAll() }
        let c = RegionStrip.decode(model.workbench.regionMapWindow(start: start, len: len, buckets: buckets))
        columns[key] = c
        return c
    }

    private func fetchItems(start: UInt32, len: UInt32) -> AtlasItemsInfo {
        let key = "\(start):\(len)"
        if let i = items[key] { return i }
        if items.count > 200 { items.removeAll() }
        let i = model.workbench.atlasItems(start: start, len: len, limit: 8000)
        items[key] = i
        return i
    }

    /// Every call, bucketed to columns about a pixel wide at this zoom.
    private func allArcs() -> (bytes: UInt32, buckets: UInt32, list: [CallArcInfo]) {
        let bytes = max(1, UInt32(1 / ppb))
        if let arcs, arcs.bytes == bytes { return arcs }
        let buckets = max(1, romLength / bytes)
        let a = (bytes, buckets, model.workbench.atlasCallArcs(start: 0, len: romLength, buckets: buckets))
        arcs = a
        return a
    }

    // MARK: Drawing

    override func draw(_ dirtyRect: NSRect) {
        NSColor.underPageBackgroundColor.setFill()
        bounds.fill()
        guard ppb > 0 else { return }
        let labels: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: min(11, max(8, rowHeight - 2)), weight: .regular),
            .foregroundColor: NSColor.secondaryLabelColor,
        ]
        let b0 = UInt32(max(0, floor(origin.x / ppb)))
        let showItems = ppb >= itemZoom
        for row in 0..<rows {
            let y = rowY(row)
            guard y + rowHeight >= 0, y <= bounds.height else { continue }
            let rowStart = UInt32(row) * bankSize
            let rowLen = min(bankSize, romLength - rowStart)
            let end = min(rowLen, UInt32(ceil((origin.x + mapWidth) / ppb)))
            // A label every row when there is room, else every fourth.
            if rowHeight >= 8 || row % 4 == 0 {
                let bank = model.rom.snesAddressFor(fileOffset: rowStart).map { String(format: "$%02X", $0 >> 16) }
                    ?? String(format: "%06X", rowStart)
                (bank as NSString).draw(at: CGPoint(x: 6, y: y + max(0, rowHeight - 13) / 2), withAttributes: labels)
            }
            guard b0 < end else { continue }
            NSGraphicsContext.saveGraphicsState()
            NSRect(x: gutter, y: y, width: mapWidth, height: rowHeight).clip()
            let len = end - b0
            let buckets = max(1, min(len, UInt32(ceil(CGFloat(len) * ppb))))
            for c in fetchColumns(start: rowStart + b0, len: len, buckets: buckets) {
                let r = NSRect(
                    x: x(ofByte: c.start - rowStart), y: y,
                    width: max(CGFloat(c.len) * ppb, 1), height: rowHeight
                )
                paint(c, in: r)
            }
            if showItems {
                drawItems(fetchItems(start: rowStart + b0, len: len), rowStart: rowStart, y: y)
            }
            NSGraphicsContext.restoreGraphicsState()
        }
        drawSelection()
        drawArcs()
    }

    private func paint(_ c: StripColumn, in r: NSRect) {
        switch overlay {
        case .kind:
            RegionStrip.color(forKindCode: c.kindCode).withAlphaComponent(0.35 + 0.65 * c.confidence).setFill()
            r.fill()
            if c.mixed { hatch(r) }
        case .confidence:
            guard c.kindCode != 0 else { return }
            NSColor(hue: 0.33 * c.confidence, saturation: 0.65, brightness: 0.85, alpha: 1).setFill()
            r.fill()
        case .entropy:
            let e = min(1, c.entropy / 8)
            NSColor(hue: 0.66 - 0.5 * e, saturation: 0.8, brightness: 0.35 + 0.6 * e, alpha: 1).setFill()
            r.fill()
        case .coverage:
            if c.executed > 0 {
                NSColor.controlAccentColor.withAlphaComponent(0.3 + 0.7 * c.executed).setFill()
            } else {
                NSColor.tertiaryLabelColor.withAlphaComponent(c.kindCode == 1 ? 0.35 : 0.12).setFill()
            }
            r.fill()
        }
    }

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

    /// Each instruction and data row outlined, so single items read as
    /// such; instructions get a rounded box.
    private func drawItems(_ found: AtlasItemsInfo, rowStart: UInt32, y: CGFloat) {
        NSColor.underPageBackgroundColor.withAlphaComponent(0.85).setStroke()
        for i in found.items {
            let r = NSRect(
                x: x(ofByte: i.offset - rowStart), y: y,
                width: CGFloat(i.len) * ppb, height: rowHeight
            ).insetBy(dx: 0.5, dy: 0.5)
            let path = i.instruction
                ? NSBezierPath(roundedRect: r, xRadius: min(3, r.width / 4), yRadius: min(3, r.height / 4))
                : NSBezierPath(rect: r)
            path.lineWidth = 1
            path.stroke()
        }
    }

    private func drawSelection() {
        guard let s = selection, s < romLength else { return }
        let p = point(ofOffset: s)
        guard p.x >= gutter - 2 else { return }
        let y = rowY(Int(s / bankSize))
        let r = NSRect(x: p.x - max(ppb, 2) / 2, y: y - 1, width: max(ppb, 2), height: rowHeight + 2)
        NSColor.labelColor.setStroke()
        let path = NSBezierPath(rect: r)
        path.lineWidth = 1.5
        path.stroke()
    }

    /// The calls into and out of the columns near the pointer: out in the
    /// accent colour, in in orange, thicker for more calls.
    private func drawArcs() {
        guard let hover, let under = offset(at: hover) else { return }
        let a = allArcs()
        let here = under / a.bytes
        let near = { (e: ArcEndInfo) -> Bool in
            if case .column(index: let c) = e { return c + 3 >= here && c <= here + 3 }
            return false
        }
        let offsetOf = { (e: ArcEndInfo) -> UInt32? in
            guard case .column(index: let c) = e else { return nil }
            return UInt32(UInt64(c) * UInt64(self.romLength) / UInt64(a.buckets)) + a.bytes / 2
        }
        for arc in a.list.prefix(20000) {
            let out = near(arc.from)
            let into = near(arc.to)
            guard out || into, let from = offsetOf(arc.from), let to = offsetOf(arc.to) else { continue }
            let p0 = point(ofOffset: min(from, romLength - 1))
            let p1 = point(ofOffset: min(to, romLength - 1))
            let path = NSBezierPath()
            path.move(to: p0)
            let lift = max(24, abs(p1.x - p0.x) / 4 + abs(p1.y - p0.y) / 4)
            // Over the rows, or under them where there is no room above.
            let up = min(p0.y, p1.y) - lift
            let mid = CGPoint(x: (p0.x + p1.x) / 2, y: up > 4 ? up : max(p0.y, p1.y) + lift)
            path.curve(to: p1, controlPoint1: mid, controlPoint2: mid)
            path.lineWidth = min(4, 1 + log2(CGFloat(arc.calls)))
            (out ? NSColor.controlAccentColor : NSColor.systemOrange).withAlphaComponent(0.85).setStroke()
            path.stroke()
            (out ? NSColor.systemOrange : NSColor.controlAccentColor).setFill()
            NSBezierPath(ovalIn: NSRect(x: (out ? p1 : p0).x - 2.5, y: (out ? p1 : p0).y - 2.5, width: 5, height: 5)).fill()
        }
    }

    // MARK: Status

    func showStatus() {
        let zoom = ppb >= 1
            ? String(format: "%.0f px a byte", ppb)
            : String(format: "%.0f bytes a pixel", 1 / max(ppb, 1e-6))
        guard let hover, let off = offset(at: hover) else {
            statusText = "\(rows) banks, \(zoom). Pinch or ⌘-scroll to zoom, scroll to move, double-click to open the listing."
            return
        }
        var parts = [formatFileOffset(offset: off)]
        if let a = model.rom.snesAddressFor(fileOffset: off) {
            parts.append(formatSnesAddress(address: a))
        }
        let rowStart = off / bankSize * bankSize
        let b0 = UInt32(max(0, floor(origin.x / ppb)))
        let end = min(min(bankSize, romLength - rowStart), UInt32(ceil((origin.x + mapWidth) / ppb)))
        if ppb >= itemZoom, b0 < end,
           let i = fetchItems(start: rowStart + b0, len: end - b0).items.last(where: { $0.offset <= off }),
           off < i.offset + i.len {
            parts.append("\(i.instruction ? "instruction" : "data row"), \(i.len) byte\(i.len == 1 ? "" : "s")")
            parts.append("\(AtlasCanvasView.kindName(i.kindCode)) \(Int((i.confidence * 100).rounded()))%")
        } else if b0 < end {
            let len = end - b0
            let buckets = max(1, min(len, UInt32(ceil(CGFloat(len) * ppb))))
            if let c = fetchColumns(start: rowStart + b0, len: len, buckets: buckets)
                .first(where: { $0.start <= off && off < $0.end }) {
                parts.append(
                    "\(c.mixed ? "mostly " : "")\(AtlasCanvasView.kindName(c.kindCode)) \(Int((c.confidence * 100).rounded()))%"
                )
                parts.append(String(format: "%.1f bits/byte", c.entropy))
                if c.executed > 0 { parts.append(String(format: "%.0f%% run", c.executed * 100)) }
            }
        }
        parts.append(zoom)
        statusText = parts.joined(separator: "  ·  ")
    }

    static func kindName(_ code: UInt8) -> String {
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
        default: "struct"
        }
    }

    // MARK: Events

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self
        ))
    }

    override func mouseMoved(with event: NSEvent) {
        hover = convert(event.locationInWindow, from: nil)
        needsDisplay = true
        showStatus()
    }

    /// The pointer at `p`, as a test would move it.
    func mouseMovedForTesting(_ p: CGPoint) {
        hover = p
        needsDisplay = true
        showStatus()
    }

    override func mouseExited(with event: NSEvent) {
        hover = nil
        needsDisplay = true
        showStatus()
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        click(at: convert(event.locationInWindow, from: nil), count: event.clickCount)
    }

    /// A click selects the byte under it; a double-click opens the listing
    /// there.
    func click(at p: CGPoint, count: Int = 1) {
        guard let off = offset(at: p) else { return }
        if count >= 2 {
            model.editorTab = .disassembly
            model.jump(to: off)
        } else {
            selecting = true
            model.select(offset: off)
        }
    }

    override func magnify(with event: NSEvent) {
        zoom(by: 1 + event.magnification, at: convert(event.locationInWindow, from: nil))
    }

    override func scrollWheel(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        let scale: CGFloat = event.hasPreciseScrollingDeltas ? 1 : 12
        if event.modifierFlags.contains(.command) {
            zoom(by: exp(event.scrollingDeltaY * scale * 0.01), at: p)
            return
        }
        origin.x -= event.scrollingDeltaX * scale
        origin.y -= event.scrollingDeltaY * scale
        clampOrigin()
        hover = p
        needsDisplay = true
        showStatus()
    }
}

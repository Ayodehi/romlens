import AppKit

/// A tab group's bar: one tab per item, the shown one joined to the view
/// below it and, while the group has focus, edged in the accent colour
/// (docs/29). Split Right and Split Down sit at the trailing end.
@MainActor
final class TabBarView: NSView, NSDraggingSource {
    static let height: CGFloat = 30
    let model: RomViewModel
    let groupID: UUID

    struct Tab {
        let item: EditorItem
        let title: String
        var rect: NSRect = .zero
        var close: NSRect = .zero
    }

    private(set) var tabs: [Tab] = []
    /// The tab being dragged, anywhere: dimmed where it was until the drop.
    static var dragging: UUID?
    /// Where a dragged tab would be inserted in this bar, while one is over it.
    var insertion: Int? {
        didSet { if insertion != oldValue { needsDisplay = true } }
    }
    private var selected: UUID?
    private var focused = false
    private var hovered: UUID?
    private let splitRight = NSButton()
    private let splitDown = NSButton()
    private var tracking: NSTrackingArea?

    private static let font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize + 1)
    private static let iconSize: CGFloat = 14
    private static let padding: CGFloat = 10
    private static let closeSize: CGFloat = 14
    private static let maxWidth: CGFloat = 220
    private static let minWidth: CGFloat = 84

    init(model: RomViewModel, groupID: UUID) {
        self.model = model
        self.groupID = groupID
        super.init(frame: .zero)
        for (button, symbol, help, edge) in [
            (splitRight, "square.split.2x1", "Split Right (⌘\\)", DropEdge.right),
            (splitDown, "square.split.1x2", "Split Down (⌥⌘\\)", DropEdge.bottom),
        ] {
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: help)
            button.isBordered = false
            button.bezelStyle = .accessoryBarAction
            button.toolTip = help
            button.setAccessibilityLabel(help)
            button.contentTintColor = .secondaryLabelColor
            button.target = self
            button.action = edge == .right ? #selector(doSplitRight) : #selector(doSplitDown)
            button.translatesAutoresizingMaskIntoConstraints = false
            addSubview(button)
        }
        NSLayoutConstraint.activate([
            splitDown.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            splitDown.centerYAnchor.constraint(equalTo: centerYAnchor),
            splitRight.trailingAnchor.constraint(equalTo: splitDown.leadingAnchor, constant: -2),
            splitRight.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        setAccessibilityRole(.tabGroup)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var isFlipped: Bool { true }

    func update(items: [EditorItem], selected: UUID?, focused: Bool, titles: [UUID: String]) {
        tabs = items.map { Tab(item: $0, title: titles[$0.id] ?? model.title(of: $0)) }
        self.selected = selected
        self.focused = focused
        layoutTabs()
        needsDisplay = true
    }

    override func layout() {
        super.layout()
        layoutTabs()
    }

    private var trailingReserve: CGFloat { 64 }

    private func layoutTabs() {
        var x: CGFloat = 0
        let attrs: [NSAttributedString.Key: Any] = [.font: Self.font]
        for i in tabs.indices {
            let text = (tabs[i].title as NSString).size(withAttributes: attrs).width
            let w = min(Self.maxWidth, max(Self.minWidth, text + Self.iconSize + Self.closeSize + 4 * Self.padding))
            tabs[i].rect = NSRect(x: x, y: 0, width: w, height: Self.height)
            tabs[i].close = NSRect(x: x + w - Self.padding - Self.closeSize, y: (Self.height - Self.closeSize) / 2, width: Self.closeSize, height: Self.closeSize)
            x += w
        }
    }

    // MARK: Drawing

    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        bounds.fill()
        for tab in tabs { draw(tab) }
        NSColor.separatorColor.setFill()
        NSRect(x: 0, y: bounds.height - 1, width: bounds.width, height: 1).fill()
        if let insertion {
            let x = insertion < tabs.count ? tabs[insertion].rect.minX : (tabs.last?.rect.maxX ?? 0)
            NSColor.controlAccentColor.setFill()
            NSRect(x: max(0, x - 1), y: 3, width: 2, height: bounds.height - 6).fill()
        }
    }

    private func draw(_ tab: Tab) {
        let isSelected = tab.item.id == selected
        let r = tab.rect
        if Self.dragging == tab.item.id {
            // Where the dragged tab was: an outline until the drop.
            NSColor.separatorColor.setStroke()
            let outline = NSBezierPath(roundedRect: r.insetBy(dx: 2, dy: 4), xRadius: 4, yRadius: 4)
            outline.lineWidth = 1
            outline.stroke()
            return
        }
        if isSelected {
            NSColor.textBackgroundColor.setFill()
            r.fill()
            (focused ? NSColor.controlAccentColor : NSColor.separatorColor).setFill()
            NSRect(x: r.minX, y: 0, width: r.width, height: 2).fill()
        }
        NSColor.separatorColor.setFill()
        NSRect(x: r.maxX - 1, y: 6, width: 1, height: r.height - 12).fill()

        let colour: NSColor = isSelected ? .labelColor : .secondaryLabelColor
        var x = r.minX + Self.padding
        if let icon = NSImage(systemSymbolName: tab.item.content.symbol, accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 11, weight: .regular)) {
            let tinted = icon.tinted(isSelected ? tab.item.content.tint : .secondaryLabelColor)
            let size = tinted.size
            tinted.draw(in: NSRect(x: x, y: (r.height - size.height) / 2, width: size.width, height: size.height))
            x += Self.iconSize + 6
        }
        let attrs: [NSAttributedString.Key: Any] = [.font: Self.font, .foregroundColor: colour]
        let title = tab.title as NSString
        let textWidth = tab.close.minX - 4 - x
        let size = title.size(withAttributes: attrs)
        title.draw(
            with: NSRect(x: x, y: (r.height - size.height) / 2, width: max(0, textWidth), height: size.height),
            options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine],
            attributes: attrs
        )
        if isSelected || hovered == tab.item.id,
           let close = NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close Tab")?
            .withSymbolConfiguration(.init(pointSize: 9, weight: .semibold)) {
            let tinted = close.tinted(.secondaryLabelColor)
            let s = tinted.size
            tinted.draw(in: NSRect(x: tab.close.midX - s.width / 2, y: tab.close.midY - s.height / 2, width: s.width, height: s.height))
        }
    }

    // MARK: Mouse

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let t = NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(t)
        tracking = t
    }

    override func mouseMoved(with event: NSEvent) {
        let id = tab(at: convert(event.locationInWindow, from: nil))?.item.id
        if id != hovered {
            hovered = id
            needsDisplay = true
        }
    }

    override func mouseExited(with event: NSEvent) {
        hovered = nil
        needsDisplay = true
    }

    func tab(at point: NSPoint) -> Tab? { tabs.first { $0.rect.contains(point) } }

    override func mouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        guard let tab = tab(at: p) else { return }
        if tab.close.insetBy(dx: -3, dy: -3).contains(p) {
            close(tab.item.id)
            return
        }
        model.focus(item: tab.item.id)
        mouseDownTab = tab.item.id
        mouseDownPoint = p
    }

    /// For dragging (W4): where a press on a tab began.
    var mouseDownTab: UUID?
    var mouseDownPoint: NSPoint = .zero

    /// The index a tab dropped at `x` goes to: before the first tab whose
    /// middle is right of it.
    func insertionIndex(at x: CGFloat) -> Int {
        tabs.firstIndex { $0.rect.midX > x } ?? tabs.count
    }

    override func mouseDragged(with event: NSEvent) {
        guard let id = mouseDownTab, let tab = tabs.first(where: { $0.item.id == id }) else { return }
        let p = convert(event.locationInWindow, from: nil)
        guard hypot(p.x - mouseDownPoint.x, p.y - mouseDownPoint.y) > 4 else { return }
        mouseDownTab = nil
        guard let data = try? JSONEncoder().encode(TabDrop.item(id)) else { return }
        let pasteboardItem = NSPasteboardItem()
        pasteboardItem.setData(data, forType: NSPasteboard.PasteboardType(TabDrop.pasteboardType))
        let dragItem = NSDraggingItem(pasteboardWriter: pasteboardItem)
        dragItem.setDraggingFrame(tab.rect, contents: image(of: tab))
        Self.dragging = id
        needsDisplay = true
        beginDraggingSession(with: [dragItem], event: event, source: self)
    }

    override func mouseUp(with event: NSEvent) {
        mouseDownTab = nil
    }

    /// The tab as it looks, for the drag.
    private func image(of tab: Tab) -> NSImage {
        let saved = Self.dragging
        Self.dragging = nil
        defer { Self.dragging = saved }
        guard let rep = bitmapImageRepForCachingDisplay(in: tab.rect) else { return NSImage(size: tab.rect.size) }
        cacheDisplay(in: tab.rect, to: rep)
        let image = NSImage(size: tab.rect.size)
        image.addRepresentation(rep)
        return image
    }

    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        context == .withinApplication ? .move : []
    }

    func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
        // Dropped, or cancelled with Esc: either way the outline goes.
        Self.dragging = nil
        window?.contentView.map(Self.redrawAll)
    }

    static func redrawAll(in view: NSView) {
        if let bar = view as? TabBarView {
            bar.insertion = nil
            bar.needsDisplay = true
        }
        view.subviews.forEach(redrawAll)
    }

    override func otherMouseDown(with event: NSEvent) {
        // A middle click closes, as in every tabbed editor.
        if let tab = tab(at: convert(event.locationInWindow, from: nil)) { close(tab.item.id) }
    }

    private func close(_ id: UUID) {
        model.close(item: id)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        guard let tab = tab(at: convert(event.locationInWindow, from: nil)) else { return nil }
        let id = tab.item.id
        model.focus(item: id)
        let menu = NSMenu()
        func add(_ title: String, state: Bool? = nil, _ action: @escaping () -> Void) {
            let item = ClosureMenuItem(title: title, action: #selector(ClosureMenuItem.fire), keyEquivalent: "")
            item.target = item
            item.handler = action
            if let state { item.state = state ? .on : .off }
            menu.addItem(item)
        }
        add("Close Tab") { [weak self] in self?.close(id) }
        add("Close Other Tabs") { [weak self] in
            guard let self else { return }
            for other in tabs where other.item.id != id { model.close(item: other.item.id) }
        }
        menu.addItem(.separator())
        add("Split Right") { [weak self] in self?.model.splitFocused(.right) }
        add("Split Down") { [weak self] in self?.model.splitFocused(.bottom) }
        menu.addItem(.separator())
        add("Follow Selection", state: tab.item.followsSelection) { [weak self] in
            self?.model.workspace.setFollowsSelection(!tab.item.followsSelection, of: id)
        }
        return menu
    }

    @objc private func doSplitRight() {
        model.focusGroup(groupID)
        model.splitFocused(.right)
    }

    @objc private func doSplitDown() {
        model.focusGroup(groupID)
        model.splitFocused(.bottom)
    }

    // MARK: Accessibility

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityLabel() -> String? {
        tabs.map(\.title).joined(separator: ", ")
    }
}

extension EditorContent {
    /// The tab's icon.
    var symbol: String {
        switch self {
        case .code(.assembly): "text.alignleft"
        case .code(.c): "chevron.left.forwardslash.chevron.right"
        case .code(.graph): "point.3.connected.trianglepath.dotted"
        case .code(.hex): "number"
        case .code(.both): "rectangle.split.2x1"
        case .atlas: "square.grid.2x2"
        case .compare: "rectangle.on.rectangle"
        case .source: "doc.text"
        case .graphics(let t): t.systemImage
        case .audio(let t): t.systemImage
        case .tutor: "graduationcap"
        }
    }

    /// The icon's colour in the shown tab: code blue, data orange (the
    /// region colours), the tutor in the accent colour.
    var tint: NSColor {
        switch self {
        case .code, .source, .compare: .systemBlue
        case .graphics, .audio, .atlas: .systemOrange
        case .tutor: .controlAccentColor
        }
    }
}

extension NSImage {
    /// A template symbol drawn in one colour.
    func tinted(_ colour: NSColor) -> NSImage {
        let image = NSImage(size: size, flipped: false) { rect in
            self.draw(in: rect)
            colour.set()
            rect.fill(using: .sourceAtop)
            return true
        }
        image.isTemplate = false
        return image
    }
}

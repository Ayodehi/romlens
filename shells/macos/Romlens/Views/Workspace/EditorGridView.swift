import AppKit
import SwiftUI

/// The editor area: the workspace's tab groups as nested split views
/// (docs/29).
///
/// AppKit rather than SwiftUI: tab dragging needs the dragging session's
/// location as it moves and Esc to cancel, and SwiftUI split views have
/// already cost a day here (`61a0b72`). Each tab's view is made once and
/// kept while the tab exists, so its scroll and zoom survive switching
/// tabs and moving it to another group.
struct EditorGridView: NSViewRepresentable {
    let model: RomViewModel

    func makeNSView(context: Context) -> EditorGridNSView {
        EditorGridNSView(model: model)
    }

    func updateNSView(_ view: EditorGridNSView, context: Context) {
        // Everything read here is observed, so a change to the layout, the
        // focus or a title calls this again.
        let layout = model.workspace.layout
        let focused = model.workspace.focusedGroup
        let titles = Dictionary(uniqueKeysWithValues: layout.items.map { ($0.id, model.title(of: $0)) })
        view.sync(layout: layout, focused: focused, titles: titles)
    }
}

@MainActor
final class EditorGridNSView: NSView {
    let model: RomViewModel
    private(set) var groupViews: [UUID: TabGroupView] = [:]
    private var hosts: [UUID: NSView] = [:]
    private var splits: [UUID: GridSplitView] = [:]
    private var signature = ""
    private var rootView: NSView?
    private var monitor: Any?

    private let overlay = DropOverlayView()

    init(model: RomViewModel) {
        self.model = model
        super.init(frame: .zero)
        registerForDraggedTypes([NSPasteboard.PasteboardType(TabDrop.pasteboardType)])
        overlay.isHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var isFlipped: Bool { true }

    /// Brings the views in line with the layout: the split tree rebuilt only
    /// when its shape changed, the tab bars, the shown tab and the fractions
    /// every time.
    func sync(layout: EditorLayout, focused: UUID, titles: [UUID: String]) {
        let shape = Self.signature(layout.root)
        if shape != signature {
            signature = shape
            rebuild(layout.root)
        }
        for group in layout.groups {
            groupViews[group.id]?.update(group: group, focused: group.id == focused, titles: titles) { [unowned self] item in
                host(for: item)
            }
        }
        applyFractions(layout.root)
        let live = Set(layout.items.map(\.id))
        for id in hosts.keys where !live.contains(id) {
            hosts[id]?.removeFromSuperview()
            hosts[id] = nil
        }
        let liveGroups = Set(layout.groups.map(\.id))
        groupViews = groupViews.filter { liveGroups.contains($0.key) }
    }

    /// The view showing a tab, made the first time it is asked for.
    private func host(for item: EditorItem) -> NSView {
        if let h = hosts[item.id] { return h }
        let host = NSHostingView(rootView: EditorItemBody(model: model, itemID: item.id))
        host.sizingOptions = []
        host.translatesAutoresizingMaskIntoConstraints = true
        host.autoresizingMask = [.width, .height]
        hosts[item.id] = host
        return host
    }

    private static func signature(_ node: LayoutNode) -> String {
        switch node {
        case .group(let g): "g\(g.id.uuidString)"
        case .split(let s): "s\(s.id.uuidString)\(s.axis.rawValue)[\(s.children.map(signature).joined(separator: ","))]"
        }
    }

    private func rebuild(_ root: LayoutNode) {
        rootView?.removeFromSuperview()
        splits = [:]
        let view = build(root)
        view.frame = bounds
        view.autoresizingMask = [.width, .height]
        addSubview(view, positioned: .below, relativeTo: nil)
        rootView = view
        if overlay.superview == nil { addSubview(overlay) }
    }

    private func build(_ node: LayoutNode) -> NSView {
        switch node {
        case .group(let g):
            let view = groupViews[g.id] ?? TabGroupView(model: model, groupID: g.id)
            view.removeFromSuperview()
            groupViews[g.id] = view
            return view
        case .split(let s):
            let split = GridSplitView(splitID: s.id, axis: s.axis)
            split.onFractions = { [weak self] f in self?.model.workspace.setFractions(s.id, f) }
            split.onEqualize = { [weak self] in self?.model.workspace.equalize(s.id) }
            for child in s.children {
                split.addArrangedSubview(build(child))
            }
            splits[s.id] = split
            return split
        }
    }

    private func applyFractions(_ node: LayoutNode) {
        guard case .split(let s) = node else { return }
        splits[s.id]?.set(fractions: s.fractions)
        s.children.forEach(applyFractions)
    }

    // MARK: Focus follows the click

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        guard window != nil else { return }
        // A click anywhere in a group, in its tab bar or in its view, gives
        // the group focus; the event goes on to the view as usual.
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self] event in
            self?.focusGroup(under: event)
            return event
        }
    }

    private func focusGroup(under event: NSEvent) {
        guard event.window === window else { return }
        let point = convert(event.locationInWindow, from: nil)
        guard bounds.contains(point) else { return }
        for (id, view) in groupViews where view.window != nil {
            if view.convert(view.bounds, to: self).contains(point) {
                if model.workspace.focusedGroup != id { model.focusGroup(id) }
                return
            }
        }
    }

    // MARK: Dropping a tab (docs/29, W4)

    /// Where a drag at `point` would land, and the rect to preview it in.
    func dropTarget(at point: NSPoint) -> (group: TabGroupView, target: DropTarget, preview: NSRect)? {
        guard let group = group(at: point) else { return nil }
        let bar = group.tabBar.convert(group.tabBar.bounds, to: self)
        if bar.contains(point) {
            let local = group.tabBar.convert(point, from: self)
            return (group, .tabBar(index: group.tabBar.insertionIndex(at: local.x)), .zero)
        }
        let content = group.content.convert(group.content.bounds, to: self)
        // This view is flipped, as `dropZone` expects: the top is minY.
        let zone = dropZone(point, in: content)
        let preview: NSRect = switch zone {
        case .center: content
        case .edge(.left): NSRect(x: content.minX, y: content.minY, width: content.width / 2, height: content.height)
        case .edge(.right): NSRect(x: content.midX, y: content.minY, width: content.width / 2, height: content.height)
        case .edge(.top): NSRect(x: content.minX, y: content.minY, width: content.width, height: content.height / 2)
        case .edge(.bottom): NSRect(x: content.minX, y: content.midY, width: content.width, height: content.height / 2)
        }
        return (group, .zone(zone), preview.insetBy(dx: 4, dy: 4))
    }

    private func payload(_ info: NSDraggingInfo) -> TabDrop? {
        guard let data = info.draggingPasteboard.data(forType: NSPasteboard.PasteboardType(TabDrop.pasteboardType)) else { return nil }
        return try? JSONDecoder().decode(TabDrop.self, from: data)
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        draggingUpdated(sender)
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard payload(sender) != nil, let found = dropTarget(at: convert(sender.draggingLocation, from: nil)) else {
            clearPreview()
            return []
        }
        for g in groupViews.values where g !== found.group { g.tabBar.insertion = nil }
        switch found.target {
        case .tabBar(let index):
            found.group.tabBar.insertion = index
            overlay.isHidden = true
        case .zone:
            found.group.tabBar.insertion = nil
            overlay.frame = found.preview
            overlay.isHidden = false
        }
        return .move
    }

    override func draggingExited(_ sender: NSDraggingInfo?) {
        clearPreview()
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        defer { clearPreview() }
        guard let drop = payload(sender), let found = dropTarget(at: convert(sender.draggingLocation, from: nil)) else { return false }
        model.drop(drop, on: found.group.groupID, at: found.target)
        return true
    }

    override func concludeDragOperation(_ sender: NSDraggingInfo?) {
        clearPreview()
    }

    private func clearPreview() {
        overlay.isHidden = true
        for g in groupViews.values { g.tabBar.insertion = nil }
    }

    /// The group at a point in this view's coordinates, for drops.
    func group(at point: NSPoint) -> TabGroupView? {
        groupViews.values.first { $0.window != nil && $0.convert($0.bounds, to: self).contains(point) }
    }
}

/// The blue area that shows where a dragged tab will go. It takes no
/// clicks or drags itself.
final class DropOverlayView: NSView {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 5
        layer?.borderWidth = 1
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override func updateLayer() {
        layer?.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.18).cgColor
        layer?.borderColor = NSColor.controlAccentColor.cgColor
    }

    override var wantsUpdateLayer: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// One split of the grid. Divider drags write the fractions back to the
/// layout; a double-click on a divider makes the panes equal.
@MainActor
final class GridSplitView: NSSplitView, NSSplitViewDelegate {
    let splitID: UUID
    var onFractions: (([Double]) -> Void)?
    var onEqualize: (() -> Void)?
    private var fractions: [Double] = []
    private var pending = false
    private var applying = false
    private var dragging = false
    static let minimumPane: CGFloat = 160

    init(splitID: UUID, axis: SplitAxis) {
        self.splitID = splitID
        super.init(frame: .zero)
        isVertical = axis == .horizontal
        dividerStyle = .thin
        delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    func set(fractions f: [Double]) {
        guard f != fractions else { return }
        fractions = f
        pending = true
        needsLayout = true
    }

    override func layout() {
        super.layout()
        if pending { apply() }
    }

    private var length: CGFloat { isVertical ? bounds.width : bounds.height }

    private func apply() {
        let n = arrangedSubviews.count
        guard n > 1, fractions.count == n, length > 0 else { return }
        applying = true
        defer { applying = false }
        let usable = length - dividerThickness * CGFloat(n - 1)
        var position: CGFloat = 0
        for i in 0..<(n - 1) {
            position += usable * fractions[i]
            setPosition(position + CGFloat(i) * dividerThickness, ofDividerAt: i)
        }
        pending = false
    }

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2, divider(at: convert(event.locationInWindow, from: nil)) != nil {
            onEqualize?()
            return
        }
        dragging = true
        super.mouseDown(with: event)
        dragging = false
    }

    private func divider(at point: NSPoint) -> Int? {
        let views = arrangedSubviews
        guard views.count > 1 else { return nil }
        for i in 0..<(views.count - 1) {
            let a = views[i].frame
            let slop: CGFloat = 3
            let rect = isVertical
                ? NSRect(x: a.maxX - slop, y: 0, width: dividerThickness + 2 * slop, height: bounds.height)
                : NSRect(x: 0, y: a.maxY - slop, width: bounds.width, height: dividerThickness + 2 * slop)
            if rect.contains(point) { return i }
        }
        return nil
    }

    func splitViewDidResizeSubviews(_ notification: Notification) {
        guard dragging, !applying, length > 0 else { return }
        let sizes = arrangedSubviews.map { isVertical ? $0.frame.width : $0.frame.height }
        let total = sizes.reduce(0, +)
        guard total > 0 else { return }
        let f = sizes.map { Double($0 / total) }
        fractions = f
        onFractions?(f)
    }

    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposed: CGFloat, ofSubviewAt index: Int) -> CGFloat {
        let a = arrangedSubviews[index].frame
        return (isVertical ? a.minX : a.minY) + Self.minimumPane
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposed: CGFloat, ofSubviewAt index: Int) -> CGFloat {
        let b = arrangedSubviews[index + 1].frame
        return (isVertical ? b.maxX : b.maxY) - Self.minimumPane - dividerThickness
    }
}

/// A tab group: its tab bar, then the shown tab's view, or a note saying
/// how to open one when the group is empty.
@MainActor
final class TabGroupView: NSView {
    let model: RomViewModel
    let groupID: UUID
    let tabBar: TabBarView
    let content = NSView()
    private let empty = NSTextField(wrappingLabelWithString: "Choose a view in the sidebar or the View menu, or drag a tab here.")
    private(set) var items: [EditorItem] = []

    init(model: RomViewModel, groupID: UUID) {
        self.model = model
        self.groupID = groupID
        tabBar = TabBarView(model: model, groupID: groupID)
        super.init(frame: .zero)
        empty.alignment = .center
        empty.textColor = .secondaryLabelColor
        empty.isHidden = true
        for v in [tabBar, content, empty] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            tabBar.topAnchor.constraint(equalTo: topAnchor),
            tabBar.leadingAnchor.constraint(equalTo: leadingAnchor),
            tabBar.trailingAnchor.constraint(equalTo: trailingAnchor),
            tabBar.heightAnchor.constraint(equalToConstant: TabBarView.height),
            content.topAnchor.constraint(equalTo: tabBar.bottomAnchor),
            content.leadingAnchor.constraint(equalTo: leadingAnchor),
            content.trailingAnchor.constraint(equalTo: trailingAnchor),
            content.bottomAnchor.constraint(equalTo: bottomAnchor),
            empty.centerYAnchor.constraint(equalTo: content.centerYAnchor),
            empty.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 24),
            empty.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -24),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    func update(group: TabGroup, focused: Bool, titles: [UUID: String], host: (EditorItem) -> NSView) {
        items = group.items
        tabBar.update(items: group.items, selected: group.selected, focused: focused, titles: titles)
        empty.isHidden = !group.items.isEmpty
        for item in group.items {
            let view = host(item)
            if view.superview !== content {
                view.removeFromSuperview()
                view.frame = content.bounds
                content.addSubview(view)
            }
            view.isHidden = item.id != group.selected
        }
        // A tab moved away takes its view with it: anything left here that
        // is not one of this group's tabs belongs elsewhere now.
        let ids = Set(group.items.map(\.id))
        for view in content.subviews {
            if let tagged = (view as? EditorItemHosting)?.itemID, !ids.contains(tagged) { view.removeFromSuperview() }
        }
    }
}

/// A tab's hosting view, which knows its tab.
@MainActor
protocol EditorItemHosting { var itemID: UUID { get } }
extension NSHostingView: EditorItemHosting where Content == EditorItemBody {
    var itemID: UUID { rootView.itemID }
}

/// What a tab shows: for code, the strip of representations, then the view.
struct EditorItemBody: View {
    let model: RomViewModel
    let itemID: UUID

    var body: some View {
        if let item = model.workspace.layout.item(itemID) {
            VStack(spacing: 0) {
                if case .code(let r) = item.content {
                    RepresentationStrip(model: model, item: item, current: r)
                    Divider()
                }
                EditorView(model: model, content: item.content)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .environment(\.editorItem, itemID)
        }
    }
}

/// The representations a code tab offers, as text tabs, and whether it
/// follows the selection.
struct RepresentationStrip: View {
    let model: RomViewModel
    let item: EditorItem
    let current: CodeRepresentation

    var body: some View {
        HStack(spacing: 16) {
            ForEach(CodeRepresentation.allCases) { r in
                Button {
                    model.setRepresentation(r, of: item.id)
                } label: {
                    Text(r.title)
                        .foregroundStyle(r == current ? .primary : .secondary)
                        .padding(.vertical, 5)
                        .overlay(alignment: .bottom) {
                            if r == current {
                                Rectangle().fill(Color.accentColor).frame(height: 2)
                            }
                        }
                }
                .buttonStyle(.plain)
                .help(r.help)
            }
            Spacer(minLength: 8)
            Button {
                model.workspace.setFollowsSelection(!item.followsSelection, of: item.id)
            } label: {
                Image(systemName: item.followsSelection ? "link" : "link.badge.plus")
                    .foregroundStyle(item.followsSelection ? Color.accentColor : .secondary)
            }
            .buttonStyle(.borderless)
            .help(item.followsSelection
                ? "Follows the selection made in other tabs. Click to keep this tab where it is."
                : "Stays where it is. Click to follow the selection made in other tabs.")
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .frame(height: 28)
    }
}

extension CodeRepresentation {
    var help: String {
        switch self {
        case .assembly: "The disassembly (⌥⌘2)"
        case .c: "The routine as C (⌥⌘8)"
        case .graph: "The routine as a graph (⌥⌘9)"
        case .hex: "The bytes (⌥⌘1)"
        case .both: "Hex and disassembly side by side (⌥⌘3)"
        }
    }
}

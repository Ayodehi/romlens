import CoreGraphics
import Foundation

/// How a code tab shows its routine (docs/29).
enum CodeRepresentation: String, Codable, CaseIterable, Identifiable, Sendable {
    case assembly, c, graph, hex, both
    var id: String { rawValue }
    var title: String {
        switch self {
        case .assembly: "Assembly"
        case .c: "C"
        case .graph: "Graph"
        case .hex: "Hex"
        case .both: "Both"
        }
    }
}

/// What a tab shows. Code may be open in several tabs; everything else has
/// at most one tab in a window, because the recording, the player and the
/// comparison behind it are the document's (docs/29, scope decisions).
enum EditorContent: Hashable, Codable, Sendable {
    case code(CodeRepresentation)
    case atlas
    case compare
    case source
    case graphics(GraphicsModel.Tab)
    case audio(AudioModel.Tab)
    case tutor

    /// Whether a window may hold only one tab of this.
    var isSingleton: Bool {
        if case .code = self { return false }
        return true
    }

    /// Two contents with the same key are the same view for the
    /// one-tab-per-window rule.
    var singletonKey: String {
        switch self {
        case .code(let r): "code.\(r.rawValue)"
        case .atlas: "atlas"
        case .compare: "compare"
        case .source: "source"
        case .graphics(let t): "graphics.\(t.rawValue)"
        case .audio(let t): "audio.\(t.rawValue)"
        case .tutor: "tutor"
        }
    }
}

extension GraphicsModel.Tab: Codable {}
extension AudioModel.Tab: Codable {}

/// One tab.
struct EditorItem: Identifiable, Hashable, Codable, Sendable {
    let id: UUID
    var content: EditorContent
    /// Whether the tab scrolls to the selection when another tab moves it.
    var followsSelection: Bool

    init(id: UUID = UUID(), _ content: EditorContent, followsSelection: Bool = true) {
        self.id = id
        self.content = content
        self.followsSelection = followsSelection
    }
}

/// A group of tabs, one of them shown.
struct TabGroup: Identifiable, Hashable, Codable, Sendable {
    let id: UUID
    var items: [EditorItem]
    var selected: UUID?

    init(id: UUID = UUID(), items: [EditorItem] = [], selected: UUID? = nil) {
        self.id = id
        self.items = items
        self.selected = selected ?? items.first?.id
    }

    var selectedItem: EditorItem? { items.first { $0.id == selected } }
}

/// `horizontal` lays children side by side, `vertical` stacks them.
enum SplitAxis: String, Codable, Sendable {
    case horizontal, vertical
}

struct LayoutSplit: Identifiable, Hashable, Codable, Sendable {
    let id: UUID
    var axis: SplitAxis
    var children: [LayoutNode]
    /// One per child, summing to 1.
    var fractions: [Double]

    init(id: UUID = UUID(), axis: SplitAxis, children: [LayoutNode], fractions: [Double]? = nil) {
        self.id = id
        self.axis = axis
        self.children = children
        self.fractions = fractions ?? Array(repeating: 1 / Double(max(1, children.count)), count: children.count)
    }
}

indirect enum LayoutNode: Hashable, Codable, Sendable {
    case group(TabGroup)
    case split(LayoutSplit)
}

/// Where a tab dropped on a group goes.
enum DropEdge: Sendable { case left, right, top, bottom }

enum DropZone: Equatable, Sendable {
    case center
    case edge(DropEdge)
}

/// Where a point in a group's content falls, in a rect whose y grows
/// downward (top is `minY`): the outer third of a side splits there, the
/// middle moves the tab into the group, and in a corner the nearer edge,
/// measured as a share of the group's width or height, wins.
func dropZone(_ point: CGPoint, in rect: CGRect, edgeShare: CGFloat = 1.0 / 3) -> DropZone {
    guard rect.width > 0, rect.height > 0 else { return .center }
    let distances: [(DropEdge, CGFloat)] = [
        (.left, (point.x - rect.minX) / rect.width),
        (.right, (rect.maxX - point.x) / rect.width),
        (.top, (point.y - rect.minY) / rect.height),
        (.bottom, (rect.maxY - point.y) / rect.height),
    ]
    let nearest = distances.min { $0.1 < $1.1 }!
    return nearest.1 < edgeShare ? .edge(nearest.0) : .center
}

/// The four arrangements in View › Editor Layout.
enum LayoutPreset: String, CaseIterable, Sendable {
    case single, twoColumns, twoRows, three
    var title: String {
        switch self {
        case .single: "Single"
        case .twoColumns: "Two Columns"
        case .twoRows: "Two Rows"
        case .three: "Three"
        }
    }
}

/// The editor area: tab groups laid out as a tree of splits (docs/29).
///
/// Every change is a method here, so the drop handler, the menus and the
/// tests all go through the same code. A group emptied by moving or closing
/// its last tab goes, unless it is the only one; a split left with one child
/// is replaced by it; and a split inside another along the same axis is
/// merged into it, so the tree stays as flat as the arrangement on screen.
struct EditorLayout: Hashable, Codable, Sendable {
    var root: LayoutNode

    init(root: LayoutNode) { self.root = root }

    /// One group holding `items`.
    static func single(_ items: [EditorItem] = []) -> EditorLayout {
        EditorLayout(root: .group(TabGroup(items: items)))
    }

    // MARK: Reading

    /// Every group, left to right and top to bottom.
    var groups: [TabGroup] {
        var out: [TabGroup] = []
        func walk(_ n: LayoutNode) {
            switch n {
            case .group(let g): out.append(g)
            case .split(let s): s.children.forEach(walk)
            }
        }
        walk(root)
        return out
    }

    var items: [EditorItem] { groups.flatMap(\.items) }

    func group(_ id: UUID) -> TabGroup? { groups.first { $0.id == id } }

    func group(containing item: UUID) -> TabGroup? {
        groups.first { g in g.items.contains { $0.id == item } }
    }

    func item(_ id: UUID) -> EditorItem? { items.first { $0.id == id } }

    /// The tab already showing a view that may have only one.
    func existing(_ content: EditorContent) -> EditorItem? {
        guard content.isSingleton else { return nil }
        return items.first { $0.content.singletonKey == content.singletonKey }
    }

    func split(_ id: UUID) -> LayoutSplit? {
        func find(_ n: LayoutNode) -> LayoutSplit? {
            guard case .split(let s) = n else { return nil }
            if s.id == id { return s }
            for c in s.children { if let f = find(c) { return f } }
            return nil
        }
        return find(root)
    }

    // MARK: Changing

    /// Opens `content` in `group`, after its shown tab, and shows it. A view
    /// that may have only one tab and already has one is shown where it is
    /// instead. Returns the tab's id, or nil if `group` does not exist.
    @discardableResult
    mutating func open(_ content: EditorContent, in groupID: UUID, followsSelection: Bool = true) -> UUID? {
        if let found = existing(content) {
            select(found.id)
            return found.id
        }
        let item = EditorItem(content, followsSelection: followsSelection)
        var placed = false
        updateGroup(groupID) { g in
            let at = g.items.firstIndex { $0.id == g.selected }.map { $0 + 1 } ?? g.items.count
            g.items.insert(item, at: at)
            g.selected = item.id
            placed = true
        }
        return placed ? item.id : nil
    }

    /// Shows `item` in its group.
    mutating func select(_ itemID: UUID) {
        guard let g = group(containing: itemID) else { return }
        updateGroup(g.id) { $0.selected = itemID }
    }

    /// Moves `item` into `group` at `index` (the end if nil) and shows it.
    /// Within one group this reorders.
    mutating func move(_ itemID: UUID, to groupID: UUID, at index: Int? = nil) {
        guard let source = group(containing: itemID), let item = item(itemID), group(groupID) != nil else { return }
        if source.id == groupID {
            updateGroup(groupID) { g in
                guard let from = g.items.firstIndex(where: { $0.id == itemID }) else { return }
                g.items.remove(at: from)
                var to = index ?? g.items.count
                if to > from { to -= 1 }
                g.items.insert(item, at: min(max(0, to), g.items.count))
                g.selected = itemID
            }
            return
        }
        take(itemID)
        updateGroup(groupID) { g in
            g.items.insert(item, at: min(max(0, index ?? g.items.count), g.items.count))
            g.selected = itemID
        }
        removeIfEmpty(source.id)
    }

    /// Takes `item` out of where it is and puts it in a new group on `edge`
    /// of `group`. Returns the new group's id, or nil when there is nothing
    /// to split (the item is the target group's only tab).
    @discardableResult
    mutating func split(_ groupID: UUID, _ edge: DropEdge, with itemID: UUID) -> UUID? {
        guard let source = group(containing: itemID), let item = item(itemID), group(groupID) != nil else { return nil }
        if source.id == groupID && source.items.count == 1 { return nil }
        take(itemID)
        let fresh = TabGroup(items: [item])
        let axis: SplitAxis = (edge == .left || edge == .right) ? .horizontal : .vertical
        let before = edge == .left || edge == .top
        root = Self.insert(.group(fresh), beside: groupID, axis: axis, before: before, in: root)
        if source.id != groupID { removeIfEmpty(source.id) }
        normalize()
        return fresh.id
    }

    /// Closes `item`. Its group goes when emptied, unless it is the last.
    mutating func close(_ itemID: UUID) {
        guard let g = group(containing: itemID) else { return }
        take(itemID)
        removeIfEmpty(g.id)
    }

    /// Makes a split's children equal.
    mutating func equalize(_ splitID: UUID) {
        updateSplit(splitID) { s in
            s.fractions = Array(repeating: 1 / Double(s.children.count), count: s.children.count)
        }
    }

    /// Sets a split's fractions, as a divider drag does; they are scaled to
    /// sum to 1, and a list of the wrong length is ignored.
    mutating func setFractions(_ splitID: UUID, _ fractions: [Double]) {
        updateSplit(splitID) { s in
            let total = fractions.reduce(0, +)
            guard fractions.count == s.children.count, total > 0 else { return }
            s.fractions = fractions.map { $0 / total }
        }
    }

    /// Rearranges into a preset, keeping every tab: groups beyond those the
    /// preset has are merged into its last, and a preset with more groups
    /// than there are gets empty ones.
    mutating func apply(_ preset: LayoutPreset) {
        let count: Int = switch preset {
        case .single: 1
        case .twoColumns, .twoRows: 2
        case .three: 3
        }
        var gs = groups
        while gs.count < count { gs.append(TabGroup()) }
        if gs.count > count {
            var last = gs[count - 1]
            for extra in gs[count...] { last.items += extra.items }
            if last.selected == nil { last.selected = last.items.first?.id }
            gs = Array(gs[..<(count - 1)]) + [last]
        }
        switch preset {
        case .single:
            root = .group(gs[0])
        case .twoColumns:
            root = .split(LayoutSplit(axis: .horizontal, children: gs.map { .group($0) }))
        case .twoRows:
            root = .split(LayoutSplit(axis: .vertical, children: gs.map { .group($0) }))
        case .three:
            let right = LayoutSplit(axis: .vertical, children: [.group(gs[1]), .group(gs[2])])
            root = .split(LayoutSplit(axis: .horizontal, children: [.group(gs[0]), .split(right)]))
        }
    }

    // MARK: Helpers

    private mutating func take(_ itemID: UUID) {
        guard let g = group(containing: itemID) else { return }
        updateGroup(g.id) { g in
            guard let i = g.items.firstIndex(where: { $0.id == itemID }) else { return }
            g.items.remove(at: i)
            if g.selected == itemID {
                // The neighbour on the right, else the left, as tab bars do.
                g.selected = g.items.isEmpty ? nil : g.items[min(i, g.items.count - 1)].id
            }
        }
    }

    private mutating func removeIfEmpty(_ groupID: UUID) {
        guard let g = group(groupID), g.items.isEmpty, groups.count > 1 else { return }
        root = Self.remove(groupID, from: root) ?? root
        normalize()
    }

    private mutating func updateGroup(_ id: UUID, _ change: (inout TabGroup) -> Void) {
        func walk(_ n: LayoutNode) -> LayoutNode {
            switch n {
            case .group(var g):
                if g.id == id { change(&g) }
                return .group(g)
            case .split(var s):
                s.children = s.children.map(walk)
                return .split(s)
            }
        }
        root = walk(root)
    }

    private mutating func updateSplit(_ id: UUID, _ change: (inout LayoutSplit) -> Void) {
        func walk(_ n: LayoutNode) -> LayoutNode {
            guard case .split(var s) = n else { return n }
            s.children = s.children.map(walk)
            if s.id == id { change(&s) }
            return .split(s)
        }
        root = walk(root)
    }

    /// `node` with the group removed; nil when the node was that group.
    private static func remove(_ groupID: UUID, from node: LayoutNode) -> LayoutNode? {
        switch node {
        case .group(let g):
            return g.id == groupID ? nil : node
        case .split(var s):
            var children: [LayoutNode] = []
            var fractions: [Double] = []
            for (c, f) in zip(s.children, s.fractions) {
                if let kept = remove(groupID, from: c) {
                    children.append(kept)
                    fractions.append(f)
                }
            }
            if children.isEmpty { return nil }
            let total = fractions.reduce(0, +)
            s.children = children
            s.fractions = fractions.map { total > 0 ? $0 / total : 1 / Double(children.count) }
            return .split(s)
        }
    }

    /// `node` with `new` placed beside the group `target`: as a sibling when
    /// the group's parent splits along `axis`, which shares out the target's
    /// fraction between the two, or else in a new split in its place.
    private static func insert(_ new: LayoutNode, beside target: UUID, axis: SplitAxis, before: Bool, in node: LayoutNode) -> LayoutNode {
        switch node {
        case .group(let g):
            guard g.id == target else { return node }
            return .split(LayoutSplit(axis: axis, children: before ? [new, node] : [node, new]))
        case .split(var s):
            if s.axis == axis, let i = s.children.firstIndex(where: {
                if case .group(let g) = $0 { return g.id == target }
                return false
            }) {
                let half = s.fractions[i] / 2
                s.fractions[i] = half
                let at = before ? i : i + 1
                s.children.insert(new, at: at)
                s.fractions.insert(half, at: at)
                return .split(s)
            }
            s.children = s.children.map { insert(new, beside: target, axis: axis, before: before, in: $0) }
            return .split(s)
        }
    }

    /// Splits with one child become the child; a split inside one along the
    /// same axis is merged into it, its fraction shared among its children.
    private mutating func normalize() {
        func walk(_ n: LayoutNode) -> LayoutNode {
            guard case .split(var s) = n else { return n }
            var children: [LayoutNode] = []
            var fractions: [Double] = []
            for (c, f) in zip(s.children.map(walk), s.fractions) {
                if case .split(let inner) = c, inner.axis == s.axis {
                    children += inner.children
                    fractions += inner.fractions.map { $0 * f }
                } else {
                    children.append(c)
                    fractions.append(f)
                }
            }
            if children.count == 1 { return children[0] }
            s.children = children
            s.fractions = fractions
            return .split(s)
        }
        root = walk(root)
    }
}

/// What a drag into the editor area carries: a tab, or something to open
/// (a row of the sidebar).
enum TabDrop: Codable, Equatable, Sendable {
    case item(UUID)
    case open(EditorContent)

    static let pasteboardType = "io.github.ayodehi.romlens.tab"
}

/// Where a drop lands in a group: between two tabs of its bar, or in a zone
/// of its view.
enum DropTarget: Equatable, Sendable {
    case tabBar(index: Int)
    case zone(DropZone)
}

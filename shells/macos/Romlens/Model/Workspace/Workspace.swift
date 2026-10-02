import Foundation
import Observation

/// The window's tabs: the layout, which group has focus, and the models a
/// code tab keeps for itself (docs/29).
///
/// Every change to the layout from the window goes through here, so the
/// focus always names a group that exists. The selection stays on
/// `RomViewModel`, shared by every tab.
@MainActor
@Observable
final class Workspace {
    private(set) var layout: EditorLayout
    private(set) var focusedGroup: UUID
    /// The code tab that last had focus: what the C and Graph models of
    /// "the" editor mean while the views move over (docs/29, W2).
    private(set) var lastCodeItem: UUID?

    @ObservationIgnored private var decompilers: [UUID: DecompileModel] = [:]
    @ObservationIgnored private var graphs: [UUID: GraphModel] = [:]
    /// For a call that names no code tab while none exists.
    @ObservationIgnored private let fallback = UUID()

    init(layout: EditorLayout = .single([EditorItem(.code(.hex))])) {
        self.layout = layout
        focusedGroup = layout.groups[0].id
        lastCodeItem = layout.items.first { if case .code = $0.content { true } else { false } }?.id
    }

    // MARK: Reading

    var focusedItem: EditorItem? { layout.group(focusedGroup)?.selectedItem }

    /// The tab shown in each group.
    var visibleItems: [EditorItem] { layout.groups.compactMap(\.selectedItem) }

    /// The code tab C and Graph act on when no tab is named.
    var currentCodeItem: UUID {
        if let f = focusedItem, case .code = f.content { return f.id }
        if let last = lastCodeItem, layout.item(last) != nil { return last }
        return fallback
    }

    func decompiler(for id: UUID?) -> DecompileModel {
        let key = id ?? currentCodeItem
        if let d = decompilers[key] { return d }
        let d = DecompileModel()
        decompilers[key] = d
        return d
    }

    func graph(for id: UUID?) -> GraphModel {
        let key = id ?? currentCodeItem
        if let g = graphs[key] { return g }
        let g = GraphModel()
        graphs[key] = g
        return g
    }

    /// The analysis changed: every tab's C and graph are stale.
    func invalidateAll() {
        decompilers.values.forEach { $0.invalidate() }
        graphs.values.forEach { $0.invalidate() }
    }

    // MARK: Changing

    /// Opens `content` in `group` (the focused one if nil), or shows its
    /// existing tab, and gives that tab focus. A second code tab showing the
    /// same representation as an open one does not follow the selection,
    /// since two tabs that always scroll together are one tab twice.
    @discardableResult
    func open(_ content: EditorContent, in group: UUID? = nil) -> UUID? {
        let target = group ?? focusedGroup
        var follows = true
        if case .code = content, layout.items.contains(where: { $0.content == content }) {
            follows = false
        }
        guard let id = layout.open(content, in: target, followsSelection: follows) else { return nil }
        focus(item: id)
        return id
    }

    /// Shows `item` in its group and gives the group focus.
    func focus(item id: UUID) {
        guard let g = layout.group(containing: id) else { return }
        layout.select(id)
        focusedGroup = g.id
        noteFocus()
    }

    func focus(group id: UUID) {
        guard layout.group(id) != nil else { return }
        focusedGroup = id
        noteFocus()
    }

    /// Changes a code tab's representation in place.
    func setRepresentation(_ r: CodeRepresentation, of id: UUID) {
        change { l in
            var root = l.root
            Self.updateItem(id, in: &root) { $0.content = .code(r) }
            l.root = root
        }
    }

    func setFollowsSelection(_ follows: Bool, of id: UUID) {
        change { l in
            var root = l.root
            Self.updateItem(id, in: &root) { $0.followsSelection = follows }
            l.root = root
        }
    }

    func close(_ id: UUID) {
        let group = layout.group(containing: id)?.id
        change { $0.close(id) }
        decompilers[id] = nil
        graphs[id] = nil
        if let group, layout.group(group) != nil { focusedGroup = group }
        repairFocus()
    }

    func move(_ id: UUID, to group: UUID, at index: Int? = nil) {
        change { $0.move(id, to: group, at: index) }
        focus(item: id)
    }

    @discardableResult
    func split(_ group: UUID, _ edge: DropEdge, with id: UUID) -> UUID? {
        var fresh: UUID?
        change { fresh = $0.split(group, edge, with: id) }
        if let fresh { focus(group: fresh) }
        return fresh
    }

    func apply(_ preset: LayoutPreset) { change { $0.apply(preset) } }
    func equalize(_ split: UUID) { change { $0.equalize(split) } }
    func setFractions(_ split: UUID, _ fractions: [Double]) { change { $0.setFractions(split, fractions) } }

    /// Replaces the whole layout, as reopening a project does. Tabs that are
    /// gone take their models with them.
    func restore(_ saved: EditorLayout, focusedGroup focus: UUID?) {
        layout = saved
        let ids = Set(saved.items.map(\.id))
        decompilers = decompilers.filter { ids.contains($0.key) }
        graphs = graphs.filter { ids.contains($0.key) }
        focusedGroup = focus.flatMap { saved.group($0)?.id } ?? saved.groups[0].id
        noteFocus()
    }

    // MARK: Helpers

    private func change(_ body: (inout EditorLayout) -> Void) {
        var l = layout
        body(&l)
        layout = l
        repairFocus()
    }

    private func repairFocus() {
        if layout.group(focusedGroup) == nil { focusedGroup = layout.groups[0].id }
        noteFocus()
    }

    private func noteFocus() {
        if let f = focusedItem, case .code = f.content { lastCodeItem = f.id }
    }

    private static func updateItem(_ id: UUID, in node: inout LayoutNode, _ change: (inout EditorItem) -> Void) {
        switch node {
        case .group(var g):
            if let i = g.items.firstIndex(where: { $0.id == id }) {
                change(&g.items[i])
                node = .group(g)
            }
        case .split(var s):
            for i in s.children.indices { updateItem(id, in: &s.children[i], change) }
            node = .split(s)
        }
    }
}

/// What `local.json` keeps of the window (docs/29): the tabs, which group
/// has focus, which panels show and the drawer's tab.
struct WorkspaceRecord: Codable, Equatable, Sendable {
    var layout: EditorLayout
    var focusedGroup: UUID?
    var sidebar: Bool = true
    var inspector: Bool = true
    var strip: Bool = true
    var tutorInDrawer: Bool = false
}

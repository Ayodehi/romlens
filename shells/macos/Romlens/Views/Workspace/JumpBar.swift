import RomlensKit
import SwiftUI

/// Where the focused tab is, as a path that is also a set of menus
/// (docs/29): the chip, the view, and for code the bank, the routine and
/// the instruction. Each segment's menu goes somewhere else at that level.
struct JumpBar: View {
    let model: RomViewModel

    private var content: EditorContent? { model.workspace.focusedItem?.content }

    private var group: SidebarView.Group? {
        guard let content else { return nil }
        return SidebarView.groups.first { $0.entries.contains { $0.content == content } }
    }

    private var entry: SidebarView.Entry? {
        guard let content else { return nil }
        return SidebarView.groups.flatMap(\.entries).first { $0.content == content }
    }

    private var chip: String {
        group.map { String($0.title.split(separator: " ").first ?? "") } ?? "Romlens"
    }

    var body: some View {
        HStack(spacing: 4) {
            segment(chip, tint: true) {
                ForEach(SidebarView.groups) { g in
                    Button(g.title) { open(g.entries.first { $0.content.map { model.unavailableReason($0) == nil } ?? false }) }
                }
            }
            if let entry, let group {
                separator
                segment(entry.title) {
                    ForEach(group.entries) { e in
                        Button(e.title) { open(e) }
                            .disabled(e.content.map { model.unavailableReason($0) != nil } ?? false)
                    }
                }
            }
            if case .code = content, let address = model.selectedAddress {
                separator
                segment(String(format: "Bank $%02X", address >> 16)) {
                    ForEach(model.navigator.banks) { bank in
                        Button(String(format: "Bank $%02X", bank.bank)) { model.jump(to: bank.fileOffset) }
                    }
                }
                if let routine = model.routineName(at: address) {
                    separator
                    segment(routine, mono: true) {
                        ForEach(routines(in: UInt8(address >> 16)), id: \.address) { label in
                            Button(label.name) { model.jump(toSnesAddress: label.address) }
                        }
                    }
                }
                if let insn = model.instruction {
                    separator
                    Text("\(formatSnesAddress(address: insn.snesAddress)) \(insn.text)")
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(.horizontal, 10)
        .frame(minWidth: 280, idealWidth: 560, maxWidth: 640, minHeight: 26)
        .help("Where the focused tab is. Each part is a menu.")
    }

    private var separator: some View {
        Image(systemName: "chevron.compact.right").foregroundStyle(.tertiary)
    }

    private func segment<Items: View>(_ title: String, tint: Bool = false, mono: Bool = false, @ViewBuilder items: () -> Items) -> some View {
        Menu {
            items()
        } label: {
            Text(title)
                .font(mono ? .callout.monospaced() : .callout)
                .foregroundStyle(tint ? AnyShapeStyle(.tint) : AnyShapeStyle(.primary))
                .lineLimit(1)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
    }

    /// The labels of a bank, in address order, at most 300.
    private func routines(in bank: UInt8) -> [LabelInfo] {
        Array(model.navigator.labels.filter { UInt8($0.address >> 16) == bank }.sorted { $0.address < $1.address }.prefix(300))
    }

    private func open(_ e: SidebarView.Entry?) {
        guard let e, let content = e.content else { return }
        switch content {
        case .graphics(let t): model.openGraphics(t)
        case .audio(let t): model.openAudio(t)
        case .tutor: NSApp.sendAction(#selector(RomWindowController.showTutor(_:)), to: nil, from: nil)
        default: model.show(content)
        }
    }
}

/// Open Quickly (⇧⌘O): type part of a label, a variable, a view's name or
/// an address, and go there (docs/29).
struct OpenQuicklySheet: View {
    @Bindable var model: RomViewModel
    @State private var query = ""
    @State private var selection: Int? = 0
    @Environment(\.dismiss) private var dismiss

    struct Result: Identifiable {
        enum Kind { case address, label, variable, view }
        let id: Int
        let kind: Kind
        let title: String
        let detail: String
        let action: @MainActor () -> Void
    }

    private var results: [Result] { Self.results(for: query, in: model) }

    /// What a query finds: an address it resolves to first, then views,
    /// labels (40 at most) and variables.
    static func results(for query: String, in model: RomViewModel) -> [Result] {
        let q = query.trimmingCharacters(in: .whitespaces)
        var out: [Result] = []
        func add(_ kind: Result.Kind, _ title: String, _ detail: String, _ action: @escaping @MainActor () -> Void) {
            out.append(Result(id: out.count, kind: kind, title: title, detail: detail, action: action))
        }
        guard !q.isEmpty else { return [] }
        if case .success(let resolved) = model.preview(text: q) {
            let offset = resolved.fileOffset
            add(.address, "Go to \(q)", formatFileOffset(offset: offset)) { model.jump(to: offset) }
        }
        let lower = q.lowercased()
        for e in SidebarView.groups.flatMap(\.entries) where e.title.lowercased().contains(lower) {
            guard let content = e.content else { continue }
            add(.view, e.title, model.unavailableReason(content) ?? "view") {
                switch content {
                case .graphics(let t): model.openGraphics(t)
                case .audio(let t): model.openAudio(t)
                case .tutor: NSApp.sendAction(#selector(RomWindowController.showTutor(_:)), to: nil, from: nil)
                default: model.show(content)
                }
            }
        }
        for label in NavigatorModel.filter(model.navigator.labels, query: q).prefix(40) {
            let address = label.address
            add(.label, label.name, formatSnesAddress(address: address)) { model.jump(toSnesAddress: address) }
        }
        for v in model.navigator.variables where v.name.lowercased().contains(lower) {
            let address = v.address
            add(.variable, v.name, formatSnesAddress(address: address)) {
                model.references.find(to: address, in: model.workbench)
                model.resultsKind = .references
                model.isResultsVisible = true
            }
        }
        return out
    }

    var body: some View {
        let found = results
        VStack(spacing: 0) {
            TextField("A label, variable, view or address", text: $query)
                .textFieldStyle(.plain)
                .font(.title3)
                .padding(12)
                .onSubmit { go(found) }
                .onChange(of: query) { selection = 0 }
                .onKeyPress(.downArrow) { move(1, found); return .handled }
                .onKeyPress(.upArrow) { move(-1, found); return .handled }
            Divider()
            List(found, selection: $selection) { r in
                HStack {
                    Image(systemName: symbol(r.kind)).foregroundStyle(.secondary).frame(width: 18)
                    Text(r.title).font(r.kind == .view ? .body : .body.monospaced()).lineLimit(1)
                    Spacer()
                    Text(r.detail).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                .tag(r.id)
                .contentShape(Rectangle())
                .onTapGesture(count: 2) {
                    selection = r.id
                    go(found)
                }
            }
            .listStyle(.plain)
            .overlay {
                if found.isEmpty {
                    Text(query.isEmpty ? "Labels, variables, views, and addresses such as $00:8000 or RESET" : "Nothing matches")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .frame(width: 560, height: 380)
        .onExitCommand { dismiss() }
    }

    private func symbol(_ kind: Result.Kind) -> String {
        switch kind {
        case .address: "arrow.right.circle"
        case .label: "tag"
        case .variable: "character.textbox"
        case .view: "rectangle.on.rectangle"
        }
    }

    private func move(_ delta: Int, _ found: [Result]) {
        guard !found.isEmpty else { return }
        selection = min(max(0, (selection ?? -1) + delta), found.count - 1)
    }

    private func go(_ found: [Result]) {
        guard let i = selection, found.indices.contains(i) else { return }
        let action = found[i].action
        dismiss()
        action()
    }
}

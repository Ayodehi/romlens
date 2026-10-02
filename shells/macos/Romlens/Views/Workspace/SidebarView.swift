import AppKit
import RomlensKit
import SwiftUI
import UniformTypeIdentifiers

/// The sidebar (docs/29): every view, grouped by the chip that owns it, then
/// Learn and the project's symbols, with one filter for all of it.
///
/// A view that cannot open yet stays listed, dimmed, saying what it needs;
/// choosing it does what would make it available (Compare With…, Open
/// Recording…, importing sources). Rows drag into the editor area like tabs.
struct SidebarView: View {
    let model: RomViewModel

    struct Entry: Identifiable, Hashable {
        let id: String
        let title: String
        let symbol: String
        let content: EditorContent?
        let shortcut: String?

        init(_ id: String, _ title: String, _ symbol: String, _ content: EditorContent?, _ shortcut: String? = nil) {
            self.id = id
            self.title = title
            self.symbol = symbol
            self.content = content
            self.shortcut = shortcut
        }
    }

    struct Group: Identifiable {
        let id: String
        let title: String
        let entries: [Entry]
    }

    static let groups: [Group] = [
        Group(id: "cartridge", title: "Cartridge", entries: [
            Entry("atlas", "Atlas", "square.grid.2x2", .atlas, "⌥⌘A"),
            Entry("header", "Header and Vectors", "doc.text.magnifyingglass", nil, "⇧⌘H"),
            Entry("compare", "Compare", "rectangle.on.rectangle", .compare),
        ]),
        Group(id: "cpu", title: "CPU · 65816", entries: [
            Entry("assembly", "Disassembly", "text.alignleft", .code(.assembly), "⌥⌘2"),
            Entry("c", "Pseudo-C", "chevron.left.forwardslash.chevron.right", .code(.c), "⌥⌘8"),
            Entry("graph", "Graph", "point.3.connected.trianglepath.dotted", .code(.graph), "⌥⌘9"),
            Entry("hex", "Hex", "number", .code(.hex), "⌥⌘1"),
            Entry("both", "Hex and Disassembly", "rectangle.split.2x1", .code(.both), "⌥⌘3"),
            Entry("source", "Source", "doc.text", .source),
        ]),
        Group(id: "ppu", title: "PPU · Picture", entries: GraphicsModel.Tab.allCases.map {
            Entry("ppu.\($0.rawValue)", $0.title, $0.systemImage, .graphics($0))
        }),
        Group(id: "apu", title: "APU · Sound", entries: AudioModel.Tab.allCases.map {
            Entry("apu.\($0.rawValue)", $0.title, $0.systemImage, .audio($0))
        }),
        Group(id: "learn", title: "Learn", entries: [
            Entry("tutor", "Tutor", "graduationcap", .tutor, "⌥⌘T"),
            Entry("lessons", "Lessons and Quizzes", "list.bullet.rectangle", nil),
        ]),
    ]

    /// The rows the list may show of the labels, for a ROM with thousands;
    /// the filter finds the rest.
    static let labelLimit = 1000

    @AppStorage("Sidebar.collapsed") private var collapsedStore = ""

    private var collapsed: Set<String> {
        Set(collapsedStore.split(separator: ",").map(String.init))
    }

    private func expanded(_ id: String) -> Binding<Bool> {
        Binding(
            get: { !collapsed.contains(id) },
            set: { open in
                var c = collapsed
                if open { c.remove(id) } else { c.insert(id) }
                collapsedStore = c.sorted().joined(separator: ",")
            }
        )
    }

    private var query: String { model.navigator.filter.trimmingCharacters(in: .whitespaces).lowercased() }

    private func matches(_ e: Entry) -> Bool {
        query.isEmpty || e.title.lowercased().contains(query)
    }

    /// The row of the focused tab's view.
    private var current: String? {
        guard let content = model.workspace.focusedItem?.content else { return nil }
        return Self.groups.flatMap(\.entries).first { $0.content == content }?.id
    }

    var body: some View {
        @Bindable var nav = model.navigator
        VStack(spacing: 0) {
            List(selection: Binding(get: { current }, set: { id in
                if let id, let e = Self.groups.flatMap(\.entries).first(where: { $0.id == id }) { choose(e) }
            })) {
                ForEach(Self.groups) { group in
                    let shown = group.entries.filter(matches)
                    if !shown.isEmpty {
                        Section(isExpanded: query.isEmpty ? expanded(group.id) : .constant(true)) {
                            ForEach(shown) { row($0) }
                        } header: {
                            Text(group.title)
                        }
                    }
                }
                Section(isExpanded: query.isEmpty ? expanded("symbols") : .constant(true)) {
                    symbols
                } header: {
                    Text("Symbols")
                }
            }
            .listStyle(.sidebar)
            Divider()
            TextField("Filter views and symbols", text: $nav.filter)
                .textFieldStyle(.roundedBorder)
                .padding(8)
        }
    }

    @ViewBuilder
    private func row(_ e: Entry) -> some View {
        let reason = e.content.flatMap(model.unavailableReason)
        HStack(spacing: 8) {
            Image(systemName: e.symbol)
                .frame(width: 18)
                .foregroundStyle(reason == nil ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
            Text(e.title)
                .foregroundStyle(reason == nil ? .primary : .secondary)
                .lineLimit(1)
            Spacer(minLength: 4)
            if let reason {
                Text(reason)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            } else if let s = e.shortcut, current == e.id {
                Text(s)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .tag(e.id)
        .help(reason.map { "\(e.title) \($0). Choose it to fix that." } ?? "Show \(e.title)")
        .onDrag {
            guard let content = e.content else { return NSItemProvider() }
            return Self.provider(.open(content))
        }
    }

    /// What a dragged row carries, under the type the editor area accepts.
    static func provider(_ drop: TabDrop) -> NSItemProvider {
        let provider = NSItemProvider()
        if let data = try? JSONEncoder().encode(drop) {
            provider.registerDataRepresentation(forTypeIdentifier: TabDrop.pasteboardType, visibility: .ownProcess) { done in
                done(data, nil)
                return nil
            }
        }
        return provider
    }

    /// A row chosen: its view, or what would make it available.
    private func choose(_ e: Entry) {
        switch e.id {
        case "header":
            model.show(.code(.hex))
            model.jump(to: model.info.headerOffset)
            return
        case "lessons":
            NSApp.sendAction(#selector(RomWindowController.showLessons(_:)), to: nil, from: nil)
            return
        case "tutor":
            model.showTutorTab()
            return
        default:
            break
        }
        guard let content = e.content else { return }
        if model.unavailableReason(content) != nil {
            switch content {
            case .compare: NSApp.sendAction(#selector(RomWindowController.compareWith(_:)), to: nil, from: nil)
            case .source: NSApp.sendAction(#selector(RomWindowController.importDbg(_:)), to: nil, from: nil)
            case .graphics: NSApp.sendAction(#selector(RomWindowController.openRecording(_:)), to: nil, from: nil)
            default: break
            }
            return
        }
        switch content {
        case .graphics(let t): model.openGraphics(t)
        case .audio(let t): model.openAudio(t)
        default: model.show(content)
        }
    }

    // MARK: Symbols

    @ViewBuilder
    private var symbols: some View {
        let nav = model.navigator
        DisclosureGroup(isExpanded: expanded("labels")) {
            let shown = nav.filteredLabels.prefix(Self.labelLimit)
            ForEach(Array(shown), id: \.address) { label in
                labelRow(label)
            }
            if nav.filteredLabels.count > Self.labelLimit {
                Text("\(Self.labelLimit) of \(nav.filteredLabels.count); filter to find the rest")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } label: {
            countRow("Labels", "tag", nav.filteredLabels.count)
        }
        DisclosureGroup(isExpanded: expanded("variables")) {
            if nav.variables.isEmpty {
                Text("None yet. Name a RAM address with Define Variable….")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            ForEach(nav.filteredVariables, id: \.address) { v in
                variableRow(v)
            }
            Button {
                model.beginNewVariable()
            } label: {
                Label("Define Variable…", systemImage: "plus")
            }
            .buttonStyle(.borderless)
            .controlSize(.small)
        } label: {
            countRow("Variables", "character.textbox", nav.filteredVariables.count)
        }
        DisclosureGroup(isExpanded: expanded("regions")) {
            if nav.regionsTruncated {
                Text("Largest \(NavigatorModel.regionLimit) of each kind")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            ForEach(nav.filteredRegions, id: \.start) { region in
                regionRow(region)
            }
        } label: {
            countRow("Regions", "square.stack.3d.up", nav.filteredRegions.count)
        }
        DisclosureGroup(isExpanded: expanded("banks")) {
            ForEach(nav.banks) { bank in
                Button {
                    model.jump(to: bank.fileOffset)
                } label: {
                    HStack {
                        Text(String(format: "Bank $%02X", bank.bank)).font(.callout.monospaced())
                        Spacer()
                        Text("\(bank.length / 1024) KB")
                            .font(.caption.monospaced())
                            .foregroundStyle(.tertiary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        } label: {
            countRow("Banks", "square.grid.2x2", nav.banks.count)
        }
    }

    private func countRow(_ title: String, _ symbol: String, _ count: Int) -> some View {
        HStack(spacing: 8) {
            Image(systemName: symbol).frame(width: 18).foregroundStyle(.secondary)
            Text(title)
            Spacer()
            Text(count.formatted())
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
        }
    }

    private func labelRow(_ label: LabelInfo) -> some View {
        Button {
            model.jump(toSnesAddress: label.address)
        } label: {
            HStack {
                Text(label.name)
                    .font(.callout.monospaced())
                    .foregroundStyle(label.source == .auto ? .secondary : .primary)
                    .lineLimit(1)
                Spacer()
                Text(formatSnesAddress(address: label.address))
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(label.source == .imported ? "Imported from \(label.origin)" : label.source == .user ? "Your label" : "Named by the analysis")
        .onDrag { Self.provider(.openAt(.assembly, address: label.address)) }
    }

    private func variableRow(_ v: VariableInfo) -> some View {
        Button {
            model.references.find(to: v.address, in: model.workbench)
            model.resultsKind = .references
            model.isResultsVisible = true
        } label: {
            HStack {
                Text(v.name.isEmpty ? "(unnamed)" : v.name)
                    .font(.callout.monospaced())
                    .lineLimit(1)
                Text(v.description)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                Spacer()
                Text(formatSnesAddress(address: v.address))
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("\(v.memory), \(v.len) byte\(v.len == 1 ? "" : "s"). Click for what uses it; double-click to edit.")
        .simultaneousGesture(TapGesture(count: 2).onEnded { model.beginDefineVariable(at: v.address) })
        .contextMenu {
            Button("Edit…") { model.beginDefineVariable(at: v.address) }
            Button("Find References") {
                model.references.find(to: v.address, in: model.workbench)
                model.resultsKind = .references
                model.isResultsVisible = true
            }
            Divider()
            Button("Remove", role: .destructive) { try? model.removeVariable(address: v.address) }
        }
    }

    private func regionRow(_ region: RegionInfo) -> some View {
        Button {
            model.jump(to: region.start)
        } label: {
            HStack {
                Circle()
                    .fill(Color(nsColor: RegionPalette.color(
                        kind: region.kind == .code ? .code : .byte,
                        confidence: Double(region.confidence)
                    )))
                    .frame(width: 7, height: 7)
                Text(region.name)
                    .font(.callout)
                    .lineLimit(1)
                Spacer()
                Text("\(region.len) B")
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("\(formatFileOffset(offset: region.start)), \(Int((region.confidence * 100).rounded()))% confident")
    }
}

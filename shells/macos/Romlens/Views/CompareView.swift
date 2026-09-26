import RomlensKit
import SwiftUI

/// The Compare tab (docs/22, D2): what changed from another version of the
/// ROM to this one. The list on the left, the chosen change on the right:
/// a changed routine's instructions side by side, aligned by the pairing.
struct CompareView: View {
    @Bindable var model: RomViewModel

    private var compare: CompareModel { model.compare }

    var body: some View {
        Group {
            switch compare.state {
            case .idle:
                ContentUnavailableView {
                    Label("Nothing to compare", systemImage: "rectangle.split.2x1")
                } description: {
                    Text("File › Compare With… opens another version of this ROM, or its saved project, beside this one.")
                } actions: {
                    Button("Compare With…") { CompareController.open(model: model, window: nil) }
                }
            case .loading(let what):
                ProgressView(what).frame(maxWidth: .infinity, maxHeight: .infinity)
            case .failed(let message):
                ContentUnavailableView("The comparison failed", systemImage: "exclamationmark.triangle", description: Text(message))
            case .ready:
                if let info = compare.info {
                    HSplitView {
                        list(info).frame(minWidth: 260, idealWidth: 320, maxWidth: 480)
                        detail(info).frame(minWidth: 360, maxWidth: .infinity)
                    }
                }
            }
        }
    }

    // MARK: The list

    private func list(_ info: ComparisonInfo) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            header(info).padding(10)
            Divider()
            List(selection: Bindable(compare).selected) {
                Section("Routines: \(info.sameRoutines) the same, \(info.routines.count) not") {
                    ForEach(Array(compare.routines.enumerated()), id: \.offset) { i, r in
                        routineRow(r).tag(CompareModel.Item.routine(i))
                    }
                }
                if !info.data.isEmpty {
                    Section("Data with changed bytes: \(info.data.count)") {
                        ForEach(Array(info.data.enumerated()), id: \.offset) { i, d in
                            dataRow(d).tag(CompareModel.Item.data(i))
                        }
                    }
                }
                if !info.moves.isEmpty {
                    Section("Moved blocks: \(info.moves.count)") {
                        ForEach(Array(info.moves.enumerated()), id: \.offset) { i, m in
                            Text("\(formatFileOffset(offset: m.a)) → \(formatFileOffset(offset: m.b)), \(bytes(m.len))")
                                .font(.callout.monospacedDigit())
                                .tag(CompareModel.Item.move(i))
                        }
                    }
                }
                Section("Bytes that differ: \(info.runs.count) stretches") {
                    ForEach(Array(compare.runs.enumerated()), id: \.offset) { i, r in
                        runRow(r).tag(CompareModel.Item.run(i))
                    }
                    if info.runs.count > compare.runs.count {
                        Text("and \(info.runs.count - compare.runs.count) more").foregroundStyle(.secondary)
                    }
                }
            }
            .listStyle(.sidebar)
        }
    }

    private func header(_ info: ComparisonInfo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("\(compare.otherName ?? "The other version") → this one").font(.headline).lineLimit(1)
                Spacer()
                Button("Close") {
                    compare.close()
                    model.editorTab = .disassembly
                }
                    .controlSize(.small)
                    .help("Stop comparing")
            }
            Text(
                "\(bytes(info.changedBytes)) changed, \(bytes(info.insertedBytes)) inserted, \(bytes(info.deletedBytes)) deleted, \(bytes(info.sameBytes)) the same"
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            if !info.namesToCarry.isEmpty {
                Button("Carry Over \(info.namesToCarry.count) Name\(info.namesToCarry.count == 1 ? "" : "s")") {
                    try? model.session.carryNames(info.namesToCarry)
                }
                .help("Name the routines here that the other version names and this one does not, as one step you can undo")
            }
        }
    }

    private func routineRow(_ r: RoutinePairInfo) -> some View {
        HStack(spacing: 6) {
            Text(CompareModel.title(r.pairing))
                .font(.caption.weight(.medium))
                .foregroundStyle(color(r.pairing))
                .frame(width: 58, alignment: .leading)
            VStack(alignment: .leading, spacing: 1) {
                Text(r.b?.name ?? r.a?.name ?? "").lineLimit(1)
                Text(routineWhere(r)).font(.caption.monospacedDigit()).foregroundStyle(.secondary).lineLimit(1)
            }
        }
    }

    private func routineWhere(_ r: RoutinePairInfo) -> String {
        switch (r.a, r.b) {
        case (let a?, let b?) where a.entry == b.entry: formatSnesAddress(address: b.entry)
        case (let a?, let b?): "\(formatSnesAddress(address: a.entry)) → \(formatSnesAddress(address: b.entry))"
        case (let a?, nil): "\(formatSnesAddress(address: a.entry)), only in the other"
        case (nil, let b?): "\(formatSnesAddress(address: b.entry)), only in this one"
        default: ""
        }
    }

    private func dataRow(_ d: DataChangeInfo) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(d.name ?? d.kind).lineLimit(1)
            Text("\(formatFileOffset(offset: d.start)), \(bytes(d.changed)) of \(bytes(d.len))\(d.inB ? ", only in this one" : "")")
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
        }
    }

    private func runRow(_ r: ByteRunInfo) -> some View {
        Text("\(CompareModel.title(r.kind)) \(formatFileOffset(offset: r.bStart)), \(bytes(UInt64(max(r.aEnd - r.aStart, r.bEnd - r.bStart))))")
            .font(.callout.monospacedDigit())
    }

    private func color(_ p: RoutinePairing) -> Color {
        switch p {
        case .same: .secondary
        case .moved: .blue
        case .changed: .orange
        case .added: .green
        case .removed: .red
        }
    }

    // MARK: The detail

    @ViewBuilder
    private func detail(_ info: ComparisonInfo) -> some View {
        switch compare.selected {
        case .routine(let i) where compare.routines.indices.contains(i):
            routineDetail(compare.routines[i])
        case .data(let i) where info.data.indices.contains(i):
            let d = info.data[i]
            place(
                title: d.name ?? d.kind,
                text: "\(bytes(d.changed)) changed in this \(d.kind) region of \(bytes(d.len)), \(formatFileOffset(offset: d.start))–\(formatFileOffset(offset: d.start + d.len)) in \(d.inB ? "this version" : "the other version").",
                offset: d.inB ? d.start : nil
            )
        case .run(let i) where compare.runs.indices.contains(i):
            let r = compare.runs[i]
            place(
                title: "Bytes \(CompareModel.title(r.kind))",
                text: "The other version's \(formatFileOffset(offset: r.aStart))–\(formatFileOffset(offset: r.aEnd)) is this one's \(formatFileOffset(offset: r.bStart))–\(formatFileOffset(offset: r.bEnd)).",
                offset: r.kind == .deleted ? nil : r.bStart
            )
        case .move(let i) where info.moves.indices.contains(i):
            let m = info.moves[i]
            place(
                title: "A block moved",
                text: "\(bytes(m.len)) at \(formatFileOffset(offset: m.a)) in the other version are at \(formatFileOffset(offset: m.b)) in this one.",
                offset: m.b
            )
        default:
            ContentUnavailableView("Choose a change", systemImage: "arrow.left", description: Text("A changed routine shows both versions' instructions side by side."))
        }
    }

    private func place(title: String, text: String, offset: UInt32?) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title).font(.title3.weight(.semibold))
            Text(text).fixedSize(horizontal: false, vertical: true)
            if let offset {
                Button("Show in Hex") {
                    model.editorTab = .hex
                    model.jump(to: offset)
                }
            }
            Spacer()
        }
        .padding()
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func routineDetail(_ r: RoutinePairInfo) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(r.b?.name ?? r.a?.name ?? "").font(.title3.weight(.semibold))
                    Text(summary(r)).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if let b = r.b {
                    Button("Show in Listing") {
                        model.editorTab = .disassembly
                        model.jump(to: b.offset)
                    }
                }
            }
            .padding(10)
            Divider()
            if r.lines.isEmpty {
                Text(summary(r)).foregroundStyle(.secondary).padding()
                Spacer()
            } else {
                HStack {
                    Text(compare.otherName ?? "The other version").frame(maxWidth: .infinity, alignment: .leading)
                    Text("This version").frame(maxWidth: .infinity, alignment: .leading)
                }
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(r.lines.enumerated()), id: \.offset) { _, l in
                            lineRow(l)
                        }
                    }
                    .padding(.horizontal, 10)
                }
            }
        }
    }

    private func summary(_ r: RoutinePairInfo) -> String {
        let changed = r.lines.filter { $0.op != .same }.count
        switch r.pairing {
        case .changed: return "\(changed) of \(r.lines.count) lines differ"
        case .moved: return "The same instructions, moved from \(r.a.map { formatSnesAddress(address: $0.entry) } ?? "")"
        case .added: return "Only in this version: \(r.b?.instructions ?? 0) instructions"
        case .removed: return "Only in the other version, at \(r.a.map { formatSnesAddress(address: $0.entry) } ?? "")"
        case .same: return "The same"
        }
    }

    private func lineRow(_ l: DiffLineInfo) -> some View {
        let tint: Color = switch l.op {
        case .same: .clear
        case .changed: .orange.opacity(0.18)
        case .added: .green.opacity(0.18)
        case .removed: .red.opacity(0.18)
        }
        return HStack(spacing: 12) {
            cell(l.aOffset, l.aText)
            cell(l.bOffset, l.bText)
                .contentShape(Rectangle())
                .onTapGesture { if let o = l.bOffset { model.select(offset: o) } }
        }
        .font(.system(.body, design: .monospaced))
        .padding(.vertical, 1)
        .background(tint)
    }

    private func cell(_ offset: UInt32?, _ text: String?) -> some View {
        HStack(spacing: 8) {
            Text(offset.map { formatFileOffset(offset: $0) } ?? "")
                .foregroundStyle(.secondary)
                .frame(width: 76, alignment: .leading)
            Text(text ?? "")
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .lineLimit(1)
    }

    private func bytes(_ n: some BinaryInteger) -> String {
        "\(n) byte\(n == 1 ? "" : "s")"
    }
}

import RomlensKit
import SwiftUI

/// The selected item's sections, or the header summary when nothing is
/// selected.
struct InspectorView: View {
    let model: RomViewModel

    var body: some View {
        Group {
            if model.selectedOffset != nil {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        SelectionHeader(model: model)
                        if model.instruction != nil {
                            InstructionSection(model: model)
                        }
                        if let x = model.explanation, x.register != nil || !x.idioms.isEmpty {
                            ExplanationSection(model: model, explanation: x)
                        }
                        RegionSection(model: model)
                        if let preview = model.preview {
                            PreviewSection(model: model, preview: preview)
                        }
                        LabelSection(model: model)
                        CommentsSection(model: model)
                        XrefsSection(model: model)
                        ByteReadingsSection(model: model)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
                }
            } else {
                HeaderSummaryView(model: model)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

/// Shared helpers for the sections.
enum InspectorStyle {
    static func hex(_ v: UInt32, _ digits: Int) -> String {
        let s = String(v, radix: 16, uppercase: true)
        return String(repeating: "0", count: max(0, digits - s.count)) + s
    }

    @ViewBuilder
    static func row(_ label: String, _ value: String) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            Text(value).monospaced().textSelection(.enabled)
        }
    }

    static func chip(_ text: String, color: Color) -> some View {
        Text(text)
            .font(.caption)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(Capsule().fill(color.opacity(0.2)))
    }
}

struct SelectionHeader: View {
    let model: RomViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(model.selectedAddress.map { formatSnesAddress(address: $0) } ?? "unmapped")
                    .font(.title3.monospaced().weight(.semibold))
                Spacer()
                if let region = model.region {
                    RegionChip(region: region) { address in
                        model.jump(toSnesAddress: address)
                    }
                }
            }
            if let offset = model.selectedOffset {
                Text("file \(formatFileOffset(offset: offset))")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            }
            if let range = model.highlightedRange, range.count > 1 {
                Text("\(range.count) bytes selected")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// The region kind with an evidence popover.
struct RegionChip: View {
    let region: RegionInfo
    /// Jump to a dispatcher or a target the evidence names.
    var goTo: ((UInt32) -> Void)?
    @State private var showEvidence = false

    /// Strongest first. A region can carry several pieces of evidence and the
    /// one that decided it should lead; "why is this data?" is answered by the
    /// best answer, not the first one the pipeline happened to record.
    private var sorted: [EvidenceInfo] {
        region.evidence.sorted { a, b in
            if a.score != b.score { return a.score > b.score }
            return rank(a.kind) < rank(b.kind)
        }
    }

    /// Observation, then the user, then inference.
    private func rank(_ kind: EvidenceKind) -> Int {
        switch kind {
        case .user: 0
        case .trace: 1
        case .imported: 2
        case .vectorReach: 3
        case .heuristic: 4
        }
    }

    var body: some View {
        Button {
            showEvidence.toggle()
        } label: {
            InspectorStyle.chip(
                "\(region.name) \(Int((region.confidence * 100).rounded()))%",
                color: Color(nsColor: RegionPalette.color(
                    kind: region.kind == .code ? .code : (region.kind == .data ? .byte : .unknown),
                    confidence: Double(region.confidence)
                ))
            )
        }
        .buttonStyle(.plain)
        .popover(isPresented: $showEvidence) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Why \(region.name)?").font(.headline)
                if region.evidence.isEmpty {
                    Text("Nothing reached these bytes.").foregroundStyle(.secondary)
                }
                ForEach(Array(sorted.enumerated()), id: \.offset) { _, e in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Label(e.detail, systemImage: icon(for: e.kind))
                        if e.score > 0, e.score < 1 {
                            Text(String(format: "%.0f%%", e.score * 100))
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.secondary)
                        }
                        if let target = dispatcher(in: e.detail), let goTo {
                            Button("Go") { goTo(target) }
                                .buttonStyle(.link)
                                .help("Go to the instruction this evidence names")
                        }
                    }
                }
                Text("\(formatFileOffset(offset: region.start)) · \(region.len) bytes")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            }
            .padding()
            .frame(minWidth: 260)
        }
    }

    /// The `$bb:aaaa` a jump table's evidence names, as a SNES address.
    ///
    /// Parsing the detail string is not elegant, but the alternative — a typed
    /// back-reference on every evidence variant, across the FFI — is a lot of
    /// surface for one link, and the string is ours.
    private func dispatcher(in detail: String) -> UInt32? {
        guard let range = detail.range(of: #"\$[0-9A-F]{2}:[0-9A-F]{4}"#, options: .regularExpression)
        else { return nil }
        let text = detail[range].dropFirst()
        let parts = text.split(separator: ":")
        guard parts.count == 2,
              let bank = UInt32(parts[0], radix: 16),
              let offset = UInt32(parts[1], radix: 16)
        else { return nil }
        return bank << 16 | offset
    }

    private func icon(for kind: EvidenceKind) -> String {
        switch kind {
        case .vectorReach: "arrow.triangle.branch"
        case .heuristic: "sparkles"
        case .user: "person"
        case .imported: "square.and.arrow.down"
        case .trace: "waveform.path"
        }
    }
}

struct InstructionSection: View {
    let model: RomViewModel

    var body: some View {
        guard let insn = model.instruction else { return AnyView(EmptyView()) }
        return AnyView(
            VStack(alignment: .leading, spacing: 6) {
                Text(insn.text).font(.body.monospaced().weight(.medium))
                Text(insn.description).font(.callout).foregroundStyle(.secondary)
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 4) {
                    InspectorStyle.row("Mode", insn.mode)
                    InspectorStyle.row("Bytes", insn.bytes.map { InspectorStyle.hex(UInt32($0), 2) }.joined(separator: " "))
                    if let target = insn.target {
                        GridRow {
                            Text("Target").foregroundStyle(.secondary)
                            HStack {
                                Text(formatSnesAddress(address: target)).monospaced()
                                if !insn.targetCertain { Text("uncertain").font(.caption).foregroundStyle(.secondary) }
                                if insn.targetFileOffset != nil {
                                    Button("Go") { model.followReference() }.controlSize(.small)
                                }
                            }
                        }
                    }
                    if let reg = insn.hardwareRegister {
                        GridRow {
                            Text("Register").foregroundStyle(.secondary)
                            VStack(alignment: .leading) {
                                Text("\(reg.name) (\(accessName(reg.access)))").monospaced()
                                Text(reg.description).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .font(.callout)
                HStack(spacing: 8) {
                    FlagsChips(title: "before", flags: insn.flagsBefore)
                    Image(systemName: "arrow.right").foregroundStyle(.secondary)
                    FlagsChips(title: "after", flags: insn.flagsAfter)
                }
                ForEach(insn.assumptions, id: \.self) { a in
                    Label(a, systemImage: "questionmark.circle").font(.caption).foregroundStyle(.orange)
                }
                // Sorted by severity so a gap in the map is not lost among
                // the notes about decisions the analyzer made on purpose.
                ForEach(Array(model.warnings.sorted { a, b in
                    a.severity == .warning && b.severity == .info
                }.enumerated()), id: \.offset) { _, w in
                    Label(w.text, systemImage: w.severity == .warning
                        ? "exclamationmark.triangle"
                        : "info.circle")
                        .font(.caption)
                        .foregroundStyle(w.severity == .warning ? Color.orange : Color.secondary)
                }
            }
        )
    }

    private func accessName(_ a: Access) -> String {
        switch a {
        case .read: "R"
        case .write: "W"
        case .readWrite: "RW"
        }
    }
}

/// What the instruction does to the hardware, field by field, and the
/// idioms it is part of (docs/20).
struct ExplanationSection: View {
    let model: RomViewModel
    let explanation: ExplanationInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Explanation").font(.headline)
            if let r = explanation.register {
                RegisterAccessView(access: r)
            }
            ForEach(Array(explanation.idioms.enumerated()), id: \.offset) { _, idiom in
                IdiomView(model: model, idiom: idiom)
            }
        }
    }
}

struct RegisterAccessView: View {
    let access: RegisterAccessInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(access.parts.enumerated()), id: \.offset) { _, part in
                VStack(alignment: .leading, spacing: 4) {
                    Text(part.short).font(.body.monospaced().weight(.medium)).textSelection(.enabled)
                    Text(part.about).font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if part.twice {
                        Text("Written twice in a row: low byte, then high.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if !part.fields.isEmpty {
                        fields(part)
                    }
                }
            }
            if let source = access.source {
                Label("The value is loaded from \(access.sourceName ?? formatSnesAddress(address: source)).",
                      systemImage: "arrow.down.to.line")
                    .font(.caption).foregroundStyle(.secondary)
            } else if access.indexed {
                Label("Indexed: which register depends on X or Y.", systemImage: "questionmark.circle")
                    .font(.caption).foregroundStyle(.secondary)
            } else if !access.store {
                Label("A read: the fields show what it reports.", systemImage: "eye")
                    .font(.caption).foregroundStyle(.secondary)
            } else if access.value == nil {
                Label("The value is worked out at run time.", systemImage: "questionmark.circle")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func fields(_ part: RegisterPartInfo) -> some View {
        let known = part.value != nil
        return Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 3) {
            GridRow {
                Text("Bits")
                Text("Field")
                if known {
                    Text("Value")
                    Text("Meaning")
                }
            }
            .font(.caption).foregroundStyle(.tertiary)
            ForEach(Array(part.fields.enumerated()), id: \.offset) { _, f in
                GridRow(alignment: .firstTextBaseline) {
                    Text(f.bits).monospaced().foregroundStyle(.secondary)
                    Text(f.name)
                    if known {
                        Text(f.raw.map { String($0) } ?? "").monospaced()
                        Text(f.meaning ?? "").fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .font(.callout)
    }
}

struct IdiomView: View {
    let model: RomViewModel
    let idiom: IdiomInfo
    @State private var showWhy = true

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(idiom.title, systemImage: "lightbulb")
                .font(.body.weight(.semibold))
                .foregroundStyle(Color(nsColor: .systemIndigo))
            Text(idiom.summary).font(.callout).textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            DisclosureGroup("Why games do this", isExpanded: $showWhy) {
                Text(idiom.why).font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(.callout)
            Button("Select Its Instructions") { model.selectIdiom(noteAt: idiom.noteAt) }
                .controlSize(.small)
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .systemIndigo).opacity(0.08)))
    }
}

struct FlagsChips: View {
    let title: String
    let flags: FlagState

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.caption2).foregroundStyle(.tertiary)
            HStack(spacing: 3) {
                InspectorStyle.chip("M\(flags.m ? 1 : 0)", color: flags.m ? .orange : .blue)
                InspectorStyle.chip("X\(flags.x ? 1 : 0)", color: flags.x ? .orange : .blue)
                InspectorStyle.chip("E\(flags.e ? 1 : 0)", color: flags.e ? .red : .gray)
                InspectorStyle.chip(flags.dbr.map { "DBR $" + InspectorStyle.hex(UInt32($0), 2) } ?? "DBR ?", color: .gray)
                InspectorStyle.chip(flags.dp.map { "DP $" + InspectorStyle.hex(UInt32($0), 4) } ?? "DP ?", color: .gray)
            }
        }
        .font(.caption.monospaced())
    }
}

struct RegionSection: View {
    let model: RomViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Region").font(.headline)
            if let region = model.region {
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 4) {
                    InspectorStyle.row("Kind", region.name)
                    InspectorStyle.row("Confidence", "\(Int((region.confidence * 100).rounded()))%")
                    InspectorStyle.row("Range", "\(formatFileOffset(offset: region.start)) + \(region.len)")
                    InspectorStyle.row("Source", region.evidence.first?.detail ?? "none")
                }
                .font(.callout)
            }
            HStack {
                Text("Mark").foregroundStyle(.secondary)
                Button("Code") { model.mark(.code) }
                Button("Data") { model.mark(.data) }
                Button("Unknown") { model.mark(.unknown) }
                Button("Clear") { model.clearMark() }
            }
            .controlSize(.small)
            .font(.callout)
            HStack {
                Text("Flags").foregroundStyle(.secondary)
                if let f = model.flagOverride {
                    Text(flagText(f)).font(.caption.monospaced())
                } else {
                    Text("from analysis").font(.caption).foregroundStyle(.tertiary)
                }
                Button("Set Flags…") { model.activeSheet = .flags }.controlSize(.small)
            }
            .font(.callout)
        }
    }

    private func flagText(_ f: FlagOverride) -> String {
        var parts: [String] = []
        if let m = f.m { parts.append("M=\(m ? 1 : 0)") }
        if let x = f.x { parts.append("X=\(x ? 1 : 0)") }
        if let e = f.e { parts.append("E=\(e ? 1 : 0)") }
        if let d = f.dbr { parts.append("DBR=$" + InspectorStyle.hex(UInt32(d), 2)) }
        if let d = f.dp { parts.append("DP=$" + InspectorStyle.hex(UInt32(d), 4)) }
        return parts.joined(separator: " ")
    }
}

struct LabelSection: View {
    let model: RomViewModel
    @State private var name = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Label").font(.headline)
            HStack {
                TextField(model.label?.source == .auto ? model.label!.name : "name", text: $name)
                    .textFieldStyle(.roundedBorder)
                    .font(.body.monospaced())
                    .onSubmit(commit)
                if model.canRemoveLabel {
                    Button("Remove") { try? model.removeLabel() }.controlSize(.small)
                }
            }
            if let label = model.label, label.source != .user {
                Text(label.source == .auto ? "automatic name" : "imported from \(label.origin)")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .onChange(of: model.selectedAddress, initial: true) { _, _ in
            name = model.label?.source == .user ? model.label!.name : ""
        }
    }

    private func commit() {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        try? model.setLabel(name: trimmed)
    }
}

struct CommentsSection: View {
    let model: RomViewModel
    @State private var line = ""
    @State private var block = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Comments").font(.headline)
            TextField("Line comment", text: $line)
                .textFieldStyle(.roundedBorder)
                .onSubmit { try? model.setComment(kind: .line, text: line) }
            TextEditor(text: $block)
                .font(.body)
                .frame(minHeight: 48, maxHeight: 96)
                .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color(nsColor: .separatorColor)))
            HStack {
                Text("Block comment above the line").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Apply") { try? model.setComment(kind: .block, text: block) }.controlSize(.small)
            }
        }
        .onChange(of: model.selectedAddress, initial: true) { _, _ in
            line = model.lineComment?.text ?? ""
            block = model.blockComment?.text ?? ""
        }
        .onChange(of: model.lineComment?.text) { _, text in line = text ?? "" }
        .onChange(of: model.blockComment?.text) { _, text in block = text ?? "" }
    }
}

struct XrefsSection: View {
    let model: RomViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("References").font(.headline)
            if model.xrefsTo.isEmpty && model.xrefsFrom.isEmpty {
                Text("None").font(.callout).foregroundStyle(.secondary)
            }
            if !model.xrefsTo.isEmpty {
                Text("Referenced by").font(.caption).foregroundStyle(.secondary)
                ForEach(Array(model.xrefsTo.prefix(50).enumerated()), id: \.offset) { _, x in
                    Button {
                        model.jump(to: x.fromOffset)
                    } label: {
                        HStack {
                            Text(x.fromAddress.map { formatSnesAddress(address: $0) } ?? formatFileOffset(offset: x.fromOffset)).monospaced()
                            Text(x.kindName).foregroundStyle(.secondary)
                            if x.observed {
                                Text("seen").foregroundStyle(.tertiary).help("An execution log saw the game do this")
                            } else if !x.certain {
                                Text("?").foregroundStyle(.tertiary)
                            }
                        }
                        .font(.callout)
                    }
                    .buttonStyle(.plain)
                }
                if model.xrefsTo.count > 50 {
                    Button("Show all \(model.xrefsTo.count)") { model.findReferences() }
                        .buttonStyle(.link)
                        .font(.caption)
                        .help("List every reference in the results pane (⇧⌘F)")
                }
            }
            if !model.xrefsFrom.isEmpty {
                Text("References").font(.caption).foregroundStyle(.secondary)
                ForEach(Array(model.xrefsFrom.enumerated()), id: \.offset) { _, x in
                    Button {
                        if let off = x.toOffset { model.jump(to: off) }
                    } label: {
                        HStack {
                            Text(formatSnesAddress(address: x.toAddress)).monospaced()
                            Text(x.kindName).foregroundStyle(.secondary)
                        }
                        .font(.callout)
                    }
                    .buttonStyle(.plain)
                    .disabled(x.toOffset == nil)
                }
            }
        }
    }
}

/// The Phase 0 byte readings, collapsed when an instruction exists.
struct ByteReadingsSection: View {
    let model: RomViewModel
    @State private var expanded = false

    var body: some View {
        if let byte = model.inspection {
            DisclosureGroup(isExpanded: Binding(
                get: { expanded || model.instruction == nil },
                set: { expanded = $0 }
            )) {
                ByteReadingsGrid(model: model, byte: byte)
            } label: {
                Text("Byte readings").font(.headline)
            }
        }
    }
}

struct ByteReadingsGrid: View {
    let model: RomViewModel
    let byte: ByteInterpretation

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let name = byte.spanName {
                Label {
                    VStack(alignment: .leading) {
                        Text(name).fontWeight(.medium)
                        if let value = byte.spanValue {
                            Text(value).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                } icon: {
                    Image(systemName: "tag.fill")
                }
            }
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
                InspectorStyle.row("File offset", formatFileOffset(offset: byte.fileOffset))
                if byte.diskOffset != byte.fileOffset {
                    InspectorStyle.row("On disk", formatFileOffset(offset: byte.diskOffset))
                }
                InspectorStyle.row("SNES address", byte.snesAddress.map { formatSnesAddress(address: $0) } ?? "unmapped")
                InspectorStyle.row("Mirrors", byte.mirrors.map { formatSnesAddress(address: $0) }.joined(separator: ", "))
                Divider().gridCellUnsizedAxes(.horizontal)
                InspectorStyle.row("u8", "\(byte.valueU8)  ($\(InspectorStyle.hex(UInt32(byte.valueU8), 2)))")
                InspectorStyle.row("i8", "\(byte.valueI8)")
                InspectorStyle.row("u16 LE", byte.valueU16Le.map { "\($0)  ($\(InspectorStyle.hex(UInt32($0), 4)))" } ?? "—")
                InspectorStyle.row("i16 LE", byte.valueI16Le.map { "\($0)" } ?? "—")
                InspectorStyle.row("u24 LE", byte.valueU24Le.map { "\($0)  ($\(InspectorStyle.hex($0, 6)))" } ?? "—")
                InspectorStyle.row("ASCII", byte.ascii.map { "\"\($0)\"" } ?? "—")
                Divider().gridCellUnsizedAxes(.horizontal)
                addressRow("u16 in bank", byte.u16AsAddressInBank)
                addressRow("u24 as address", byte.u24AsSnesAddress, region: byte.pointerTargetRegion)
                if let target = byte.pointerTargetFileOffset {
                    GridRow {
                        Text("Points to").foregroundStyle(.secondary)
                        HStack {
                            Text(formatFileOffset(offset: target)).monospaced()
                            Button("Go") { model.jump(to: target) }.controlSize(.small)
                        }
                    }
                }
            }
            .font(.callout)
        }
    }

    @ViewBuilder
    private func addressRow(_ label: String, _ address: UInt32?, region: String? = nil) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            if let address {
                HStack {
                    Text(formatSnesAddress(address: address)).monospaced()
                    if let region { Text(region).font(.caption).foregroundStyle(.secondary) }
                    if model.rom.fileOffsetFor(snesAddress: address) != nil {
                        Button("Go") { model.jump(toSnesAddress: address) }.controlSize(.small)
                    }
                }
            } else {
                Text("—")
            }
        }
    }
}

/// What a typed range looks like (checklist 2.25, 2.27), with a way into the
/// full view. Compressed data previews its decompressed output, so this is
/// also where "decompress and preview" lives.
struct PreviewSection: View {
    let model: RomViewModel
    let preview: PreviewInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Preview").font(.headline)
            if let bitmap = preview.bitmap {
                PixelImage(bitmap: bitmap, scale: scale(for: bitmap))
            }
            Text(preview.summary)
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(openTitle) { model.open(preview: preview) }
                    .disabled(preview.kind == "compressed" && preview.decompressed == nil)
                Button("Options…") { showingOptions = true }
                    .disabled(model.markedRangeForPreview == nil)
                    .help(model.markedRangeForPreview == nil
                        ? "Mark the range first; preview options belong to a mark"
                        : "Palette, tiles across, and a tilemap's size and tiles")
                    .popover(isPresented: $showingOptions, arrowEdge: .leading) {
                        PreviewOptionsForm(model: model, preview: preview)
                    }
            }
            .controlSize(.small)
        }
    }

    @State private var showingOptions = false

    private var openTitle: String {
        let view = switch preview.view {
        case .tileDecoder: "Tile Decoder"
        case .palette: "Palette"
        case .tilemap: "Tilemap"
        }
        return preview.kind == "compressed" ? "Decompress and Open in \(view)" : "Open in \(view)"
    }

    /// Fit the inspector's width: whole-number scales only, so pixels stay
    /// square.
    private func scale(for bitmap: BitmapInfo) -> CGFloat {
        let fit = 256 / CGFloat(max(bitmap.width, 1))
        return max(1, min(8, fit.rounded(.down)))
    }
}

/// The marked range's preview options. Addresses are typed the way the jump
/// sheet takes them and resolved by the core; an empty field is the default.
struct PreviewOptionsForm: View {
    let model: RomViewModel
    let preview: PreviewInfo
    @State private var palette = ""
    @State private var tiles = ""
    @State private var columns = 16
    @State private var size: ScreenSize = .s32x32
    @State private var error: String?

    var body: some View {
        Form {
            TextField("Palette at", text: $palette, prompt: Text("grayscale"))
            if preview.kind == "tilemap" {
                TextField("Tiles at", text: $tiles, prompt: Text("none"))
                Picker("Size", selection: $size) {
                    ForEach([ScreenSize.s32x32, .s64x32, .s32x64, .s64x64], id: \.self) { Text($0.title).tag($0) }
                }
            } else {
                Stepper("\(columns) tiles across", value: $columns, in: 1...64)
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(.red)
            }
            HStack {
                Spacer()
                Button("Set") { apply() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding()
        .frame(width: 280)
        .onAppear(perform: load)
    }

    private func load() {
        guard let range = model.markedRangeForPreview,
              let p = model.workbench.regionParamsAt(fileOffset: range.start) else { return }
        palette = p.palette.map { formatSnesAddress(address: $0) } ?? ""
        tiles = p.tiles.map { formatSnesAddress(address: $0) } ?? ""
        columns = Int(p.columns ?? 16)
        size = p.screenSize ?? .s32x32
    }

    private func address(_ text: String) throws -> UInt32? {
        let t = text.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty else { return nil }
        let r = try model.rom.resolve(text: t)
        return r.snesAddress ?? model.rom.snesAddressFor(fileOffset: r.fileOffset)
    }

    private func apply() {
        do {
            let isMap = preview.kind == "tilemap"
            try model.setPreviewOptions(RegionParamsInfo(
                palette: try address(palette),
                columns: isMap ? nil : UInt16(columns),
                screenSize: isMap ? size : nil,
                tiles: isMap ? try address(tiles) : nil
            ))
            error = nil
        } catch let e as RomlensError {
            error = "\(e)"
        } catch {
            self.error = error.localizedDescription
        }
    }
}

import AppKit
import Foundation
import Observation
import RomlensKit

/// Per-document state shared by the hex and disassembly canvases, the
/// navigator, the inspector and the sheets. Selection and jump history stay
/// here (shell-side) in Phase 1; the core provides the line/offset mappings.
@MainActor
@Observable
final class RomViewModel {
    struct ScrollRequest: Equatable {
        let id: Int
        let offset: UInt32
        /// Compatibility for hex-only callers.
        var row: UInt32 { offset / 16 }
    }

    enum EditorTab: String, CaseIterable, Identifiable {
        case hex, disassembly, both, c, graph
        var id: String { rawValue }
        var title: String {
            switch self {
            case .hex: "Hex"
            case .disassembly: "Disassembly"
            case .both: "Both"
            case .c: "C"
            case .graph: "Graph"
            }
        }
    }

    enum Sheet: Identifiable {
        case jump, renameLabel, comment, flags, find, dataType, variable
        var id: Self { self }
    }

    /// The one-case right pane keeps room for the tutor later.
    enum RightPane: Hashable {
        case inspector
    }

    let rom: Rom
    let info: RomInfo
    let spans: [Span]
    let palette: SpanPalette
    let rowCount: UInt32
    let session: WorkbenchSession
    let navigator = NavigatorModel()
    let search = SearchModel()
    let references = ReferencesModel()
    /// The C tab's routine and text (docs/18).
    let decompiler = DecompileModel()
    let graph = GraphModel()
    @ObservationIgnored let cache: HexRowCache
    @ObservationIgnored let asmCache: AsmLineCache
    let metrics = MonoMetrics()

    var addressStyle: AddressStyle = .both {
        didSet {
            guard addressStyle != oldValue else { return }
            layout = HexRowLayout(style: addressStyle, metrics: metrics)
            asmLayout = AsmLineLayout(style: addressStyle, metrics: metrics)
            lineGeneration += 1
            asmGeneration += 1
        }
    }
    private(set) var layout: HexRowLayout
    private(set) var asmLayout: AsmLineLayout
    /// Bumped whenever cached hex `CTLine`s must be rebuilt.
    private(set) var lineGeneration = 0
    /// Bumped whenever cached asm lines must be rebuilt (also on snapshot change).
    private(set) var asmGeneration = 0
    private(set) var asmLineCount: UInt32 = 0

    private(set) var selectedOffset: UInt32?
    private(set) var selectionAnchor: UInt32?
    private(set) var inspection: ByteInterpretation?
    private(set) var instruction: InstructionInfo?
    private(set) var region: RegionInfo?
    private(set) var label: LabelInfo?
    private(set) var lineComment: CommentInfo?
    private(set) var blockComment: CommentInfo?
    private(set) var xrefsTo: [XRefInfo] = []
    private(set) var xrefsFrom: [XRefInfo] = []
    private(set) var warnings: [WarningInfo] = []
    private(set) var flagOverride: FlagOverride?
    private(set) var preview: PreviewInfo?
    private(set) var history: [UInt32] = []
    private(set) var forwardHistory: [UInt32] = []
    private(set) var scrollRequest: ScrollRequest?
    /// Choosing a text tab closes any graphics view, which is how the
    /// segmented control and the Graphics picker share the editor area.
    var editorTab: EditorTab = .hex {
        didSet {
            graphicsTab = nil
            refreshDecompile()
        }
    }
    /// The graphics view in the editor area, if one is open.
    var graphicsTab: GraphicsModel.Tab?
    let graphics: GraphicsModel
    var activeSheet: Sheet?
    var rightPane: RightPane = .inspector
    var isNavigatorVisible = true
    var isInspectorVisible = true
    var isResultsVisible = false
    /// Which list the results pane shows: the last Find, or the last Find
    /// References. Each keeps its own list, so switching loses neither.
    var resultsKind: ResultsKind = .find
    enum ResultsKind { case find, references }
    var isStripVisible = true
    /// What Focus on Code hid, to put back when it is turned off.
    private var unfocused: (navigator: Bool, inspector: Bool, strip: Bool, results: Bool)?
    /// Only the editor showing: the sidebars, the strip and the results
    /// pane are hidden.
    var isFocused: Bool { unfocused != nil }

    /// Focus on Code: hide everything but the editor, or put it all back.
    func toggleFocus() {
        if let saved = unfocused {
            isNavigatorVisible = saved.navigator
            isInspectorVisible = saved.inspector
            isStripVisible = saved.strip
            isResultsVisible = saved.results
            unfocused = nil
        } else {
            unfocused = (isNavigatorVisible, isInspectorVisible, isStripVisible, isResultsVisible)
            isNavigatorVisible = false
            isInspectorVisible = false
            isStripVisible = false
            isResultsVisible = false
        }
    }
    /// Bumped when the strip's data is stale; it re-reduces rather than
    /// redrawing what the last analysis said.
    private(set) var stripGeneration = 0

    /// Compatibility with the Phase 0 jump sheet binding.
    var isShowingJumpSheet: Bool {
        get { activeSheet == .jump }
        set { activeSheet = newValue ? .jump : (activeSheet == .jump ? nil : activeSheet) }
    }

    init(rom: Rom, workbench: Workbench? = nil, cacheCapacity: Int = 64, startAnalysis: Bool = true) {
        self.rom = rom
        info = rom.info()
        spans = rom.spans()
        palette = SpanPalette(spans: spans)
        rowCount = rom.rowCount()
        let workbench = workbench ?? Workbench(rom: rom)
        session = WorkbenchSession(workbench: workbench)
        cache = HexRowCache(workbench: workbench, capacity: cacheCapacity)
        asmCache = AsmLineCache(workbench: workbench, capacity: cacheCapacity)
        layout = HexRowLayout(style: .both, metrics: metrics)
        asmLayout = AsmLineLayout(style: .both, metrics: metrics)
        asmLineCount = workbench.lineCount()
        graphics = GraphicsModel(rom: rom)
        session.onChange = { [weak self] kind in self?.handleChange(kind) }
        graphics.selectBytes = { [weak self] range in self?.selectRange(range) }
        graphics.revealTile = { [weak self] in self?.graphicsTab = .tiles }
        if startAnalysis {
            session.startAnalysis()
        }
    }

    var workbench: Workbench { session.workbench }
    var byteCount: UInt32 { info.byteLen }
    var canGoBack: Bool { !history.isEmpty }
    var canGoForward: Bool { !forwardHistory.isEmpty }
    /// The disassembly exists once the first analysis has landed.
    var hasDisassembly: Bool { session.hasSnapshot && asmLineCount > 0 }

    /// Bytes to highlight: the shift-selected range, else the selected
    /// instruction's bytes, else the byte.
    var highlightedRange: Range<UInt32>? {
        guard let selected = selectedOffset else { return nil }
        if let anchor = selectionAnchor {
            let lo = min(anchor, selected)
            let hi = max(anchor, selected)
            return lo..<min(hi + 1, byteCount)
        }
        if let insn = instruction, insn.fileOffset <= selected, selected < insn.fileOffset + UInt32(insn.len) {
            return insn.fileOffset..<insn.fileOffset + UInt32(insn.len)
        }
        return selected..<selected + 1
    }

    // MARK: Caches

    func batch(containingRow row: UInt32) -> HexBatch {
        cache.batch(containingRow: row)
    }

    func asmBatch(containingLine line: UInt32) -> AsmBatch {
        asmCache.batch(containingRow: line)
    }

    private func handleChange(_ kind: WorkbenchSession.ChangeKind) {
        switch kind {
        case .snapshot, .view:
            cache.invalidateAll()
            asmCache.invalidateAll()
            asmLineCount = workbench.lineCount()
            asmGeneration += 1
            lineGeneration += 1
            stripGeneration += 1
            refreshSelectionDetails()
            decompiler.invalidate()
            graph.invalidate()
            refreshDecompile()
            Task { await navigator.reload(workbench: workbench, rom: rom) }
        case .project:
            break
        }
    }

    // MARK: C and Graph

    /// Keep the C and Graph tabs on the routine at the selection. Only while
    /// a tab is showing: decompiling costs a summary of every routine the
    /// first time after an analysis.
    func refreshDecompile() {
        refreshGraph()
        guard editorTab == .c, graphicsTab == nil, hasDisassembly else { return }
        decompiler.follow(
            workbench: workbench,
            instructionStart: instruction?.fileOffset ?? selectedOffset,
            generation: asmGeneration
        )
    }

    /// Keep the Graph tab on the routine at the selection, while it shows.
    func refreshGraph() {
        guard editorTab == .graph, graphicsTab == nil, hasDisassembly else { return }
        graph.follow(
            workbench: workbench,
            instructionStart: instruction?.fileOffset ?? selectedOffset,
            generation: asmGeneration
        )
    }

    /// Show Graph: the Graph tab on the routine at the selection.
    func showGraph() {
        editorTab = .graph
    }

    /// Decompile Routine: the C tab on the routine at the selection.
    func showDecompiled() {
        editorTab = .c
    }

    // MARK: Selection

    /// Select a byte without scrolling (mouse, arrow keys). Clears any range.
    func select(offset: UInt32?) {
        selectionAnchor = nil
        setSelected(offset)
    }

    /// Select a byte range without scrolling or leaving the current view:
    /// what a click in a graphics view does, so the hex view shows the same
    /// bytes when you go back to it.
    func selectRange(_ range: Range<UInt32>) {
        guard range.lowerBound < byteCount, !range.isEmpty else { return }
        select(offset: range.lowerBound)
        if range.count > 1 {
            extendSelection(to: min(range.upperBound, byteCount) - 1)
        }
    }

    /// Open a graphics view on the ROM bytes at the selection.
    func openGraphics(_ tab: GraphicsModel.Tab) {
        if graphics.source == .rom, let range = highlightedRange {
            graphics.romOffset = range.lowerBound
        }
        graphicsTab = tab
    }

    /// The inspector's "Open in …": the preview's view on the range it
    /// previewed, or on the decompressed bytes for compressed data.
    func open(preview: PreviewInfo) {
        if let data = preview.decompressed {
            graphics.source = .bytes(label: "Decompressed", data: data)
        } else {
            graphics.source = .rom
            graphics.romOffset = preview.start
        }
        if let format = preview.format { graphics.format = format }
        if let params = workbench.regionParamsAt(fileOffset: preview.start) {
            if let columns = params.columns { graphics.columns = Int(columns) }
            if let size = params.screenSize { graphics.screenSize = size }
            if let palette = params.palette, let offset = rom.fileOffsetFor(snesAddress: palette) {
                graphics.palette = .rom(offset)
            }
        }
        graphics.selectedTile = 0
        switch preview.view {
        case .tileDecoder: graphicsTab = .tiles
        case .palette: graphicsTab = .palette
        case .tilemap: graphicsTab = .tilemap
        }
    }

    /// Extend the range from the current selection (shift-click, shift+arrows).
    func extendSelection(to offset: UInt32) {
        guard offset < byteCount else { return }
        if selectionAnchor == nil { selectionAnchor = selectedOffset ?? offset }
        setSelected(offset)
    }

    private func setSelected(_ offset: UInt32?) {
        guard let offset else {
            selectedOffset = nil
            inspection = nil
            clearDetails()
            return
        }
        guard offset < byteCount else { return }
        selectedOffset = offset
        inspection = rom.inspect(fileOffset: offset)
        refreshSelectionDetails()
        refreshDecompile()
    }

    private func clearDetails() {
        instruction = nil
        region = nil
        label = nil
        lineComment = nil
        blockComment = nil
        xrefsTo = []
        xrefsFrom = []
        warnings = []
        flagOverride = nil
        preview = nil
    }

    private func refreshSelectionDetails() {
        guard let offset = selectedOffset else { return }
        instruction = workbench.instructionAt(fileOffset: offset)
        region = workbench.regionAt(fileOffset: offset)
        let itemStart = instruction?.fileOffset ?? offset
        if let address = rom.snesAddressFor(fileOffset: itemStart) {
            label = workbench.labelAt(snesAddress: address)
            lineComment = workbench.commentAt(snesAddress: address, kind: .line)
            blockComment = workbench.commentAt(snesAddress: address, kind: .block)
            xrefsTo = workbench.xrefsTo(snesAddress: address)
        } else {
            label = nil
            lineComment = nil
            blockComment = nil
            xrefsTo = []
        }
        xrefsFrom = workbench.xrefsFrom(fileOffset: itemStart)
        warnings = workbench.warningsAt(fileOffset: itemStart)
        flagOverride = workbench.flagOverrideAt(fileOffset: itemStart)
        preview = workbench.previewAt(fileOffset: offset)
    }

    /// The address of the selected item (instruction start or byte).
    var selectedAddress: UInt32? {
        guard let offset = selectedOffset else { return nil }
        return rom.snesAddressFor(fileOffset: instruction?.fileOffset ?? offset)
    }

    /// Move the selection by `delta` bytes, clamped, and keep it visible.
    func moveSelection(by delta: Int, extend: Bool = false) {
        let current = Int(selectedOffset ?? 0)
        let next = UInt32(min(max(current + delta, 0), Int(byteCount) - 1))
        if extend { extendSelection(to: next) } else { select(offset: next) }
        requestScroll(toOffset: next)
    }

    /// Move to the previous or next content line of the disassembly.
    func moveLine(by delta: Int, extend: Bool = false) {
        guard asmLineCount > 0 else { return }
        let current = selectedOffset.flatMap { workbench.lineForOffset(fileOffset: $0) } ?? 0
        var line = Int(current)
        var steps = abs(delta)
        let step = delta < 0 ? -1 : 1
        var landed: UInt32? = nil
        while steps > 0 {
            let next = line + step
            guard next >= 0, next < Int(asmLineCount) else { break }
            line = next
            if let range = workbench.itemRange(line: UInt32(line)), range.len > 0 {
                steps -= 1
                landed = range.start
            }
        }
        guard let target = landed else { return }
        if extend { extendSelection(to: target) } else { select(offset: target) }
        requestScroll(toOffset: target)
    }

    // MARK: Navigation

    /// Jump to a file offset: select it, centre it, remember where we were.
    func jump(to offset: UInt32, recordHistory: Bool = true) {
        guard offset < byteCount else { return }
        if recordHistory, let from = selectedOffset, from != offset {
            history.append(from)
            if history.count > 100 { history.removeFirst() }
            forwardHistory.removeAll()
        }
        select(offset: offset)
        requestScroll(toOffset: offset)
    }

    /// Jump to a 24-bit SNES address when it maps to ROM.
    func jump(toSnesAddress address: UInt32) {
        if let offset = rom.fileOffsetFor(snesAddress: address) {
            jump(to: offset)
        }
    }

    /// Resolve and jump to an address expression; throws the core's message.
    @discardableResult
    func jump(text: String) throws -> ResolvedAddress {
        let resolved = try rom.resolve(text: text)
        jump(to: resolved.fileOffset)
        return resolved
    }

    /// Live preview for the jump sheet: what the expression resolves to.
    func preview(text: String) -> Result<ResolvedAddress, RomlensError> {
        do {
            return .success(try rom.resolve(text: text))
        } catch let error as RomlensError {
            return .failure(error)
        } catch {
            return .failure(.BadAddress(msg: error.localizedDescription))
        }
    }

    func goBack() {
        guard let previous = history.popLast() else { return }
        if let current = selectedOffset { forwardHistory.append(current) }
        jump(to: previous, recordHistory: false)
    }

    func goForward() {
        guard let next = forwardHistory.popLast() else { return }
        if let current = selectedOffset { history.append(current) }
        jump(to: next, recordHistory: false)
    }

    /// Follow the selected instruction's target (or a pointer's).
    func followReference() {
        if let target = instruction?.targetFileOffset {
            jump(to: target)
        } else if let p = inspection?.pointerTargetFileOffset, instruction == nil {
            jump(to: p)
        }
    }

    func requestScroll(toOffset offset: UInt32) {
        scrollRequest = ScrollRequest(id: (scrollRequest?.id ?? 0) + 1, offset: offset)
    }

    // MARK: Key commands from either canvas

    func perform(_ command: EditorKeyCommand, from source: EditorSource, visibleItems: Int = 20) {
        switch command {
        case .left: moveSelection(by: -1)
        case .right: moveSelection(by: 1)
        case .extendLeft: moveSelection(by: -1, extend: true)
        case .extendRight: moveSelection(by: 1, extend: true)
        case .up: source == .hex ? moveSelection(by: -16) : moveLine(by: -1)
        case .down: source == .hex ? moveSelection(by: 16) : moveLine(by: 1)
        case .extendUp: source == .hex ? moveSelection(by: -16, extend: true) : moveLine(by: -1, extend: true)
        case .extendDown: source == .hex ? moveSelection(by: 16, extend: true) : moveLine(by: 1, extend: true)
        case .pageUp: source == .hex ? moveSelection(by: -16 * max(1, visibleItems)) : moveLine(by: -max(1, visibleItems))
        case .pageDown: source == .hex ? moveSelection(by: 16 * max(1, visibleItems)) : moveLine(by: max(1, visibleItems))
        case .home: jump(to: 0)
        case .end: jump(to: byteCount - 1)
        case .back: goBack()
        case .follow: followReference()
        case .rename: if selectedOffset != nil { activeSheet = .renameLabel }
        case .comment: if selectedOffset != nil { activeSheet = .comment }
        case .markCode: mark(.code)
        case .markData: mark(.data)
        case .markUnknown: mark(.unknown)
        }
    }

    // MARK: Editing

    /// Mark the highlighted range (or the selected item) with a kind.
    func mark(
        _ kind: OverrideKind,
        dataKind: DataKind = .byte,
        stride: UInt8? = nil,
        bpp: UInt8? = nil,
        elem: TableElem? = nil,
        bank: BankRule? = nil
    ) {
        guard let range = highlightedRange else { return }
        try? session.mark(
            start: range.lowerBound,
            len: UInt32(range.count),
            kind: kind,
            dataKind: kind == .data ? dataKind : nil,
            stride: stride,
            bpp: bpp,
            elem: elem,
            bank: bank
        )
        refreshSelectionDetails()
    }

    // MARK: Find

    func runSearch() {
        resultsKind = .find
        search.search(in: workbench)
        if let hit = search.hits.first {
            jump(to: hit.fileOffset)
        }
    }

    /// ⌘G / ⇧⌘G.
    func stepSearch(by delta: Int) {
        guard let hit = search.step(by: delta) else { return }
        jump(to: hit.fileOffset)
    }

    func goToHit(at index: Int) {
        guard let hit = search.select(index) else { return }
        jump(to: hit.fileOffset)
    }

    // MARK: Find References

    /// What Find References would look for: the selected item's label, or
    /// its address, with how many references there are.
    var referenceTarget: (name: String, count: Int)? {
        guard let address = selectedAddress else { return nil }
        return (label?.name ?? formatSnesAddress(address: address), xrefsTo.count)
    }

    /// List everything that refers to the selected item, in the results
    /// pane. The selection stays where it is until a row is chosen.
    func findReferences() {
        guard let address = selectedAddress else { return }
        references.find(to: address, in: workbench)
        resultsKind = .references
        isResultsVisible = true
    }

    func goToReference(at index: Int) {
        guard let row = references.select(index) else { return }
        jump(to: row.fileOffset)
    }

    /// The marked range the selection is in, whose preview options can be
    /// set. `nil` for a range only the analyzer typed: options live on a
    /// user's mark, so there has to be one.
    var markedRangeForPreview: ByteRange? {
        guard let offset = selectedOffset else { return nil }
        return workbench.regionOverrideAt(fileOffset: offset)
    }

    /// Set how the marked range previews; undoable, and never re-analyzes.
    func setPreviewOptions(_ params: RegionParamsInfo) throws {
        guard let range = markedRangeForPreview else { return }
        try session.setRegionParams(start: range.start, params: params)
        refreshSelectionDetails()
    }

    func clearMark() {
        guard let range = highlightedRange else { return }
        try? session.clearMark(start: range.lowerBound, len: UInt32(range.count))
        refreshSelectionDetails()
    }

    func setLabel(name: String?) throws {
        guard let address = selectedAddress else { return }
        try session.setLabel(address: address, name: name)
        refreshSelectionDetails()
    }

    /// A label a person or an import chose, which Remove Label takes away.
    var canRemoveLabel: Bool {
        guard let label else { return false }
        return label.source == .user || label.source == .imported
    }

    /// Remove the selected item's label. A variable's label goes with its
    /// type, as one step, since a type without a name names nothing.
    func removeLabel() throws {
        guard let address = selectedAddress, canRemoveLabel else { return }
        if workbench.variables().contains(where: { $0.address == workbench.canonicalAddress(snesAddress: address) }) {
            try session.removeVariable(address: address)
        } else {
            try session.setLabel(address: address, name: nil)
        }
        refreshSelectionDetails()
    }

    // MARK: Variables

    /// What the Define Variable sheet edits.
    struct VariableDraft: Equatable {
        var address = ""
        var name = ""
        var width: VarWidth = .byte
        var count: UInt16 = 1
        /// The variable being edited, if any; its address cannot change.
        var existing: UInt32?
    }

    var variableDraft = VariableDraft()

    /// The data address the selected instruction's operand names, canonical:
    /// `STA $0094` run from bank $80 gives $7E:0094.
    var operandAddress: UInt32? {
        guard let target = instruction?.target, instruction?.targetKind != .code else { return nil }
        return workbench.canonicalAddress(snesAddress: target)
    }

    /// Open Define Variable for `address`, or for the selected operand, or
    /// empty. An address inside a variable edits that variable.
    func beginDefineVariable(at address: UInt32? = nil) {
        let at = address ?? operandAddress
        var draft = VariableDraft()
        if let at {
            if let v = workbench.variableContaining(snesAddress: at) {
                draft = VariableDraft(
                    address: formatSnesAddress(address: v.address), name: v.name,
                    width: v.width, count: v.count, existing: v.address)
            } else {
                draft.address = formatSnesAddress(address: at)
                draft.name = workbench.labelAt(snesAddress: at).flatMap { $0.source == .auto ? nil : $0.name } ?? ""
            }
        }
        variableDraft = draft
        activeSheet = .variable
    }

    /// The Variables list's + button: always a new, empty definition, never
    /// the variable the selection happens to be in.
    func beginNewVariable() {
        variableDraft = VariableDraft()
        activeSheet = .variable
    }

    /// `$7E:0094`, `7E0094` or `$0094`; four digits or fewer below $2000 is
    /// low RAM (bank $7E), otherwise a register in bank $00.
    nonisolated static func parseAddress(_ text: String) -> UInt32? {
        let hex = text.filter { $0 != "$" && $0 != ":" && !$0.isWhitespace }
        guard !hex.isEmpty, hex.count <= 6, let v = UInt32(hex, radix: 16) else { return nil }
        if hex.count <= 4 { return v < 0x2000 ? 0x7E_0000 | v : v }
        return v
    }

    /// Define (or redefine) the variable the draft describes.
    func defineVariable(_ draft: VariableDraft) throws {
        guard let address = draft.existing ?? Self.parseAddress(draft.address) else {
            throw RomlensError.BadAddress(msg: "\(draft.address) is not an address; use a form like $7E:0094")
        }
        try session.defineVariable(
            address: address,
            name: draft.name.trimmingCharacters(in: .whitespaces),
            type: VarTypeInfo(width: draft.width, count: max(1, draft.count)))
        refreshSelectionDetails()
    }

    func removeVariable(address: UInt32) throws {
        try session.removeVariable(address: address)
        refreshSelectionDetails()
    }

    func setComment(kind: CommentKind, text: String?) throws {
        guard let address = selectedAddress else { return }
        try session.setComment(address: address, kind: kind, text: text)
        refreshSelectionDetails()
    }

    func setFlagOverride(_ flags: FlagOverride?) throws {
        guard let offset = instruction?.fileOffset ?? selectedOffset else { return }
        try session.setFlagOverride(offset: offset, flags: flags)
        refreshSelectionDetails()
    }

    func undo() {
        session.undo()
        refreshSelectionDetails()
    }

    func redo() {
        session.redo()
        refreshSelectionDetails()
    }

    /// The selected address as text, for Copy Address.
    var selectedAddressText: String? {
        selectedAddress.map { formatSnesAddress(address: $0) }
    }

    /// The selected line as text, for Copy Line.
    var selectedLineText: String? {
        guard let offset = selectedOffset, let line = workbench.lineForOffset(fileOffset: offset) else { return nil }
        return workbench.asmLinesText(startLine: line, count: 1, style: .both).trimmingCharacters(in: .newlines)
    }
}

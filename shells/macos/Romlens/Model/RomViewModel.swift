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
        /// The tabs that should scroll: the focused tab and those following
        /// the selection (docs/29). Nil for every view.
        var targets: Set<UUID>? = nil
        /// Whether a view in the tab `item` should act on it. A view that
        /// names no tab acts on every request.
        func applies(to item: UUID?) -> Bool {
            guard let targets, let item else { return true }
            return targets.contains(item)
        }
        /// Compatibility for hex-only callers.
        var row: UInt32 { offset / 16 }
    }

    enum EditorTab: String, CaseIterable, Identifiable {
        case hex, disassembly, both, c, graph, source, atlas, compare
        var id: String { rawValue }
        /// What a tab of this shows (docs/29).
        var content: EditorContent {
            switch self {
            case .hex: .code(.hex)
            case .disassembly: .code(.assembly)
            case .both: .code(.both)
            case .c: .code(.c)
            case .graph: .code(.graph)
            case .source: .source
            case .atlas: .atlas
            case .compare: .compare
            }
        }
        init?(content: EditorContent) {
            switch content {
            case .code(.hex): self = .hex
            case .code(.assembly): self = .disassembly
            case .code(.both): self = .both
            case .code(.c): self = .c
            case .code(.graph): self = .graph
            case .source: self = .source
            case .atlas: self = .atlas
            case .compare: self = .compare
            default: return nil
            }
        }
        var title: String {
            switch self {
            case .hex: "Hex"
            case .disassembly: "Disassembly"
            case .both: "Both"
            case .c: "C"
            case .graph: "Graph"
            case .source: "Source"
            case .atlas: "Atlas"
            case .compare: "Compare"
            }
        }
    }

    enum Sheet: Identifiable {
        case jump, renameLabel, comment, flags, find, dataType, variable, cEdit
        var id: Self { self }
    }

    /// What the C sheet edits (docs/24, U10): a local's name, a routine's
    /// note, a C comment, or a C version.
    enum CEdit: Equatable {
        case local(routine: UInt32, local: String)
        case note(routine: UInt32)
        case comment(address: UInt32)
        case version(routine: UInt32, name: String?)
    }

    /// The C sheet's subject while it is open.
    var cEdit: CEdit?

    func beginCEdit(_ e: CEdit) {
        cEdit = e
        activeSheet = .cEdit
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
    /// Brings the project's main window to the front: set by its window
    /// controller, used when the tutor's citation is followed.
    @ObservationIgnored var bringMainWindowForward: (() -> Void)?
    let navigator = NavigatorModel()
    let search = SearchModel()
    let references = ReferencesModel()
    /// The window's tabs and their layout (docs/29).
    let workspace = Workspace()
    /// The C of the focused code tab, or the last one focused (docs/18).
    var decompiler: DecompileModel { workspace.decompiler(for: nil) }
    /// The graph of the focused code tab, or the last one focused.
    var graph: GraphModel { workspace.graph(for: nil) }
    let atlas = AtlasModel()
    let compare = CompareModel()
    /// The Source tab's files, from the project's imported `.dbg` files.
    let source = SourceModel()

    /// The tabs the toolbar offers: Source only with sources imported,
    /// Compare only while comparing.
    var editorTabs: [EditorTab] {
        EditorTab.allCases.filter {
            switch $0 {
            case .compare: compare.isActive
            case .source: source.hasFiles
            default: true
            }
        }
    }
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
    /// What the selected instruction does to the hardware, and the idioms
    /// it is part of (docs/20).
    private(set) var explanation: ExplanationInfo?
    /// What the screen is set up to be at the selected instruction
    /// (docs/21), worked out only while the inspector's Screen section is
    /// open.
    private(set) var screen: ScreenSetupInfo?
    private(set) var screenLoading = false
    /// The Screen section is open.
    var showScreen = false {
        didSet { if showScreen != oldValue { refreshScreen() } }
    }
    @ObservationIgnored private var screenTask: Task<Void, Never>?
    @ObservationIgnored private var screenFor: UInt32?
    private(set) var region: RegionInfo?
    private(set) var label: LabelInfo?
    private(set) var lineComment: CommentInfo?
    private(set) var blockComment: CommentInfo?
    private(set) var xrefsTo: [XRefInfo] = []
    private(set) var xrefsFrom: [XRefInfo] = []
    private(set) var warnings: [WarningInfo] = []
    private(set) var flagOverride: FlagOverride?
    private(set) var preview: PreviewInfo?
    /// Where Back goes: an offset, and the tab it was seen in.
    struct HistoryEntry: Equatable {
        let offset: UInt32
        let item: UUID?
    }
    private(set) var historyEntries: [HistoryEntry] = []
    private(set) var forwardEntries: [HistoryEntry] = []
    var history: [UInt32] { historyEntries.map(\.offset) }
    var forwardHistory: [UInt32] { forwardEntries.map(\.offset) }
    private(set) var scrollRequest: ScrollRequest?

    // MARK: The focused tab, as the one editor it used to be

    /// The text view of the focused tab, or the last text view while a
    /// graphics, sound or tutor tab has focus. Setting it shows that view in
    /// the focused group: a code representation changes the focused code
    /// tab in place, or opens one; the others open or show their one tab.
    var editorTab: EditorTab {
        get {
            if let item = workspace.focusedItem, let t = EditorTab(content: item.content) { return t }
            return lastTextTab
        }
        set { show(newValue.content) }
    }
    @ObservationIgnored private var lastTextTab: EditorTab = .hex
    /// The graphics view in the focused tab, if it is one.
    var graphicsTab: GraphicsModel.Tab? {
        get {
            if case .graphics(let t) = workspace.focusedItem?.content { return t }
            return nil
        }
        set {
            if let t = newValue { show(.graphics(t)) } else if graphicsTab != nil { leaveViewTab() }
        }
    }
    let graphics: GraphicsModel
    /// The sound view in the focused tab, if it is one (docs/23).
    var audioTab: AudioModel.Tab? {
        get {
            if case .audio(let t) = workspace.focusedItem?.content { return t }
            return nil
        }
        set {
            if let t = newValue { show(.audio(t)) } else if audioTab != nil { leaveViewTab() }
        }
    }
    let audio: AudioModel
    /// The focused tab is a text view: no graphics, sound or tutor tab.
    var showsTextEditor: Bool {
        guard let item = workspace.focusedItem else { return true }
        return EditorTab(content: item.content) != nil
    }

    /// Shows `content` in the focused group. A code representation changes
    /// the focused code tab, else the group's code tab, in place; anything
    /// else opens, or shows, its tab.
    func show(_ content: EditorContent) {
        if case .code(let r) = content {
            let group = workspace.layout.group(workspace.focusedGroup)
            let target: EditorItem? = {
                if let f = workspace.focusedItem, case .code = f.content { return f }
                return group?.items.first { if case .code = $0.content { true } else { false } }
            }()
            if let target {
                workspace.setRepresentation(r, of: target.id)
                workspace.focus(item: target.id)
            } else {
                workspace.open(content)
            }
        } else {
            workspace.open(content)
        }
        if let t = EditorTab(content: content) { lastTextTab = t }
        refreshDecompile()
    }

    /// Opens a tab and focuses it: the sidebar, the menus and drops call
    /// this rather than the workspace, so C and Graph follow.
    func focus(item id: UUID) {
        workspace.focus(item: id)
        if let item = workspace.layout.item(id), let t = EditorTab(content: item.content) { lastTextTab = t }
        refreshDecompile()
    }

    // MARK: Tabs (docs/29)

    /// The routine each code tab was last on, by the label at or before it.
    private(set) var codeTitles: [UUID: String] = [:]
    /// Labels by address, for naming a routine; rebuilt after the navigator
    /// reloads.
    @ObservationIgnored private var labelIndex: [(address: UInt32, name: String)]?

    /// A tab's name: a code tab by its routine, else by its view.
    func title(of item: EditorItem) -> String {
        switch item.content {
        case .code(let r): codeTitles[item.id] ?? r.title
        case .atlas: "Atlas"
        case .compare: "Compare"
        case .source: "Source"
        case .graphics(let t): t.title
        case .audio(let t): t.title
        case .tutor: "Tutor"
        }
    }

    /// The label at or before `address` in its bank.
    func routineName(at address: UInt32) -> String? {
        // Rebuilt when the navigator's list changed size, which is also
        // what happens when it first loads.
        if labelIndex?.count != navigator.labels.count {
            labelIndex = navigator.labels.map { ($0.address, $0.name) }.sorted { $0.address < $1.address }
        }
        guard let index = labelIndex, !index.isEmpty else { return nil }
        var lo = 0, hi = index.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if index[mid].address <= address { lo = mid + 1 } else { hi = mid }
        }
        guard lo > 0 else { return nil }
        let found = index[lo - 1]
        return found.address >> 16 == address >> 16 ? found.name : nil
    }

    /// Name the code tabs on the selection by its routine.
    private func refreshTitles() {
        guard let address = selectedAddress, let name = routineName(at: address) else { return }
        let focused = workspace.focusedItem?.id
        for item in workspace.layout.items {
            guard case .code = item.content, item.id == focused || item.followsSelection else { continue }
            if codeTitles[item.id] != name { codeTitles[item.id] = name }
        }
    }

    /// Split Right and Split Down: the focused code tab again in a new group
    /// on that side, as Visual Studio Code's Split Editor does; a view with
    /// one tab moves there instead, when its group has others.
    func splitFocused(_ edge: DropEdge) {
        guard let item = workspace.focusedItem else { return }
        let group = workspace.focusedGroup
        if case .code = item.content {
            guard let copy = workspace.open(item.content, in: group) else { return }
            if let title = codeTitles[item.id] { codeTitles[copy] = title }
            workspace.split(group, edge, with: copy)
        } else {
            workspace.split(group, edge, with: item.id)
        }
        refreshDecompile()
    }

    /// Close Tab.
    func closeFocusedTab() {
        guard let item = workspace.focusedItem else { return }
        close(item: item.id)
    }

    /// Closes a tab, from its close button or Close Tab.
    func close(item id: UUID) {
        workspace.close(id)
        codeTitles[id] = nil
        refreshDecompile()
    }

    /// Gives a group focus, as a click in it does.
    func focusGroup(_ id: UUID) {
        workspace.focus(group: id)
        if let item = workspace.focusedItem, let t = EditorTab(content: item.content) { lastTextTab = t }
        refreshDecompile()
    }

    /// The representation strip: a code tab shown another way, focused.
    func setRepresentation(_ r: CodeRepresentation, of id: UUID) {
        workspace.setRepresentation(r, of: id)
        focus(item: id)
    }

    /// Next Tab and Previous Tab, within the focused group, wrapping.
    func selectAdjacentTab(_ delta: Int) {
        guard let g = workspace.layout.group(workspace.focusedGroup), g.items.count > 1,
              let i = g.items.firstIndex(where: { $0.id == g.selected })
        else { return }
        let n = g.items.count
        focus(item: g.items[((i + delta) % n + n) % n].id)
    }

    /// Focus Group 1 to 4, in reading order.
    func focusGroup(at index: Int) {
        let groups = workspace.layout.groups
        guard groups.indices.contains(index) else { return }
        workspace.focus(group: groups[index].id)
        refreshDecompile()
    }

    /// A graphics or sound view was cleared the old way: show the group's
    /// text tab again, opening one if there is none.
    private func leaveViewTab() {
        let group = workspace.layout.group(workspace.focusedGroup)
        if let text = group?.items.first(where: { EditorTab(content: $0.content) != nil }) {
            focus(item: text.id)
        } else {
            show(lastTextTab.content)
        }
    }
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
        audio = AudioModel(rom: rom, workbench: workbench, graphics: graphics)
        session.onChange = { [weak self] kind in self?.handleChange(kind) }
        graphics.selectBytes = { [weak self] range in self?.selectRange(range) }
        graphics.revealTile = { [weak self] in self?.graphicsTab = .tiles }
        audio.showInRom = { [weak self] offset in self?.showInRom(offset) }
        audio.openTab = { [weak self] tab in self?.audioTab = tab }
        // The workbench shows them unless told otherwise.
        if !explanationsShown {
            workbench.setShowExplanations(show: false)
        }
        source.reload(workbench: workbench)
        if startAnalysis {
            session.startAnalysis()
        }
    }

    /// Where View › Show Explanations remembers being turned off.
    static let hideExplanationsKey = "HideExplanations"

    /// Explanations in the listing and the C: explained comments on
    /// hardware writes, and a note above each idiom (docs/20).
    var showExplanations: Bool {
        get { explanationsShown }
        set {
            guard newValue != explanationsShown else { return }
            explanationsShown = newValue
            workbench.setShowExplanations(show: newValue)
            // The listing has new lines and the C new text.
            handleChange(.view)
        }
    }
    private var explanationsShown = !UserDefaults.standard.bool(forKey: RomViewModel.hideExplanationsKey)

    /// Work out the screen at the selected instruction, if the Screen
    /// section is open and it is not already shown.
    func refreshScreen(force: Bool = false) {
        guard showScreen, let at = instruction?.fileOffset else {
            if instruction == nil { screen = nil }
            return
        }
        guard force || screenFor != at else { return }
        screenFor = at
        screenTask?.cancel()
        screenLoading = true
        let workbench = workbench
        screenTask = Task { [weak self] in
            let s = await workbench.screenAt(fileOffset: at)
            guard let self, !Task.isCancelled, self.screenFor == at else { return }
            self.screen = s
            self.screenLoading = false
        }
    }

    /// A Screen row's view: the ROM bytes a DMA sends to VRAM or the
    /// palette, in the Tile Decoder, Tilemap or Palette view.
    func open(screenLink link: ScreenLinkInfo) {
        graphics.source = .rom
        switch link {
        case .tiles(let rom, let bpp):
            graphics.romOffset = rom
            graphics.format = switch bpp {
            case 2: .bpp2
            case 8: .bpp8
            case 7: .mode7
            default: .bpp4
            }
            // The palette the same setup loads, where it is in ROM.
            let palette = screen?.sections.flatMap(\.rows).compactMap(\.link).first {
                if case .palette = $0 { true } else { false }
            }
            if case .palette(let p) = palette { graphics.palette = .rom(p) }
            graphics.selectedTile = 0
            graphicsTab = .tiles
        case .tilemap(let rom):
            graphics.romOffset = rom
            graphicsTab = .tilemap
        case .palette(let rom):
            graphics.romOffset = rom
            graphicsTab = .palette
        }
    }

    /// Select every instruction of the idiom whose note is at `offset`:
    /// what clicking its note line does.
    func selectIdiom(noteAt offset: UInt32) {
        guard let idiom = workbench.explainAt(fileOffset: offset).idioms.first(where: { $0.noteAt == offset }),
              let first = idiom.offsets.first, let last = idiom.offsets.last
        else {
            select(offset: offset)
            return
        }
        let end = last + UInt32(workbench.instructionAt(fileOffset: last)?.len ?? 1)
        selectRange(first..<end)
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

    /// The document closed: stop the analysis, the sound and a comparison.
    func close() {
        session.close()
        audio.shutDown()
        compare.close()
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
            refreshScreen(force: true)
            workspace.invalidateAll()
            refreshDecompile()
            source.reload(workbench: workbench)
            if compare.state == .ready {
                let generation = asmGeneration
                Task { await compare.refresh(this: workbench, generation: generation) }
            }
            Task {
                await navigator.reload(workbench: workbench, rom: rom)
                labelIndex = nil
                refreshTitles()
            }
        case .project:
            break
        }
    }

    // MARK: C and Graph

    /// The code tabs that should be on the routine at the selection: those
    /// shown, in their groups, that have focus or follow the selection.
    private func followingCodeTabs(_ r: CodeRepresentation) -> [UUID] {
        let focused = workspace.focusedItem?.id
        return workspace.visibleItems
            .filter { $0.content == .code(r) && ($0.id == focused || $0.followsSelection) }
            .map(\.id)
    }

    /// Keep the C tabs on the routine at the selection. Only while one is
    /// showing: decompiling costs a summary of every routine the first time
    /// after an analysis.
    func refreshDecompile() {
        refreshGraph()
        guard hasDisassembly else { return }
        for id in followingCodeTabs(.c) {
            workspace.decompiler(for: id).follow(
                workbench: workbench,
                instructionStart: instruction?.fileOffset ?? selectedOffset,
                generation: asmGeneration
            )
        }
    }

    /// Keep the Graph tabs on the routine at the selection, while one shows.
    func refreshGraph() {
        guard hasDisassembly else { return }
        for id in followingCodeTabs(.graph) {
            workspace.graph(for: id).follow(
                workbench: workbench,
                instructionStart: instruction?.fileOffset ?? selectedOffset,
                generation: asmGeneration
            )
        }
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
        if tab.needsRecording {
            if graphics.hasRecording { graphics.source = .recording }
        } else if graphics.source == .rom, let range = highlightedRange {
            graphics.romOffset = range.lowerBound
        }
        graphicsTab = tab
    }

    /// Open a sound view, on the recording's sound if it has any, else on
    /// the ROM's upload.
    func openAudio(_ tab: AudioModel.Tab) {
        audio.opened()
        audioTab = tab
    }

    /// Leave the sound view for the listing at a ROM offset.
    func showInRom(_ offset: UInt32) {
        editorTab = hasDisassembly ? .disassembly : .hex
        jump(to: offset)
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
        refreshTitles()
    }

    private func clearDetails() {
        instruction = nil
        explanation = nil
        screen = nil
        screenFor = nil
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
        explanation = instruction.map { workbench.explainAt(fileOffset: $0.fileOffset) }
        refreshScreen()
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
            historyEntries.append(HistoryEntry(offset: from, item: workspace.focusedItem?.id))
            if historyEntries.count > 100 { historyEntries.removeFirst() }
            forwardEntries.removeAll()
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
        guard let previous = historyEntries.popLast() else { return }
        if let current = selectedOffset {
            forwardEntries.append(HistoryEntry(offset: current, item: workspace.focusedItem?.id))
        }
        revisit(previous)
    }

    func goForward() {
        guard let next = forwardEntries.popLast() else { return }
        if let current = selectedOffset {
            historyEntries.append(HistoryEntry(offset: current, item: workspace.focusedItem?.id))
        }
        revisit(next)
    }

    /// Back to where an entry was seen: its tab, if it is still open.
    private func revisit(_ entry: HistoryEntry) {
        if let item = entry.item, workspace.layout.item(item) != nil, item != workspace.focusedItem?.id {
            focus(item: item)
        }
        jump(to: entry.offset, recordHistory: false)
    }

    /// Follow the selected instruction's target (or a pointer's).
    func followReference() {
        if let target = instruction?.targetFileOffset {
            jump(to: target)
        } else if let p = inspection?.pointerTargetFileOffset, instruction == nil {
            jump(to: p)
        }
    }

    /// Scroll the focused tab, and every tab following the selection, to
    /// `offset`.
    func requestScroll(toOffset offset: UInt32) {
        var targets = Set(workspace.layout.items.filter(\.followsSelection).map(\.id))
        if let focused = workspace.focusedItem?.id { targets.insert(focused) }
        scrollRequest = ScrollRequest(id: (scrollRequest?.id ?? 0) + 1, offset: offset, targets: targets)
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

import AppKit
import RomlensKit
import SwiftUI

/// The C tab: the disassembly on the left, the routine at the cursor as
/// pseudo-C on the right (docs/18). Selecting a line on either side
/// highlights its counterpart on the other.
struct CSplitView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSSplitView {
        let asm = AsmPaneController(model: model, item: context.environment.editorItem)
        let c = CPaneController(model: model, item: context.environment.editorItem)
        let split = CSplitPaneView()
        split.isVertical = true
        split.dividerStyle = .thin
        split.autosaveName = "decompile"
        split.addArrangedSubview(asm.scrollView)
        split.addArrangedSubview(c.view)
        context.coordinator.asm = asm
        context.coordinator.c = c
        return split
    }

    func updateNSView(_ split: NSSplitView, context: Context) {
        context.coordinator.asm?.update()
        context.coordinator.c?.update()
    }

    /// Whatever the editor area offers: the text's width must never widen
    /// the window's columns.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSSplitView, context: Context) -> CGSize? {
        proposal.replacingUnspecifiedDimensions(by: CGSize(width: 400, height: 300))
    }

    @MainActor
    final class Coordinator {
        var asm: AsmPaneController?
        var c: CPaneController?
    }
}

/// The Pseudo-C tab (docs/29): the C across the whole tab, under a bar that
/// picks the routine from a list (the user's choice, 2 October 2026). The
/// disassembly beside it is a Disassembly tab of its own now, in a split
/// if wanted, following the same selection.
struct CTabView: View {
    let model: RomViewModel
    @Environment(\.editorItem) private var item

    var body: some View {
        VStack(spacing: 0) {
            RoutineBar(model: model, decompiler: model.workspace.decompiler(for: item), item: item)
            Divider()
            CPaneView(model: model)
        }
    }
}

/// The C pane on its own.
struct CPaneView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let c = CPaneController(model: model, item: context.environment.editorItem)
        context.coordinator.c = c
        return c.view
    }

    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.c?.update()
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSView, context: Context) -> CGSize? {
        proposal.replacingUnspecifiedDimensions(by: CGSize(width: 400, height: 300))
    }

    @MainActor
    final class Coordinator {
        var c: CPaneController?
    }
}

/// Which routine the C shows, and a searchable list of every routine to
/// choose another; choosing one selects its first instruction, so the
/// Disassembly tabs that follow the selection go there too.
struct RoutineBar: View {
    let model: RomViewModel
    let decompiler: DecompileModel
    let item: UUID?
    @State private var choosing = false
    @State private var filter = ""

    var body: some View {
        HStack(spacing: 8) {
            Text("Routine").foregroundStyle(.secondary)
            Button {
                choosing = true
            } label: {
                HStack(spacing: 4) {
                    Text(decompiler.result?.name ?? "None selected")
                        .font(.callout.monospaced())
                        .lineLimit(1)
                    Image(systemName: "chevron.up.chevron.down").font(.caption2).foregroundStyle(.secondary)
                }
            }
            .buttonStyle(.borderless)
            .help("Choose a routine to read as C")
            .popover(isPresented: $choosing, arrowEdge: .bottom) {
                list
            }
            if let entry = decompiler.result?.entry {
                Text(formatSnesAddress(address: entry))
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            Text("\(model.routines.count) routines")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .frame(height: 30)
    }

    private var shown: [LabelInfo] {
        let q = filter.trimmingCharacters(in: .whitespaces)
        return q.isEmpty ? model.routines : NavigatorModel.filter(model.routines, query: q)
    }

    private var list: some View {
        VStack(spacing: 0) {
            TextField("Filter routines, or $BB:AAAA", text: $filter)
                .textFieldStyle(.roundedBorder)
                .padding(8)
            List(shown, id: \.address) { label in
                Button {
                    choose(label)
                } label: {
                    HStack {
                        Text(label.name).font(.callout.monospaced()).lineLimit(1)
                        Spacer()
                        Text(formatSnesAddress(address: label.address))
                            .font(.caption.monospaced())
                            .foregroundStyle(.tertiary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .listStyle(.plain)
        }
        .frame(width: 340, height: 420)
    }

    private func choose(_ label: LabelInfo) {
        choosing = false
        if let item { model.focus(item: item) }
        model.jump(toSnesAddress: label.address)
    }
}

/// Splits in half the first time it is laid out.
final class CSplitPaneView: NSSplitView {
    private var placed = false

    override func layout() {
        super.layout()
        if !placed, arrangedSubviews.count == 2, bounds.width > 400 {
            placed = true
            if arrangedSubviews[0].frame.width < 200 || arrangedSubviews[1].frame.width < 200 {
                setPosition(bounds.width * 0.5, ofDividerAt: 0)
            }
        }
    }
}

/// Token kind → colour for the C, in the disassembly's palette where the
/// two name the same thing.
enum CTokenPalette {
    static func color(for kind: CTokenKind) -> NSColor {
        switch kind {
        case .keyword: .systemPurple
        case .type: .systemIndigo
        case .number: .systemTeal
        case .comment: .systemGreen
        case .function: .controlAccentColor
        case .variable: .systemOrange
        case .register: .systemPink
        case .label: .controlAccentColor
        case .helper: .secondaryLabelColor
        case .local: .labelColor
        case .gotoLabel: .systemBrown
        }
    }
}

/// The right-hand pane: a header naming the routine with the level picker
/// and Export, over the read-only text.
@MainActor
final class CPaneController: NSObject, NSTextViewDelegate {
    let model: RomViewModel
    let view: NSView
    private let title = NSTextField(labelWithString: "")
    private let status = NSTextField(labelWithString: "")
    private let levels = NSSegmentedControl(
        labels: ["Lift", "Clean", "Full"], trackingMode: .selectOne, target: nil, action: nil
    )
    private let numbers = NSSegmentedControl(
        labels: ["Auto", "Hex", "Dec", "Bin"], trackingMode: .selectOne, target: nil, action: nil
    )
    private let scrollView: NSScrollView
    let textView: CTextView
    /// Which blocks are folded (the gutter's arrows, ⌥⌘← and ⌥⌘→).
    let folder = CFolder()
    private var gutter: CFoldGutter?
    private var shownGeneration = -1
    private var highlighted: [Int] = []
    /// The instruction the highlight is for: a block opens when the
    /// selection moves into it, not when the same C comes back.
    private var highlightedFor: UInt32?
    /// UTF-16 offset where each line starts, plus the end.
    private var lineStarts: [Int] = []
    /// A click in the C moved the selection: the C need not scroll to it.
    private var selectingFromText = false
    /// The text is being replaced: the selection moves, but nobody clicked.
    private var replacingText = false
    /// The routine whose C is shown.
    private var shownEntry: UInt32?
    /// A C version shown instead of the generated C, by name (docs/24).
    private(set) var shownVersion: String? {
        get { decompiler.shownVersion }
        set { decompiler.shownVersion = newValue }
    }
    private var shownVersionText: String?
    private let versions = NSPopUpButton(frame: .zero, pullsDown: false)
    private var versionsKey = ""

    /// The tab this pane is in, if any (docs/29).
    let item: UUID?
    /// This tab's C.
    private var decompiler: DecompileModel { model.workspace.decompiler(for: item) }

    init(model: RomViewModel, item: UUID? = nil) {
        self.item = item
        self.model = model
        textView = CTextView(usingTextLayoutManager: false)
        scrollView = NSScrollView()
        view = NSView()
        super.init()

        textView.isEditable = false
        textView.isSelectable = true
        textView.isRichText = false
        textView.drawsBackground = true
        textView.backgroundColor = .textBackgroundColor
        textView.font = model.metrics.font
        textView.textContainerInset = NSSize(width: 8, height: 6)
        textView.isHorizontallyResizable = true
        textView.isVerticallyResizable = true
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = false
        textView.textContainer?.containerSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.delegate = self
        textView.setAccessibilityIdentifier("decompiled-c")
        textView.onDoubleClick = { [weak self] index in self?.follow(tokenAt: index) }
        textView.layoutManager?.delegate = folder
        folder.textView = textView
        textView.onClickCharacter = { [weak self] index in
            guard let self, let f = folder.placeholder(at: index) else { return false }
            folder.toggle(f)
            return true
        }
        textView.onFoldKey = { [weak self] key in self?.foldKey(key) ?? false }

        scrollView.documentView = textView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor
        let gutter = CFoldGutter(scrollView: scrollView, textView: textView, folder: folder)
        scrollView.verticalRulerView = gutter
        scrollView.hasVerticalRuler = true
        scrollView.rulersVisible = true
        folder.changed = { [weak gutter] in gutter?.needsDisplay = true }
        self.gutter = gutter

        title.font = .boldSystemFont(ofSize: NSFont.systemFontSize)
        title.lineBreakMode = .byTruncatingTail
        status.textColor = .secondaryLabelColor
        status.lineBreakMode = .byTruncatingTail
        levels.target = self
        levels.action = #selector(levelChanged(_:))
        levels.controlSize = .small
        levels.toolTip = "Lift: every instruction in full. Clean: after data flow. Full: with if, loops and signatures."
        levels.setAccessibilityIdentifier("decompile-level")
        numbers.target = self
        numbers.action = #selector(numbersChanged(_:))
        numbers.controlSize = .small
        numbers.toolTip = "How numbers print: small ones in decimal and the rest in hex, or all in hex, decimal or binary. Addresses stay hex. Hover over a number to see it in every base."
        numbers.setAccessibilityIdentifier("decompile-numbers")
        if let saved = UserDefaults.standard.string(forKey: Self.numbersKey),
           let style = Self.style(named: saved) {
            decompiler.numbers = style
        }
        let export = NSButton(title: "Export C…", target: nil, action: #selector(RomWindowController.exportC(_:)))
        export.controlSize = .small
        export.bezelStyle = .rounded

        versions.controlSize = .small
        versions.target = self
        versions.action = #selector(versionChosen(_:))
        versions.toolTip = "The C Romlens generates, or a C version written by you or the tutor"
        versions.setAccessibilityIdentifier("c-version")
        let menu = NSMenu()
        menu.delegate = self
        textView.menu = menu

        let header = NSStackView(views: [title, status, NSView(), versions, numbers, levels, export])
        header.orientation = .horizontal
        header.spacing = 8
        header.edgeInsets = NSEdgeInsets(top: 4, left: 8, bottom: 4, right: 8)
        header.setHuggingPriority(.defaultHigh, for: .vertical)
        // Below the fitting-size priority, so long text truncates instead of
        // asking for width; the status gives way before the title.
        status.setContentCompressionResistancePriority(.init(1), for: .horizontal)
        title.setContentCompressionResistancePriority(.init(2), for: .horizontal)
        let rule = NSBox()
        rule.boxType = .separator
        let stack = NSStackView(views: [header, rule, scrollView])
        stack.orientation = .vertical
        stack.spacing = 0
        stack.alignment = .leading
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            stack.topAnchor.constraint(equalTo: view.topAnchor),
            stack.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            header.widthAnchor.constraint(equalTo: stack.widthAnchor),
            rule.widthAnchor.constraint(equalTo: stack.widthAnchor),
            scrollView.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ])
    }

    func update() {
        let d = decompiler
        levels.selectedSegment = switch d.level {
        case .lift: 0
        case .clean: 1
        case .full: 2
        }
        numbers.selectedSegment = Self.styles.firstIndex(of: d.numbers) ?? 0
        switch d.state {
        case .idle:
            title.stringValue = "No routine"
            status.stringValue = "Select an instruction to see its routine as C."
        case .loading:
            status.stringValue = "Decompiling…"
        case .ready:
            if let r = d.result {
                title.stringValue = "\(r.name)  \(formatSnesAddress(address: r.entry))"
                status.stringValue = r.warnings.isEmpty
                    ? "\(r.instructions) instructions, \(r.statements) statements, \(r.gotos) gotos"
                    : "\(r.warnings.count) note\(r.warnings.count == 1 ? "" : "s"): \(r.warnings[0])"
                status.toolTip = r.warnings.joined(separator: "\n")
            }
        case .notInRoutine:
            title.stringValue = "No routine"
            status.stringValue = "The selection is not inside a routine the analysis found."
        case .failed(let message):
            status.stringValue = message
        }
        refreshVersions()
        if let v = shownVersion, let entry = d.result?.entry {
            let version = model.session.workbench.cVersions(routine: entry).first { $0.name == v }?.version
            if let version {
                if shownVersionText != version.text || shownGeneration != d.resultGeneration {
                    shownGeneration = d.resultGeneration
                    setVersionText(version)
                }
                highlightVersion(version)
                selectingFromText = false
                return
            }
            shownVersion = nil
            shownGeneration = -1
        }
        if shownGeneration != d.resultGeneration || shownVersionText != nil {
            shownGeneration = d.resultGeneration
            shownVersionText = nil
            setText(d.result)
        }
        let start = model.instruction?.fileOffset ?? model.selectedOffset
        let lines = start.map { d.lines(forInstructionAt: $0) } ?? []
        if lines != highlighted {
            setHighlight(lines, reveal: start != highlightedFor)
            highlightedFor = start
            if !selectingFromText, let first = lines.first {
                scrollToLine(first)
            }
        }
        selectingFromText = false
    }

    /// A C literal's value: `12`, `0x81`, `0b1000`.
    static func value(of literal: String) -> UInt32? {
        let l = literal.lowercased()
        if l.hasPrefix("0x") { return UInt32(l.dropFirst(2), radix: 16) }
        if l.hasPrefix("0b") { return UInt32(l.dropFirst(2), radix: 2) }
        return UInt32(l)
    }

    /// `129 = 0x81 = 0b10000001`.
    static func bases(_ v: UInt32) -> String {
        [NumberStyle.decimal, .hex, .binary]
            .map { formatCNumber(value: v, style: $0) }
            .joined(separator: " = ")
    }

    private func setText(_ result: DecompiledInfo?) {
        let text = result?.text ?? ""
        // A live session re-analyses every few seconds and the routine's C
        // usually comes back the same: leave the caret and the scroll alone.
        if text == textView.string, result?.entry == shownEntry { return }
        let sameRoutine = result != nil && result?.entry == shownEntry
        shownEntry = result?.entry
        let origin = scrollView.contentView.bounds.origin
        let s = NSMutableAttributedString(
            string: text,
            attributes: [.font: model.metrics.font, .foregroundColor: NSColor.labelColor]
        )
        for t in result?.tokens ?? [] {
            let range = NSRange(location: Int(t.start), length: Int(t.len))
            guard NSMaxRange(range) <= s.length else { continue }
            s.addAttribute(.foregroundColor, value: CTokenPalette.color(for: t.kind), range: range)
            if t.kind == .number, let v = Self.value(of: (text as NSString).substring(with: range)) {
                s.addAttribute(.toolTip, value: Self.bases(v), range: range)
            }
        }
        // The same routine's C again, a label changed: what was folded
        // stays folded, by line.
        let foldedLines = sameRoutine ? Set(folder.folded.map { line(at: $0) }) : []
        folder.clear()
        // A new text keeps the old caret's character index, which falls on
        // some line of the new routine; that must not read as a click there.
        replacingText = true
        textView.textStorage?.setAttributedString(s)
        textView.setSelectedRange(NSRange(location: 0, length: 0))
        replacingText = false
        let ns = text as NSString
        var starts = [0]
        var at = 0
        while at < ns.length {
            let r = ns.range(of: "\n", range: NSRange(location: at, length: ns.length - at))
            if r.location == NSNotFound { break }
            at = r.location + 1
            starts.append(at)
        }
        lineStarts = starts
        highlighted = []
        let folds = CFold.find(in: text)
        folder.set(folds, folded: Set(folds.filter { foldedLines.contains(line(at: $0.open)) }.map(\.open)))
        if sameRoutine {
            scrollView.contentView.scroll(to: origin)
            scrollView.reflectScrolledClipView(scrollView.contentView)
        }
    }

    private func range(ofLine line: Int) -> NSRange? {
        guard line >= 0, line + 1 < lineStarts.count else { return nil }
        return NSRange(location: lineStarts[line], length: lineStarts[line + 1] - lineStarts[line])
    }

    private func setHighlight(_ lines: [Int], reveal: Bool = true) {
        guard let lm = textView.layoutManager else { return }
        // The selection moved into a folded block: open it, as an IDE does.
        // A block's first line and its closing brace show while it is
        // folded: a line is hidden when its first word is.
        let ns = textView.string as NSString
        for l in lines where reveal {
            guard let r = range(ofLine: l) else { continue }
            let word = ns.rangeOfCharacter(from: CharacterSet.whitespacesAndNewlines.inverted, range: r)
            if word.location != NSNotFound { folder.reveal(NSRange(location: word.location, length: 1)) }
        }
        let all = NSRange(location: 0, length: textView.textStorage?.length ?? 0)
        lm.removeTemporaryAttribute(.backgroundColor, forCharacterRange: all)
        let color = NSColor.selectedTextBackgroundColor.withAlphaComponent(0.45)
        for l in lines {
            if let r = range(ofLine: l) {
                lm.addTemporaryAttribute(.backgroundColor, value: color, forCharacterRange: r)
            }
        }
        highlighted = lines
    }

    private func scrollToLine(_ line: Int) {
        guard let r = range(ofLine: line), let lm = textView.layoutManager,
              let container = textView.textContainer else { return }
        let glyphs = lm.glyphRange(forCharacterRange: r, actualCharacterRange: nil)
        var rect = lm.boundingRect(forGlyphRange: glyphs, in: container)
        rect.origin.y += textView.textContainerInset.height
        let visible = scrollView.contentView.bounds
        if !visible.contains(rect) {
            let y = max(0, rect.midY - visible.height / 2)
            scrollView.contentView.scroll(to: NSPoint(x: 0, y: y))
            scrollView.reflectScrolledClipView(scrollView.contentView)
        }
    }

    /// The line holding UTF-16 offset `index`.
    private func line(at index: Int) -> Int {
        var lo = 0
        var hi = lineStarts.count - 1
        while lo < hi {
            let mid = (lo + hi + 1) / 2
            if lineStarts[mid] <= index { lo = mid } else { hi = mid - 1 }
        }
        return lo
    }

    /// Where the C tab remembers its number style.
    static let numbersKey = "CNumberStyle"
    static let styles: [NumberStyle] = [.auto, .hex, .decimal, .binary]
    private static let names = ["auto", "hex", "decimal", "binary"]

    static func style(named name: String) -> NumberStyle? {
        names.firstIndex(of: name).map { styles[$0] }
    }

    @objc private func numbersChanged(_ sender: NSSegmentedControl) {
        let i = max(0, min(sender.selectedSegment, Self.styles.count - 1))
        decompiler.numbers = Self.styles[i]
        UserDefaults.standard.set(Self.names[i], forKey: Self.numbersKey)
        model.refreshDecompile()
    }

    @objc private func levelChanged(_ sender: NSSegmentedControl) {
        decompiler.level = switch sender.selectedSegment {
        case 0: .lift
        case 1: .clean
        default: .full
        }
        model.refreshDecompile()
    }

    // A click in the C selects the instructions its line came from.
    func textViewDidChangeSelection(_ notification: Notification) {
        guard !replacingText, !lineStarts.isEmpty else { return }
        if shownVersion != nil {
            followVersionLine(line(at: textView.selectedRange().location))
            return
        }
        let at = textView.selectedRange().location
        let offsets = decompiler.offsets(forLine: line(at: at))
        guard let first = offsets.min() else { return }
        guard first != (model.instruction?.fileOffset ?? model.selectedOffset) else { return }
        selectingFromText = true
        model.select(offset: first)
        model.requestScroll(toOffset: first)
    }

    /// Double-click on a routine's or a label's name goes there.
    private func follow(tokenAt index: Int) {
        guard let t = decompiler.result?.tokens.first(where: {
            Int($0.start) <= index && index < Int($0.start + $0.len)
        }), let address = t.address else { return }
        switch t.kind {
        case .function, .label:
            model.jump(toSnesAddress: address)
        default:
            break
        }
    }
}

// MARK: C versions and the C's annotations (docs/24, U10)

extension CPaneController: NSMenuDelegate {
    private var entry: UInt32? { decompiler.result?.entry }

    /// The picker: Generated, each version, then what can be done.
    func refreshVersions() {
        let names = entry.map { model.session.workbench.cVersions(routine: $0).map(\.name) } ?? []
        let key = "\(entry ?? 0)|\(names.joined(separator: "|"))|\(shownVersion ?? "")"
        guard key != versionsKey else { return }
        versionsKey = key
        versions.removeAllItems()
        versions.addItem(withTitle: "Generated")
        for n in names { versions.addItem(withTitle: n) }
        versions.menu?.addItem(.separator())
        versions.addItem(withTitle: "New Version…")
        if shownVersion != nil {
            versions.addItem(withTitle: "Edit Version…")
            versions.addItem(withTitle: "Delete Version")
        }
        versions.selectItem(withTitle: shownVersion ?? "Generated")
        versions.isEnabled = entry != nil
    }

    @objc func versionChosen(_ sender: NSPopUpButton) {
        guard let title = sender.titleOfSelectedItem, let entry else { return }
        switch title {
        case "Generated":
            shownVersion = nil
        case "New Version…":
            model.beginCEdit(.version(routine: entry, name: nil))
        case "Edit Version…":
            model.beginCEdit(.version(routine: entry, name: shownVersion))
        case "Delete Version":
            if let v = shownVersion {
                try? model.session.execute(.setCVersion(routine: entry, name: v, version: nil))
                shownVersion = nil
            }
        default:
            shownVersion = title
        }
        versionsKey = ""
        update()
    }

    func showVersion(_ name: String?) {
        shownVersion = name
        versionsKey = ""
        update()
    }

    private func setVersionText(_ v: CVersionInfo) {
        shownVersionText = v.text
        let s = NSMutableAttributedString(
            string: v.text, attributes: [.font: model.metrics.font, .foregroundColor: NSColor.labelColor])
        for t in model.session.workbench.lexC(text: v.text) {
            let r = NSRange(location: Int(t.start), length: Int(t.len))
            guard NSMaxRange(r) <= s.length else { continue }
            s.addAttribute(.foregroundColor, value: CTokenPalette.color(for: t.kind), range: r)
        }
        folder.clear()
        replacingText = true
        textView.textStorage?.setAttributedString(s)
        textView.setSelectedRange(NSRange(location: 0, length: 0))
        replacingText = false
        let ns = v.text as NSString
        var starts = [0]
        var at = 0
        while at < ns.length {
            let r = ns.range(of: "\n", range: NSRange(location: at, length: ns.length - at))
            if r.location == NSNotFound { break }
            at = r.location + 1
            starts.append(at)
        }
        if starts.last != ns.length { starts.append(ns.length) }
        lineStarts = starts
        highlighted = []
        folder.set(CFold.find(in: v.text), folded: [])
        title.stringValue = "\(title.stringValue.split(separator: " ").first ?? "")  version “\(shownVersion ?? "")”"
        status.stringValue = v.author == .tutor ? "Written by the tutor; not checked against the code" : "Written by you; not checked against the code"
    }

    /// The version's lines anchored to the selected instruction.
    private func highlightVersion(_ v: CVersionInfo) {
        guard let a = model.selectedAddress else { return }
        let lines = Self.anchoredLines(v.anchors, at: a)
        if lines != highlighted {
            setHighlight(lines)
            if !selectingFromText, let first = lines.first { scrollToLine(first) }
        }
    }

    /// The zero-based lines of the anchors holding `address`. Lines count
    /// from one; an anchor that ends before it starts, or names line 0, is
    /// one a project file got wrong and is skipped rather than trusted.
    static func anchoredLines(_ anchors: [CAnchorInfo], at address: UInt32) -> [Int] {
        anchors.filter { $0.start <= address && address <= $0.end && $0.first >= 1 && $0.first <= $0.last }
            .flatMap { Int($0.first) - 1...Int($0.last) - 1 }
    }

    private func followVersionLine(_ line: Int) {
        guard let entry, let v = shownVersion,
              let version = model.session.workbench.cVersions(routine: entry).first(where: { $0.name == v })?.version,
              let a = version.anchors.first(where: { Int($0.first) - 1 <= line && line <= Int($0.last) - 1 }) else { return }
        guard a.start != model.selectedAddress else { return }
        selectingFromText = true
        model.jump(toSnesAddress: a.start)
    }

    /// Right-click in the C: name a local, comment the line, note the
    /// routine, or write a version.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        guard let entry else { return }
        let point = textView.convert(textView.window?.mouseLocationOutsideOfEventStream ?? .zero, from: nil)
        let index = textView.characterIndexForInsertion(at: point)
        if shownVersion == nil, let r = decompiler.result {
            if let t = r.tokens.first(where: { Int($0.start) <= index && index < Int($0.start + $0.len) }), t.kind == .local {
                let word = (textView.string as NSString).substring(with: NSRange(location: Int(t.start), length: Int(t.len)))
                let original = model.session.workbench.localNames(routine: entry).first { $0.name == word }?.local ?? word
                menu.addItem(item("Name “\(word)”…") { [weak self] in self?.model.beginCEdit(.local(routine: entry, local: original)) })
            }
            let offsets = decompiler.offsets(forLine: line(at: index))
            if let first = offsets.min(), let address = model.rom.snesAddressFor(fileOffset: first) {
                menu.addItem(item("C Comment Here…") { [weak self] in self?.model.beginCEdit(.comment(address: address)) })
            }
            menu.addItem(item("Routine Note…") { [weak self] in self?.model.beginCEdit(.note(routine: entry)) })
            menu.addItem(.separator())
        }
        menu.addItem(item("New C Version…") { [weak self] in self?.model.beginCEdit(.version(routine: entry, name: nil)) })
        if let v = shownVersion {
            menu.addItem(item("Edit C Version…") { [weak self] in self?.model.beginCEdit(.version(routine: entry, name: v)) })
        }
        menu.addItem(.separator())
        addFoldItems(to: menu, at: index)
        menu.addItem(NSMenuItem(title: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: ""))
    }

    /// Fold and Unfold at the click, and every block at once, with the keys
    /// Xcode uses.
    private func addFoldItems(to menu: NSMenu, at index: Int) {
        guard !folder.folds.isEmpty else { return }
        let line = range(ofLine: self.line(at: index)) ?? NSRange(location: index, length: 0)
        let entries: [(String, CFoldKey, Bool)] = [
            ("Fold", .fold, folder.folds.contains { !folder.isFolded($0) && (NSLocationInRange($0.open, line) || $0.contains(index)) }),
            ("Unfold", .unfold, folder.folds.contains { folder.isFolded($0) && (NSLocationInRange($0.open, line) || $0.contains(index)) }),
            ("Fold All", .foldAll, true),
            ("Unfold All", .unfoldAll, !folder.folded.isEmpty),
        ]
        for (title, key, enabled) in entries {
            let i = item(title) { [weak self] in _ = self?.foldKey(key, at: index) }
            i.keyEquivalent = String(UnicodeScalar(key.unfolds ? NSRightArrowFunctionKey : NSLeftArrowFunctionKey)!)
            i.keyEquivalentModifierMask = key.all ? [.command, .option, .shift] : [.command, .option]
            if !enabled { i.action = nil }
            menu.addItem(i)
        }
        menu.addItem(.separator())
    }

    /// ⌥⌘← folds the block at the caret, ⌥⌘→ opens it; with ⇧, every block.
    func foldKey(_ key: CFoldKey, at index: Int? = nil) -> Bool {
        let at = index ?? textView.selectedRange().location
        let line = range(ofLine: self.line(at: at)) ?? NSRange(location: at, length: 0)
        switch key {
        case .fold: return folder.fold(at: at, line: line)
        case .unfold: return folder.unfold(at: at, line: line)
        case .foldAll: folder.foldAll()
        case .unfoldAll: folder.unfoldAll()
        }
        return true
    }

    private func item(_ title: String, _ action: @escaping () -> Void) -> NSMenuItem {
        let i = ClosureMenuItem(title: title, action: #selector(ClosureMenuItem.fire), keyEquivalent: "")
        i.target = i
        i.handler = action
        return i
    }
}

/// A menu item that runs a closure.
final class ClosureMenuItem: NSMenuItem {
    var handler: (() -> Void)?
    @objc func fire() { handler?() }
}

/// The fold commands (docs/18).
enum CFoldKey {
    case fold, unfold, foldAll, unfoldAll

    var unfolds: Bool { self == .unfold || self == .unfoldAll }
    var all: Bool { self == .foldAll || self == .unfoldAll }
}

/// The C's text view: reports double-clicks by character index, a click on
/// a character (a folded block's `…`), and the fold keys.
final class CTextView: NSTextView {
    var onDoubleClick: ((Int) -> Void)?
    /// A click on the character at this index; true when it was handled.
    var onClickCharacter: ((Int) -> Bool)?
    var onFoldKey: ((CFoldKey) -> Bool)?

    override func keyDown(with event: NSEvent) {
        let mods = event.modifierFlags.intersection([.command, .option, .shift, .control])
        let arrow = event.specialKey
        if mods.isSuperset(of: [.command, .option]), !mods.contains(.control),
           arrow == .leftArrow || arrow == .rightArrow {
            let all = mods.contains(.shift)
            let key: CFoldKey = arrow == .leftArrow ? (all ? .foldAll : .fold) : (all ? .unfoldAll : .unfold)
            if onFoldKey?(key) == true { return }
        }
        super.keyDown(with: event)
    }

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 1, let index = character(at: event), onClickCharacter?(index) == true {
            return
        }
        super.mouseDown(with: event)
        if event.clickCount == 2 {
            let point = convert(event.locationInWindow, from: nil)
            onDoubleClick?(characterIndexForInsertion(at: point))
        }
    }

    /// The character drawn under the event, if any.
    private func character(at event: NSEvent) -> Int? {
        guard let lm = layoutManager, let tc = textContainer else { return nil }
        let p = convert(event.locationInWindow, from: nil)
        let inContainer = NSPoint(x: p.x - textContainerOrigin.x, y: p.y - textContainerOrigin.y)
        let g = lm.glyphIndex(for: inContainer, in: tc)
        guard g < lm.numberOfGlyphs,
              lm.boundingRect(forGlyphRange: NSRange(location: g, length: 1), in: tc).contains(inContainer)
        else { return nil }
        return lm.characterIndexForGlyph(at: g)
    }
}

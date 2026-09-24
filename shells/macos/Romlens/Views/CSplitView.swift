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
        let asm = AsmPaneController(model: model)
        let c = CPaneController(model: model)
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
    private var shownGeneration = -1
    private var highlighted: [Int] = []
    /// UTF-16 offset where each line starts, plus the end.
    private var lineStarts: [Int] = []
    /// A click in the C moved the selection: the C need not scroll to it.
    private var selectingFromText = false
    /// The text is being replaced: the selection moves, but nobody clicked.
    private var replacingText = false
    /// The routine whose C is shown.
    private var shownEntry: UInt32?

    init(model: RomViewModel) {
        self.model = model
        textView = CTextView()
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

        scrollView.documentView = textView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor

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
            model.decompiler.numbers = style
        }
        let export = NSButton(title: "Export C…", target: nil, action: #selector(RomWindowController.exportC(_:)))
        export.controlSize = .small
        export.bezelStyle = .rounded

        let header = NSStackView(views: [title, status, NSView(), numbers, levels, export])
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
        let d = model.decompiler
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
        if shownGeneration != d.resultGeneration {
            shownGeneration = d.resultGeneration
            setText(d.result)
        }
        let start = model.instruction?.fileOffset ?? model.selectedOffset
        let lines = start.map { d.lines(forInstructionAt: $0) } ?? []
        if lines != highlighted {
            setHighlight(lines)
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
        // A new text keeps the old caret's character index, which falls on
        // some line of the new routine; that must not read as a click there.
        replacingText = true
        textView.textStorage?.setAttributedString(s)
        textView.setSelectedRange(NSRange(location: 0, length: 0))
        replacingText = false
        if sameRoutine {
            scrollView.contentView.scroll(to: origin)
            scrollView.reflectScrolledClipView(scrollView.contentView)
        }
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
    }

    private func range(ofLine line: Int) -> NSRange? {
        guard line >= 0, line + 1 < lineStarts.count else { return nil }
        return NSRange(location: lineStarts[line], length: lineStarts[line + 1] - lineStarts[line])
    }

    private func setHighlight(_ lines: [Int]) {
        guard let lm = textView.layoutManager else { return }
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
        model.decompiler.numbers = Self.styles[i]
        UserDefaults.standard.set(Self.names[i], forKey: Self.numbersKey)
        model.refreshDecompile()
    }

    @objc private func levelChanged(_ sender: NSSegmentedControl) {
        model.decompiler.level = switch sender.selectedSegment {
        case 0: .lift
        case 1: .clean
        default: .full
        }
        model.refreshDecompile()
    }

    // A click in the C selects the instructions its line came from.
    func textViewDidChangeSelection(_ notification: Notification) {
        guard !replacingText, !lineStarts.isEmpty else { return }
        let at = textView.selectedRange().location
        let offsets = model.decompiler.offsets(forLine: line(at: at))
        guard let first = offsets.min() else { return }
        guard first != (model.instruction?.fileOffset ?? model.selectedOffset) else { return }
        selectingFromText = true
        model.select(offset: first)
        model.requestScroll(toOffset: first)
    }

    /// Double-click on a routine's or a label's name goes there.
    private func follow(tokenAt index: Int) {
        guard let t = model.decompiler.result?.tokens.first(where: {
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

/// The C's text view: reports double-clicks by character index.
final class CTextView: NSTextView {
    var onDoubleClick: ((Int) -> Void)?

    override func mouseDown(with event: NSEvent) {
        super.mouseDown(with: event)
        if event.clickCount == 2 {
            let point = convert(event.locationInWindow, from: nil)
            onDoubleClick?(characterIndexForInsertion(at: point))
        }
    }
}

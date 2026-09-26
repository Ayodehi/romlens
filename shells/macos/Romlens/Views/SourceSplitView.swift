import AppKit
import RomlensKit
import SwiftUI

/// The Source tab (docs/22, S2): a source file of an imported `.dbg` on
/// the left, the disassembly on the right. A line selects the bytes it
/// made; a selection shows the line that made it, in whichever file.
struct SourceSplitView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSSplitView {
        let source = SourcePaneController(model: model)
        let asm = AsmPaneController(model: model)
        let split = CSplitPaneView()
        split.isVertical = true
        split.dividerStyle = .thin
        split.autosaveName = "source"
        split.addArrangedSubview(source.view)
        split.addArrangedSubview(asm.scrollView)
        context.coordinator.source = source
        context.coordinator.asm = asm
        return split
    }

    func updateNSView(_ split: NSSplitView, context: Context) {
        context.coordinator.source?.update()
        context.coordinator.asm?.update()
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSSplitView, context: Context) -> CGSize? {
        proposal.replacingUnspecifiedDimensions(by: CGSize(width: 400, height: 300))
    }

    @MainActor
    final class Coordinator {
        var source: SourcePaneController?
        var asm: AsmPaneController?
    }
}

@MainActor
final class SourcePaneController: NSObject, NSTextViewDelegate {
    let model: RomViewModel
    let view = NSView()
    private let files = NSPopUpButton(frame: .zero, pullsDown: false)
    private let status = NSTextField(labelWithString: "")
    private let allow = NSButton(title: "Allow Access…", target: nil, action: nil)
    private let scrollView = NSScrollView()
    let textView = NSTextView()
    private var shownGeneration = -1
    private var shownFiles: [SourceFileInfo] = []
    /// The selection the highlight was worked out for.
    private var highlightedFor: UInt32?
    /// The selection whose file was last shown: a file chosen by hand stays
    /// until the selection moves.
    private var followed: UInt32?
    private var highlighted: [Int] = []
    /// UTF-16 offset where each line starts, plus the end.
    private var lineStarts: [Int] = []
    /// A click on a line moved the selection: the source need not scroll.
    private var selectingFromText = false
    private var replacingText = false
    /// Width of the line-number gutter, in characters with its spaces.
    private var gutter = 0

    init(model: RomViewModel) {
        self.model = model
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
        textView.setAccessibilityIdentifier("source-text")
        scrollView.documentView = textView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor

        files.controlSize = .small
        files.target = self
        files.action = #selector(fileChosen(_:))
        files.setAccessibilityIdentifier("source-file")
        status.textColor = .secondaryLabelColor
        status.lineBreakMode = .byTruncatingTail
        status.setContentCompressionResistancePriority(.init(1), for: .horizontal)
        allow.controlSize = .small
        allow.bezelStyle = .rounded
        allow.target = self
        allow.action = #selector(allowAccess(_:))
        allow.isHidden = true

        let header = NSStackView(views: [files, status, NSView(), allow])
        header.orientation = .horizontal
        header.spacing = 8
        header.edgeInsets = NSEdgeInsets(top: 4, left: 8, bottom: 4, right: 8)
        header.setHuggingPriority(.defaultHigh, for: .vertical)
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

    private var source: SourceModel { model.source }

    func update() {
        // The line that made the selection, in whichever file it is.
        let selected = model.instruction?.fileOffset ?? model.selectedOffset
        let lines = selected.map { model.workbench.sourceLinesAt(fileOffset: $0) } ?? []
        if selected != followed {
            followed = selected
            if let first = lines.first { source.show(fileOf: first, workbench: model.workbench) }
        }
        if source.files != shownFiles {
            shownFiles = source.files
            files.removeAllItems()
            for f in source.files {
                files.addItem(withTitle: source.files.filter({ $0.name == f.name }).count > 1 ? "\(f.name) (\(f.import))" : f.name)
            }
        }
        if let i = source.shown, i < files.numberOfItems { files.selectItem(at: i) }
        if shownGeneration != source.generation {
            shownGeneration = source.generation
            setText()
            highlightedFor = nil
        }
        let f = source.shownFile
        let here = lines.filter { l in f.map { $0.map == l.map && $0.file == l.file } ?? false }
        let wanted = here.map { Int($0.line) - 1 }
        if selected != highlightedFor || wanted != highlighted {
            highlightedFor = selected
            setHighlight(wanted)
            if !selectingFromText, let first = wanted.first { scrollToLine(first) }
        }
        selectingFromText = false
        if let f {
            let made = "\(f.lines) of its lines made bytes"
            status.stringValue = source.problem.map { "\(made). \($0)" } ?? made
            status.toolTip = "\(f.path)\nfrom \(f.import)"
        } else {
            status.stringValue = "No source files"
        }
        allow.isHidden = source.text != nil
    }

    private func setText() {
        let text = source.text ?? []
        let digits = max(3, String(text.count).count)
        gutter = digits + 2
        let font = model.metrics.font
        let s = NSMutableAttributedString()
        let number: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.tertiaryLabelColor]
        let made: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.labelColor]
        let none: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.secondaryLabelColor]
        var starts: [Int] = []
        for (i, line) in text.enumerated() {
            starts.append(s.length)
            let n = String(i + 1)
            s.append(NSAttributedString(string: String(repeating: " ", count: digits - n.count) + n + "  ", attributes: number))
            let bytes = source.byLine[UInt32(i + 1)] != nil
            s.append(NSAttributedString(string: line + "\n", attributes: bytes ? made : none))
        }
        starts.append(s.length)
        lineStarts = starts
        replacingText = true
        textView.textStorage?.setAttributedString(s)
        textView.setSelectedRange(NSRange(location: 0, length: 0))
        replacingText = false
        highlighted = []
        scrollView.contentView.scroll(to: .zero)
        scrollView.reflectScrolledClipView(scrollView.contentView)
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

    /// The 0-based line holding UTF-16 offset `index`.
    private func line(at index: Int) -> Int {
        var lo = 0
        var hi = max(0, lineStarts.count - 2)
        while lo < hi {
            let mid = (lo + hi + 1) / 2
            if lineStarts[mid] <= index { lo = mid } else { hi = mid - 1 }
        }
        return lo
    }

    /// A click on a line selects the bytes it made. A macro's line made
    /// bytes once per expansion: clicking it again goes to the next.
    func textViewDidChangeSelection(_ notification: Notification) {
        guard !replacingText, lineStarts.count > 1 else { return }
        let n = UInt32(line(at: textView.selectedRange().location) + 1)
        guard let info = source.byLine[n], !info.ranges.isEmpty else { return }
        let current = model.selectedOffset
        let at = info.ranges.firstIndex { r in current.map { $0 >= r.start && $0 < r.start + r.len } ?? false }
        let pick = info.ranges[at.map { ($0 + 1) % info.ranges.count } ?? 0]
        selectingFromText = true
        model.selectRange(pick.start..<pick.start + pick.len)
        model.requestScroll(toOffset: pick.start)
    }

    @objc private func fileChosen(_ sender: NSPopUpButton) {
        source.show(sender.indexOfSelectedItem, workbench: model.workbench)
        update()
    }

    @objc private func allowAccess(_ sender: Any?) {
        guard let f = source.shownFile else { return }
        let folder = URL(fileURLWithPath: f.path).deletingLastPathComponent()
        SourceFolders.ask(
            start: folder,
            message: "Choose the folder holding \(f.name), or one above it, so Romlens can show the source.",
            window: view.window
        ) { [weak self] url in
            guard url != nil, let self else { return }
            self.source.retry()
            self.update()
        }
    }
}

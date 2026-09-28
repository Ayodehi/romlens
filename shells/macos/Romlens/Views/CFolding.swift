import AppKit

/// A `{ … }` block of the C that spans lines, which the C pane can fold
/// the way an IDE does: `if (c) {…}` on one line until it is opened again.
struct CFold: Equatable {
    /// UTF-16 offsets of the two braces.
    let open: Int
    let close: Int

    /// What folding hides: everything between the braces. Its first
    /// character draws as `…`.
    var hidden: NSRange { NSRange(location: open + 1, length: close - open - 1) }

    func contains(_ index: Int) -> Bool { open < index && index <= close }

    /// The blocks in C text that span lines, in the order they open.
    /// Braces in comments, strings and character literals are not blocks.
    static func find(in text: String) -> [CFold] {
        let u = Array(text.utf16)
        let slash = UInt16(UInt8(ascii: "/")), star = UInt16(UInt8(ascii: "*"))
        let quote = UInt16(UInt8(ascii: "\"")), tick = UInt16(UInt8(ascii: "'"))
        let backslash = UInt16(UInt8(ascii: "\\")), newline = UInt16(UInt8(ascii: "\n"))
        let lbrace = UInt16(UInt8(ascii: "{")), rbrace = UInt16(UInt8(ascii: "}"))
        var out: [CFold] = []
        // Each open brace, with how many lines had ended before it.
        var opens: [(at: Int, lines: Int)] = []
        var lines = 0
        var i = 0
        while i < u.count {
            let c = u[i]
            let next = i + 1 < u.count ? u[i + 1] : 0
            if c == slash, next == star {
                i += 2
                while i < u.count, !(u[i] == star && i + 1 < u.count && u[i + 1] == slash) {
                    if u[i] == newline { lines += 1 }
                    i += 1
                }
                i += 2
                continue
            }
            if c == slash, next == slash {
                while i < u.count, u[i] != newline { i += 1 }
                continue
            }
            if c == quote || c == tick {
                i += 1
                while i < u.count, u[i] != c, u[i] != newline {
                    i += u[i] == backslash ? 2 : 1
                }
                i += 1
                continue
            }
            switch c {
            case newline: lines += 1
            case lbrace: opens.append((i, lines))
            case rbrace:
                if let o = opens.popLast(), o.lines < lines {
                    out.append(CFold(open: o.at, close: i))
                }
            default: break
            }
            i += 1
        }
        return out.sorted { $0.open < $1.open }
    }
}

/// Which of the C's blocks are folded, and the layout manager's delegate
/// that hides them. The text itself never changes, so every offset the pane
/// keeps (lines, tokens, highlights, a version's anchors) stays good.
final class CFolder: NSObject, NSLayoutManagerDelegate {
    private(set) var folds: [CFold] = []
    private(set) var folded: Set<Int> = []
    /// What is hidden: the outermost folded blocks, in order.
    private(set) var hidden: [NSRange] = []
    weak var textView: NSTextView?
    /// The folds changed: the gutter redraws.
    var changed: (() -> Void)?

    /// New text is coming: nothing is folded until `set` says so.
    func clear() {
        folds = []
        folded = []
        hidden = []
    }

    /// The text's blocks, and which of them start folded.
    func set(_ folds: [CFold], folded: Set<Int>) {
        self.folds = folds
        self.folded = folded.intersection(folds.map(\.open))
        apply()
    }

    func isFolded(_ f: CFold) -> Bool { folded.contains(f.open) }

    /// Whether `index` is hidden inside a folded block.
    func isHidden(_ index: Int) -> Bool {
        hidden.contains { NSLocationInRange(index, $0) }
    }

    func toggle(_ f: CFold) {
        if folded.remove(f.open) == nil { folded.insert(f.open) }
        apply()
    }

    /// Fold the block opened on the line `line`, else the innermost open one
    /// around `index`.
    @discardableResult
    func fold(at index: Int, line: NSRange) -> Bool {
        let f = folds.last { NSLocationInRange($0.open, line) && !isFolded($0) }
            ?? folds.last { $0.contains(index) && !isFolded($0) }
        guard let f else { return false }
        folded.insert(f.open)
        apply()
        return true
    }

    /// Open the folded blocks on the line `line` and around `index`.
    @discardableResult
    func unfold(at index: Int, line: NSRange) -> Bool {
        let opened = folds.filter {
            isFolded($0) && (NSLocationInRange($0.open, line) || $0.contains(index))
        }
        guard !opened.isEmpty else { return false }
        folded.subtract(opened.map(\.open))
        apply()
        return true
    }

    /// Fold every block inside the routine, so its body reads as its outline
    /// and each block opens one level at a time.
    func foldAll() {
        let outermost = folds.filter { f in !folds.contains { $0 != f && $0.contains(f.open) } }
        folded = Set(folds.map(\.open)).subtracting(outermost.map(\.open))
        apply()
    }

    func unfoldAll() {
        folded = []
        apply()
    }

    /// Open whatever hides any of `range`: the selection moved there.
    @discardableResult
    func reveal(_ range: NSRange) -> Bool {
        let opened = folds.filter { f in
            isFolded(f) && NSIntersectionRange(f.hidden, range).length > 0
        }
        guard !opened.isEmpty else { return false }
        folded.subtract(opened.map(\.open))
        apply()
        return true
    }

    /// The folded block whose `…` is the character at `index`.
    func placeholder(at index: Int) -> CFold? {
        folds.first { isFolded($0) && $0.hidden.location == index && !isHidden($0.open) }
    }

    private func apply() {
        var out: [NSRange] = []
        for f in folds where folded.contains(f.open) {
            if let last = out.last, NSLocationInRange(f.open, last) { continue }
            out.append(f.hidden)
        }
        hidden = out
        if let lm = textView?.layoutManager, let storage = textView?.textStorage {
            let all = NSRange(location: 0, length: storage.length)
            lm.invalidateGlyphs(forCharacterRange: all, changeInLength: 0, actualCharacterRange: nil)
            lm.invalidateLayout(forCharacterRange: all, actualCharacterRange: nil)
            lm.removeTemporaryAttribute(.foregroundColor, forCharacterRange: all)
            for r in hidden {
                lm.addTemporaryAttribute(
                    .foregroundColor, value: NSColor.secondaryLabelColor,
                    forCharacterRange: NSRange(location: r.location, length: 1))
            }
            if let container = textView?.textContainer { lm.ensureLayout(for: container) }
            textView?.needsDisplay = true
        }
        changed?()
    }

    // MARK: NSLayoutManagerDelegate

    func layoutManager(
        _ layoutManager: NSLayoutManager,
        shouldGenerateGlyphs glyphs: UnsafePointer<CGGlyph>,
        properties props: UnsafePointer<NSLayoutManager.GlyphProperty>,
        characterIndexes charIndexes: UnsafePointer<Int>,
        font aFont: NSFont,
        forGlyphRange glyphRange: NSRange
    ) -> Int {
        guard !hidden.isEmpty, glyphRange.length > 0 else { return 0 }
        let n = glyphRange.length
        let first = charIndexes[0], last = charIndexes[n - 1]
        guard hidden.contains(where: { $0.location <= last && first < NSMaxRange($0) }) else { return 0 }
        var g = Array(UnsafeBufferPointer(start: glyphs, count: n))
        var p = Array(UnsafeBufferPointer(start: props, count: n))
        var ellipsis: [CGGlyph] = [0]
        var dots: [UniChar] = [0x2026]
        CTFontGetGlyphsForCharacters(aFont as CTFont, &dots, &ellipsis, 1)
        for i in 0..<n {
            let c = charIndexes[i]
            guard let r = hidden.first(where: { NSLocationInRange(c, $0) }) else { continue }
            if c == r.location, ellipsis[0] != 0 {
                g[i] = ellipsis[0]
                p[i] = []
            } else {
                p[i] = .null
            }
        }
        layoutManager.setGlyphs(g, properties: p, characterIndexes: charIndexes, font: aFont, forGlyphRange: glyphRange)
        return n
    }

    /// A hidden line end takes no room and ends no line, so the block's
    /// closing brace lands after the `…`.
    func layoutManager(
        _ layoutManager: NSLayoutManager,
        shouldUse action: NSLayoutManager.ControlCharacterAction,
        forControlCharacterAt charIndex: Int
    ) -> NSLayoutManager.ControlCharacterAction {
        isHidden(charIndex) ? .zeroAdvancement : action
    }
}

/// The C pane's fold gutter: an arrow beside each block that spans lines,
/// pointing down while it is open and right while it is folded.
final class CFoldGutter: NSRulerView {
    let folder: CFolder

    init(scrollView: NSScrollView, textView: NSTextView, folder: CFolder) {
        self.folder = folder
        super.init(scrollView: scrollView, orientation: .verticalRuler)
        clientView = textView
        ruleThickness = 14
        clipsToBounds = true
        reservedThicknessForMarkers = 0
        reservedThicknessForAccessoryView = 0
        setAccessibilityIdentifier("c-folds")
        NotificationCenter.default.addObserver(
            self, selector: #selector(scrolled), name: NSView.boundsDidChangeNotification,
            object: scrollView.contentView)
        scrollView.contentView.postsBoundsChangedNotifications = true
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError("not used") }

    @objc private func scrolled(_ n: Notification) { needsDisplay = true }

    /// Each block whose first line shows, with that line's rect in the
    /// gutter's coordinates.
    func arrows() -> [(fold: CFold, rect: NSRect)] {
        guard let tv = clientView as? NSTextView, let lm = tv.layoutManager,
              let storage = tv.textStorage else { return [] }
        var out: [(CFold, NSRect)] = []
        var seenLine = -1.0
        for f in folder.folds where f.open < storage.length && !folder.isHidden(f.open) {
            let g = lm.glyphIndexForCharacter(at: f.open)
            let line = lm.lineFragmentRect(forGlyphAt: g, effectiveRange: nil, withoutAdditionalLayout: true)
            // Two blocks opening on one line: the arrow is the first's.
            guard line.minY != seenLine else { continue }
            seenLine = line.minY
            let inView = line.offsetBy(dx: 0, dy: tv.textContainerOrigin.y)
            let top = convert(NSPoint(x: 0, y: inView.minY), from: tv).y
            let bottom = convert(NSPoint(x: 0, y: inView.maxY), from: tv).y
            out.append((f, NSRect(x: 0, y: min(top, bottom), width: ruleThickness, height: abs(bottom - top))))
        }
        return out
    }

    override func draw(_ dirtyRect: NSRect) {
        // Views need not clip to their bounds (macOS 14): the dirty rect
        // can reach over the text beside the gutter.
        let rect = dirtyRect.intersection(bounds)
        NSColor.textBackgroundColor.setFill()
        rect.fill()
        drawHashMarksAndLabels(in: rect)
    }

    override func drawHashMarksAndLabels(in rect: NSRect) {
        for (f, line) in arrows() where line.intersects(rect) {
            let folded = folder.isFolded(f)
            let name = folded ? "chevron.right" : "chevron.down"
            let config = NSImage.SymbolConfiguration(pointSize: 8, weight: .semibold)
                .applying(.init(paletteColors: [folded ? .secondaryLabelColor : .tertiaryLabelColor]))
            guard let image = NSImage(systemSymbolName: name, accessibilityDescription: folded ? "Unfold" : "Fold")?
                .withSymbolConfiguration(config) else { continue }
            let size = image.size
            let at = NSRect(
                x: (ruleThickness - size.width) / 2, y: line.midY - size.height / 2,
                width: size.width, height: size.height)
            image.draw(in: at, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        }
    }

    override func mouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        if let hit = arrows().first(where: { $0.rect.minY <= p.y && p.y < $0.rect.maxY }) {
            folder.toggle(hit.fold)
        }
    }
}

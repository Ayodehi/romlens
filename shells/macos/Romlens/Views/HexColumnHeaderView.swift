import AppKit
import CoreText

/// The sticky row above the hex canvas: column labels for the address
/// columns, `00`–`0F` over the bytes and `ASCII` over the text, aligned with
/// the same `HexRowLayout`, so a byte's offset within the row can be read
/// off without counting. The selected byte's column is emphasised.
final class HexColumnHeaderView: NSView {
    let model: RomViewModel
    /// Horizontal scroll offset of the canvas, mirrored so columns line up.
    var scrollOffsetX: CGFloat = 0 { didSet { if scrollOffsetX != oldValue { needsDisplay = true } } }
    var selectedByte: Int? { didSet { if selectedByte != oldValue { needsDisplay = true } } }

    init(model: RomViewModel) {
        self.model = model
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }

    var headerHeight: CGFloat { model.layout.rowHeight + 2 }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.setFillColor(NSColor.windowBackgroundColor.cgColor)
        context.fill(bounds)
        context.setFillColor(NSColor.separatorColor.cgColor)
        context.fill(CGRect(x: 0, y: bounds.height - 1, width: bounds.width, height: 1))

        let layout = model.layout
        let h = bounds.height - 1
        let baseline = (h - (layout.ascent + layout.descent)) / 2 + layout.ascent
        let dim = NSColor.secondaryLabelColor.cgColor
        let strong = NSColor.labelColor.cgColor
        func label(_ text: String, atChar c: Int, emphasised: Bool = false) {
            let attributes: [NSAttributedString.Key: Any] = [
                .font: emphasised ? NSFont.monospacedSystemFont(ofSize: layout.font.pointSize, weight: .bold) : layout.font,
                NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
            ]
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attributes))
            context.saveGState()
            context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
            context.setFillColor(emphasised ? strong : dim)
            context.textPosition = CGPoint(x: layout.x(ofChar: c) - scrollOffsetX, y: baseline)
            CTLineDraw(line, context)
            context.restoreGState()
        }
        if let selectedByte {
            context.setFillColor(NSColor.controlAccentColor.withAlphaComponent(0.25).cgColor)
            var r = layout.hexRect(byte: selectedByte, height: h)
            r.origin.x -= scrollOffsetX
            r.origin.y = 1
            r.size.height = h - 2
            context.fill(r)
        }
        switch layout.style {
        case .both:
            label("File", atChar: 0)
            label("SNES", atChar: 10)
        case .file:
            label("File", atChar: 0)
        case .snes:
            label("SNES", atChar: 0)
        }
        for i in 0..<HexRowRecord.bytesPerRow {
            label(String(format: "%02X", i), atChar: layout.hexColumn(byte: i), emphasised: i == selectedByte)
        }
        label("ASCII", atChar: layout.asciiColumn(byte: 0))
    }
}

/// Header on top, scroll view below; keeps the header's columns aligned with
/// a horizontally scrolled canvas.
final class HexPaneView: NSView {
    let header: HexColumnHeaderView
    let scrollView: NSScrollView

    init(header: HexColumnHeaderView, scrollView: NSScrollView) {
        self.header = header
        self.scrollView = scrollView
        super.init(frame: .zero)
        addSubview(header)
        addSubview(scrollView)
        scrollView.contentView.postsBoundsChangedNotifications = true
        // Selector-based observers are unregistered automatically on dealloc.
        NotificationCenter.default.addObserver(
            self, selector: #selector(clipBoundsChanged(_:)),
            name: NSView.boundsDidChangeNotification, object: scrollView.contentView
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    @objc private func clipBoundsChanged(_ note: Notification) {
        header.scrollOffsetX = scrollView.contentView.bounds.origin.x
    }

    override var isFlipped: Bool { true }

    override func layout() {
        super.layout()
        let h = header.headerHeight
        header.frame = NSRect(x: 0, y: 0, width: bounds.width, height: h)
        scrollView.frame = NSRect(x: 0, y: h, width: bounds.width, height: max(0, bounds.height - h))
    }

    func headerHeightDidChange() {
        needsLayout = true
        header.needsDisplay = true
    }
}

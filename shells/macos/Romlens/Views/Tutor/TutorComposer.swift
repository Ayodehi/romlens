import AppKit
import SwiftUI

/// The composer and what sits over it: the selection chip, the pictures to
/// send, and the commands `/` offers.
struct ComposerArea: View {
    @Bindable var tutor: TutorModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            let commands = tutor.matchingCommands
            if !commands.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(commands) { c in
                        Button {
                            tutor.composer = c.name + " "
                        } label: {
                            HStack {
                                Text(c.name).font(.callout.monospaced())
                                Text(c.about).font(.caption).foregroundStyle(.secondary)
                                Spacer()
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(8)
                .background(RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .controlBackgroundColor)))
            }
            HStack(spacing: 6) {
                if let chip = tutor.selectionChip {
                    Button {
                        tutor.includeSelection.toggle()
                    } label: {
                        Label(chip, systemImage: tutor.includeSelection ? "scope" : "circle.slash")
                            .font(.caption.monospaced())
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Capsule().fill(Color.accentColor.opacity(tutor.includeSelection ? 0.18 : 0.05)))
                    }
                    .buttonStyle(.plain)
                    .help(tutor.includeSelection ? "The question goes with this selection; click to leave it out" : "Sent without the selection; click to send it")
                }
                if tutor.canAttachFrame {
                    Button { tutor.attachFrame() } label: {
                        Label("Frame \(tutor.rom.graphics.frame)", systemImage: "photo.badge.plus").font(.caption)
                    }
                    .buttonStyle(.plain)
                    .help("Attach the recording's frame as a picture")
                }
                ForEach(tutor.attachments) { a in
                    HStack(spacing: 2) {
                        if let image = a.image {
                            Image(nsImage: image).resizable().interpolation(.none).aspectRatio(contentMode: .fit).frame(height: 22)
                        }
                        Button { tutor.attachments.removeAll { $0.id == a.id } } label: { Image(systemName: "xmark.circle.fill") }
                            .buttonStyle(.plain).foregroundStyle(.secondary)
                    }
                    .help(a.name)
                }
                Spacer()
            }
            HStack(alignment: .bottom, spacing: 8) {
                ComposerField(tutor: tutor)
                    .frame(height: Self.height(tutor.composer))
                Button {
                    tutor.submit()
                } label: {
                    Image(systemName: "arrow.up.circle.fill").font(.title2)
                }
                .buttonStyle(.plain)
                .disabled(!tutor.canSend)
                .help("Send (Return)")
            }
        }
        .padding(10)
    }
}

extension ComposerArea {
    /// One line to seven, as the text grows; past that it scrolls.
    static func height(_ text: String) -> CGFloat {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).reduce(0) { n, l in
            n + max(1, Int((Double(l.count) / 60).rounded(.up)))
        }
        return CGFloat(min(max(lines, 1), 7)) * 17 + 14
    }
}

/// The text field itself: an `NSTextView`, for the keys a chat needs.
struct ComposerField: NSViewRepresentable {
    let tutor: TutorModel

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        let tv = ComposerTextView()
        tv.tutor = tutor
        tv.delegate = context.coordinator
        tv.isRichText = false
        tv.allowsUndo = true
        tv.font = .systemFont(ofSize: NSFont.systemFontSize)
        tv.textContainerInset = NSSize(width: 4, height: 6)
        tv.isAutomaticQuoteSubstitutionEnabled = false
        tv.isAutomaticDashSubstitutionEnabled = false
        tv.minSize = NSSize(width: 0, height: 0)
        tv.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        tv.isVerticallyResizable = true
        tv.autoresizingMask = [.width]
        tv.textContainer?.widthTracksTextView = true
        tv.setAccessibilityLabel("Ask the tutor")
        scroll.documentView = tv
        scroll.wantsLayer = true
        scroll.layer?.cornerRadius = 8
        scroll.layer?.borderWidth = 1
        scroll.layer?.borderColor = NSColor.separatorColor.cgColor
        DispatchQueue.main.async { tv.window?.makeFirstResponder(tv) }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let tv = scroll.documentView as? ComposerTextView else { return }
        if tv.string != tutor.composer {
            tv.string = tutor.composer
            tv.setSelectedRange(NSRange(location: (tv.string as NSString).length, length: 0))
        }
        tv.placeholder = tutor.busy ? "Esc stops it" : "Ask about the ROM, or / for commands"
        tv.needsDisplay = true
    }

    func makeCoordinator() -> Coordinator { Coordinator(tutor: tutor) }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        let tutor: TutorModel
        init(tutor: TutorModel) { self.tutor = tutor }
        func textDidChange(_ n: Notification) {
            guard let tv = n.object as? NSTextView else { return }
            if tutor.composer != tv.string { tutor.composer = tv.string }
        }
    }
}

/// Return sends and ⇧Return starts a line; ↑ on the first line and ↓ on
/// the last walk the history; ⇧⇥ changes the mode; Tab completes a
/// command; Esc stops a turn, and twice opens rewind; a picture pasted is
/// attached.
final class ComposerTextView: NSTextView {
    weak var tutor: TutorModel?
    var placeholder = ""
    private var lastEscape = Date.distantPast

    override func keyDown(with event: NSEvent) {
        guard let tutor else { return super.keyDown(with: event) }
        let shift = event.modifierFlags.contains(.shift)
        switch event.keyCode {
        case 36 where !shift, 76 where !shift: // Return, Enter
            if !event.modifierFlags.contains(.option) {
                tutor.submit()
                return
            }
        case 126 where onFirstLine: // ↑
            if tutor.historyUp() { return }
        case 125 where onLastLine: // ↓
            if tutor.historyDown() { return }
        case 48 where !shift: // Tab
            if let first = tutor.matchingCommands.first {
                tutor.composer = first.name + " "
                return
            }
        default:
            break
        }
        super.keyDown(with: event)
    }

    override func insertBacktab(_ sender: Any?) { tutor?.cycleMode() }

    override func cancelOperation(_ sender: Any?) {
        guard let tutor else { return }
        if tutor.busy {
            tutor.stop()
        } else if Date().timeIntervalSince(lastEscape) < 0.6 {
            tutor.sheet = .rewind
        } else if !string.isEmpty && tutor.matchingCommands.isEmpty == false {
            tutor.composer = ""
        }
        lastEscape = Date()
    }

    private var onFirstLine: Bool {
        let s = string as NSString
        let at = selectedRange().location
        return s.range(of: "\n", options: [], range: NSRange(location: 0, length: min(at, s.length))).location == NSNotFound
    }

    private var onLastLine: Bool {
        let s = string as NSString
        let at = selectedRange().location + selectedRange().length
        return s.range(of: "\n", options: [], range: NSRange(location: at, length: s.length - at)).location == NSNotFound
    }

    override func paste(_ sender: Any?) {
        let pb = NSPasteboard.general
        if pb.string(forType: .string) == nil, let tutor,
           let image = NSImage(pasteboard: pb) {
            tutor.attach(image: image)
            return
        }
        if let urls = pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL],
           !urls.isEmpty, let tutor, pb.string(forType: .string) == nil {
            urls.forEach { tutor.attach(file: $0) }
            return
        }
        super.paste(sender)
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty else { return }
        let attrs: [NSAttributedString.Key: Any] = [
            .font: font ?? .systemFont(ofSize: NSFont.systemFontSize),
            .foregroundColor: NSColor.placeholderTextColor,
        ]
        let x = textContainerInset.width + (textContainer?.lineFragmentPadding ?? 0)
        (placeholder as NSString).draw(at: NSPoint(x: x, y: textContainerInset.height), withAttributes: attrs)
    }
}

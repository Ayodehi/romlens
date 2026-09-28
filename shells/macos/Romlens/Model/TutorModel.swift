import AppKit
import Foundation
import Observation
import RomlensKit
import UniformTypeIdentifiers

/// The Tutor window's model (docs/24): one per project. It holds the core's
/// `TutorSession`, the turn streaming in, the edit cards waiting, the
/// composer and its history, and the slash commands.
@MainActor @Observable
final class TutorModel {
    /// A tool call in the log, with its result once it has one.
    struct ToolRow: Identifiable, Equatable {
        let id: String
        var name: String
        var input: String
        var summary: String?
        /// Pictures its result holds.
        var images: [String] = []
        var isError = false
        var done = false
    }

    enum CardState: Equatable { case waiting, accepted, declined }

    struct Card: Identifiable, Equatable {
        let proposal: ProposalInfo
        var state: CardState
        var id: String { proposal.id }
    }

    /// The turn streaming in.
    struct Live: Equatable {
        /// The question, shown until the transcript has it.
        var question = ""
        var pictures: [Data] = []
        var text = ""
        var reasoning = ""
        var tools: [ToolRow] = []
        var cards: [Card] = []
        var status: String?
    }

    struct Attachment: Identifiable {
        let id = UUID()
        let data: Data
        let mediaType: String
        let image: NSImage?
        let name: String
    }

    enum Sheet: Identifiable {
        case resume, rewind, model, help
        var id: Self { self }
    }

    let rom: RomViewModel
    let settings: TutorSettings
    private let root: URL
    private(set) var session: TutorSession?
    private var bridge: Bridge?

    private(set) var turns: [TurnInfo] = []
    private(set) var live: Live?
    private(set) var busy = false
    var error: String?
    var composer = ""
    var attachments: [Attachment] = []
    /// Send the main window's selection with the question.
    var includeSelection = true
    var sheet: Sheet?
    private(set) var cost = 0.0
    /// The conversation's name: the model's, after the first answer.
    private(set) var title: String?
    private(set) var contextUsed = 0.0
    private(set) var modelName: String?
    private(set) var endpointName: String?
    private(set) var mode: TutorModePreference
    /// Cards accepted without asking for the rest of this turn.
    private var acceptRest = false
    /// ↑ and ↓ walk this; `nil` when not walking.
    private var history: [String] = []
    private var historyIndex: Int?
    private var draft = ""

    init(rom: RomViewModel, settings: TutorSettings = .shared, root: URL? = nil) {
        self.rom = rom
        self.settings = settings
        self.root = root ?? Self.defaultRoot
        mode = settings.mode
    }

    /// Conversations live in the app's own folder, never in a project
    /// (`12-content-policy.md` rule 8).
    static var defaultRoot: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("Tutor", isDirectory: true)
    }

    // MARK: The session

    /// Made on first use, so a project that never opens the tutor costs
    /// nothing.
    func ensureSession() -> TutorSession {
        if let session { return session }
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let bridge = Bridge()
        let s = TutorSession(
            workbench: rom.session.workbench, root: root.path,
            credentials: TutorCredentials(store: settings.keys), listener: bridge)
        bridge.model = self
        self.bridge = bridge
        session = s
        history = s.promptHistory(limit: 500)
        return s
    }

    /// Starts a conversation on the default provider unless one is open.
    @discardableResult
    func startIfNeeded() -> Bool {
        let s = ensureSession()
        if s.conversationId() != nil { return true }
        return newConversation()
    }

    @discardableResult
    func newConversation() -> Bool {
        let s = ensureSession()
        let e = settings.defaultEndpoint
        guard let model = settings.model(for: e), !model.isEmpty else {
            error = "Choose a model for \(e.name) in Settings."
            return false
        }
        do {
            _ = try s.newConversation(
                endpoint: e.info, model: model, effort: settings.effort(for: e),
                mode: mode.mode, costCap: settings.costCap)
            refresh()
            return true
        } catch {
            self.error = Self.message(error)
            return false
        }
    }

    func resume(_ id: String) {
        do {
            _ = try ensureSession().resume(id: id)
            refresh()
        } catch { self.error = Self.message(error) }
    }

    var conversations: [ConversationSummaryInfo] { ensureSession().conversations() }

    func refresh() {
        guard let s = session else { return }
        turns = s.transcript()
        title = s.title()
        cost = s.cost()
        contextUsed = s.contextUsed()
        modelName = s.model()
        endpointName = s.endpoint().flatMap { e in settings.endpoint(e.id)?.name ?? e.id }
        mode = TutorModePreference(s.mode())
    }

    // MARK: Asking

    var canSend: Bool {
        !busy && !(composer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && attachments.isEmpty)
    }

    /// The composer's text: a slash command, or a question.
    func submit() {
        let text = composer.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("/") {
            composer = ""
            historyIndex = nil
            run(command: text)
            return
        }
        send()
    }

    func send() {
        guard canSend else { return }
        let text = composer.trimmingCharacters(in: .whitespacesAndNewlines)
        let e = settings.endpoint(session?.endpoint()?.id ?? settings.defaultEndpoint.id) ?? settings.defaultEndpoint
        guard settings.ready(e) else {
            error = "Add a key for \(e.name) in Settings (⌘,)."
            return
        }
        guard startIfNeeded(), let s = session else { return }
        s.setRecording(recording: rom.graphics.recording)
        s.setImageProvider(endpoint: settings.imageEndpoint?.info, model: settings.imageModel)
        let files = attachments.map { AttachmentInfo(bytes: $0.data, mediaType: $0.mediaType) }
        do {
            try s.send(text: text, attachments: files, selection: includeSelection ? selectionText() : nil)
        } catch {
            self.error = Self.message(error)
            return
        }
        if !text.isEmpty, history.last != text { history.append(text) }
        historyIndex = nil
        composer = ""
        attachments = []
        error = nil
        acceptRest = false
        busy = true
        live = Live(question: text, pictures: files.map(\.bytes))
        turns = s.transcript()
    }

    /// Esc: stops the turn, and a waiting card says no.
    func stop() {
        session?.cancel()
    }

    func answer(_ card: Card, accept: Bool, why: String? = nil, andTheRest: Bool = false) {
        acceptRest = accept && andTheRest
        session?.answer(editId: card.id, accept: accept, why: why)
        setCard(card.id, accept ? .accepted : .declined)
    }

    private func setCard(_ id: String, _ state: CardState) {
        guard var l = live, let i = l.cards.firstIndex(where: { $0.id == id }) else { return }
        l.cards[i].state = state
        live = l
    }

    // MARK: Events

    func handle(_ e: TutorEventInfo) {
        switch e {
        case .textDelta(let t):
            live?.text += t
        case .reasoningDelta(let t):
            live?.reasoning += t
        case .toolCallStarted(let id, let name):
            live?.status = "Asking for \(Self.toolTitle(name))…"
            if live?.tools.contains(where: { $0.id == id }) == false {
                live?.tools.append(ToolRow(id: id, name: name, input: ""))
            }
        case .toolArguments(let id, let t):
            if let i = live?.tools.firstIndex(where: { $0.id == id }) { live?.tools[i].input += t }
        case .requesting(let round):
            live?.status = round == 0 ? "Thinking…" : "Reading the results…"
        case .retrying(let attempt, let ms, let why):
            live?.status = "Trying again in \(ms / 1000) s (\(attempt)): \(why)"
        case .toolStarted(let id, let name, let input):
            if let i = live?.tools.firstIndex(where: { $0.id == id }) {
                live?.tools[i].input = input
            } else {
                live?.tools.append(ToolRow(id: id, name: name, input: input))
            }
            live?.status = "\(Self.toolTitle(name))…"
        case .toolFinished(let id, _, let summary, let isError):
            if let i = live?.tools.firstIndex(where: { $0.id == id }) {
                live?.tools[i].summary = summary
                live?.tools[i].isError = isError
                live?.tools[i].done = true
            }
        case .editProposed(let p):
            live?.cards.append(Card(proposal: p, state: .waiting))
            if acceptRest {
                answer(Card(proposal: p, state: .waiting), accept: true, andTheRest: true)
            } else {
                NSApp.requestUserAttention(.informationalRequest)
            }
        case .editDecided(let id, let applied):
            setCard(id, applied ? .accepted : .declined)
            if applied { rom.session.tutorEdited() }
        case .cost(_, let total, _):
            cost = total
        case .discarded:
            live?.text = ""
            live?.reasoning = ""
        case .compacted:
            live?.status = "Summarised to make room."
        case .named(let name, let total):
            title = name
            if !busy { cost = total }
        case .ended:
            finish(error: nil)
        case .failed(let message):
            finish(error: message)
        }
    }

    private func finish(error message: String?) {
        busy = false
        live = nil
        error = message
        refresh()
        rom.session.tutorEdited()
    }

    // MARK: The composer

    /// ↑ at the first line: the prompt before.
    func historyUp() -> Bool {
        guard !history.isEmpty else { return false }
        let i = (historyIndex ?? history.count) - 1
        guard i >= 0 else { return false }
        if historyIndex == nil { draft = composer }
        historyIndex = i
        composer = history[i]
        return true
    }

    /// ↓ at the last line: the one after, then back to the draft.
    func historyDown() -> Bool {
        guard let i = historyIndex else { return false }
        if i + 1 < history.count {
            historyIndex = i + 1
            composer = history[i + 1]
        } else {
            historyIndex = nil
            composer = draft
        }
        return true
    }

    /// Shift-Tab.
    func cycleMode() {
        let all = TutorModePreference.allCases
        let next = all[(all.firstIndex(of: mode)! + 1) % all.count]
        setMode(next)
    }

    func setMode(_ m: TutorModePreference) {
        mode = m
        session?.setMode(mode: m.mode)
    }

    func attach(data: Data, mediaType: String, name: String) {
        let (bytes, type) = Self.fitForModel(data: data, mediaType: mediaType)
        attachments.append(Attachment(data: bytes, mediaType: type, image: NSImage(data: bytes), name: name))
    }

    /// The longest side a provider takes without shrinking it itself
    /// (Claude's high-resolution limit), and the most bytes sent.
    static let maxSide: CGFloat = 2576
    static let maxBytes = 3_500_000

    /// A picture as it can be sent: a photo too big in pixels or bytes for
    /// Claude or OpenAI is scaled down and sent as JPEG; the rest as it is.
    static func fitForModel(data: Data, mediaType: String) -> (Data, String) {
        guard let rep = NSBitmapImageRep(data: data) else { return (data, mediaType) }
        let (w, h) = (CGFloat(rep.pixelsWide), CGFloat(rep.pixelsHigh))
        let long = max(w, h)
        guard long > maxSide || data.count > maxBytes else { return (data, mediaType) }
        let scale = min(1, maxSide / long)
        let size = NSSize(width: (w * scale).rounded(), height: (h * scale).rounded())
        guard let out = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(size.width), pixelsHigh: Int(size.height),
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { return (data, mediaType) }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: out)
        NSGraphicsContext.current?.imageInterpolation = .high
        rep.draw(in: NSRect(origin: .zero, size: size))
        NSGraphicsContext.restoreGraphicsState()
        for quality in [0.85, 0.7, 0.5] {
            if let jpeg = out.representation(using: .jpeg, properties: [.compressionFactor: quality]),
               jpeg.count <= maxBytes {
                return (jpeg, "image/jpeg")
            }
        }
        return (data, mediaType)
    }

    /// A picture from the pasteboard or a drop: PNG, JPEG, GIF, WebP; others
    /// (TIFF, HEIC) are turned into PNG.
    func attach(image: NSImage, name: String = "Pasted picture") {
        guard let tiff = image.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:]) else { return }
        attach(data: png, mediaType: "image/png", name: name)
    }

    func attach(file url: URL) {
        let type = UTType(filenameExtension: url.pathExtension)
        let known: [(UTType, String)] = [(.png, "image/png"), (.jpeg, "image/jpeg"), (.gif, "image/gif"), (.webP, "image/webp")]
        if let t = type, let (_, media) = known.first(where: { t.conforms(to: $0.0) }), let data = try? Data(contentsOf: url) {
            attach(data: data, mediaType: media, name: url.lastPathComponent)
        } else if let image = NSImage(contentsOf: url) {
            attach(image: image, name: url.lastPathComponent)
        } else {
            error = "\(url.lastPathComponent) is not a picture."
        }
    }

    /// The recording's frame the main window shows, as a picture.
    func attachFrame() {
        guard let frame = rom.graphics.frameImage(), let cg = frame.image.cgImage,
              let png = NSBitmapImageRep(cgImage: cg).representation(using: .png, properties: [:]) else {
            error = "Open a recording to attach its frame."
            return
        }
        attach(data: png, mediaType: "image/png", name: "Frame \(rom.graphics.frame)")
    }

    var canAttachFrame: Bool { rom.graphics.hasRecording }

    // MARK: The selection

    /// The chip over the composer.
    var selectionChip: String? {
        guard let a = rom.selectedAddress else { return nil }
        let name = rom.label.map { " \($0.name)" } ?? ""
        return "\(Self.address(a))\(name)"
    }

    /// What goes with a question: where, and the listing there.
    func selectionText() -> String? {
        guard let off = rom.selectedOffset, let a = rom.selectedAddress else { return nil }
        let wb = rom.session.workbench
        var s = Self.address(a)
        if let l = rom.label { s += " (\(l.name))" }
        if let line = wb.lineForOffset(fileOffset: off) {
            s += "\n" + wb.asmLinesText(startLine: line > 4 ? line - 4 : 0, count: 16, style: .snes)
        }
        if let c = cText(at: rom.instruction?.fileOffset ?? off) {
            s += "\n" + c
        }
        if rom.graphics.hasRecording {
            s += "\nA recording is open in Romlens, at frame \(rom.graphics.frame)."
        }
        return s
    }

    /// The C the student is reading when the C tab has the editor: the
    /// version picked there, or the generated C around the selection.
    func cText(at offset: UInt32) -> String? {
        let d = rom.decompiler
        guard rom.editorTab == .c, rom.showsTextEditor, d.state == .ready, let r = d.result else { return nil }
        if let v = d.shownVersion,
           let version = rom.session.workbench.cVersions(routine: r.entry).first(where: { $0.name == v })?.version {
            return "[The student is reading the C tab: the C version “\(v)” of \(r.name), not the generated C.]\n```c\n"
                + Self.clip(version.text, around: nil) + "\n```"
        }
        let at = d.lines(forInstructionAt: offset).first
        return "[The student is reading the C tab: \(r.name) as Romlens generates it, at level \(d.level).]\n```c\n"
            + Self.clip(r.text, around: at) + "\n```"
    }

    /// At most `limit` lines of `text`, centred on line `around`.
    static func clip(_ text: String, around: Int?, limit: Int = 160) -> String {
        let lines = text.components(separatedBy: "\n")
        guard lines.count > limit else { return text }
        let centre = around ?? 0
        let start = max(0, min(centre - limit / 2, lines.count - limit))
        let end = start + limit
        var out: [String] = []
        if start > 0 { out.append("/* … \(start) lines above left out */") }
        out += lines[start..<end]
        if end < lines.count { out.append("/* … \(lines.count - end) lines below left out */") }
        return out.joined(separator: "\n")
    }

    // MARK: Citations

    /// A link from an answer: an address or a frame, shown in the main
    /// window.
    func follow(_ url: URL) -> Bool {
        guard url.scheme == "romlens" else { return false }
        let value = url.lastPathComponent
        switch url.host() {
        case "a":
            guard let a = UInt32(value, radix: 16) else { return false }
            rom.jump(toSnesAddress: a)
        case "f":
            guard let f = UInt64(value), rom.graphics.hasRecording else { return false }
            rom.graphics.frame = f
            rom.graphicsTab = .frame
        default:
            return false
        }
        rom.bringMainWindowForward?()
        return true
    }

    // MARK: Slash commands

    struct Command: Identifiable {
        let name: String
        let about: String
        var id: String { name }
    }

    static let commands: [Command] = [
        Command(name: "/new", about: "Start a new conversation"),
        Command(name: "/clear", about: "Start a new conversation"),
        Command(name: "/resume", about: "Go back to an earlier conversation"),
        Command(name: "/rewind", about: "Go back to an earlier question, and take back the tutor's edits"),
        Command(name: "/model", about: "Change the provider, model or effort"),
        Command(name: "/mode", about: "read-only, ask or accept: what the tutor may change"),
        Command(name: "/compact", about: "Summarise the conversation to make room"),
        Command(name: "/cost", about: "What this conversation has cost"),
        Command(name: "/attach", about: "/attach frame: the recording's frame as a picture"),
        Command(name: "/selection", about: "Send the main window's selection with questions, or not"),
        Command(name: "/details", about: "Show or hide the tutor's thinking and tool calls"),
        Command(name: "/help", about: "What the tutor can do and the keys it takes"),
    ]

    /// The commands the composer's text starts.
    var matchingCommands: [Command] {
        let t = composer.lowercased()
        guard t.hasPrefix("/"), !t.contains(" "), !t.contains("\n") else { return [] }
        return Self.commands.filter { $0.name.hasPrefix(t) && $0.name != "/clear" || ($0.name == "/clear" && t.count > 2 && $0.name.hasPrefix(t)) }
    }

    func run(command line: String) {
        let parts = line.split(separator: " ", maxSplits: 1).map(String.init)
        let arg = parts.count > 1 ? parts[1].trimmingCharacters(in: .whitespaces) : ""
        error = nil
        switch parts.first?.lowercased() ?? "" {
        case "/new", "/clear":
            guard !busy else { error = "Wait for the answer, or press Esc."; return }
            newConversation()
        case "/resume":
            sheet = .resume
        case "/rewind":
            sheet = .rewind
        case "/model":
            sheet = .model
        case "/mode":
            switch arg.lowercased() {
            case "read-only", "readonly", "read": setMode(.readOnly)
            case "ask": setMode(.askBeforeEdits)
            case "accept", "edit", "edits": setMode(.acceptEdits)
            case "": cycleMode()
            default: error = "The modes are read-only, ask and accept."
            }
        case "/compact":
            guard startIfNeeded(), let s = session, !busy else { return }
            do {
                try s.compact()
                busy = true
                live = Live(status: "Summarising…")
            } catch { self.error = Self.message(error) }
        case "/cost":
            live = nil
            error = nil
            costNote = String(format: "This conversation has cost $%.4f.", cost)
        case "/attach":
            if arg.lowercased().hasPrefix("frame") { attachFrame() } else { error = "Try /attach frame, or paste or drop a picture." }
        case "/selection":
            includeSelection.toggle()
        case "/details":
            settings.showWork.toggle()
        case "/help":
            sheet = .help
        default:
            error = "\(parts.first ?? line) is not a command. /help lists them."
        }
    }

    /// A line shown under the transcript by /cost.
    var costNote: String?

    // MARK: Words

    static func address(_ a: UInt32) -> String {
        String(format: "$%02X:%04X", (a >> 16) & 0xFF, a & 0xFFFF)
    }

    static func toolTitle(_ name: String) -> String {
        name.replacingOccurrences(of: "_", with: " ").capitalized(with: nil)
    }

    static func message(_ error: Error) -> String {
        if let e = error as? RomlensError {
            switch e {
            case .Io(let m), .InvalidRom(let m), .BadAddress(let m), .Project(let m), .RomMismatch(let m),
                 .InvalidLabel(let m), .Recording(let m), .Tutor(let m):
                return m
            case .Cancelled:
                return "Stopped."
            }
        }
        return error.localizedDescription
    }

    /// Hands the core's events to the main actor, in order.
    final class Bridge: TutorListener, @unchecked Sendable {
        weak var model: TutorModel?
        func onEvent(event: TutorEventInfo) {
            DispatchQueue.main.async { [weak self] in
                MainActor.assumeIsolated { self?.model?.handle(event) }
            }
        }
    }
}

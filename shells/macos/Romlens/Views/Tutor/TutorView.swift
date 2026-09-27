import AppKit
import RomlensKit
import SwiftUI
import UniformTypeIdentifiers

/// The Tutor window (docs/24): the conversation, then the composer, then a
/// status line.
struct TutorView: View {
    @Bindable var tutor: TutorModel

    var body: some View {
        VStack(spacing: 0) {
            TranscriptView(tutor: tutor)
            Divider()
            ComposerArea(tutor: tutor)
            StatusLine(tutor: tutor)
        }
        .frame(minWidth: 380, minHeight: 360)
        .environment(\.openURL, OpenURLAction { url in tutor.follow(url) ? .handled : .systemAction })
        .onDrop(of: [.fileURL, .image], isTargeted: nil) { providers in
            drop(providers)
            return true
        }
        .sheet(item: $tutor.sheet) { sheet in
            switch sheet {
            case .resume: ResumeSheet(tutor: tutor)
            case .rewind: RewindSheet(tutor: tutor)
            case .model: ModelSheet(tutor: tutor)
            case .help: HelpSheet()
            }
        }
        .onAppear {
            _ = tutor.ensureSession()
            tutor.refresh()
        }
    }

    private func drop(_ providers: [NSItemProvider]) {
        for p in providers {
            if p.canLoadObject(ofClass: URL.self) {
                _ = p.loadObject(ofClass: URL.self) { url, _ in
                    guard let url else { return }
                    DispatchQueue.main.async { MainActor.assumeIsolated { tutor.attach(file: url) } }
                }
            } else if p.canLoadObject(ofClass: NSImage.self) {
                _ = p.loadObject(ofClass: NSImage.self) { image, _ in
                    guard let image = image as? NSImage else { return }
                    let sendable = UncheckedImage(image: image)
                    DispatchQueue.main.async { MainActor.assumeIsolated { tutor.attach(image: sendable.image, name: "Dropped picture") } }
                }
            }
        }
    }
}

/// An `NSImage` handed from a loader's thread to the main one.
struct UncheckedImage: @unchecked Sendable { let image: NSImage }

// MARK: The transcript

/// What the transcript shows, built from the turns.
enum TranscriptRow: Identifiable {
    case question(index: UInt32, text: String, images: [String], selection: Bool)
    case answer(index: UInt32, text: String, reasoning: String, tools: [TutorModel.ToolRow], model: String?, cost: Double)
    case note(index: UInt32, text: String)

    var id: String {
        switch self {
        case .question(let i, _, _, _): "q\(i)"
        case .answer(let i, _, _, _, _, _): "a\(i)"
        case .note(let i, _): "n\(i)"
        }
    }

    static func rows(_ turns: [TurnInfo]) -> [TranscriptRow] {
        var results: [String: (String, Bool, [String])] = [:]
        for t in turns where t.user {
            for case let .toolResult(id, text, images, isError) in t.blocks {
                results[id] = (text.split(separator: "\n").first.map(String.init) ?? "", isError, images)
            }
        }
        var out: [TranscriptRow] = []
        for t in turns where t.sent {
            if t.user {
                var words: [String] = []
                var images: [String] = []
                var selection = false
                var notes: [String] = []
                for b in t.blocks {
                    switch b {
                    case .text(let text):
                        if text.hasPrefix("[The mode is now") || text.hasPrefix("[The student's selection") {
                            selection = selection || text.contains("selection")
                        } else {
                            words.append(text)
                        }
                    case .image(let id, _): images.append(id)
                    case .note(let text): notes.append(text)
                    default: break
                    }
                }
                for n in notes { out.append(.note(index: t.index, text: n)) }
                if !words.isEmpty || !images.isEmpty {
                    out.append(.question(index: t.index, text: words.joined(separator: "\n\n"), images: images, selection: selection))
                }
            } else {
                var text = ""
                var reasoning = ""
                var tools: [TutorModel.ToolRow] = []
                for b in t.blocks {
                    switch b {
                    case .text(let s): text += s
                    case .reasoning(let s): reasoning += reasoning.isEmpty ? s : "\n\n" + s
                    case .toolCall(let id, let name, let input):
                        let r = results[id]
                        tools.append(TutorModel.ToolRow(id: id, name: name, input: input, summary: r?.0, images: r?.2 ?? [], isError: r?.1 ?? false, done: true))
                    default: break
                    }
                }
                out.append(.answer(index: t.index, text: text, reasoning: reasoning, tools: tools, model: t.model, cost: t.cost))
            }
        }
        return out
    }
}

struct TranscriptView: View {
    let tutor: TutorModel
    /// Whether new text scrolls the transcript to the end: until the
    /// student scrolls up, and again once they scroll back to the end.
    @State private var following = true

    /// Where the transcript is scrolled, as far as following cares.
    struct Place: Equatable {
        var offset: CGFloat
        var atEnd: Bool
    }

    /// A scroll up is the student's (text only ever grows below); one that
    /// reaches the end again picks the stream back up.
    static func following(_ was: Bool, from old: Place, to new: Place) -> Bool {
        if new.offset < old.offset - 1 { return false }
        if new.offset > old.offset + 1, new.atEnd { return true }
        return was
    }

    var body: some View {
        let rows = TranscriptRow.rows(tutor.turns)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    if rows.isEmpty && tutor.live == nil {
                        EmptyTutor(tutor: tutor)
                    }
                    ForEach(rows) { row in
                        RowView(tutor: tutor, row: row).id(row.id)
                    }
                    if let live = tutor.live {
                        LiveAnswer(tutor: tutor, live: live).id("live")
                    }
                    if let note = tutor.costNote {
                        Text(note).font(.caption).foregroundStyle(.secondary)
                    }
                    if let e = tutor.error {
                        Label(e, systemImage: "exclamationmark.triangle")
                            .font(.callout).foregroundStyle(.orange).textSelection(.enabled)
                    }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(14)
            }
            .onScrollGeometryChange(for: Place.self) { g in
                Place(offset: g.contentOffset.y,
                      atEnd: g.contentOffset.y + g.containerSize.height >= g.contentSize.height - 40)
            } action: { old, new in
                following = Self.following(following, from: old, to: new)
            }
            .onChange(of: tutor.live?.text.count) { _, _ in follow(proxy) }
            .onChange(of: tutor.live?.cards.count) { _, _ in follow(proxy) }
            .onChange(of: tutor.turns.count) { _, _ in follow(proxy) }
            .onChange(of: tutor.busy) { _, busy in
                // A new question: back to the end, following again.
                if busy {
                    following = true
                    proxy.scrollTo("end", anchor: .bottom)
                }
            }
        }
    }
}

extension TranscriptView {
    private func follow(_ proxy: ScrollViewProxy) {
        if following { proxy.scrollTo("end", anchor: .bottom) }
    }
}

struct EmptyTutor: View {
    let tutor: TutorModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Ask about this ROM").font(.title3.bold())
            Text("Select an instruction, a routine or a frame in the main window, then ask: what it does, why it is written that way, what would change if… The tutor reads Romlens's analysis with its tools and cites every address.")
                .foregroundStyle(.secondary)
            Text("Return sends, ⇧Return starts a line, ↑ brings back what you asked, ⇧⇥ changes what it may edit, Esc stops it, / lists the commands.")
                .font(.caption).foregroundStyle(.secondary)
            if !tutor.settings.ready(tutor.settings.defaultEndpoint) {
                Button("Add a key in Settings…") { SettingsWindowController.shared.show() }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct RowView: View {
    let tutor: TutorModel
    let row: TranscriptRow

    var body: some View {
        switch row {
        case .question(_, let text, let images, let selection):
            VStack(alignment: .trailing, spacing: 6) {
                if !images.isEmpty {
                    HStack { ForEach(images, id: \.self) { PictureView(tutor: tutor, id: $0, height: 90) } }
                }
                if !text.isEmpty {
                    Text(text)
                        .textSelection(.enabled)
                        .padding(.horizontal, 10).padding(.vertical, 7)
                        .background(RoundedRectangle(cornerRadius: 10).fill(Color.accentColor.opacity(0.14)))
                }
                if selection {
                    Text("with the selection").font(.caption2).foregroundStyle(.tertiary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        case .answer(_, let text, let reasoning, let tools, let model, let cost):
            VStack(alignment: .leading, spacing: 8) {
                if !reasoning.isEmpty && tutor.settings.showThinking {
                    ThinkingView(text: reasoning)
                }
                if !tools.isEmpty { ToolLog(tutor: tutor, tools: tools, open: false) }
                // Pictures the tutor drew come first and large; ones a tool
                // read from the ROM or the recording sit in the log.
                ForEach(tools.filter { $0.name == "generate_image" }.flatMap(\.images), id: \.self) { id in
                    VStack(alignment: .leading, spacing: 2) {
                        PictureView(tutor: tutor, id: id, height: 360)
                        Text("Generated by an image model, not from the ROM").font(.caption2).foregroundStyle(.tertiary)
                    }
                }
                if !text.isEmpty { MessageText(tutor: tutor, text: text) }
                HStack(spacing: 6) {
                    if let model { Text(model) }
                    if cost > 0 { Text(String(format: "$%.4f", cost)) }
                }
                .font(.caption2).foregroundStyle(.tertiary)
            }
        case .note(_, let text):
            Text(text).font(.caption).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .center)
        }
    }
}

struct LiveAnswer: View {
    let tutor: TutorModel
    let live: TutorModel.Live

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !live.question.isEmpty || !live.pictures.isEmpty {
                VStack(alignment: .trailing, spacing: 6) {
                    HStack {
                        ForEach(Array(live.pictures.enumerated()), id: \.offset) { _, d in
                            if let i = NSImage(data: d) {
                                Image(nsImage: i).resizable().interpolation(.none).aspectRatio(contentMode: .fit).frame(maxHeight: 90)
                            }
                        }
                    }
                    if !live.question.isEmpty {
                        Text(live.question)
                            .padding(.horizontal, 10).padding(.vertical, 7)
                            .background(RoundedRectangle(cornerRadius: 10).fill(Color.accentColor.opacity(0.14)))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .trailing)
            }
            if !live.reasoning.isEmpty && tutor.settings.showThinking {
                ThinkingView(text: live.reasoning, open: live.text.isEmpty)
            }
            if !live.tools.isEmpty { ToolLog(tutor: tutor, tools: live.tools, open: true) }
            ForEach(live.cards) { card in EditCard(tutor: tutor, card: card) }
            if !live.text.isEmpty { MessageText(tutor: tutor, text: live.text) }
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text(live.status ?? "Thinking…").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

struct ThinkingView: View {
    let text: String
    @State var open = false

    init(text: String, open: Bool = false) {
        self.text = text
        _open = State(initialValue: open)
    }

    var body: some View {
        DisclosureGroup(isExpanded: $open) {
            Text(text).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        } label: {
            Label("How the tutor got here", systemImage: "brain").font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// The tool calls of a turn, collapsed to a line: the method is part of the
/// lesson (docs/06 principle 4).
struct ToolLog: View {
    let tutor: TutorModel
    let tools: [TutorModel.ToolRow]
    @State var open: Bool

    init(tutor: TutorModel, tools: [TutorModel.ToolRow], open: Bool) {
        self.tutor = tutor
        self.tools = tools
        _open = State(initialValue: open)
    }

    var body: some View {
        DisclosureGroup(isExpanded: $open) {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(tools) { t in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Image(systemName: !t.done ? "circle.dotted" : t.isError ? "xmark.circle" : "checkmark.circle")
                            .foregroundStyle(t.isError ? .orange : .secondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(TutorModel.toolTitle(t.name)).font(.caption.bold())
                            + Text("  " + Self.args(t.input)).font(.caption.monospaced()).foregroundColor(.secondary)
                            if let s = t.summary, !s.isEmpty {
                                Text(s).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                            }
                            if t.name != "generate_image", !t.images.isEmpty {
                                HStack { ForEach(t.images, id: \.self) { PictureView(tutor: tutor, id: $0, height: 120) } }
                            }
                        }
                    }
                    .textSelection(.enabled)
                }
            }
            .padding(.leading, 4)
        } label: {
            Text(tools.count == 1 ? "1 tool call" : "\(tools.count) tool calls")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    /// `{"address":"$80:8000","lines":30}` as `address $80:8000, lines 30`.
    static func args(_ json: String) -> String {
        guard let data = json.data(using: .utf8),
              let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return String(json.prefix(80)) }
        let parts = o.keys.sorted().compactMap { k -> String? in
            guard let v = o[k], !(v is NSNull), k != "reason", k != "text" else { return nil }
            return "\(k) \(v)"
        }
        return String(parts.joined(separator: ", ").prefix(120))
    }
}

/// An edit the tutor wants to make, in "ask before edits" (docs/24).
struct EditCard: View {
    let tutor: TutorModel
    let card: TutorModel.Card
    @State private var why = ""

    var body: some View {
        let p = card.proposal
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Image(systemName: "pencil.circle.fill").foregroundStyle(Color.accentColor)
                Text(p.summary).font(.callout.bold())
                Spacer()
                switch card.state {
                case .accepted: Label("Made", systemImage: "checkmark").font(.caption).foregroundStyle(.green)
                case .declined: Label("Declined", systemImage: "xmark").font(.caption).foregroundStyle(.secondary)
                case .waiting: EmptyView()
                }
            }
            if !p.reason.isEmpty { Text(p.reason).font(.callout).foregroundStyle(.secondary) }
            if p.before != nil || p.after != nil {
                VStack(alignment: .leading, spacing: 2) {
                    if let b = p.before { Text("now:   " + b).strikethrough(p.after != nil) }
                    if let a = p.after { Text("after: " + a) }
                }
                .font(.caption.monospaced())
                .lineLimit(6)
            }
            if card.state == .waiting {
                HStack {
                    TextField("Why not? (optional)", text: $why).textFieldStyle(.roundedBorder).font(.caption)
                    Button("Decline") { tutor.answer(card, accept: false, why: why.isEmpty ? nil : why) }
                    Button("Accept") { tutor.answer(card, accept: true) }.keyboardShortcut(.defaultAction)
                    Button("Accept All") { tutor.answer(card, accept: true, andTheRest: true) }
                        .help("Accept this and the tutor's other edits in this answer")
                }
                .controlSize(.small)
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.accentColor.opacity(card.state == .waiting ? 0.8 : 0.2)))
    }
}

struct PictureView: View {
    let tutor: TutorModel
    let id: String
    let height: CGFloat

    var body: some View {
        if let data = tutor.session?.picture(id: id), let image = NSImage(data: data) {
            Image(nsImage: image)
                .interpolation(.none)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(maxHeight: height)
                .clipShape(RoundedRectangle(cornerRadius: 6))
        } else {
            Image(systemName: "photo").frame(height: 24)
        }
    }
}

// MARK: The status line

struct StatusLine: View {
    let tutor: TutorModel

    var body: some View {
        HStack(spacing: 10) {
            Button {
                tutor.sheet = .model
            } label: {
                Text([tutor.endpointName, tutor.modelName ?? tutor.settings.model(for: tutor.settings.defaultEndpoint)]
                    .compactMap { $0 }.joined(separator: " · "))
            }
            .buttonStyle(.plain)
            .help("Change the provider or model (/model)")
            Button { tutor.cycleMode() } label: {
                Label(tutor.mode.title, systemImage: tutor.mode == .readOnly ? "eye" : tutor.mode == .askBeforeEdits ? "hand.raised" : "pencil")
            }
            .buttonStyle(.plain)
            .help("What the tutor may change: ⇧⇥ cycles")
            if tutor.contextUsed > 0 {
                Text("context \(Int(tutor.contextUsed * 100))%")
                    .help("How full the model's context was at the last answer; past 80% it summarises first")
            }
            Text(String(format: "$%.3f", tutor.cost)).help("What this conversation has cost")
            Spacer()
            if tutor.busy {
                Button("Stop") { tutor.stop() }.keyboardShortcut(.cancelAction).controlSize(.small)
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }
}

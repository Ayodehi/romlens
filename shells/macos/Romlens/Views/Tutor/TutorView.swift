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
        .environment(\.openURL, OpenURLAction { url in
            if let e = Glossary.entry(for: url) {
                GlossaryPopover.show(e) { term in tutor.composer = "Tell me more about \(term)." }
                return .handled
            }
            return tutor.follow(url) ? .handled : .systemAction
        })
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
            case .lessons: LessonsSheet(tutor: tutor, tab: .lessons)
            case .quiz: QuizSheet(tutor: tutor)
            case .map: LessonsSheet(tutor: tutor, tab: .map)
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
    /// A lesson the tutor wrote in this turn (docs/25).
    case lesson(index: UInt32, id: String)

    var id: String {
        switch self {
        case .question(let i, _, _, _): "q\(i)"
        case .answer(let i, _, _, _, _, _): "a\(i)"
        case .note(let i, _): "n\(i)"
        case .lesson(_, let id): "l\(id)"
        }
    }

    /// The notes Romlens adds before the student's words, which the window
    /// does not show as theirs.
    static func isNote(_ text: String) -> Bool {
        ["[The mode is now", "[The student's selection", "[The changes you proposed last time",
         "[Explain mode is", "[The student is at step"].contains { text.hasPrefix($0) }
    }

    static func rows(_ turns: [TurnInfo]) -> [TranscriptRow] {
        var results: [String: (String, Bool, [String])] = [:]
        for t in turns where t.user {
            for case let .toolResult(id, text, images, isError) in t.blocks {
                results[id] = (text.split(separator: "\n").first.map(String.init) ?? "", isError, images)
            }
        }
        // A diagram a lesson step shows is in the lesson's card, so the
        // answer does not show it again.
        var inLessons = Set<String>()
        for t in turns where !t.user {
            for case let .toolCall(_, "lesson_step", input) in t.blocks {
                if let data = input.data(using: .utf8),
                   let v = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let p = v["picture"] as? String {
                    inLessons.insert(p)
                }
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
                        if isNote(text) {
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
                var lessons: [String] = []
                for b in t.blocks {
                    switch b {
                    case .text(let s): text += s
                    case .reasoning(let s): reasoning += reasoning.isEmpty ? s : "\n\n" + s
                    case .toolCall(let id, let name, let input):
                        let r = results[id]
                        var images = r?.2 ?? []
                        if name.hasPrefix("draw_") { images.removeAll { inLessons.contains($0) } }
                        tools.append(TutorModel.ToolRow(id: id, name: name, input: input, summary: r?.0, images: images, isError: r?.1 ?? false, done: true))
                        if name == "begin_lesson", let r, !r.1, let l = lessonIdFromResult(text: r.0) {
                            lessons.append(l)
                        }
                    default: break
                    }
                }
                out.append(.answer(index: t.index, text: text, reasoning: reasoning, tools: tools, model: t.model, cost: t.cost))
                for l in lessons { out.append(.lesson(index: t.index, id: l)) }
            }
        }
        return out
    }

    /// The rows as the window shows them. A reply's rounds share one
    /// footer, on its last answer, with the reply's whole cost; without
    /// the thinking and tool calls, rounds with nothing else are left out.
    static func shown(_ rows: [TranscriptRow], work: Bool) -> [TranscriptRow] {
        var out: [TranscriptRow] = []
        var reply: [TranscriptRow] = []
        func close() {
            var kept = reply.filter { r in
                guard !work, case .answer(_, let text, _, let tools, _, _) = r else { return true }
                return !text.isEmpty || tools.contains { TutorModel.drawing[$0.name] != nil && !$0.images.isEmpty }
            }
            let cost = reply.reduce(0.0) { if case .answer(_, _, _, _, _, let c) = $1 { $0 + c } else { $0 } }
            let model = reply.last.flatMap { if case .answer(_, _, _, _, let m, _) = $0 { m } else { nil } }
            kept = kept.enumerated().map { n, r in
                guard case .answer(let i, let text, let reasoning, let tools, _, _) = r else { return r }
                let last = n == kept.count - 1
                return .answer(index: i, text: text, reasoning: reasoning, tools: tools,
                               model: last ? model : nil, cost: last ? cost : 0)
            }
            out += kept
            reply = []
        }
        for r in rows {
            if case .answer = r { reply.append(r) } else { close(); out.append(r) }
        }
        close()
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
        let rows = TranscriptRow.shown(TranscriptRow.rows(tutor.turns), work: tutor.settings.showWork)
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
                if tutor.settings.showWork {
                    if !reasoning.isEmpty { ThinkingView(text: reasoning, open: true) }
                    if !tools.isEmpty { ToolLog(tutor: tutor, tools: tools, open: true) }
                }
                // Pictures drawn for the answer come first and large; ones a
                // tool read from the ROM or the recording sit in the log.
                ForEach(tools.filter { TutorModel.drawing[$0.name] != nil }, id: \.id) { t in
                    ForEach(t.images, id: \.self) { id in
                        VStack(alignment: .leading, spacing: 2) {
                            PictureView(tutor: tutor, id: id, height: 360)
                            Text(TutorModel.drawing[t.name] ?? "").font(.caption2).foregroundStyle(.tertiary)
                        }
                    }
                }
                if !text.isEmpty { MessageText(tutor: tutor, text: text) }
                if model != nil || cost > 0 {
                    HStack(spacing: 6) {
                        if let model { Text(model) }
                        if cost > 0 { Text(String(format: "$%.4f", cost)) }
                    }
                    .font(.caption2).foregroundStyle(.tertiary)
                }
            }
        case .note(_, let text):
            Text(text).font(.caption).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .center)
        case .lesson(_, let id):
            LessonCard(tutor: tutor, id: id)
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
            if tutor.settings.showWork {
                if !live.reasoning.isEmpty { ThinkingView(text: live.reasoning, open: true) }
                if !live.tools.isEmpty { ToolLog(tutor: tutor, tools: live.tools, open: true) }
            }
            ForEach(live.cards) { card in EditCard(tutor: tutor, card: card) }
            if !live.text.isEmpty { MessageText(tutor: tutor, text: live.text) }
            if let l = live.lesson { LessonCard(tutor: tutor, id: l) }
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
                            if TutorModel.drawing[t.name] == nil, !t.images.isEmpty {
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
            Picture(image: image, id: id, height: height)
        } else {
            Image(systemName: "photo").frame(height: 24)
        }
    }
}

/// A picture as its kind wants it. A diagram (`draw-` Romlens's, `svg-`
/// the tutor's, checked) is
/// drawn at twice its size in points, so it shows at half its pixels,
/// scaled smoothly, never taller than it was drawn; pixel art keeps its
/// pixels square, within `height`.
struct Picture: View {
    let image: NSImage
    let id: String
    let height: CGFloat

    static func isDiagram(_ id: String) -> Bool { id.hasPrefix("draw-") || id.hasPrefix("svg-") }

    /// Where a diagram came from, in a line under it.
    static func caption(_ id: String) -> String? {
        id.hasPrefix("draw-") ? TutorModel.drawing["draw_diagram"]
            : id.hasPrefix("svg-") ? TutorModel.drawing["draw_svg"] : nil
    }

    var body: some View {
        if Self.isDiagram(id) {
            let px = image.representations.first.map { CGSize(width: $0.pixelsWide, height: $0.pixelsHigh) } ?? image.size
            Image(nsImage: image)
                .interpolation(.high)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(maxWidth: px.width / 2, maxHeight: px.height / 2)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.3)))
        } else {
            Image(nsImage: image)
                .interpolation(.none)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(maxHeight: height)
                .clipShape(RoundedRectangle(cornerRadius: 6))
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
                    .lineLimit(1).fixedSize()
            }
            .buttonStyle(.plain)
            .help("What the tutor may change: ⇧⇥ cycles")
            if tutor.contextUsed > 0 {
                Text("context \(Int(tutor.contextUsed * 100))%")
                    .help("How full the model's context was at the last answer; past 80% it summarises first")
            }
            Text(String(format: "$%.3f", tutor.cost)).help("What this conversation has cost")
            Spacer()
            Button { tutor.sheet = .resume } label: {
                Label("Conversations", systemImage: "clock.arrow.circlepath").labelStyle(.iconOnly)
            }
            .buttonStyle(.plain)
            .disabled(tutor.busy)
            .help("Go back to an earlier conversation (/resume)")
            Button { tutor.sheet = .lessons } label: {
                Label("Lessons", systemImage: "books.vertical").labelStyle(.iconOnly)
            }
            .buttonStyle(.plain)
            .help("Your lessons, and what you have learned (/lessons, /map)")
            Button { tutor.setExplain(!tutor.explain) } label: {
                Label("Explain", systemImage: tutor.explain ? "graduationcap.fill" : "graduationcap")
            }
            .buttonStyle(.plain)
            .foregroundStyle(tutor.explain ? Color.accentColor : Color.secondary)
            .help(tutor.explain ? "Answering with lessons: click for plain answers (/explain)" : "Answer with lessons, as deep as you have got (/explain, /learn <topic>)")
            Button { tutor.settings.showWork.toggle() } label: {
                Label("Details", systemImage: tutor.settings.showWork ? "list.bullet.rectangle.fill" : "list.bullet.rectangle")
            }
            .buttonStyle(.plain)
            .foregroundStyle(tutor.settings.showWork ? Color.accentColor : Color.secondary)
            .help(tutor.settings.showWork ? "Hide the tutor's thinking and tool calls (/details)" : "Show the tutor's thinking and tool calls (/details)")
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

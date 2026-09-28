import AppKit
import Foundation
import RomlensKit
import SwiftUI
import Testing
@testable import Romlens

/// The Tutor window (docs/24, U8 and U9), against a model server on the
/// loopback that answers from a script: nothing leaves the machine.
@MainActor
@Suite(.serialized) struct TutorShellTests {
    struct Rig {
        let rom: RomViewModel
        let tutor: TutorModel
        let root: URL
    }

    /// A project, and a tutor on a local endpoint that answers `replies`.
    private func rig(_ replies: [String], mode: TutorModePreference = .readOnly) async throws -> Rig {
        let rom = try await Fixture.analyzedModel(rom: try Fixture.smallRom())
        let name = "romlens-tutor-shell-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        let settings = TutorSettings(defaults: defaults, keys: MemoryKeyStore())
        let local = TutorEndpoint.local(name: "Test", baseURL: tutorTestServer(replies: replies), kind: .chat, taken: [])
        settings.add(local)
        settings.defaultEndpoint = local
        settings.setModel("test-model", for: local)
        settings.mode = mode
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(name)
        return Rig(rom: rom, tutor: TutorModel(rom: rom, settings: settings, root: root), root: root)
    }

    private func ask(_ t: TutorModel, _ text: String) async throws {
        t.composer = text
        t.submit()
        #expect(t.busy)
        try await Fixture.settle(timeout: 20) { !t.busy }
    }

    @Test func aQuestionIsAnsweredWithItsToolsAndCitations() async throws {
        let r = try await rig([
            tutorTestCallReply(name: "listing", arguments: #"{"address":"$00:8000","lines":4}"#),
            tutorTestTextReply(text: "RESET at `$00:8000` masks interrupts:\n```asm\n$00:8000  78  SEI\n```"),
        ])
        defer { try? FileManager.default.removeItem(at: r.root) }
        r.rom.select(offset: 0)
        #expect(r.tutor.selectionChip?.hasPrefix("$00:8000") == true)
        try await ask(r.tutor, "What does RESET do?")
        #expect(r.tutor.error == nil)
        let rows = TranscriptRow.rows(r.tutor.turns)
        guard case .question(_, let q, _, let selection) = rows[0] else { Issue.record("no question"); return }
        #expect(q == "What does RESET do?" && selection)
        guard case .answer(_, _, _, let tools, _, _) = rows[1] else { Issue.record("no tool round"); return }
        #expect(tools.map(\.name) == ["listing"] && tools[0].summary?.contains("SEI") == true)
        guard case .answer(_, let text, _, _, let model, _) = rows.last else { Issue.record("no answer"); return }
        #expect(text.hasPrefix("RESET at `$00:8000`") && model == "test-model")
        #expect(r.tutor.cost == 0, "a local model costs nothing")

        // The citation leads to the main window.
        r.rom.select(offset: 1)
        #expect(r.tutor.follow(URL(string: "romlens://a/008000")!))
        #expect(r.rom.selectedAddress == 0x00_8000)
        #expect(!r.tutor.follow(URL(string: "https://example.com")!))

        // ↑ brings the question back, ↓ the draft.
        r.tutor.composer = "draft"
        #expect(r.tutor.historyUp())
        #expect(r.tutor.composer == "What does RESET do?")
        #expect(r.tutor.historyDown())
        #expect(r.tutor.composer == "draft")

        // It was saved: the conversation lists and the rewind points hold it.
        #expect(r.tutor.conversations.count == 1)
        #expect(r.tutor.session?.rewindPoints().count == 1)

        // After the answer the model names it, for the list and the window.
        try await Fixture.settle(timeout: 10) { r.tutor.title == "Test conversation" }
        #expect(r.tutor.conversations.first?.title == "Test conversation")
        let window = TutorWindowController(tutor: r.tutor, title: "t.sfc")
        #expect(window.window?.subtitle == "Test conversation")
    }

    @Test func aLessonIsReadAStepAtATime() async throws {
        let step: (String, String) -> String = { title, extra in
            #"{"lesson":"","title":"\#(title)","predict":null,"body":"Words for \#(title).","focus_address":null,"focus_end":null,"focus_in":null,"focus_frame":null,"focus_view":null,"picture":null\#(extra)}"#
        }
        let r = try await rig([
            tutorTestCallReply(name: "begin_lesson", arguments: #"{"title":"How a sprite reaches the screen","from_level":1,"to_level":2,"concepts":[{"id":"sprites","level":2}],"builds_on":[]}"#),
            tutorTestCallReply(name: "lesson_step", arguments: step("Where the code starts", "").replacingOccurrences(of: #""focus_address":null"#, with: #""focus_address":"$00:8000""#)),
            tutorTestCallReply(name: "lesson_step", arguments: step("Two chips", "").replacingOccurrences(of: #""predict":null"#, with: #""predict":"Which chip draws?""#).replacingOccurrences(of: #""focus_address":null,"focus_end":null,"focus_in":null"#, with: #""focus_address":"$00:8000","focus_end":null,"focus_in":"c""#)),
            tutorTestCallReply(name: "end_lesson", arguments: #"{"lesson":"","next":[{"title":"Sprites in this game","concept":"oam","level":3}]}"#),
            tutorTestTextReply(text: "Next takes you through it."),
        ])
        defer { try? FileManager.default.removeItem(at: r.root) }
        r.tutor.composer = "/learn How does a sprite get rendered to the screen?"
        r.tutor.submit()
        #expect(r.tutor.explain)
        try await Fixture.settle(timeout: 20) { !r.tutor.busy }
        #expect(r.tutor.error == nil, "\(r.tutor.error ?? "")")

        // The question reads as asked; the lesson is a row of its own.
        let rows = TranscriptRow.rows(r.tutor.turns)
        guard case .question(_, let q, _, _) = rows[0] else { Issue.record("no question"); return }
        #expect(q == "How does a sprite get rendered to the screen?")
        let ids = rows.compactMap { if case .lesson(_, let id) = $0 { id } else { nil } }
        #expect(ids.count == 1)
        let lesson = try #require(r.tutor.lesson(ids[0]))
        #expect(lesson.finished && lesson.steps.count == 2 && lesson.levelName == "The idea to the hardware")

        // Next moves the main window to each step's focus; a predict
        // question keeps its answer back until Show.
        r.rom.select(offset: 0x10)
        r.tutor.show(step: 0, of: lesson)
        #expect(r.rom.selectedAddress == 0x00_8000)
        #expect(r.tutor.step(of: lesson.id) == 0)
        r.tutor.show(step: 1, of: lesson)
        #expect(r.rom.editorTab == .c)
        #expect(!r.tutor.isRevealed(lesson.id, 1))
        r.tutor.reveal(lesson.id, 1)
        #expect(r.tutor.isRevealed(lesson.id, 1))
        r.tutor.show(step: 5, of: lesson)
        #expect(r.tutor.step(of: lesson.id) == 1, "past the end stays put")

        // The record has it, for the next lesson.
        let sprites = try #require(r.tutor.session?.learner().concepts.first { $0.id == "sprites" })
        #expect(sprites.level == 2)

        // Go deeper asks for the offer, with Explain on.
        r.tutor.take(lesson.next[0])
        #expect(r.tutor.busy && r.tutor.live?.question == "Go deeper: Sprites in this game")
        try await Fixture.settle(timeout: 20) { !r.tutor.busy }
    }

    @Test func anEditWaitsForItsCardAndUndoes() async throws {
        let r = try await rig([
            tutorTestCallReply(name: "set_label", arguments: #"{"address":"$00:8000","name":"Boot","reason":"the reset vector points here"}"#),
            tutorTestTextReply(text: "Named it Boot."),
        ], mode: .askBeforeEdits)
        defer { try? FileManager.default.removeItem(at: r.root) }
        r.tutor.composer = "Name RESET"
        r.tutor.submit()
        try await Fixture.settle(timeout: 20) { r.tutor.live?.cards.first?.state == .waiting }
        let card = try #require(r.tutor.live?.cards.first)
        #expect(card.proposal.summary == "Name $00:8000 `Boot`")
        #expect(card.proposal.reason == "the reset vector points here")
        r.tutor.answer(card, accept: true)
        try await Fixture.settle(timeout: 20) { !r.tutor.busy }
        #expect(r.rom.session.workbench.labelAt(snesAddress: 0x00_8000)?.name == "Boot")
        #expect(r.rom.session.undoTitle == "Tutor: Rename Label")
        #expect(r.rom.session.canUndo)
        // /rewind of the edits alone takes the name back.
        let point = try #require(r.tutor.session?.rewindPoints().first)
        let back = try #require(try r.tutor.session?.rewind(index: point.index, what: .edits))
        #expect(back.edits?.undone == 1)
        #expect(r.rom.session.workbench.labelAt(snesAddress: 0x00_8000)?.name != "Boot")
    }

    @Test func commandsAndModes() async throws {
        let r = try await rig([])
        defer { try? FileManager.default.removeItem(at: r.root) }
        r.tutor.composer = "/re"
        #expect(r.tutor.matchingCommands.map(\.name) == ["/resume", "/rewind"])
        r.tutor.composer = "/mode accept"
        r.tutor.submit()
        #expect(r.tutor.mode == .acceptEdits && r.tutor.composer.isEmpty)
        r.tutor.cycleMode()
        #expect(r.tutor.mode == .readOnly)
        r.tutor.run(command: "/model")
        #expect(r.tutor.sheet == .model)
        r.tutor.run(command: "/nonsense")
        #expect(r.tutor.error?.contains("not a command") == true)
        r.tutor.run(command: "/selection")
        #expect(!r.tutor.includeSelection)
        #expect(!r.tutor.settings.showWork, "thinking and tool calls start hidden")
        r.tutor.run(command: "/details")
        #expect(r.tutor.settings.showWork)
        r.tutor.run(command: "/details")
        #expect(!r.tutor.settings.showWork)
        r.tutor.run(command: "/new")
        #expect(r.tutor.session?.conversationId() != nil)
        r.tutor.run(command: "/attach frame")
        #expect(r.tutor.error?.contains("recording") == true, "no recording is open")
    }

    @Test func citationsAndCodeAreFoundInTheText() {
        #expect(MessageText.link("See `$80:8000` and $7E:0AF6, frame 12.")
            == "See [`$80:8000`](romlens://a/808000) and [$7E:0AF6](romlens://a/7E0AF6), [frame 12](romlens://f/12).")
        let s = MessageText.segments("Look:\n```c\nx = 1;\n```\nThen `a`.\n```asm\nSEI")
        #expect(s == [.prose("Look:"), .code(language: "c", body: "x = 1;"), .prose("Then `a`."), .code(language: "asm", body: "SEI")])
        let p = MessageText.prose("## Why\n- one `$00:8000`")
        let plain = String(p.characters)
        #expect(plain == "Why\n• one $00:8000")
        #expect(p.runs.contains { $0.link == URL(string: "romlens://a/008000") })
    }

    @Test func aReplyShowsAsOneAnswer() {
        let tool = TutorModel.ToolRow(id: "1", name: "listing", input: "{}", summary: "SEI", done: true)
        let rows: [TranscriptRow] = [
            .question(index: 0, text: "Q", images: [], selection: false),
            .answer(index: 1, text: "", reasoning: "look", tools: [tool], model: "m", cost: 0.25),
            .answer(index: 3, text: "Answer.", reasoning: "", tools: [], model: "m", cost: 0.5),
        ]
        let quiet = TranscriptRow.shown(rows, work: false)
        #expect(quiet.map(\.id) == ["q0", "a3"], "the round with only a tool call is left out")
        guard case .answer(_, _, _, _, let model, let cost) = quiet[1] else { Issue.record("no answer"); return }
        #expect(model == "m" && cost == 0.75, "one footer with the reply's whole cost")
        let all = TranscriptRow.shown(rows, work: true)
        #expect(all.map(\.id) == ["q0", "a1", "a3"])
        guard case .answer(_, _, _, _, let m1, let c1) = all[1] else { Issue.record("no round"); return }
        #expect(m1 == nil && c1 == 0)
    }

    @Test func aScrollUpStopsFollowingTheStream() {
        typealias P = TranscriptView.Place
        let end = P(offset: 500, atEnd: true)
        // Text grows below: the offset holds, the end moves away.
        #expect(TranscriptView.following(true, from: end, to: P(offset: 500, atEnd: false)))
        // The student scrolls up: stop.
        #expect(!TranscriptView.following(true, from: end, to: P(offset: 300, atEnd: false)))
        // Growth while stopped keeps it stopped.
        #expect(!TranscriptView.following(false, from: P(offset: 300, atEnd: false), to: P(offset: 300, atEnd: false)))
        // Back down to the end: follow again; part way down: not yet.
        #expect(!TranscriptView.following(false, from: P(offset: 300, atEnd: false), to: P(offset: 400, atEnd: false)))
        #expect(TranscriptView.following(false, from: P(offset: 400, atEnd: false), to: P(offset: 520, atEnd: true)))
    }

    @Test func tablesAndRulesAreFoundInTheText() {
        let s = MessageText.segments("""
        How:

        | Where | Bytes |
        |---|:--:|
        | `$7F:8000` | `A9 F0` |
        | `$7F:8003,X` | a \\| b | extra |
        | short |
        ---
        After.
        """)
        #expect(s == [
            .prose("How:"),
            .table(header: ["Where", "Bytes"], rows: [["`$7F:8000`", "`A9 F0`"], ["`$7F:8003,X`", "a | b"], ["short", ""]]),
            .rule,
            .prose("After."),
        ])
        #expect(MessageText.cells("| `a|b` | c |") == ["`a|b`", "c"])
        // A header still streaming, before its rule, stays prose.
        #expect(MessageText.segments("| a | b |") == [.prose("| a | b |")])
    }

    @Test func theWindowShowsAConversation() async throws {
        let r = try await rig([tutorTestCallReply(name: "listing", arguments: #"{"address":"$00:8000","lines":4}"#), tutorTestTextReply(text: "It is `$00:8000`.\n```c\nvoid Reset(void);\n```\n| Where | Bytes | As code |\n|---|---|---|\n| `$7F:8000` | `A9 F0` | `LDA #$F0` |\n| `$7F:8182` | `6B` | **RTL**, the routine's end |\n\nThen more.")])
        defer { try? FileManager.default.removeItem(at: r.root) }
        try await ask(r.tutor, "Where is RESET?")
        let c = TutorWindowController(tutor: r.tutor, title: "Test")
        let w = try #require(c.window)
        w.appearance = NSAppearance(named: .aqua)
        w.setContentSize(NSSize(width: 520, height: 640))
        w.orderFront(nil)
        defer { w.close() }
        w.contentView?.layoutSubtreeIfNeeded()
        Fixture.spin(0.3)
        w.display()
        #expect(w.title == "Tutor — Test")
        #expect(!c.shouldCloseDocument)
        // With ROMLENS_SNAPSHOTS set, saved in the sandbox's temporary folder.
        if ProcessInfo.processInfo.environment["ROMLENS_SNAPSHOTS"] != nil, let view = w.contentView,
           let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
            view.cacheDisplay(in: view.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?
                .write(to: FileManager.default.temporaryDirectory.appendingPathComponent("tutor.png"))
        }
    }

    @Test func onTheCTabTheQuestionCarriesTheC() async throws {
        let r = try await rig([])
        defer { try? FileManager.default.removeItem(at: r.root) }
        r.rom.select(offset: 0)
        #expect(r.tutor.selectionText()?.contains("C tab") == false)
        r.rom.editorTab = .c
        try await Fixture.settle(timeout: 20) { r.rom.decompiler.state == .ready }
        let s = try #require(r.tutor.selectionText())
        #expect(s.contains("[The student is reading the C tab:") && s.contains("```c\n"))
        #expect(s.contains(try #require(r.rom.decompiler.result?.name)))
    }

    @Test func longCIsClippedAroundTheSelection() {
        let text = (0..<400).map { "line \($0)" }.joined(separator: "\n")
        let c = TutorModel.clip(text, around: 300)
        let lines = c.components(separatedBy: "\n")
        #expect(lines.count == 162)
        #expect(lines.first == "/* … 220 lines above left out */" && lines.last == "/* … 20 lines below left out */")
        #expect(lines.contains("line 300"))
        #expect(TutorModel.clip("a\nb", around: nil) == "a\nb")
    }

    @Test func theDocumentOpensOneTutorWindow() throws {
        let doc = ProjectDocument()
        try doc.read(from: makeTestRom(mapping: .loRom), ofType: Fixture.romType)
        doc.model?.session.cancelAnalysis()
        doc.makeWindowControllers()
        doc.showTutor()
        let first = try #require(doc.tutorController)
        doc.showTutor()
        #expect(doc.tutorController === first)
        #expect(doc.windowControllers.count == 2)
        first.window?.close()
        doc.close()
    }
}

/// Pictures sent to the model fit what Claude and OpenAI take.
@MainActor
@Suite struct AttachmentFitTests {
    private func png(_ w: Int, _ h: Int) throws -> Data {
        let rep = try #require(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8, samplesPerPixel: 4,
            hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        return try #require(rep.representation(using: .png, properties: [:]))
    }

    @Test func aScreenshotGoesAsItIs() throws {
        let small = try png(256, 224)
        let (d, t) = TutorModel.fitForModel(data: small, mediaType: "image/png")
        #expect(d == small && t == "image/png")
    }

    @Test func aBigPhotoIsScaledDown() throws {
        let big = try png(4032, 3024)
        let (d, t) = TutorModel.fitForModel(data: big, mediaType: "image/png")
        #expect(t == "image/jpeg")
        let rep = try #require(NSBitmapImageRep(data: d))
        #expect(max(rep.pixelsWide, rep.pixelsHigh) == 2576)
        #expect(d.count <= TutorModel.maxBytes)
    }
}

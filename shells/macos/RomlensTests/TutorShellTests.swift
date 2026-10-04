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
        // A guess first (docs/28): kept, and the step shown.
        r.tutor.answerPredict(lesson.id, 1, "The PPU draws")
        #expect(r.tutor.isRevealed(lesson.id, 1))
        let guessed = try #require(r.tutor.lesson(lesson.id)?.steps[1])
        #expect(guessed.guessed == "The PPU draws" && guessed.guessRight == nil && !guessed.checksGuess)
        r.tutor.show(step: 5, of: lesson)
        #expect(r.tutor.step(of: lesson.id) == 1, "past the end stays put")

        // The record has it, for the next lesson.
        let sprites = try #require(r.tutor.session?.learner().concepts.first { $0.id == "sprites" })
        #expect(sprites.level == 2)

        // The library and the map.
        #expect(r.tutor.lessons.map(\.id) == [lesson.id])
        r.tutor.markKnown("dma", level: 2)
        let dma = try #require(r.tutor.learner.concepts.first { $0.id == "dma" })
        #expect(dma.marked && dma.level == 2)
        r.tutor.markKnown("dma", level: nil)
        #expect(r.tutor.learner.concepts.first { $0.id == "dma" }?.level == 0)
        for tab in [LessonsSheet.Tab.lessons, .map] {
            let host = NSHostingController(rootView: LessonsSheet(tutor: r.tutor, tab: tab))
            host.view.frame = NSRect(x: 0, y: 0, width: 720, height: 560)
            host.view.layoutSubtreeIfNeeded()
        }
        r.tutor.run(command: "/map")
        #expect(r.tutor.sheet == .map)
        r.tutor.sheet = nil

        // With ROMLENS_SNAPSHOTS set, the card and the map as pictures.
        if ProcessInfo.processInfo.environment["ROMLENS_SNAPSHOTS"] != nil {
            r.tutor.markKnown("vblank", level: 4)
            r.tutor.show(step: 0, of: lesson)
            let c = Self.tutorWindow(r.tutor)
            let map = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 720, height: 560), styleMask: [.titled], backing: .buffered, defer: false)
            map.isReleasedWhenClosed = false
            map.contentViewController = NSHostingController(rootView: LessonsSheet(tutor: r.tutor, tab: .map))
            for (w, name) in [(c, "lesson.png"), (map, "map.png")] {
                w.appearance = NSAppearance(named: .aqua)
                if name == "lesson.png" { w.setContentSize(NSSize(width: 520, height: 640)) }
                w.orderFront(nil)
                w.contentView?.layoutSubtreeIfNeeded()
                Fixture.spin(0.3)
                w.display()
                if let view = w.contentView, let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
                    view.cacheDisplay(in: view.bounds, to: rep)
                    try? rep.representation(using: .png, properties: [:])?
                        .write(to: FileManager.default.temporaryDirectory.appendingPathComponent(name))
                }
                w.close()
            }
            r.tutor.markKnown("vblank", level: nil)
        }

        // Go deeper asks for the offer, with Explain on.
        r.tutor.take(lesson.next[0])
        #expect(r.tutor.busy && r.tutor.live?.question == "Go deeper: Sprites in this game")
        try await Fixture.settle(timeout: 20) { !r.tutor.busy }
    }

    /// A diagram Romlens drew (docs/26): large and captioned in an answer,
    /// and in a lesson's card instead when a step shows it.
    @Test func aDiagramShowsInTheAnswerAndInItsLesson() async throws {
        let machine = #"{"preset":"machine","highlight":["ppu"]}"#
        let fields = #"{"register":"INIDISP","value":"$80"}"#
        let id = tutorTestDiagramId(kind: "blocks", spec: machine)
        #expect(id.hasPrefix("draw-"))
        let r = try await rig([
            tutorTestCallReply(name: "draw_diagram", arguments: #"{"kind":"blocks","spec":\#(machine)}"#),
            tutorTestCallReply(name: "draw_diagram", arguments: #"{"kind":"fields","spec":\#(fields)}"#),
            tutorTestCallReply(name: "begin_lesson", arguments: #"{"title":"Inside the SNES","from_level":1,"to_level":1,"concepts":[{"id":"ppu","level":1}],"builds_on":[]}"#),
            tutorTestCallReply(name: "lesson_step", arguments: #"{"lesson":"","title":"Two chips","predict":null,"body":"The CPU decides; the PPU draws.","focus_address":null,"focus_end":null,"focus_in":null,"focus_frame":null,"focus_view":null,"picture":"\#(id)"}"#),
            tutorTestCallReply(name: "end_lesson", arguments: #"{"lesson":"","next":[]}"#),
            tutorTestTextReply(text: "That is the machine."),
        ])
        defer { try? FileManager.default.removeItem(at: r.root) }
        try await ask(r.tutor, "What is inside an SNES?")
        #expect(r.tutor.error == nil, "\(r.tutor.error ?? "")")

        // The lesson's diagram is in its card, not the answer; the other one
        // is the answer's, large and captioned.
        let rows = TranscriptRow.rows(r.tutor.turns)
        let drawn = rows.flatMap { row -> [TutorModel.ToolRow] in
            if case .answer(_, _, _, let tools, _, _) = row { tools.filter { $0.name == "draw_diagram" } } else { [] }
        }
        #expect(drawn.count == 2)
        #expect(drawn[0].images.isEmpty, "the lesson shows it")
        #expect(drawn[1].images.count == 1 && drawn[1].images[0].hasPrefix("draw-"))
        #expect(TutorModel.drawing["draw_diagram"] == "Drawn by Romlens from the ROM")
        let shown = TranscriptRow.shown(rows, work: false)
        #expect(shown.contains { if case .answer(_, _, _, let t, _, _) = $0 { t.contains { !$0.images.isEmpty } } else { false } },
                "a round with only a diagram is still shown")
        let lessonId = try #require(rows.compactMap { if case .lesson(_, let l) = $0 { l } else { nil } }.first)
        let lesson = try #require(r.tutor.lesson(lessonId))
        #expect(lesson.steps[0].picture == id)
        let data = try #require(r.tutor.session?.lessonPicture(lesson: lessonId, picture: id))
        let image = try #require(NSImage(data: data))
        #expect(image.representations.first?.pixelsWide == 1440, "twice its 720 points")
        #expect(Picture.isDiagram(id) && Picture.caption(id) == "Drawn by Romlens from the ROM")
        #expect(Picture.caption("svg-1") == "Drawn by the tutor, checked by Romlens" && Picture.caption("tool-1") == nil)

        if ProcessInfo.processInfo.environment["ROMLENS_SNAPSHOTS"] != nil {
            r.tutor.show(step: 0, of: lesson)
            let c = Self.tutorWindow(r.tutor)
            for (appearance, name) in [(NSAppearance.Name.aqua, "diagram-light.png"), (.darkAqua, "diagram-dark.png")] {
                let w = c
                w.appearance = NSAppearance(named: appearance)
                w.setContentSize(NSSize(width: 560, height: 900))
                w.orderFront(nil)
                w.contentView?.layoutSubtreeIfNeeded()
                Fixture.spin(0.5)
                w.display()
                if let view = w.contentView, let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
                    view.cacheDisplay(in: view.bounds, to: rep)
                    try? rep.representation(using: .png, properties: [:])?
                        .write(to: FileManager.default.temporaryDirectory.appendingPathComponent(name))
                }
            }
            c.close()
        }
    }

    /// A finished lesson is checked in the background (docs/25): the card
    /// says so, the step is rewritten in place, and the cost is shown.
    @Test func aLessonIsCheckedAfterItEnds() async throws {
        let r = try await rig([
            tutorTestCallReply(name: "begin_lesson", arguments: #"{"title":"DMA","from_level":1,"to_level":1,"concepts":[{"id":"dma","level":1}],"builds_on":[]}"#),
            tutorTestCallReply(name: "lesson_step", arguments: #"{"lesson":"","title":"The cost","predict":null,"body":"DMA takes about 8 CPU cycles a byte.","focus_address":null,"focus_end":null,"focus_in":null,"focus_frame":null,"focus_view":null,"picture":null}"#),
            tutorTestCallReply(name: "end_lesson", arguments: #"{"lesson":"","next":[]}"#),
            tutorTestTextReply(text: "That is DMA."),
            tutorTestCallReply(name: "revise_lesson_step", arguments: #"{"lesson":"","step":1,"title":"The cost","predict":null,"body":"DMA takes 8 master cycles a byte.","reason":"units"}"#),
            tutorTestTextReply(text: "Corrected step 1."),
        ])
        defer { try? FileManager.default.removeItem(at: r.root) }
        r.tutor.settings.checkLessons = true
        try await ask(r.tutor, "/learn What is DMA?")
        let rows = TranscriptRow.rows(r.tutor.turns)
        let id = try #require(rows.compactMap { if case .lesson(_, let l) = $0 { l } else { nil } }.first)
        try await Fixture.settle(timeout: 20) { r.tutor.lesson(id)?.checked != nil }
        let lesson = try #require(r.tutor.lesson(id))
        #expect(lesson.steps[0].body == "DMA takes 8 master cycles a byte.")
        #expect(!lesson.checking)
        let checked = try #require(lesson.checked)
        #expect(checked.changed == 1)
        #expect(LessonCard.checked(checked).hasPrefix("Checked · 1 step corrected · $"))
        #expect(LessonCard.checked(LessonCheckedInfo(changed: 0, cost: 0.031)) == "Checked · $0.03")
        #expect(TranscriptRow.rows(r.tutor.turns).count == rows.count, "the check adds nothing to the transcript")

        if ProcessInfo.processInfo.environment["ROMLENS_SNAPSHOTS"] != nil {
            let host = NSHostingController(rootView: LessonCard(tutor: r.tutor, id: id).frame(width: 480).padding())
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 260), styleMask: [.titled], backing: .buffered, defer: false)
            w.isReleasedWhenClosed = false
            w.contentViewController = host
            w.appearance = NSAppearance(named: .aqua)
            w.orderFront(nil)
            Fixture.spin(0.3)
            w.display()
            if let view = w.contentView, let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
                view.cacheDisplay(in: view.bounds, to: rep)
                try? rep.representation(using: .png, properties: [:])?
                    .write(to: FileManager.default.temporaryDirectory.appendingPathComponent("checked.png"))
            }
            w.close()
        }
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
        #expect(r.tutor.matchingCommands.map(\.name) == ["/resume", "/rewind", "/review"])
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

    @Test func aQuizProvesALevelAndTheMapShowsIt() async throws {
        let r = try await rig([])
        defer { try? FileManager.default.removeItem(at: r.root) }
        r.tutor.run(command: "/quiz")
        #expect(r.tutor.error?.contains("/quiz sprites") == true, "a quiz needs a topic")
        r.tutor.run(command: "/quiz sprites")
        let quiz = try #require(r.tutor.quiz)
        guard case .quiz = r.tutor.sheet else { Issue.record("the quiz sheet opens"); return }
        #expect(quiz.conceptName == "Sprites" && quiz.level == 1 && quiz.questions.count == 5)
        #expect(quiz.questions.allSatisfy { $0.certain })

        // The sheet lays out on its first question.
        let sheet = NSHostingView(rootView: QuizSheet(tutor: r.tutor))
        sheet.frame = NSRect(x: 0, y: 0, width: 580, height: 560)
        sheet.layoutSubtreeIfNeeded()
        #expect(sheet.fittingSize.width > 0)

        // Right answers, then the quiz is done and the level proven.
        let session = try #require(r.tutor.session)
        let answers = tutorTestQuizAnswers(session: session, quiz: quiz.id)
        for (q, a) in zip(quiz.questions, answers) {
            r.tutor.answer(q.id, a)
            #expect(r.tutor.result(q.id)?.credit == 1, "\(q.prompt)")
        }
        r.tutor.finishQuiz()
        #expect(r.tutor.quiz?.outcome.passed == true)
        let sprites = r.tutor.learner.concepts.first { $0.id == "sprites" }
        #expect(sprites?.proven == 1 && sprites?.level == 1, "proven is learned, and the chip rings")
        #expect(sprites?.due != nil)

        // Nothing is due yet, so a review says so.
        r.tutor.run(command: "/review")
        #expect(r.tutor.error?.contains("Nothing is due") == true)

        // The proof earned points and a banner; the Progress tab lays out.
        try await Fixture.settle(timeout: 5) { r.tutor.banner != nil }
        #expect(r.tutor.banner?.lines.contains("Proven: Sprites, level 1") == true)
        let p = try #require(r.tutor.progress)
        #expect(p.xp >= 50 && p.rank == "Reset")
        #expect(p.achievements.contains { $0.id == "first_proof" && $0.unlocked != nil })
        r.tutor.run(command: "/progress")
        guard case .progress = r.tutor.sheet else { Issue.record("the progress sheet opens"); return }
        let host = NSHostingController(rootView: LessonsSheet(tutor: r.tutor, tab: .progress))
        host.view.frame = NSRect(x: 0, y: 0, width: 720, height: 560)
        host.view.layoutSubtreeIfNeeded()

        // With points hidden, nothing is announced.
        r.tutor.banner = nil
        r.tutor.settings.showProgress = false
        let practice = try #require(try? session.startQuiz(concept: "sprites", level: 2, purpose: .prove, tutor: false))
        r.tutor.quiz = practice
        for (q, a) in zip(practice.questions, tutorTestQuizAnswers(session: session, quiz: practice.id)) {
            r.tutor.answer(q.id, a)
        }
        r.tutor.finishQuiz()
        Fixture.spin(0.5)
        #expect(r.tutor.banner == nil, "no banner with points hidden")
        #expect(r.tutor.learner.concepts.first { $0.id == "sprites" }?.proven == 2, "but the proof counts")
    }

    @Test func glossaryTermsAreLinkedTheFirstTimeOnly() {
        var seen = Set<String>()
        let line = Glossary.link("DMA fills VRAM in V-blank; DMA again, `VMAIN` and `LDA`, [OAM](romlens://a/008000) at $00:DMA0.", seen: &seen)
        let dma = Glossary.url("DMA").absoluteString
        #expect(line.hasPrefix("[DMA](\(dma)) fills [VRAM]("), "\(line)")
        #expect(line.contains("in [V-blank](\(Glossary.url("VBlank").absoluteString))"), "\(line)")
        #expect(line.contains("; DMA again"), "the second DMA is not linked: \(line)")
        #expect(line.contains("[`VMAIN`]("), "\(line)")
        #expect(line.contains("and `LDA`"), "code that is not a term is left alone: \(line)")
        #expect(line.contains("[OAM](romlens://a/008000)"), "a link already there is kept: \(line)")
        #expect(seen == ["DMA", "VRAM", "VBlank", "VMAIN"])
        #expect(Glossary.entry(for: Glossary.url("I/O"))?.words == "Input/Output")
        #expect(Glossary.entry(for: URL(string: "romlens://a/808000")!) == nil)

        let parts = MessageText.rendered("HDMA and OAM.\n\n| Chip | Memory |\n|---|---|\n| PPU | OAM |")
        guard case .prose(let p) = parts.first, case .table(let h, let rows) = parts.last else {
            Issue.record("prose then a table"); return
        }
        #expect(p.runs.contains { $0.link == Glossary.url("HDMA") && $0.underlineStyle != nil })
        #expect(!h.contains { $0.runs.contains { $0.link != nil } }, "no terms in the header")
        #expect(rows[0][0].runs.contains { $0.link == Glossary.url("PPU") })
        #expect(!rows[0][1].runs.contains { $0.link != nil }, "OAM was linked in the prose")
    }

    @MainActor @Test func aGlossaryCardLaysOut() throws {
        let e = try #require(Glossary.entry("NMI"))
        let host = NSHostingView(rootView: GlossaryCard(entry: e, ask: {}))
        let size = host.fittingSize
        #expect(size.width == 300)
        #expect(size.height > 80 && size.height < 260, "\(size)")
        // With ROMLENS_SNAPSHOTS set, a line of prose and the card.
        if ProcessInfo.processInfo.environment["ROMLENS_SNAPSHOTS"] != nil {
            let page = VStack(alignment: .leading, spacing: 12) {
                Text(MessageText.prose("The NMI handler starts a DMA to VRAM at $00:8123, then sets VMAIN; the NMI returns."))
                GlossaryCard(entry: e, ask: {})
                    .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .windowBackgroundColor)))
            }
            .padding(16)
            .frame(width: 420)
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 300), styleMask: [.titled], backing: .buffered, defer: false)
            w.contentView = NSHostingView(rootView: page)
            w.setContentSize(w.contentView!.fittingSize)
            w.display()
            if let view = w.contentView, let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
                view.cacheDisplay(in: view.bounds, to: rep)
                try? rep.representation(using: .png, properties: [:])?
                    .write(to: FileManager.default.temporaryDirectory.appendingPathComponent("glossary.png"))
            }
        }
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
        let w = Self.tutorWindow(r.tutor)
        w.appearance = NSAppearance(named: .aqua)
        w.setContentSize(NSSize(width: 520, height: 640))
        w.orderFront(nil)
        defer { w.close() }
        w.contentView?.layoutSubtreeIfNeeded()
        Fixture.spin(0.3)
        w.display()
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
        r.rom.showTab(.c)
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

    /// The tutor's view in a window of its own, for looking at: in the app
    /// it is in the inspector or a tab (docs/29).
    static func tutorWindow(_ tutor: TutorModel) -> NSWindow {
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 640), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        w.isReleasedWhenClosed = false
        w.contentViewController = NSHostingController(rootView: TutorView(tutor: tutor))
        return w
    }

    @Test func theTutorIsInTheInspectorWithOneConversation() throws {
        let doc = ProjectDocument()
        try doc.read(from: makeTestRom(mapping: .loRom), ofType: Fixture.romType)
        let model = try #require(doc.model)
        model.session.cancelAnalysis()
        doc.makeWindowControllers()
        model.isInspectorVisible = false
        doc.showTutor()
        #expect(model.rightPane == .tutor && model.isInspectorVisible)
        let first = try #require(model.tutor)
        doc.showTutor()
        #expect(model.tutor === first)
        #expect(doc.windowControllers.count == 1, "no second window")
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

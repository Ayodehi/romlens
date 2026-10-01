import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// The C's annotations in the app (docs/24, U10): a C version shown in the
/// C pane and followed by its anchors, the C sheet, and a picture drawn
/// for the tutor.
@MainActor
@Suite(.serialized) struct CAnnotationTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    @Test func aVersionShowsBesideTheGeneratedC() async throws {
        let m = try await model()
        m.select(offset: 0x22)
        m.editorTab = .c
        try await Fixture.settle(until: { m.decompiler.result?.name == "SUB_008020" })
        try m.session.execute(.setCVersion(routine: 0x00_8020, name: "Plain", version: CVersionInfo(
            text: "void ClearSlots(void)\n{\n    memset(slots, 0, 16);\n}\n", author: .tutor,
            anchors: [CAnchorInfo(first: 3, last: 3, start: 0x00_8020, end: 0x00_8027)])))
        try m.session.execute(.setRoutineNote(routine: 0x00_8020, text: "Clears the slots."))
        m.refreshDecompile()
        try await Fixture.settle(until: { m.decompiler.result?.text.contains("/* Clears the slots. */") == true })

        let pane = CPaneController(model: m)
        pane.update()
        pane.showVersion("Plain")
        #expect(pane.textView.string.hasPrefix("void ClearSlots(void)"))
        // A click on the anchored line selects the instruction it stands for.
        m.select(offset: 0x44)
        let at = (pane.textView.string as NSString).range(of: "memset").location
        pane.textView.setSelectedRange(NSRange(location: at, length: 0))
        #expect(m.selectedAddress == 0x00_8020)
        pane.showVersion(nil)
        #expect(pane.textView.string.contains("/* Clears the slots. */"))

        m.beginCEdit(.local(routine: 0x00_8020, local: "x"))
        #expect(m.activeSheet == .cEdit && m.cEdit == .local(routine: 0x00_8020, local: "x"))
    }

    /// An anchor that ends before it starts, or names line 0, is skipped
    /// rather than turned into a range that traps.
    @Test func badAnchorsAreSkipped() {
        let anchors = [
            CAnchorInfo(first: 5, last: 3, start: 0x00_8020, end: 0x00_8027),
            CAnchorInfo(first: 0, last: 2, start: 0x00_8020, end: 0x00_8027),
            CAnchorInfo(first: 2, last: 3, start: 0x00_8020, end: 0x00_8027),
            CAnchorInfo(first: 7, last: 7, start: 0x00_8030, end: 0x00_8031),
        ]
        #expect(CPaneController.anchoredLines(anchors, at: 0x00_8022) == [1, 2])
    }

    @Test func aPictureIsDrawnForTheTutor() async throws {
        let m = try await Fixture.analyzedModel(rom: try Fixture.smallRom())
        let image = NSImage(size: NSSize(width: 4, height: 4))
        image.lockFocus()
        NSColor.systemTeal.setFill()
        NSRect(x: 0, y: 0, width: 4, height: 4).fill()
        image.unlockFocus()
        let tiff = try #require(image.tiffRepresentation)
        let png = try #require(NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]))
        let answer = #"{"data":[{"b64_json":"\#(png.base64EncodedString())"}]}"#
        let name = "romlens-images-\(UUID().uuidString)"
        let settings = TutorSettings(defaults: UserDefaults(suiteName: name)!, keys: MemoryKeyStore())
        let chat = TutorEndpoint.local(name: "Chat", baseURL: tutorTestServer(replies: [
            tutorTestCallReply(name: "generate_image", arguments: #"{"prompt":"the SNES memory map as a diagram","size":"1024x1024"}"#),
            tutorTestTextReply(text: "Here is the map (generated)."),
        ]), kind: .chat, taken: [])
        let draw = TutorEndpoint.local(name: "Draw", baseURL: tutorTestServer(replies: [answer]), kind: .responses, taken: [chat.id])
        settings.add(chat)
        settings.add(draw)
        settings.defaultEndpoint = chat
        settings.setModel("test-model", for: chat)
        settings.imageEndpoint = draw
        settings.imageModel = "test-image"
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(name)
        defer { try? FileManager.default.removeItem(at: root) }
        let tutor = TutorModel(rom: m, settings: settings, root: root)
        tutor.composer = "Draw me the memory map"
        tutor.submit()
        try await Fixture.settle(timeout: 20) { !tutor.busy }
        #expect(tutor.error == nil, "\(tutor.error ?? "")")
        let tools = TranscriptRow.rows(tutor.turns).compactMap { row -> [TutorModel.ToolRow]? in
            if case .answer(_, _, _, let t, _, _) = row { return t } else { return nil }
        }.flatMap { $0 }
        let drawn = try #require(tools.first { $0.name == "generate_image" })
        #expect(drawn.summary == "Drew it with test-image")
        let id = try #require(drawn.images.first)
        #expect(tutor.session?.picture(id: id) == png)
        #expect(settings.imageEndpoints.map(\.id) == ["openai", chat.id, draw.id])
    }
}

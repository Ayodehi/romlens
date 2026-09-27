import Foundation
import Testing
@testable import RomlensKit

/// The tutor as Swift sees it (docs/24, U6): the C's annotations, C text
/// coloured, and a session built with Swift's own key store and listener.
/// Nothing here reaches the network.
@Suite struct TutorTests {
    final class Keys: CredentialStore {
        func key(endpoint: String) -> String? { endpoint == "anthropic" ? "sk-test" : nil }
    }

    final class Heard: TutorListener, @unchecked Sendable {
        var events: [TutorEventInfo] = []
        func onEvent(event: TutorEventInfo) { events.append(event) }
    }

    func workbench() throws -> Workbench {
        let rom = try Rom.fromBytes(bytes: makeSoundTestRom(), name: "sound.sfc")
        let wb = Workbench(rom: rom)
        _ = try wb.analyzeBlocking()
        return wb
    }

    @Test func theCTakesANoteAndAComment() throws {
        let wb = try workbench()
        let entry = UInt32(0x00_8000)
        try wb.executeBatch(commands: [
            .setRoutineNote(routine: entry, text: "Starts the machine."),
            .setCVersion(routine: entry, name: "Plain", version: CVersionInfo(
                text: "void Reset(void) {}\n", author: .tutor,
                anchors: [CAnchorInfo(first: 1, last: 1, start: entry, end: entry)])),
        ], origin: .tutor(conversation: "c1", turn: 1))
        #expect(wb.routineNote(routine: entry) == "Starts the machine.")
        #expect(wb.undoTitle() == "Tutor: 2 Changes")
        let versions = wb.cVersions(routine: entry)
        #expect(versions.count == 1 && versions[0].version.author == .tutor)
        let c = try wb.decompileBlocking(snesAddress: entry, level: .full)
        #expect(c.text.contains("/* Starts the machine. */"))
        let rewound = try wb.rewindTutorEdits(conversation: "c1", fromTurn: 1)
        #expect(rewound.undone == 1)
        #expect(wb.routineNote(routine: entry) == nil)
    }

    @Test func cTextIsColouredInUtf16() throws {
        let wb = try workbench()
        let text = "// é\nINIDISP = 0x80;"
        let tokens = wb.lexC(text: text)
        let ns = text as NSString
        let words = tokens.map { ns.substring(with: NSRange(location: Int($0.start), length: Int($0.len))) }
        #expect(words == ["// é", "INIDISP", "0x80"])
        #expect(tokens[1].kind == .register)
    }

    @Test func aSessionStartsWithoutTheNetwork() throws {
        let wb = try workbench()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("tutor-kit-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let heard = Heard()
        let t = TutorSession(workbench: wb, root: root.path, credentials: Keys(), listener: heard)
        #expect(t.conversations().isEmpty)
        let anthropic = tutorDefaultEndpoints()[0]
        #expect(anthropic.id == "anthropic")
        let model = try #require(tutorDefaultModel(protocol: .anthropic))
        #expect(tutorModels().contains { $0.id == model && $0.vision })
        let id = try t.newConversation(endpoint: anthropic, model: model, effort: nil, mode: .askBeforeEdits, costCap: 1.0)
        #expect(t.conversationId() == id)
        #expect(t.mode() == .askBeforeEdits)
        t.setMode(mode: .readOnly)
        #expect(t.mode() == .readOnly)
        #expect(t.transcript().isEmpty && t.rewindPoints().isEmpty)
        #expect(!t.isBusy() && t.cost() == 0)
    }
}

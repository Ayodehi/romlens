import AppKit
import RomlensKit
import Testing
@testable import Romlens

/// The Timeline, Ports and Echo views (docs/23, A13): notes over a
/// recording and over the ROM's machine as it plays, each led back to the
/// instruction that wrote it and the S-CPU's request before it.
@MainActor
@Suite(.serialized) struct AudioTimelineTests {
    init() { ApuAudio.deviceEnabled = false }

    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeSoundTestRom(), name: "sound.sfc")
        let m = try await Fixture.analyzedModel(rom: rom)
        m.openAudio(.timeline)
        try await Fixture.settle { m.audio.state != nil }
        return m
    }

    @Test func aRecordedNoteLeadsToItsInstructionAndItsCommand() async throws {
        let m = try await model()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("romlens-notes-\(UUID().uuidString).romrec")
        try makeSoundTestRecording(frames: 5).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        #expect(RecordingController.attach(url: url, model: m, window: nil))
        m.audio.pick(.recording)
        m.audio.loadNotes()
        try await Fixture.settle { !m.audio.notes.isEmpty }
        let on = try #require(m.audio.notes.first { $0.kind == .on })
        #expect(on.voice == 0 && on.pitch == 0x1000)
        let spans = PianoRoll.spans(m.audio.notes, until: 4)
        #expect(spans.count == 1 && spans[0].end == 4, "still sounding at the end")
        m.audio.select(note: on)
        let source = try #require(m.audio.noteSource)
        let pc = try #require(source.spcPc)
        #expect(source.commands.first?.port == 0 && source.commands.first?.value == 1)
        // Show in Audio RAM: the part with the instruction, and the line.
        m.audio.showSpc(pc)
        #expect(m.audio.listingTarget == pc)
        #expect(m.audio.state?.map.first { $0.start == m.audio.selectedPart }?.kind == .code)
        // The ports both ways around frame 1.
        let ports = m.audio.portEvents(around: 1)
        #expect(ports.contains { $0.fromCpu && $0.value == 1 } && ports.contains { !$0.fromCpu && $0.value == 1 })
        #expect(m.audio.sentUploads().isEmpty, "the fixture recording starts with its driver in RAM")
    }

    @Test func theRomsNotesAreLoggedAsItPlays() async throws {
        let m = try await model()
        let a = m.audio
        #expect(a.notes.isEmpty)
        a.send(port: 0, value: 1)
        a.output.fill()
        a.tick()
        let on = try #require(a.notes.first { $0.kind == .on })
        #expect(on.spcPc != nil, "logged with its instruction")
        a.select(note: on)
        #expect(a.noteSource?.spcPc == on.spcPc)
        a.pause()
    }

    @Test func theFirResponseOfOneTapIsFlat() {
        let flat = FirResponse.response([0, 0, 0, 0, 0, 0, 0, 64], count: 16)
        #expect(flat.allSatisfy { abs($0 - -6.02) < 0.05 }, "half, at every frequency")
        // Two equal taps side by side pass the lows and cancel at 16 kHz.
        let low = FirResponse.response([0, 0, 0, 0, 0, 0, 64, 64], count: 16)
        #expect(abs(low[0]) < 0.05 && low[15] < -30)
    }
}

import AppKit
import RomlensKit
import Testing
@testable import Romlens

/// Playback (docs/23, A12): the ring the audio thread reads, the transport,
/// the ports, mute and solo, a sample on the keyboard. Nothing is played out
/// loud: the device is off, and the tests read the ring as the audio thread
/// would.
@MainActor
@Suite(.serialized) struct AudioPlaybackTests {
    init() { ApuAudio.deviceEnabled = false }

    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeSoundTestRom(), name: "sound.sfc")
        let m = try await Fixture.analyzedModel(rom: rom)
        m.openAudio(.voices)
        try await Fixture.settle { m.audio.state != nil }
        return m
    }

    /// Read what the ring holds, as the audio thread would.
    private func drain(_ ring: SampleRing, frames: Int = 3200) -> [Float] {
        var l = [Float](repeating: 0, count: frames)
        var r = [Float](repeating: 0, count: frames)
        l.withUnsafeMutableBufferPointer { lp in
            r.withUnsafeMutableBufferPointer { rp in
                _ = ring.read(into: lp.baseAddress!, rp.baseAddress!, frames: frames)
            }
        }
        return l + r
    }

    @Test func theRingHandsOnWhatWasWrittenThenSilence() {
        let ring = SampleRing(capacity: 16)
        ring.write([16384, -16384, 8192, 0])
        #expect(ring.available == 2)
        var l = [Float](repeating: 9, count: 4)
        var r = [Float](repeating: 9, count: 4)
        let n = l.withUnsafeMutableBufferPointer { lp in
            r.withUnsafeMutableBufferPointer { rp in ring.read(into: lp.baseAddress!, rp.baseAddress!, frames: 4) }
        }
        #expect(n == 2)
        #expect(l == [0.5, 0.25, 0, 0] && r == [-0.5, 0, 0, 0])
        // Full, it drops what does not fit rather than overwrite.
        ring.write([Int16](repeating: 1, count: 40))
        #expect(ring.available == 16)
    }

    /// A clear is honoured by the reader: what was written before it, and
    /// what the old player rendered after it, is never heard.
    @Test func aClearDropsWhatTheOldPlayerWrote() {
        let ring = SampleRing(capacity: 16)
        var l = [Float](repeating: 9, count: 4)
        var r = [Float](repeating: 9, count: 4)
        func read() -> Int {
            l.withUnsafeMutableBufferPointer { lp in
                r.withUnsafeMutableBufferPointer { rp in ring.read(into: lp.baseAddress!, rp.baseAddress!, frames: 4) }
            }
        }
        ring.write([100, 100, 100, 100])
        let generation = ring.clear()
        ring.write([200, 200])
        #expect(read() == 0, "nothing of the new player yet")
        #expect(l == [0, 0, 0, 0])
        ring.write([200, 200])
        ring.write([16384, 16384], generation: generation)
        #expect(ring.pending(generation: generation) == 1)
        #expect(read() == 1)
        #expect(l == [0.5, 0, 0, 0])
        #expect(ring.pending(generation: generation) == 0)
    }

    @Test func theRomsDriverPlaysACommandSentOnAPort() async throws {
        let m = try await model()
        let a = m.audio
        a.play()
        #expect(a.isPlaying && a.livePlayer != nil)
        a.output.fill()
        #expect(drain(a.output.ring).allSatisfy { $0 == 0 }, "nothing asked yet")
        a.send(port: 0, value: 1)
        #expect(a.readPort(0) == 1 || a.readPort(0) == 0)
        a.output.fill()
        #expect(drain(a.output.ring).contains { $0 != 0 }, "the note")
        #expect(a.readPort(0) == 1, "the driver echoes the command")
        #expect(a.sent.last?.value == 1)
        // Solo another voice: silence, and voice 0 still runs.
        a.toggleSolo(3)
        #expect(a.isSolo(3) && a.livePlayer?.muted() == ~UInt8(8))
        a.output.fill()
        #expect(drain(a.output.ring).allSatisfy { $0 == 0 })
        #expect(a.scope(0, count: 64).contains { $0 != 0 })
        a.toggleSolo(3)
        #expect(a.muted == 0)
        a.tick()
        #expect(a.playedSeconds > 0)
        a.pause()
        #expect(!a.isPlaying)
    }

    @Test func playThisCommandBootsTheDriverAndSendsIt() async throws {
        let m = try await model()
        m.showTab(.disassembly)
        m.openAudio(.voices)
        m.audio.playCommand(port: 0, value: 1)
        #expect(m.audio.isPlaying && m.audio.source == .rom)
        #expect(m.audio.sent.last?.port == 0)
        m.audio.pause()
    }

    @Test func aRecordingPlaysFromTheFrameAndStopsAtItsEnd() async throws {
        let m = try await model()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("romlens-play-\(UUID().uuidString).romrec")
        try makeSoundTestRecording(frames: 5).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        #expect(RecordingController.attach(url: url, model: m, window: nil))
        m.audio.pick(.recording)
        m.graphics.frame = 0
        m.audio.play()
        #expect(m.audio.isPlaying)
        m.audio.output.fill()
        #expect(drain(m.audio.output.ring).contains { $0 != 0 }, "frame 1's command, followed")
        // 0.1 s is six frames: past the fifth, it stops.
        m.audio.tick()
        #expect(!m.audio.isPlaying)
    }

    /// The real output, at zero volume: the device pulls from the ring and
    /// the queue keeps it filled, so time moves on its own. Skipped where
    /// there is no output device.
    @Test func theDevicePullsFromTheRing() async throws {
        let m = try await model()
        let a = m.audio
        ApuAudio.deviceEnabled = true
        defer { ApuAudio.deviceEnabled = false }
        a.output.volume = 0
        a.play()
        defer { a.output.shutDown() }
        guard a.output.isEngineRunning else { return }
        a.send(port: 0, value: 1)
        try await Task.sleep(for: .milliseconds(400))
        a.tick()
        #expect(a.playedSeconds > 0.25, "rendered \(a.playedSeconds) s")
        a.pause()
    }

    @Test func aSamplePlaysFromTheKeyboard() async throws {
        let m = try await model()
        let a = m.audio
        #expect(a.pitch(sample: 0, semitones: 12) > a.pitch(sample: 0, semitones: 0))
        a.press(sample: 0, pitch: 0x1000)
        #expect(a.playing == .sample(0))
        a.output.fill()
        #expect(drain(a.output.ring).contains { $0 != 0 })
        a.release()
        a.stopSample()
        #expect(a.playing == nil)
    }
}

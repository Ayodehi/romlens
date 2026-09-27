import Foundation
import Testing
@testable import RomlensKit

/// The sound side as Swift sees it (docs/23, A10): a recording's voices and
/// audio RAM, the upload traced in the ROM, and a player rendering sound.
@Suite struct AudioTests {
    @Test func aRecordingFrameReadsAsVoicesAndAudioRam() throws {
        let rec = try RecordingSession.fromBytes(bytes: makeSoundTestRecording(frames: 5))
        #expect(rec.hasSound())
        let voices = try rec.voices(frame: 3)
        #expect(voices.count == 8)
        #expect(voices[0].sounding && voices[0].pitch == 0x1000)
        let regs = try rec.dspRegisters(frame: 3)
        #expect(regs[0x4C].name == "KON")
        let map = try rec.aramMap(frame: 3)
        #expect(map.contains { $0.kind == .directory })
        let sample = try rec.sample(frame: 3, index: 0, maxBlocks: 64)
        #expect(sample.blocks.count == 4 && sample.loopBlock == 2)
        let ports = try rec.portEvents(from: 1, to: 1)
        #expect(ports.contains { $0.fromCpu && $0.value == 1 })
        #expect(!(try rec.noteTimeline(from: 0, to: 4)).isEmpty)
        #expect(!(try rec.spcListing(frame: 3, from: 0x0200, count: 16)).isEmpty)
    }

    @Test func theRomUploadPlaysWhenAsked() throws {
        let rom = try Rom.fromBytes(bytes: makeSoundTestRom(), name: "sound.sfc")
        let wb = Workbench(rom: rom)
        _ = try wb.analyzeBlocking()
        let report = wb.soundUploadBlocking()
        #expect(report.uploads.count == 1 && report.uploads[0].driver)
        let player = try ApuPlayer.fromUpload(workbench: wb, with: [], settleSeconds: 0.05)
        player.sendPort(port: 0, value: 1)
        let out = player.render(samples: 3200)
        #expect(out.count == 6400)
        #expect(out.contains { $0 != 0 })
        #expect(player.scope(which: 0, count: 128).count == 128)
        player.solo(voice: 5)
        #expect(player.render(samples: 320).allSatisfy { $0 == 0 })
    }

    @Test func anEnvelopeCurveComesFromTheDsp() {
        let c = envelopeCurve(adsr1: 0x8F, adsr2: 0xE0, gain: 0, holdMs: 10, releaseMs: 10, stride: 1)
        #expect(c.count == 640 && c[1] == 1024)
        #expect(noteForFrequency(hz: 440) == "A4")
    }
}

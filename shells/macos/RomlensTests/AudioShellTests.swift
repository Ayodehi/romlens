import AppKit
import RomlensKit
import SwiftUI
import Testing
@testable import Romlens

/// The sound views (docs/23, A11): the ROM's upload booted by Romlens, a
/// recording's sound side at a frame, and the Voices, Samples and Audio RAM
/// views drawn over each.
@MainActor
@Suite struct AudioShellTests {
    private func model() async throws -> RomViewModel {
        let rom = try Rom.fromBytes(bytes: makeSoundTestRom(), name: "sound.sfc")
        return try await Fixture.analyzedModel(rom: rom)
    }

    private func recordingURL() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("romlens-sound-\(UUID().uuidString).romrec")
        try makeSoundTestRecording(frames: 5).write(to: url)
        return url
    }

    @Test func theRomsUploadIsBootedAndShown() async throws {
        let m = try await model()
        m.openAudio(.voices)
        #expect(m.audioTab == .voices && m.graphicsTab == nil)
        #expect(m.audio.source == .rom)
        try await Fixture.settle { m.audio.state != nil }
        let state = try #require(m.audio.state)
        #expect(state.voices.count == 8)
        #expect(state.registers[0x5D].value == 0x3C, "the driver set DIR as it started")
        #expect(state.map.contains { $0.kind == .code })
        let driver = try #require(m.audio.driverUpload)
        #expect(driver.entry == 0x0200)
        #expect(m.audio.otherUploads.isEmpty)
        #expect(m.audio.sourceDescription.contains("run by Romlens"))
        // The directory came from the ROM, and Show in ROM goes there.
        let dir = try #require(state.map.first { $0.kind == .directory })
        let blocks = m.audio.blocks(filling: dir)
        #expect(blocks.count == 1)
        #expect(dir.romOffset == blocks[0].block.romOffset)
        let sample = try #require(m.audio.sample(0))
        #expect(sample.blocks.count == 4 && sample.loopBlock == 2)
        #expect(m.audio.romOrigin(0x4000) == driver.blocks[2].romOffset)
        m.audio.showInRom?(driver.blocks[2].romOffset)
        #expect(m.audioTab == nil && m.editorTab == .disassembly)
        #expect(m.selectedOffset == driver.blocks[2].romOffset)
    }

    @Test func aRecordingsSoundIsReadAtTheGraphicsFrame() async throws {
        let m = try await model()
        let url = try recordingURL()
        defer { try? FileManager.default.removeItem(at: url) }
        #expect(RecordingController.attach(url: url, model: m, window: nil))
        m.openAudio(.samples)
        #expect(m.audio.source == .recording)
        m.graphics.frame = 3
        let state = try #require(m.audio.state)
        #expect(state.voices[0].sounding, "frame 1's command keyed voice 0 on")
        #expect(state.samples.first?.start == 0x4000)
        #expect(m.audio.frame == 3)
        #expect(!m.audio.listing(from: state.pc, count: 8).isEmpty)
        #expect(m.audio.aram(start: 0x3C00, len: 2) == Data([0x00, 0x40]))
        // One frame for both: stepping the audio's steps the graphics'.
        m.audio.frame = 2
        #expect(m.graphics.frame == 2)
        // The ROM is still there to choose.
        m.audio.pick(.rom)
        try await Fixture.settle { m.audio.state != nil }
        #expect(m.audio.source == .rom)
        m.openAudio(.voices)
        #expect(m.audio.source == .rom, "picked by hand, so kept")
    }

    @Test func soundAndGraphicsViewsShareTheEditor() async throws {
        let m = try await model()
        m.openAudio(.aram)
        m.openGraphics(.tiles)
        #expect(m.audioTab == nil && m.graphicsTab == .tiles)
        m.openAudio(.voices)
        #expect(m.graphicsTab == nil && m.audioTab == .voices && !m.showsTextEditor)
        m.editorTab = .hex
        #expect(m.audioTab == nil && m.showsTextEditor)
    }

    @Test func theUploadsLaidOverTheDriverSkipOnesThatReplaceOthers() {
        func upload(_ list: UInt32, driver: Bool, _ aram: UInt16, _ len: UInt16) -> UploadInfo {
            UploadInfo(
                list: list, setAt: 0, setIn: 0,
                blocks: [UploadBlockInfo(aram: aram, len: len, romOffset: 0, snes: list)],
                entry: 0x0500, bytes: UInt32(len), driver: driver
            )
        }
        let report = UploadReportInfo(routines: [], uploads: [
            upload(0x0E8000, driver: true, 0x0500, 0x1000),
            upload(0x0F8000, driver: false, 0x8000, 0x7000),
            upload(0x0E98B1, driver: false, 0x1500, 0x2000),
            upload(0x0EAED6, driver: false, 0x1500, 0x2000),
        ], commands: [])
        #expect(AudioModel.defaultLists(report) == [0x0F8000, 0x0E98B1])
    }

    @Test func everySoundViewLaysOutInAWindow() async throws {
        ApuAudio.deviceEnabled = false
        let m = try await model()
        defer { m.audio.pause() }
        let url = try recordingURL()
        defer { try? FileManager.default.removeItem(at: url) }
        let controller = RomWindowController(model: m)
        controller.window?.setContentSize(NSSize(width: 1500, height: 950))
        controller.window?.orderFront(nil)
        defer { controller.window?.close() }
        let content = try #require(controller.window?.contentView)
        for withRecording in [false, true] {
            if withRecording {
                #expect(RecordingController.attach(url: url, model: m, window: nil))
                m.graphics.frame = 3
            }
            for tab in AudioModel.Tab.allCases {
                m.openAudio(tab)
                try await Fixture.settle { m.audio.state != nil }
                switch tab {
                case .voices: m.audio.selectedVoice = 0
                case .samples:
                    m.audio.selectedSample = 0
                    m.audio.selectedBlock = 1
                case .aram: m.audio.selectedPart = m.audio.state?.map.first { $0.kind == .code }?.start
                case .scope: if !withRecording { m.audio.play() }
                }
                content.layoutSubtreeIfNeeded()
                Fixture.spin(0.05)
                content.layoutSubtreeIfNeeded()
                content.display()
                #expect(m.audioTab == tab)
                // With ROMLENS_SNAPSHOTS set, each view is saved as a PNG in the
                // sandbox's temporary folder, to look at.
                if ProcessInfo.processInfo.environment["ROMLENS_SNAPSHOTS"] != nil,
                   let rep = content.bitmapImageRepForCachingDisplay(in: content.bounds) {
                    content.cacheDisplay(in: content.bounds, to: rep)
                    let name = "\(tab.rawValue)-\(withRecording ? "rec" : "rom").png"
                    try rep.representation(using: .png, properties: [:])?
                        .write(to: FileManager.default.temporaryDirectory.appendingPathComponent(name))
                }
            }
        }
    }
}

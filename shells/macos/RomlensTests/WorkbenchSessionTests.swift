import Foundation
import RomlensKit
import Testing
@testable import Romlens

@MainActor
@Suite struct WorkbenchSessionTests {
    @Test func generationUndoTitlesAndCommandOrder() async throws {
        let rom = try Fixture.smallRom()
        let session = WorkbenchSession(workbench: Workbench(rom: rom))
        session.reanalysisDelay = .milliseconds(10)
        var log: [String] = []
        session.onChange = { kind in log.append("change:\(kind)") }
        session.onCommand = { kind in log.append("command:\(kind)") }
        _ = try await session.workbench.analyze()
        try await Fixture.settle { session.hasSnapshot }
        #expect(session.analysisGeneration == 1)
        #expect(session.stats?.instructions == 8)
        let before = session.generation
        try session.setLabel(address: 0x8000, name: "Boot")
        #expect(session.generation == before + 1)
        #expect(session.canUndo && !session.canRedo)
        #expect(session.undoTitle == "Rename Label")
        #expect(session.isDirty)
        #expect(log.suffix(2) == ["change:view", "command:project(dirty: true)"], "\(log)")
        #expect(session.undo())
        #expect(session.canRedo && session.redoTitle == "Rename Label")
        #expect(session.redo())
        // An analysis-affecting command schedules a re-run.
        try session.mark(start: 12, len: 2, kind: .data, dataKind: .byte)
        #expect(session.workbench.needsAnalysis())
        try await Fixture.settle { session.analysisGeneration >= 2 && !session.workbench.needsAnalysis() }
        #expect(session.analysis == .idle)
        #expect(!session.undo() == false)
    }

    @Test func progressEventsReachTheMainActor() async throws {
        let rom = try Fixture.rom(megabytes: 1)
        let session = WorkbenchSession(workbench: Workbench(rom: rom))
        session.startAnalysis()
        #expect(session.analysis.isRunning)
        try await Fixture.settle { session.hasSnapshot && !session.analysis.isRunning }
        #expect(session.stats != nil)
        session.cancelAnalysis()
        #expect(session.analysis == .idle)
    }

    /// A rerun asked for during a run waits for that run instead of
    /// cancelling it. Cancelling meant a live session, merging its execution
    /// log every second, could cancel every run and the numbers never moved.
    @Test func aRerunDuringARunFollowsIt() async throws {
        let session = WorkbenchSession(workbench: Workbench(rom: try Fixture.rom(megabytes: 1)))
        session.reanalysisDelay = .zero
        let before = session.analysisGeneration
        session.startAnalysis()
        session.scheduleReanalysis()
        try await Fixture.settle { session.analysisGeneration >= before + 2 && !session.analysis.isRunning }
        #expect(session.analysisGeneration == before + 2, "both runs finished")
    }

    /// The last progress report can arrive after the analysis finished; it
    /// must not start the bar again, or "building lines" stays up for good.
    @Test func progressAfterTheRunEndsIsIgnored() async throws {
        let session = WorkbenchSession(workbench: Workbench(rom: try Fixture.smallRom()))
        #expect(session.analysis == .idle)
        session.handle(.analysisProgress(phase: .lines, done: 1, total: 1))
        #expect(session.analysis == .idle)

        session.startAnalysis()
        session.handle(.analysisProgress(phase: .lines, done: 0, total: 1))
        #expect(session.analysis == .running(fraction: 0, phase: "building lines"))
        try await Fixture.settle { session.hasSnapshot && !session.analysis.isRunning }
        session.handle(.analysisProgress(phase: .lines, done: 1, total: 1))
        #expect(session.analysis == .idle)
    }
}

@MainActor
@Suite struct NavigatorModelTests {
    @Test func filtersTwentyThousandLabelsQuickly() {
        var labels: [LabelInfo] = []
        for i in 0..<20_000 {
            let address = UInt32(0x808000 + i * 3)
            labels.append(LabelInfo(
                address: address, name: i % 7 == 0 ? "Player_\(i)" : "CODE_\(String(address, radix: 16, uppercase: true))",
                source: i % 50 == 0 ? .user : .auto, origin: "", fileOffset: UInt32(i * 3)
            ))
        }
        let start = Date()
        let byName = NavigatorModel.filter(labels, query: "player_1")
        let byAddress = NavigatorModel.filter(labels, query: "$80:80")
        let all = NavigatorModel.filter(labels, query: "")
        let elapsed = Date().timeIntervalSince(start)
        print("navigator filter: 3 passes over 20k labels in \(Int(elapsed * 1000)) ms")
        #expect(elapsed < 0.02 * 3, "\(elapsed)")
        #expect(!byName.isEmpty && byName.allSatisfy { $0.name.lowercased().contains("player_1") })
        #expect(!byAddress.isEmpty && byAddress.allSatisfy { $0.address >> 8 == 0x8080 })
        #expect(all.count == 20_000)
        #expect(all.first?.source == .user, "user labels first")
    }

    @Test func banksStepThroughTheImage() throws {
        let banks = NavigatorModel.banks(rom: try Fixture.rom(megabytes: 1))
        #expect(banks.count == 32)
        #expect(banks.first?.bank == 0 && banks.first?.fileOffset == 0)
        #expect(banks.allSatisfy { $0.length == 0x8000 })
    }
}

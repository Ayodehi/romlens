import Foundation
import Testing
@testable import RomlensKit

@Suite struct RomlensKitTests {
    @Test func opensTheHomebrewFixture() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let info = rom.info()
        #expect(info.title == "ROMLENS TEST")
        #expect(info.mapping == .loRom)
        #expect(info.checksumOk)
        #expect(rom.rowCount() == 2048)
    }

    @Test func hexRowsAreFlatBatches() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let batch = rom.hexRows(startRow: 0, count: 4)
        #expect(batch.count == 8 + 4 * Int(hexRowStride()))
        #expect(Int(hexBatchHeaderLen()) == 8)
        #expect(batch[8 + 12] == 0x78)
        #expect(batch[8 + 13] == 0x18)
        #expect(batch[8 + 14] == 0xFB)
    }

    @Test func resolvesAndFormats() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let r = try rom.resolve(text: "$00:8000")
        #expect(r.fileOffset == 0)
        #expect(r.row == 0)
        #expect(formatSnesAddress(address: 0x80841C) == "$80:841C")
        #expect(formatFileOffset(offset: 0x41C) == "0x00041C")
        #expect(throws: RomlensError.self) { try rom.resolve(text: "$7E:0000") }
        #expect(apiVersion() == "0.2.0")
    }

    @Test func inspectorAndSpans() throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .hiRom), name: "t.sfc")
        let spans = rom.spans()
        #expect(spans.count == 22)
        let reset = try #require(spans.first { $0.name == "Emulation RESET" })
        #expect(reset.start == 0xFFFC)
        let byte = try #require(rom.inspect(fileOffset: 0xFFFC))
        #expect(byte.valueU16Le == 0x8000)
        #expect(byte.spanName == "Emulation RESET")
    }
}

final class EventLog: WorkbenchListener, @unchecked Sendable {
    private let lock = NSLock()
    private var events: [WorkbenchEvent] = []
    func onEvent(event: WorkbenchEvent) {
        lock.lock()
        events.append(event)
        lock.unlock()
    }
    var all: [WorkbenchEvent] {
        lock.lock()
        defer { lock.unlock() }
        return events
    }
}

@Suite struct WorkbenchTests {
    @Test func analyzesEditsAndRoundTrips() async throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let workbench = Workbench(rom: rom)
        let log = EventLog()
        workbench.setListener(listener: log)
        #expect(workbench.needsAnalysis())
        let stats = try await workbench.analyze()
        #expect(stats.instructions == 8)
        #expect(workbench.lineCount() > 0)
        #expect(workbench.asmLinesText(startLine: 0, count: 4, style: .snes).contains("SEI"))
        let batch = workbench.asmLines(startLine: 0, count: 4)
        #expect(batch.count >= Int(asmBatchHeaderLen()) + 4 * Int(asmLineStride()))
        #expect(log.all.contains { if case .snapshotChanged(let g) = $0 { return g == 1 } else { return false } })

        try workbench.execute(command: .setLabel(address: 0x8000, name: "Boot"))
        #expect(workbench.isDirty())
        #expect(workbench.undoTitle() == "Rename Label")
        #expect(workbench.labelAt(snesAddress: 0x8000)?.name == "Boot")
        #expect(try workbench.undo())
        #expect(workbench.labelAt(snesAddress: 0x8000)?.name == "RESET_008000")
        #expect(try workbench.redo())
        let files = workbench.projectFiles()
        #expect(files.count == 5)
        let again = try Workbench.withProjectFiles(rom: rom, files: files)
        #expect(again.labelAt(snesAddress: 0x8000)?.name == "Boot")
        #expect(throws: RomlensError.self) {
            try workbench.execute(command: .setLabel(address: 0x8000, name: "bad name"))
        }
        #expect(workbench.instructionAt(fileOffset: 8)?.hardwareRegister?.name == "INIDISP")
        #expect(hardwareRegister(address: 0x420D)?.name == "MEMSEL")
        #expect(workbench.xrefsTo(snesAddress: 0x2100).count == 1)
    }

    @Test func cancellationIsAnError() async throws {
        let rom = try Rom.fromBytes(bytes: makeTestRom(mapping: .loRom), name: "t.sfc")
        let workbench = Workbench(rom: rom)
        let task = Task { try await workbench.analyze() }
        task.cancel()
        // Either the run finished before the cancel landed or it reports cancellation.
        do {
            _ = try await task.value
        } catch RomlensError.Cancelled {
        } catch is CancellationError {
        }
        _ = try workbench.analyzeBlocking()
        #expect(!workbench.needsAnalysis())
    }
}

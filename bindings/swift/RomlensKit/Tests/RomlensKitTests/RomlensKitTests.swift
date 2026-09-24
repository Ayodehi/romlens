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
        #expect(apiVersion() == "0.7.0")
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

@Suite struct DecompileTests {
    @Test func decompilesTheRoutineAtAnAddress() async throws {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        let workbench = Workbench(rom: rom)
        _ = try await workbench.analyze()
        let d = try await workbench.decompile(snesAddress: 0x008040, level: .full)
        #expect(d.name == "SUB_008040")
        #expect(d.text.contains("if (a < ADDR_7E0021) {"))
        #expect(d.lines.count == d.text.split(separator: "\n", omittingEmptySubsequences: false).count - 1)
        let name = d.tokens.first { $0.kind == .function }
        #expect(name?.address == 0x008040)
        let text = d.text as NSString
        #expect(text.substring(with: NSRange(location: Int(name!.start), length: Int(name!.len))) == "SUB_008040")
        #expect(workbench.functionContaining(fileOffset: 0x46) == 0x008040)
        #expect(snesHeader().contains("#define INIDISP MEM8(0x2100)"))
        await #expect(throws: RomlensError.self) {
            try await workbench.decompile(snesAddress: 0x008041, level: .lift)
        }
    }

    @Test func graphsARoutineAndItsCalls() async throws {
        let rom = try Rom.fromBytes(bytes: makeRoutinesTestRom(), name: "r.sfc")
        let workbench = Workbench(rom: rom)
        _ = try await workbench.analyze()
        let g = try await workbench.routineGraph(snesAddress: 0x008020)
        #expect(g.name == "SUB_008020")
        #expect(g.blocks.count == 3)
        #expect(g.edges.contains { $0.back && $0.kind == .taken })
        #expect(g.blocks[1].loopHeader)
        let calls = try await workbench.callNeighbourhood(snesAddress: 0x008020)
        #expect(calls.callers.map(\.entry) == [0x008000])
        let layout = layoutGraph(
            nodes: g.blocks.map { GraphNodeSize(width: 120, height: 14 * Double(max($0.lineCount, 1))) },
            edges: g.edges.map { GraphEdgeSpec(from: $0.from, to: $0.to, back: $0.back) },
            options: GraphLayoutOptions(nodeGap: 20, layerGap: 30, edgeGap: 10)
        )
        #expect(layout.rows == [0, 1, 2])
        #expect(layout.edges.count == g.edges.count)
    }

    @Test func explainsHardwareWritesAndIdioms() async throws {
        let rom = try Rom.fromBytes(bytes: makeExplainTestRom(), name: "e.sfc")
        let workbench = Workbench(rom: rom)
        _ = try await workbench.analyze()
        let x = workbench.explainAt(fileOffset: 0x53)
        #expect(x.register?.short == "NMITIMEN = $81: NMI on, joypad auto-read on")
        #expect(x.register?.parts.first?.fields.first?.meaning == "NMI on")
        let wait = workbench.explainAt(fileOffset: 0x56)
        #expect(wait.register?.store == false)
        #expect(wait.idioms.first?.kind == .wait)
        #expect(wait.idioms.first?.title == "Wait for vertical blank")
        #expect(workbench.showExplanations())
        let lines = workbench.lineCount()
        workbench.setShowExplanations(show: false)
        #expect(workbench.lineCount() < lines)
    }
}

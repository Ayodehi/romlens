import Foundation
import Observation
import RomlensKit

/// The C tab's state: which routine it shows, at which level, and the
/// decompiled text. The routine follows the selection; the text is rebuilt
/// when the routine, the level or the analysis changes (docs/18).
@MainActor
@Observable
final class DecompileModel {
    enum State: Equatable {
        case idle
        case loading
        case ready
        /// The selection is not inside a routine the analysis found.
        case notInRoutine
        case failed(String)
    }

    var level: DecompileLevel = .full {
        didSet { if level != oldValue { invalidate() } }
    }
    private(set) var state: State = .idle
    private(set) var result: DecompiledInfo?
    /// Bumped whenever `result` changes, so views rebuild their text once.
    private(set) var resultGeneration = 0
    /// The routine shown (or being decompiled), by entry address.
    private(set) var entry: UInt32?
    /// For each file offset an instruction starts at, the C lines it made.
    @ObservationIgnored private var linesByOffset: [UInt32: [Int]] = [:]
    @ObservationIgnored private var key: Key?
    @ObservationIgnored private var task: Task<Void, Never>?

    private struct Key: Equatable {
        let entry: UInt32
        let level: DecompileLevel
        let generation: Int
    }

    /// Show the routine containing `instructionStart` (an instruction's file
    /// offset), decompiling it if the key changed. `generation` changes
    /// with every analysis, rename or variable.
    func follow(workbench: Workbench, instructionStart: UInt32?, generation: Int) {
        guard let start = instructionStart else { return }
        // Still inside the routine shown: nothing to do.
        if let key, key.level == level, key.generation == generation, linesByOffset[start] != nil {
            return
        }
        guard let entry = workbench.functionContaining(fileOffset: start) else {
            if linesByOffset[start] == nil {
                task?.cancel()
                self.key = nil
                self.entry = nil
                result = nil
                linesByOffset = [:]
                resultGeneration += 1
                state = .notInRoutine
            }
            return
        }
        let next = Key(entry: entry, level: level, generation: generation)
        guard next != key else { return }
        key = next
        self.entry = entry
        state = .loading
        task?.cancel()
        task = Task { [weak self] in
            do {
                let d = try await workbench.decompile(snesAddress: entry, level: next.level)
                guard let self, !Task.isCancelled, self.key == next else { return }
                self.install(d)
            } catch {
                guard let self, !Task.isCancelled, self.key == next else { return }
                self.state = .failed("\(error)")
            }
        }
    }

    /// Forget the key so the next `follow` decompiles again.
    func invalidate() {
        key = nil
        linesByOffset = [:]
    }

    private func install(_ d: DecompiledInfo) {
        result = d
        var map: [UInt32: [Int]] = [:]
        for (line, offsets) in d.lines.enumerated() {
            for o in offsets {
                map[o, default: []].append(line)
            }
        }
        linesByOffset = map
        resultGeneration += 1
        state = .ready
    }

    /// The C lines the instruction at `offset` made.
    func lines(forInstructionAt offset: UInt32) -> [Int] {
        linesByOffset[offset] ?? []
    }

    /// The instructions line `line` came from.
    func offsets(forLine line: Int) -> [UInt32] {
        guard let d = result, line >= 0, line < d.lines.count else { return [] }
        return d.lines[line]
    }

    /// The unit and `snes.h`, for Export C….
    var exportText: String? { result?.text }
}

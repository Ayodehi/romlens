import Foundation
import Observation
import RomlensKit

/// The Graph tab's state (docs/19): the routine at the cursor as blocks, or
/// with its callers and callees. Like the C tab it follows the selection
/// and is rebuilt when the analysis or the names change; one build runs at
/// a time, and a newer request follows it, so a live session's analyses
/// never keep it from finishing.
@MainActor
@Observable
final class GraphModel {
    enum Mode: String, CaseIterable, Identifiable {
        case blocks, calls
        var id: String { rawValue }
        var title: String {
            switch self {
            case .blocks: "Blocks"
            case .calls: "Calls"
            }
        }
    }

    enum State: Equatable {
        case idle
        case loading
        case ready
        /// The selection is not inside a routine the analysis found.
        case notInRoutine
        case failed(String)
    }

    enum Zoom { case zoomIn, zoomOut, fit }

    /// View › Zoom In, Zoom Out and Zoom to Fit, for the canvas to act on.
    private(set) var zoomRequest: (kind: Zoom, id: Int)?

    func requestZoom(_ kind: Zoom) {
        zoomRequest = (kind, (zoomRequest?.id ?? 0) + 1)
    }

    var mode: Mode = .blocks {
        didSet { if mode != oldValue { invalidate() } }
    }
    private(set) var state: State = .idle
    private(set) var blocks: RoutineGraphInfo?
    private(set) var calls: CallNeighbourhoodInfo?
    /// Bumped whenever `blocks` or `calls` changes.
    private(set) var resultGeneration = 0
    /// The routine shown (or being built), by entry address.
    private(set) var entry: UInt32?
    /// The block holding each instruction, by file offset.
    @ObservationIgnored private var blockByOffset: [UInt32: Int] = [:]
    @ObservationIgnored private var key: Key?
    @ObservationIgnored private var shown: Key?
    @ObservationIgnored private var task: Task<Void, Never>?

    private struct Key: Equatable {
        let entry: UInt32
        let mode: Mode
        let generation: Int
    }

    private enum Built {
        case blocks(RoutineGraphInfo)
        case calls(CallNeighbourhoodInfo)
    }

    /// Show the routine containing `instructionStart`. `generation` changes
    /// with every analysis, rename or variable.
    func follow(workbench: Workbench, instructionStart: UInt32?, generation: Int) {
        guard let start = instructionStart else { return }
        // Still inside the routine shown.
        if let key, key.mode == mode, key.generation == generation, contains(start) {
            return
        }
        guard let entry = workbench.functionContaining(fileOffset: start) else {
            if !contains(start) {
                key = nil
                shown = nil
                self.entry = nil
                blocks = nil
                calls = nil
                blockByOffset = [:]
                resultGeneration += 1
                state = .notInRoutine
            }
            return
        }
        let next = Key(entry: entry, mode: mode, generation: generation)
        guard next != key else { return }
        key = next
        self.entry = entry
        if shown?.entry != entry || shown?.mode != mode || !hasResult {
            state = .loading
        }
        if task == nil {
            begin(next, workbench: workbench)
        }
    }

    /// Forget the key so the next `follow` builds again; what is shown
    /// stays until the new graph replaces it.
    func invalidate() {
        key = nil
    }

    /// The block holding the instruction at `offset`, in Blocks mode.
    func block(containing offset: UInt32) -> Int? {
        blockByOffset[offset]
    }

    private var hasResult: Bool {
        switch shown?.mode {
        case .blocks: blocks != nil
        case .calls: calls != nil
        case nil: false
        }
    }

    private func contains(_ offset: UInt32) -> Bool {
        switch mode {
        case .blocks: blockByOffset[offset] != nil
        // The neighbourhood is the routine's; any instruction of it will do,
        // and the blocks are not built in this mode.
        case .calls: false
        }
    }

    private func begin(_ run: Key, workbench: Workbench) {
        task = Task { [weak self] in
            let outcome: Result<Built, Error>
            do {
                switch run.mode {
                case .blocks:
                    outcome = .success(.blocks(try await workbench.routineGraph(snesAddress: run.entry)))
                case .calls:
                    outcome = .success(.calls(try await workbench.callNeighbourhood(snesAddress: run.entry)))
                }
            } catch {
                outcome = .failure(error)
            }
            guard let self else { return }
            self.task = nil
            guard let key = self.key else { return }
            if key.entry == run.entry, key.mode == run.mode {
                switch outcome {
                case .success(let built):
                    self.install(built, for: run)
                case .failure(let error):
                    if key == run { self.state = .failed("\(error)") }
                }
            }
            if key != run {
                self.begin(key, workbench: workbench)
            }
        }
    }

    private func install(_ built: Built, for run: Key) {
        shown = run
        switch built {
        case .blocks(let g):
            blocks = g
            calls = nil
            var map: [UInt32: Int] = [:]
            for (i, b) in g.blocks.enumerated() {
                for o in b.offsets {
                    map[o] = i
                }
            }
            blockByOffset = map
        case .calls(let n):
            calls = n
            blocks = nil
            blockByOffset = [:]
        }
        resultGeneration += 1
        state = .ready
    }
}

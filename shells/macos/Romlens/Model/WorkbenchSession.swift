import Foundation
import Observation
import RomlensKit

/// Owns the core `Workbench` for one document: runs analysis, applies
/// commands, mirrors undo state, and turns core events into main-actor
/// observable changes.
@MainActor
@Observable
final class WorkbenchSession {
    enum AnalysisState: Equatable {
        case idle
        case running(fraction: Double, phase: String)
        case failed(String)

        var isRunning: Bool {
            if case .running = self { return true }
            return false
        }
    }

    enum ChangeKind: Equatable {
        case snapshot
        case view
        case project(dirty: Bool)
    }

    let workbench: Workbench
    private(set) var analysis: AnalysisState = .idle
    /// Bumped on every snapshot or view change; the view model watches this.
    private(set) var generation = 0
    private(set) var analysisGeneration: UInt64 = 0
    private(set) var viewGeneration: UInt64 = 0
    private(set) var stats: AnalysisStats?
    private(set) var hasSnapshot = false
    private(set) var canUndo = false
    private(set) var canRedo = false
    private(set) var undoTitle: String?
    private(set) var redoTitle: String?
    private(set) var isDirty = false
    /// The document sets this to count changes; called after every command.
    @ObservationIgnored var onCommand: ((ChangeKind) -> Void)?
    /// The view model sets this to drop caches.
    @ObservationIgnored var onChange: ((ChangeKind) -> Void)?
    @ObservationIgnored private var bridge: ListenerBridge?
    @ObservationIgnored private var analysisTask: Task<Void, Never>?
    @ObservationIgnored private var reanalysisTask: Task<Void, Never>?
    /// Another run was asked for while one was running; start it when that
    /// one ends.
    @ObservationIgnored private var rerunRequested = false
    /// Debounce for analysis-affecting commands.
    @ObservationIgnored var reanalysisDelay: Duration = .milliseconds(300)

    init(workbench: Workbench) {
        self.workbench = workbench
        let bridge = ListenerBridge(session: self)
        self.bridge = bridge
        workbench.setListener(listener: bridge)
        refreshUndoState()
    }

    // MARK: Analysis

    func startAnalysis() {
        analysisTask?.cancel()
        analysis = .running(fraction: 0, phase: "starting")
        let workbench = self.workbench
        analysisTask = Task { [weak self] in
            do {
                let stats = try await workbench.analyze()
                guard let self, !Task.isCancelled else { return }
                self.stats = stats
                self.analysis = .idle
                self.startRequestedRerun()
            } catch is CancellationError {
                self?.analysis = .idle
            } catch {
                guard let self else { return }
                if case RomlensError.Cancelled = error {
                    self.analysis = .idle
                } else {
                    self.analysis = .failed(error.localizedDescription)
                }
                self.startRequestedRerun()
            }
        }
    }

    private func startRequestedRerun() {
        guard rerunRequested else { return }
        rerunRequested = false
        startAnalysis()
    }

    func cancelAnalysis() {
        rerunRequested = false
        workbench.cancelAnalysis()
        analysisTask?.cancel()
        analysisTask = nil
        analysis = .idle
    }

    /// Re-run after a short pause. A run already under way finishes first
    /// and the new one follows it: cancelling it instead meant that changes
    /// arriving faster than a run takes, as a live session's execution log
    /// does every second, could cancel every run and the numbers never moved.
    func scheduleReanalysis() {
        reanalysisTask?.cancel()
        let delay = reanalysisDelay
        reanalysisTask = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard let self, !Task.isCancelled else { return }
            if self.analysis.isRunning {
                self.rerunRequested = true
            } else {
                self.startAnalysis()
            }
        }
    }

    // MARK: Commands

    func setLabel(address: UInt32, name: String?) throws {
        try execute(.setLabel(address: address, name: name))
    }

    func removeLabel(address: UInt32) throws {
        try execute(.setLabel(address: address, name: nil))
    }

    /// Name routines from another version of the ROM, as one undo step
    /// (docs/22, D2).
    func carryNames(_ names: [CarriedNameInfo]) throws {
        guard !names.isEmpty else { return }
        try workbench.carryNames(names: names)
        finishCommand(affectsAnalysis: false)
    }

    /// Name and type an address as one undo step ("Define Variable").
    func defineVariable(address: UInt32, name: String, type: VarTypeInfo) throws {
        try workbench.defineVariable(snesAddress: address, name: name, ty: type)
        finishCommand(affectsAnalysis: false)
    }

    /// Remove a variable's name and type as one undo step.
    func removeVariable(address: UInt32) throws {
        try workbench.removeVariable(snesAddress: address)
        finishCommand(affectsAnalysis: false)
    }

    func setComment(address: UInt32, kind: CommentKind, text: String?) throws {
        try execute(.setComment(address: address, kind: kind, text: text))
    }

    /// `stride`, `bpp`, `elem` and `bank` are the data-kind parameters; the
    /// defaults are a two-byte raw table of same-bank entries, which is what
    /// the View menu's plain "Mark as Data" means.
    func mark(
        start: UInt32,
        len: UInt32,
        kind: OverrideKind,
        dataKind: DataKind? = nil,
        stride: UInt8? = nil,
        bpp: UInt8? = nil,
        elem: TableElem? = nil,
        bank: BankRule? = nil
    ) throws {
        try execute(.markRegion(
            start: start, len: len, kind: kind, dataKind: dataKind,
            stride: stride, bpp: bpp, elem: elem, bank: bank
        ))
    }

    /// How the marked range starting at `start` previews. Never re-runs the
    /// analysis.
    func setRegionParams(start: UInt32, params: RegionParamsInfo) throws {
        try execute(.setRegionParams(start: start, params: params))
    }

    func clearMark(start: UInt32, len: UInt32) throws {
        try execute(.clearRegionOverride(start: start, len: len))
    }

    func setFlagOverride(offset: UInt32, flags: FlagOverride?) throws {
        try execute(.setFlagOverride(offset: offset, flags: flags))
    }

    func execute(_ command: Command) throws {
        let affects = Self.affectsAnalysis(command)
        try workbench.execute(command: command)
        finishCommand(affectsAnalysis: affects)
    }

    /// Import a trace or a symbol file.
    ///
    /// Through the session rather than straight to the workbench, so an import
    /// schedules the re-analysis and marks the document dirty exactly as an
    /// edit does. An import that left the map showing the state before it
    /// would be worse than no import.
    func importTrace(source: String, bytes: Data) throws -> ImportResult {
        let result = try workbench.importTrace(source: source, bytes: bytes)
        finishCommand(affectsAnalysis: true)
        return result
    }

    /// Merge a live session's execution log; see `Workbench.mergeLiveLog`.
    /// Returns how many instructions are new to the project.
    @discardableResult
    func mergeLiveLog(_ bytes: Data) throws -> UInt64 {
        let added = try workbench.mergeLiveLog(source: "live session", bytes: bytes)
        liveLogMerged()
        return added
    }

    /// After `Workbench.mergeLiveLog` ran off the main actor: the bookkeeping
    /// a command does, and the re-analysis.
    func liveLogMerged() {
        finishCommand(affectsAnalysis: true)
    }

    func importSymbols(source: String, text: String) throws -> ImportResult {
        let result = try workbench.importSymbols(source: source, text: text)
        finishCommand(affectsAnalysis: true)
        return result
    }

    @discardableResult
    func undo() -> Bool {
        let affects = workbench.needsAnalysis()
        guard (try? workbench.undo()) == true else { return false }
        finishCommand(affectsAnalysis: affects || workbench.needsAnalysis())
        return true
    }

    @discardableResult
    func redo() -> Bool {
        guard (try? workbench.redo()) == true else { return false }
        finishCommand(affectsAnalysis: workbench.needsAnalysis())
        return true
    }

    static func affectsAnalysis(_ command: Command) -> Bool {
        switch command {
        case .setLabel, .setComment, .setRegionParams, .setVariable: false
        case .markRegion, .clearRegionOverride, .setFlagOverride: true
        }
    }

    private func finishCommand(affectsAnalysis: Bool) {
        refreshUndoState()
        generation += 1
        onChange?(.view)
        onCommand?(.project(dirty: workbench.isDirty()))
        if affectsAnalysis || workbench.needsAnalysis() {
            scheduleReanalysis()
        }
    }

    func refreshUndoState() {
        canUndo = workbench.canUndo()
        canRedo = workbench.canRedo()
        undoTitle = workbench.undoTitle()
        redoTitle = workbench.redoTitle()
        isDirty = workbench.isDirty()
    }

    // MARK: Events

    func handle(_ event: WorkbenchEvent) {
        switch event {
        case .snapshotChanged(let g):
            analysisGeneration = g
            hasSnapshot = true
            stats = workbench.stats()
            generation += 1
            onChange?(.snapshot)
        case .viewChanged(let g):
            viewGeneration = g
        case .projectChanged(let dirty):
            isDirty = dirty
        case .analysisProgress(let phase, let done, let total):
            // Progress travels on its own hop at a lower priority than the
            // finished analysis, so the run's last report ("building lines,
            // 1 of 1") can land after the run ended and set the bar going
            // again, where it stayed. Only a running analysis shows progress.
            guard analysis.isRunning else { return }
            let fraction = total == 0 ? 0 : Double(done) / Double(total)
            analysis = .running(fraction: fraction, phase: Self.phaseName(phase))
        }
    }

    static func phaseName(_ phase: AnalysisPhase) -> String {
        switch phase {
        case .descent: "walking code"
        case .tables: "resolving jump tables"
        case .sweep: "sweeping gaps"
        case .heuristics: "scoring the rest"
        case .labels: "naming"
        case .lines: "building lines"
        }
    }
}

/// Receives core events on any thread and forwards them to the session on
/// the main actor. Progress is coalesced: only the latest report is kept
/// while a hop is pending.
final class ListenerBridge: WorkbenchListener, @unchecked Sendable {
    private weak var session: WorkbenchSession?
    private let lock = NSLock()
    private var pendingProgress: WorkbenchEvent?
    private var hopScheduled = false

    init(session: WorkbenchSession) {
        self.session = session
    }

    /// The latest coalesced progress event, clearing the pending hop.
    private func takePendingProgress() -> WorkbenchEvent? {
        lock.withLock {
            let latest = pendingProgress
            pendingProgress = nil
            hopScheduled = false
            return latest
        }
    }

    func onEvent(event: WorkbenchEvent) {
        if case .analysisProgress = event {
            lock.lock()
            pendingProgress = event
            let schedule = !hopScheduled
            hopScheduled = true
            lock.unlock()
            guard schedule else { return }
            Task { @MainActor [weak self] in
                guard let self, let latest = self.takePendingProgress() else { return }
                self.session?.handle(latest)
            }
            return
        }
        Task { @MainActor [weak self] in
            self?.session?.handle(event)
        }
    }
}

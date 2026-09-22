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
            } catch is CancellationError {
                self?.analysis = .idle
            } catch {
                guard let self else { return }
                if case RomlensError.Cancelled = error {
                    self.analysis = .idle
                } else {
                    self.analysis = .failed(error.localizedDescription)
                }
            }
        }
    }

    func cancelAnalysis() {
        workbench.cancelAnalysis()
        analysisTask?.cancel()
        analysisTask = nil
        analysis = .idle
    }

    /// Re-run after a short pause; a running analysis is cancelled first.
    func scheduleReanalysis() {
        reanalysisTask?.cancel()
        let delay = reanalysisDelay
        reanalysisTask = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard let self, !Task.isCancelled else { return }
            if self.analysis.isRunning { self.cancelAnalysis() }
            self.startAnalysis()
        }
    }

    // MARK: Commands

    func setLabel(address: UInt32, name: String?) throws {
        try execute(.setLabel(address: address, name: name))
    }

    func removeLabel(address: UInt32) throws {
        try execute(.setLabel(address: address, name: nil))
    }

    func setComment(address: UInt32, kind: CommentKind, text: String?) throws {
        try execute(.setComment(address: address, kind: kind, text: text))
    }

    func mark(start: UInt32, len: UInt32, kind: OverrideKind, dataKind: DataKind? = nil) throws {
        try execute(.markRegion(start: start, len: len, kind: kind, dataKind: dataKind, stride: nil, bpp: nil))
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
        case .setLabel, .setComment: false
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

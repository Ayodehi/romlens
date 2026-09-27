import Foundation
import Observation
import RomlensKit

/// The sound views' shared state (docs/23): what they read and what is
/// selected in them.
///
/// They read either the attached recording at the Graphics views' frame
/// (one frame for both, so the screen and the sound are the same moment),
/// or the ROM: the driver its upload routine sends, booted on Romlens's own
/// SPC700 with the other uploads laid over it, as it stands after starting.
@MainActor
@Observable
final class AudioModel {
    enum Tab: String, CaseIterable, Identifiable {
        case voices, samples, aram
        var id: String { rawValue }
        var title: String {
            switch self {
            case .voices: "Voices"
            case .samples: "Samples"
            case .aram: "Audio RAM"
            }
        }
        var systemImage: String {
            switch self {
            case .voices: "slider.vertical.3"
            case .samples: "waveform"
            case .aram: "memorychip"
            }
        }
    }

    enum Source: Equatable {
        /// The attached recording at the frame.
        case recording
        /// The ROM's upload, run by Romlens.
        case rom
    }

    /// Everything the views show about one moment, read once.
    struct State {
        var voices: [VoiceInfo]
        var registers: [DspRegisterInfo]
        var map: [AramRegionInfo]
        var samples: [SampleInfo]
        var pc: UInt16
    }

    let rom: Rom
    let workbench: Workbench
    let graphics: GraphicsModel
    var source: Source = .rom

    // Selection
    var selectedVoice: Int? = 0
    var selectedSample: UInt8?
    var selectedBlock: Int?
    /// The audio RAM part selected, by its start.
    var selectedPart: UInt16?

    /// Where "Show in ROM" sends a file offset.
    @ObservationIgnored var showInRom: ((UInt32) -> Void)?

    // The ROM's upload
    /// What the analysis traced, once asked for.
    private(set) var upload: UploadReportInfo?
    private(set) var uploadLoading = false
    /// The block lists laid over the driver, by SNES address.
    var includedLists: Set<UInt32> = [] {
        didSet { if includedLists != oldValue { rebuildRomMachine() } }
    }
    /// Why the ROM's machine could not be built.
    private(set) var romProblem: String?
    /// The ROM's machine: its state is what the views show, and in A12 it
    /// is what plays.
    private(set) var romPlayer: ApuPlayer?
    /// Bumped whenever the ROM's machine changes, so the state is read again.
    private(set) var machineGeneration = 0
    @ObservationIgnored private var uploadFor: UInt64?
    @ObservationIgnored private var uploadTask: Task<Void, Never>?

    @ObservationIgnored private var cache: (key: Key, state: State)?
    private struct Key: Equatable {
        let source: Source
        let frame: UInt64
        let machine: Int
        let recording: ObjectIdentifier?
    }

    init(rom: Rom, workbench: Workbench, graphics: GraphicsModel) {
        self.rom = rom
        self.workbench = workbench
        self.graphics = graphics
    }

    // MARK: Sources

    /// The attached recording, when it has the sound side.
    var recording: RecordingSession? {
        guard let r = graphics.recording, r.hasSound() else { return nil }
        return r
    }

    var hasRecordingSound: Bool { recording != nil }

    /// Graphics or sound, one frame.
    var frame: UInt64 {
        get { graphics.frame }
        set { graphics.frame = newValue }
    }

    /// The source was picked by hand, so opening a view keeps it.
    @ObservationIgnored private var sourcePicked = false

    /// A view opens: the recording's sound if it has any and nothing was
    /// picked, else what was; the ROM when the recording has no sound.
    func opened() {
        if !sourcePicked, hasRecordingSound { source = .recording }
        if source == .recording, !hasRecordingSound { source = .rom }
        if source == .rom { loadUpload() }
    }

    /// The source bar's picker.
    func pick(_ s: Source) {
        sourcePicked = true
        source = s
        opened()
    }

    var sourceDescription: String {
        switch source {
        case .recording:
            return "\(graphics.recordingName ?? "Recording") · frame \(frame)"
        case .rom:
            if uploadLoading { return "tracing the upload…" }
            if let romProblem { return romProblem }
            guard let d = driverUpload else { return "the ROM" }
            let lists = [d.list] + (upload?.uploads.filter { includedLists.contains($0.list) }.map(\.list) ?? [])
            return "the driver from \(lists.map { formatSnesAddress(address: $0) }.joined(separator: " + ")), run by Romlens"
        }
    }

    // MARK: The ROM's upload

    var driverUpload: UploadInfo? { upload?.uploads.first(where: \.driver) }

    /// The uploads a running driver is sent: songs and samples.
    var otherUploads: [UploadInfo] { upload?.uploads.filter { !$0.driver } ?? [] }

    /// Trace the upload in the current analysis, once per analysis.
    func loadUpload() {
        let generation = workbench.analysisGeneration()
        guard uploadFor != generation, !uploadLoading else { return }
        uploadLoading = true
        let workbench = workbench
        uploadTask = Task { [weak self] in
            let report = await workbench.soundUpload()
            guard let self else { return }
            self.uploadFor = generation
            self.uploadLoading = false
            self.upload = report
            self.includedLists = Self.defaultLists(report)
            self.rebuildRomMachine()
        }
    }

    /// Every upload besides the driver's whose audio RAM does not overlap
    /// one before it: the samples and the first song bank, not a second
    /// bank meant to replace the first.
    static func defaultLists(_ report: UploadReportInfo) -> Set<UInt32> {
        var taken: [Range<UInt32>] = []
        var out: Set<UInt32> = []
        for u in report.uploads {
            let ranges = u.blocks.map { UInt32($0.aram)..<UInt32($0.aram) + UInt32($0.len) }
            if !u.driver, ranges.contains(where: { r in taken.contains { $0.overlaps(r) } }) { continue }
            taken += ranges
            if !u.driver { out.insert(u.list) }
        }
        return out
    }

    func rebuildRomMachine() {
        guard driverUpload != nil else {
            romPlayer = nil
            romProblem = upload == nil ? nil : "no upload of a sound driver was traced in this ROM"
            machineGeneration += 1
            return
        }
        let lists = otherUploads.map(\.list).filter { includedLists.contains($0) }
        do {
            romPlayer = try ApuPlayer.fromUpload(workbench: workbench, with: lists, settleSeconds: 1.0)
            romProblem = nil
        } catch {
            romPlayer = nil
            romProblem = "\(error)"
        }
        machineGeneration += 1
    }

    /// Something changed the ROM's machine (it played, or a port was
    /// written): read its state again.
    func machineChanged() {
        machineGeneration += 1
    }

    // MARK: Reading

    /// The moment the views show, read once per frame or machine change.
    var state: State? {
        let key = Key(
            source: source,
            frame: source == .recording ? frame : 0,
            machine: source == .rom ? machineGeneration : 0,
            recording: source == .recording ? graphics.recording.map(ObjectIdentifier.init) : nil
        )
        if let cache, cache.key == key { return cache.state }
        let s: State?
        switch source {
        case .recording:
            guard let r = recording else { return nil }
            s = try? State(
                voices: r.voices(frame: frame),
                registers: r.dspRegisters(frame: frame),
                map: r.aramMap(frame: frame),
                samples: r.samples(frame: frame),
                pc: r.spcPc(frame: frame)
            )
        case .rom:
            guard let p = romPlayer else { return nil }
            s = State(voices: p.voices(), registers: p.dspRegisters(), map: p.aramMap(), samples: p.samples(), pc: p.spcPc())
        }
        if let s { cache = (key, s) }
        return s
    }

    func sample(_ index: UInt8, maxBlocks: UInt32 = 4096) -> BrrSampleInfo? {
        switch source {
        case .recording: try? recording?.sample(frame: frame, index: index, maxBlocks: maxBlocks)
        case .rom: romPlayer?.sample(index: index, maxBlocks: maxBlocks)
        }
    }

    func listing(from: UInt16, count: UInt32) -> [SpcLineInfo] {
        switch source {
        case .recording: (try? recording?.spcListing(frame: frame, from: from, count: count)) ?? []
        case .rom: romPlayer?.spcListing(from: from, count: count) ?? []
        }
    }

    func aram(start: UInt16, len: UInt32) -> Data {
        switch source {
        case .recording: (try? recording?.aram(frame: frame, start: start, len: len)) ?? Data()
        case .rom: romPlayer?.aram(start: start, len: len) ?? Data()
        }
    }

    /// Where audio RAM byte `address` came from in the ROM.
    func romOrigin(_ address: UInt16) -> UInt32? {
        source == .rom ? romPlayer?.romOrigin(address: address) : nil
    }

    /// The part holding `address`.
    func part(containing address: UInt16) -> AramRegionInfo? {
        state?.map.first { UInt32(address) >= UInt32($0.start) && UInt32(address) < UInt32($0.start) + $0.len }
    }

    /// The upload blocks that filled a part, with their ROM ranges.
    func blocks(filling part: AramRegionInfo) -> [(upload: UploadInfo, block: UploadBlockInfo)] {
        guard source == .rom, let upload else { return [] }
        let lists = Set([driverUpload?.list].compactMap { $0 }).union(includedLists)
        let range = UInt32(part.start)..<UInt32(part.start) + part.len
        var out: [(UploadInfo, UploadBlockInfo)] = []
        for u in upload.uploads where lists.contains(u.list) {
            for b in u.blocks where (UInt32(b.aram)..<UInt32(b.aram) + UInt32(b.len)).overlaps(range) {
                out.append((u, b))
            }
        }
        return out
    }
}

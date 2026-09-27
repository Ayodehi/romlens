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
        case voices, timeline, samples, aram, ports, echo, scope
        var id: String { rawValue }
        var title: String {
            switch self {
            case .voices: "Voices"
            case .timeline: "Timeline"
            case .samples: "Samples"
            case .aram: "Audio RAM"
            case .ports: "Ports"
            case .echo: "Echo & Effects"
            case .scope: "Scope"
            }
        }
        var systemImage: String {
            switch self {
            case .voices: "slider.vertical.3"
            case .timeline: "pianokeys"
            case .samples: "waveform"
            case .aram: "memorychip"
            case .ports: "arrow.left.arrow.right"
            case .echo: "dot.radiowaves.right"
            case .scope: "waveform.path.ecg"
            }
        }
    }

    /// What is heard.
    enum Playing: Equatable {
        /// The machine: the ROM's driver, or the recording from a frame.
        case machine
        /// One sample alone on voice 0, played from the keyboard.
        case sample(UInt8)
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
    /// Switch the editor to another sound view.
    @ObservationIgnored var openTab: ((Tab) -> Void)?

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

    // Playback (A12)
    @ObservationIgnored let output = ApuAudio()
    /// What is playing, if anything.
    private(set) var playing: Playing?
    var isPlaying: Bool { playing == .machine }
    /// The player heard: the ROM's machine, a recording's from a frame, or
    /// a sample's.
    private(set) var livePlayer: ApuPlayer?
    /// Voices left out of the mix, bit n for voice n.
    var muted: UInt8 = 0 {
        didSet { livePlayer?.setMuted(mask: muted) }
    }
    /// Playing a recording: its port writes arrive as they did, and the
    /// frame moves with the sound.
    var followRecording = true {
        didSet { if followRecording != oldValue, isPlaying, source == .recording { restartRecording() } }
    }
    /// Seconds heard since play.
    private(set) var playedSeconds = 0.0
    /// The last port writes sent by hand, newest last.
    private(set) var sent: [(port: UInt8, value: UInt8)] = []
    @ObservationIgnored private var playFrom: UInt64 = 0
    @ObservationIgnored private var frameSet: UInt64?
    @ObservationIgnored private var ticker: Task<Void, Never>?
    /// A recording frame lasts 1 / 60.0988 s (NTSC).
    static let framesPerSecond = 60.0988

    // Timeline (A13)
    /// The notes: the whole recording's, or those the ROM's machine has
    /// played since it was built.
    private(set) var notes: [NoteEventInfo] = []
    private(set) var notesLoading = false
    var selectedNote: NoteEventInfo?
    /// Where the selected note came from, once worked out.
    private(set) var noteSource: NoteSourceInfo?
    /// Frames the timeline shows across.
    var timelineSpan: UInt64 = 600
    /// An address in audio RAM to show in its listing.
    var listingTarget: UInt16?
    @ObservationIgnored private var notesFor: ObjectIdentifier?
    @ObservationIgnored private var notesTask: Task<Void, Never>?

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
        if s != source {
            if playing != nil { output.play(nil) }
            playing = nil
            livePlayer = nil
            samplePlayer = nil
            notes = []
            notesFor = nil
            selectedNote = nil
            noteSource = nil
        }
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
        if source == .rom, playing != nil {
            output.play(nil)
            playing = nil
            livePlayer = nil
        }
        samplePlayer = nil
        if source == .rom {
            notes = []
            selectedNote = nil
        }
        guard driverUpload != nil else {
            romPlayer = nil
            romProblem = upload == nil ? nil : "no upload of a sound driver was traced in this ROM"
            machineGeneration += 1
            return
        }
        let lists = otherUploads.map(\.list).filter { includedLists.contains($0) }
        do {
            romPlayer = try ApuPlayer.fromUpload(workbench: workbench, with: lists, settleSeconds: 1.0)
            romPlayer?.logNotes()
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

    // MARK: Playing

    func togglePlay() {
        if isPlaying { pause() } else { play() }
    }

    /// Play the source from where it is: the ROM's machine carries on;
    /// a recording starts from the frame.
    func play() {
        switch source {
        case .rom:
            guard let p = romPlayer else { return }
            start(p)
        case .recording:
            restartRecording()
        }
    }

    private func restartRecording() {
        guard let r = recording, let p = try? ApuPlayer.fromRecording(recording: r, frame: frame, follow: followRecording) else { return }
        playFrom = frame
        frameSet = frame
        start(p)
    }

    private func start(_ p: ApuPlayer) {
        p.setMuted(mask: muted)
        livePlayer = p
        playing = .machine
        playedSeconds = 0
        output.play(p)
        startTicking()
    }

    func pause() {
        output.play(nil)
        playing = nil
        ticker?.cancel()
        ticker = nil
        machineChanged()
    }

    /// Mute voice `v`, or hear it again.
    func toggleMute(_ v: Int) {
        muted ^= 1 << UInt8(v)
    }

    /// Hear only voice `v`, or every voice if it already is.
    func toggleSolo(_ v: Int) {
        let only: UInt8 = ~(1 << UInt8(v))
        muted = muted == only ? 0 : only
    }

    func isSolo(_ v: Int) -> Bool { muted == ~(1 << UInt8(v)) }

    /// The S-CPU writes a port, as the game would to ask for a sound:
    /// playing the ROM's machine (started if it was not), or the recording.
    func send(port: UInt8, value: UInt8) {
        guard let p = isPlaying ? livePlayer : (source == .rom ? romPlayer : livePlayer) else { return }
        // Written before playing starts: starting renders ahead at once.
        p.sendPort(port: port & 3, value: value)
        if source == .rom, !isPlaying { play() }
        sent.append((port & 3, value))
        if sent.count > 16 { sent.removeFirst() }
        machineChanged()
    }

    /// What the S-CPU would read back from a port now.
    func readPort(_ port: UInt8) -> UInt8? {
        (livePlayer ?? romPlayer)?.readPort(port: port)
    }

    /// The values the game's code sends on each port, from the trace.
    func commandValues(port: UInt8) -> [UInt32] {
        Array(Set(upload?.commands.filter { $0.port == port }.map(\.value) ?? [])).sorted()
    }

    /// Play This Command: the ROM's driver, sent `value` on `port`.
    func playCommand(port: UInt8, value: UInt8) {
        pick(.rom)
        if romPlayer == nil { return }
        send(port: port, value: value)
    }

    // MARK: A sample on the keyboard

    @ObservationIgnored private var samplePlayer: (index: UInt8, player: ApuPlayer)?

    /// Sound directory entry `index` at `pitch` until `release`.
    func press(sample index: UInt8, pitch: UInt16) {
        if samplePlayer?.index != index {
            samplePlayer = makeSamplePlayer(index).map { (index, $0) }
        }
        guard let p = samplePlayer?.player else { return }
        // Keyed first: starting the output renders ahead at once.
        p.setPitch(voice: 0, pitch: pitch)
        p.keyOn(mask: 1)
        if playing != .sample(index) {
            if isPlaying { pause() }
            livePlayer = p
            playing = .sample(index)
            output.play(p)
        }
    }

    func release() {
        samplePlayer?.player.keyOff(mask: 1)
    }

    /// Stop the sample, leaving the machine paused.
    func stopSample() {
        guard case .sample = playing else { return }
        output.play(nil)
        playing = nil
        livePlayer = nil
    }

    private func makeSamplePlayer(_ index: UInt8) -> ApuPlayer? {
        switch source {
        case .recording:
            guard let r = recording else { return nil }
            return try? ApuPlayer.fromRecordedSample(recording: r, frame: frame, index: index, pitch: 0x1000)
        case .rom:
            guard let entry = state?.samples.first(where: { $0.index == index }), let rom = entry.romOffset else { return nil }
            return ApuPlayer.fromRomSample(rom: self.rom, offset: rom, loopOffset: UInt32(entry.loopAt &- entry.start), pitch: 0x1000)
        }
    }

    /// The pitch that plays directory entry `index` at `semitones` from
    /// A4, from its tuning; `$1000` shifted by semitones from its own rate
    /// when it has none.
    func pitch(sample index: UInt8, semitones: Int) -> UInt16 {
        let hz = 440 * pow(2, Double(semitones) / 12)
        let base = state?.samples.first(where: { $0.index == index })?.tuningHz ?? 440
        return UInt16(min(max(4096 * hz / base, 1), 0x3FFF))
    }

    // MARK: The clock

    /// While playing, a few times a second: read the machine again, and
    /// move the frame along with a recording.
    private func startTicking() {
        ticker?.cancel()
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(66))
                self?.tick()
            }
        }
    }

    func tick() {
        guard isPlaying, let p = livePlayer else { return }
        playedSeconds = p.seconds()
        if source == .recording {
            // Scrubbed by hand while playing: play from there.
            if let set = frameSet, frame != set {
                restartRecording()
                return
            }
            let at = playFrom + UInt64(playedSeconds * Self.framesPerSecond)
            if at >= graphics.frameCount {
                pause()
                return
            }
            if followRecording {
                frameSet = at
                frame = at
            }
        } else {
            machineChanged()
            refreshRomNotes()
        }
    }

    /// The last `count` samples of voice 0–7, or 8 and 9 for left and
    /// right, from what is playing.
    func scope(_ which: UInt8, count: UInt32) -> [Int16] {
        livePlayer?.scope(which: which, count: count) ?? []
    }

    // MARK: Notes

    /// The notes for the timeline: the recording's read once, off the main
    /// thread; the ROM's machine's as it plays.
    func loadNotes() {
        switch source {
        case .rom:
            refreshRomNotes()
        case .recording:
            guard let r = recording else { return }
            let id = ObjectIdentifier(r)
            guard notesFor != id, !notesLoading else { return }
            notesLoading = true
            let last = max(graphics.frameCount, 1) - 1
            notesTask = Task { [weak self] in
                let notes = await Task.detached { (try? r.noteTimeline(from: 0, to: last)) ?? [] }.value
                guard let self else { return }
                self.notes = notes
                self.notesFor = id
                self.notesLoading = false
            }
        }
    }

    private func refreshRomNotes() {
        guard source == .rom, let p = romPlayer else { return }
        let have = notes.count
        let count = Int(p.noteCount())
        if count < have { notes = [] }
        if count != notes.count { notes += p.notes(since: UInt32(notes.count)) }
        notesFor = nil
    }

    /// The frame the timeline centres on: the recording's, or the ROM
    /// machine's time in frames.
    var timelineNow: UInt64 {
        switch source {
        case .recording: frame
        case .rom: UInt64((romPlayer?.seconds() ?? 0) * Self.framesPerSecond)
        }
    }

    /// Select a note and work out where it came from.
    func select(note: NoteEventInfo?) {
        selectedNote = note
        noteSource = nil
        guard let note else { return }
        if let pc = note.spcPc {
            noteSource = NoteSourceInfo(spcPc: pc, commands: [])
        } else if source == .recording, let r = recording {
            noteSource = try? r.noteSource(frame: note.frame, spcCycle: note.spcCycle)
        }
    }

    /// The code the game sends a port value from, per the trace.
    func commandSites(port: UInt8, value: UInt8) -> [SoundCommandInfo] {
        upload?.commands.filter { $0.port == port && $0.value == UInt32(value) } ?? []
    }

    /// Show the listing at an SPC700 address.
    func showSpc(_ address: UInt16) {
        selectedPart = part(containing: address)?.start
        listingTarget = address
    }

    /// The uploads the recording saw, up to the frame.
    func sentUploads() -> [SentUploadInfo] {
        guard source == .recording, let r = recording else { return [] }
        return (try? r.sentUploads(from: 0, to: frame)) ?? []
    }

    /// The ports both ways around the frame.
    func portEvents(around: UInt64, frames: UInt64 = 30) -> [PortEventInfo] {
        guard source == .recording, let r = recording else { return [] }
        return (try? r.portEvents(from: around > frames ? around - frames : 0, to: around + frames)) ?? []
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

import RomlensKit
import SwiftUI

/// A piano roll per voice (docs/23, A13): each note from its key-on to its
/// key-off, placed up and down its lane by pitch and coloured by sample,
/// around the frame. A note opens where it came from: the SPC700
/// instruction that wrote it, and the S-CPU's requests before it with the
/// 65816 code that sends them.
struct NoteTimelineView: View {
    @Bindable var audio: AudioModel

    var body: some View {
        HSplitView {
            roll
                .frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
            SongPane(audio: audio)
                .frame(minWidth: 280, idealWidth: 360, maxWidth: 520, maxHeight: .infinity)
        }
        .onAppear { audio.loadNotes() }
        .onChange(of: audio.source) { _, _ in audio.loadNotes() }
    }

    private var roll: some View {
        VSplitView {
            VStack(spacing: 0) {
                HStack {
                    Text(summary).font(.callout).foregroundStyle(.secondary)
                    Spacer()
                    Text("Frames across").font(.caption).foregroundStyle(.secondary)
                    Picker("Frames across", selection: $audio.timelineSpan) {
                        ForEach([120, 300, 600, 1800, 3600] as [UInt64], id: \.self) { Text("\($0)").tag($0) }
                    }
                    .labelsHidden()
                    .fixedSize()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                PianoRoll(audio: audio)
                    .frame(minHeight: 8 * 20, idealHeight: 8 * 44)
            }
            NoteDetail(audio: audio)
                .frame(minHeight: 120, idealHeight: 180)
        }
    }

    private var summary: String {
        if audio.notesLoading { return "Reading the notes…" }
        let ons = audio.notes.filter { $0.kind == .on }.count
        switch audio.source {
        case .recording: return "\(ons) notes keyed on in the recording, from its DSP writes"
        case .rom: return "\(ons) notes the ROM's driver has played in Romlens since it started" + (ons == 0 ? ": send it a command on the ports" : "")
        }
    }
}

/// A note from its key-on to where it ends.
struct NoteSpan: Identifiable {
    let on: NoteEventInfo
    let end: UInt64
    var id: String { "\(on.voice)-\(on.spcCycle)" }
}

struct PianoRoll: View {
    @Bindable var audio: AudioModel

    var body: some View {
        let now = audio.timelineNow
        let span = audio.timelineSpan
        let from = now > span / 2 ? now - span / 2 : 0
        let spans = Self.spans(audio.notes, until: max(now, from + span))
            .filter { $0.end >= from && $0.on.frame <= from + span }
        GeometryReader { g in
            let w = g.size.width
            let lane = g.size.height / 8
            let x = { (f: UInt64) in CGFloat(Double(f) - Double(from)) / CGFloat(span) * w }
            ZStack(alignment: .topLeading) {
                Canvas { ctx, size in
                    for v in 0..<8 {
                        let y = CGFloat(v) * lane
                        if v % 2 == 1 {
                            ctx.fill(Path(CGRect(x: 0, y: y, width: size.width, height: lane)), with: .color(.secondary.opacity(0.05)))
                        }
                    }
                    for s in spans {
                        let y0 = CGFloat(s.on.voice) * lane
                        let y = y0 + Self.pitchY(s.on.pitch) * (lane - 6)
                        let rect = CGRect(x: x(s.on.frame), y: y, width: max(2, x(s.end) - x(s.on.frame)), height: 5)
                        let selected = audio.selectedNote?.spcCycle == s.on.spcCycle && audio.selectedNote?.voice == s.on.voice
                        ctx.fill(Path(roundedRect: rect, cornerRadius: 2), with: .color(selected ? .white : Self.colour(s.on.source)))
                    }
                    let nx = x(now)
                    ctx.stroke(Path { $0.move(to: CGPoint(x: nx, y: 0)); $0.addLine(to: CGPoint(x: nx, y: size.height)) },
                               with: .color(.accentColor), lineWidth: 1)
                }
                ForEach(0..<8, id: \.self) { v in
                    Text("\(v)").font(.caption2.monospaced()).foregroundStyle(.secondary)
                        .offset(x: 3, y: CGFloat(v) * lane + 2)
                }
            }
            .contentShape(Rectangle())
            .onTapGesture { at in
                let voice = Int(at.y / lane)
                let f = Double(from) + Double(at.x / w) * Double(span)
                let hit = spans.filter { Int($0.on.voice) == voice && Double($0.on.frame) <= f + 2 && Double($0.end) >= f - 2 }
                    .min { abs(Double($0.on.frame) - f) < abs(Double($1.on.frame) - f) }
                audio.select(note: hit?.on)
            }
        }
        .clipped()
        .background(Color.black.opacity(0.25))
        .help("Each voice's notes: up and down by pitch, coloured by sample; click one to see where it came from")
    }

    /// Up and down a lane by pitch, on a log scale: `$0100` at the bottom,
    /// `$3FFF` at the top.
    static func pitchY(_ pitch: UInt16) -> CGFloat {
        let l = log2(Double(max(pitch, 0x100)))
        return CGFloat(1 - (l - 8) / 6)
    }

    static func colour(_ source: UInt8) -> Color {
        Color(hue: Double((Int(source) * 37) % 360) / 360, saturation: 0.6, brightness: 0.9)
    }

    /// Each key-on to its key-off, or to the voice's next key-on.
    static func spans(_ notes: [NoteEventInfo], until: UInt64) -> [NoteSpan] {
        var open: [UInt8: NoteEventInfo] = [:]
        var out: [NoteSpan] = []
        for n in notes {
            switch n.kind {
            case .on:
                if let o = open[n.voice] { out.append(NoteSpan(on: o, end: n.frame)) }
                open[n.voice] = n
            case .off:
                if let o = open.removeValue(forKey: n.voice) { out.append(NoteSpan(on: o, end: n.frame)) }
            case .pitch:
                break
            }
        }
        out += open.values.map { NoteSpan(on: $0, end: until) }
        return out
    }
}

/// The note selected: what it is, and the code on both CPUs behind it.
struct NoteDetail: View {
    @Bindable var audio: AudioModel

    var body: some View {
        if let n = audio.selectedNote {
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text("Voice \(n.voice): keyed on at frame \(n.frame)").font(.headline)
                        Spacer()
                        if audio.source == .recording {
                            Button("Go to Frame") { audio.frame = n.frame }
                        }
                    }
                    Text("pitch \(AudioStyle.hex(n.pitch, 4)), sample \(n.source)\(noteName(n).map { ", ≈ \($0)" } ?? "")")
                        .font(.callout)
                    if let pc = audio.noteSource?.spcPc {
                        HStack {
                            Text("Written by the SPC700 at \(AudioStyle.hex(pc, 4)): its KON write to the DSP.")
                                .font(.callout)
                            Button("Show in Audio RAM") {
                                audio.showSpc(pc)
                                audio.openTab?(.aram)
                            }
                            .buttonStyle(.link)
                        }
                    } else if audio.source == .recording, audio.noteSource != nil {
                        Text("The replay could not place the instruction that wrote it.").font(.caption).foregroundStyle(.secondary)
                    }
                    if let cmds = audio.noteSource?.commands, !cmds.isEmpty {
                        Text("What the S-CPU last asked on each port before it:").font(.callout)
                        ForEach(Array(cmds.enumerated()), id: \.offset) { _, c in
                            HStack(spacing: 8) {
                                Text("port \(c.port) = \(AudioStyle.hex(c.value, 2)) at frame \(c.frame)")
                                    .font(.callout.monospaced())
                                ForEach(Array(audio.commandSites(port: c.port, value: c.value).prefix(3).enumerated()), id: \.offset) { _, site in
                                    Button("sent at \(formatSnesAddress(address: site.routine))\(site.via.map { " via \(formatSnesAddress(address: $0))" } ?? "")") {
                                        audio.showInRom?(site.at)
                                    }
                                    .buttonStyle(.link)
                                    .font(.caption)
                                    .help("The 65816 code that stores this value, found by tracing it")
                                }
                            }
                        }
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        } else {
            Text("Click a note to see the DSP write that started it, the SPC700 instruction that made the write, and the S-CPU's request before it.")
                .font(.callout).foregroundStyle(.secondary)
                .padding(12)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    private func noteName(_ n: NoteEventInfo) -> String? {
        guard let hz = audio.state?.samples.first(where: { $0.index == n.source })?.tuningHz else { return nil }
        return noteForFrequency(hz: hz * Double(n.pitch) / 4096)
    }
}

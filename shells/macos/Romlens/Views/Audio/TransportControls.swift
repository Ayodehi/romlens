import RomlensKit
import SwiftUI

/// Play and pause, following the recording, the time heard, and the port
/// console (docs/23, A12).
struct TransportControls: View {
    @Bindable var audio: AudioModel
    @State private var showPorts = false

    var body: some View {
        HStack(spacing: 6) {
            Button {
                audio.togglePlay()
            } label: {
                Image(systemName: audio.isPlaying ? "pause.fill" : "play.fill")
            }
            .keyboardShortcut(.space, modifiers: [])
            .disabled(audio.state == nil)
            .help(audio.isPlaying ? "Pause (Space)" : playHelp)
            if audio.isPlaying {
                Text(String(format: "%.1f s", audio.playedSeconds))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(width: 44, alignment: .trailing)
            }
            if audio.source == .recording {
                Toggle(isOn: $audio.followRecording) {
                    Image(systemName: "arrow.forward.to.line")
                }
                .toggleStyle(.button)
                .help(audio.followRecording
                    ? "Following the recording: its port writes arrive as they did, and the frame moves with the sound"
                    : "The driver on its own from this frame, as if the game sent nothing more")
            }
            Button {
                showPorts.toggle()
            } label: {
                Label("Ports", systemImage: "arrow.left.arrow.right")
            }
            .help("Write the four ports as the S-CPU does: how a game asks its driver for a song or a sound effect")
            .popover(isPresented: $showPorts, arrowEdge: .bottom) {
                PortConsole(audio: audio)
            }
            if let problem = audio.output.problem {
                Image(systemName: "speaker.slash").foregroundStyle(.orange).help(problem)
            }
        }
    }

    private var playHelp: String {
        switch audio.source {
        case .rom: "Play the ROM's driver, run by Romlens (Space)"
        case .recording: "Play from this frame, run by Romlens from the recording's snapshot (Space)"
        }
    }
}

/// The four ports, the S-CPU's side: a byte to send on each, with the
/// values the game's own code sends offered, and what the driver answers.
struct PortConsole: View {
    @Bindable var audio: AudioModel
    @State private var values: [String] = ["00", "00", "00", "00"]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Ports $2140–$2143").font(.headline)
            Text("The S-CPU and the SPC700 share only these four bytes each way. A game asks its driver for a song or a sound by writing a number to a port; the driver reads it and answers on the same port.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(width: 360, alignment: .leading)
            Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 6) {
                ForEach(0..<4, id: \.self) { p in
                    GridRow {
                        Text("Port \(p)").font(.callout)
                        TextField("hex", text: $values[p])
                            .font(.callout.monospaced())
                            .frame(width: 44)
                            .onSubmit { send(p) }
                        Button("Send") { send(p) }
                        let offered = audio.commandValues(port: UInt8(p))
                        if !offered.isEmpty {
                            Menu("\(offered.count) from the code") {
                                ForEach(offered, id: \.self) { v in
                                    Button("$\(GraphicsStyle.hex(v, 2))") {
                                        values[p] = GraphicsStyle.hex(v, 2)
                                        send(p)
                                    }
                                }
                            }
                            .fixedSize()
                            .help("The values the game's code sends on this port, found by tracing it")
                        }
                        Text(audio.readPort(UInt8(p)).map { "reads back $\(GraphicsStyle.hex($0, 2))" } ?? "")
                            .font(.caption.monospaced()).foregroundStyle(.secondary)
                    }
                }
            }
            if !audio.sent.isEmpty {
                Text("Sent: " + audio.sent.suffix(8).map { "\($0.port)=$\(GraphicsStyle.hex($0.value, 2))" }.joined(separator: " "))
                    .font(.caption.monospaced()).foregroundStyle(.secondary)
            }
        }
        .padding(14)
    }

    private func send(_ p: Int) {
        let text = values[p].trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "$", with: "")
        guard let v = UInt8(text, radix: 16) else { return }
        audio.send(port: UInt8(p), value: v)
    }
}

/// Each voice's wave and the two outputs, while something plays.
struct ScopeView: View {
    let audio: AudioModel

    var body: some View {
        if audio.playing == nil {
            ContentUnavailableView {
                Label("Nothing Playing", systemImage: "waveform.path.ecg")
            } description: {
                Text("Press Play (Space) to hear the driver: each voice's wave after its envelope, and the mix left and right, are drawn here as they play.")
            }
        } else {
            TimelineView(.animation(minimumInterval: 1.0 / 30)) { _ in
                ScrollView {
                    VStack(spacing: 8) {
                        HStack(spacing: 8) {
                            ScopeTrace(title: "Left", samples: audio.scope(8, count: 1024))
                            ScopeTrace(title: "Right", samples: audio.scope(9, count: 1024))
                        }
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 260), spacing: 8)], spacing: 8) {
                            ForEach(0..<8, id: \.self) { v in
                                ScopeTrace(
                                    title: "Voice \(v)" + (audio.muted & (1 << UInt8(v)) != 0 ? " (muted)" : ""),
                                    samples: audio.scope(UInt8(v), count: 512)
                                )
                            }
                        }
                    }
                    .padding(12)
                }
            }
        }
    }
}

struct ScopeTrace: View {
    let title: String
    let samples: [Int16]

    var body: some View {
        Canvas { ctx, size in
            let mid = size.height / 2
            ctx.stroke(Path { $0.move(to: CGPoint(x: 0, y: mid)); $0.addLine(to: CGPoint(x: size.width, y: mid)) },
                       with: .color(.secondary.opacity(0.3)), lineWidth: 0.5)
            guard samples.count > 1 else { return }
            var p = Path()
            let n = CGFloat(samples.count - 1)
            for (i, v) in samples.enumerated() {
                let pt = CGPoint(x: CGFloat(i) / n * size.width, y: mid - CGFloat(v) / 32768 * mid)
                if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
            }
            ctx.stroke(p, with: .color(.green), lineWidth: 1)
        }
        .frame(height: 90)
        .background(Color.black.opacity(0.85))
        .clipShape(RoundedRectangle(cornerRadius: 4))
        .overlay(alignment: .topLeading) {
            Text(title).font(.caption2).foregroundStyle(.white.opacity(0.7)).padding(4)
        }
    }
}

/// Two octaves of keys, C4 to B5, for playing one sample by hand, laid
/// out as a piano's: the white keys side by side, each black key shorter,
/// on top and across the gap between its two white keys.
struct SampleKeyboard: View {
    let audio: AudioModel
    let sample: UInt8
    @State private var down: Int?

    static let whiteWidth: CGFloat = 24
    static let whiteHeight: CGFloat = 76
    static let blackWidth: CGFloat = 15
    static let blackHeight: CGFloat = 46
    static let gap: CGFloat = 1

    /// Semitones from A4 of each white key, and of each black key with the
    /// white key it follows.
    private static let whites: [Int] = (0..<2).flatMap { o in [0, 2, 4, 5, 7, 9, 11].map { o * 12 + $0 - 9 } }
    private static let blacks: [(after: Int, semitone: Int)] = (0..<2).flatMap { o in
        [(0, 1), (1, 3), (3, 6), (4, 8), (5, 10)].map { (o * 7 + $0.0, o * 12 + $0.1 - 9) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ZStack(alignment: .topLeading) {
                HStack(spacing: Self.gap) {
                    ForEach(Self.whites, id: \.self) { k in
                        key(k, black: false)
                            .frame(width: Self.whiteWidth, height: Self.whiteHeight)
                    }
                }
                ForEach(Self.blacks, id: \.semitone) { b in
                    key(b.semitone, black: true)
                        .frame(width: Self.blackWidth, height: Self.blackHeight)
                        .offset(x: CGFloat(b.after + 1) * (Self.whiteWidth + Self.gap) - Self.gap / 2 - Self.blackWidth / 2)
                }
            }
            .frame(
                width: CGFloat(Self.whites.count) * (Self.whiteWidth + Self.gap) - Self.gap,
                height: Self.whiteHeight,
                alignment: .topLeading
            )
            Text(caption).font(.caption).foregroundStyle(.secondary)
        }
    }

    private func key(_ k: Int, black: Bool) -> some View {
        UnevenRoundedRectangle(bottomLeadingRadius: 3, bottomTrailingRadius: 3)
            .fill(down == k ? Color.accentColor : black ? Color.black : Color.white)
            .overlay(
                UnevenRoundedRectangle(bottomLeadingRadius: 3, bottomTrailingRadius: 3)
                    .stroke(Color.secondary.opacity(black ? 0.8 : 0.5), lineWidth: 0.5)
            )
            .overlay(alignment: .bottom) {
                if !black, (k + 9) % 12 == 0 {
                    Text("C\(4 + (k + 9) / 12)").font(.system(size: 8)).foregroundStyle(.black.opacity(0.5)).padding(.bottom, 3)
                }
            }
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { _ in
                    guard down != k else { return }
                    down = k
                    audio.press(sample: sample, pitch: audio.pitch(sample: sample, semitones: k))
                }
                .onEnded { _ in
                    down = nil
                    audio.release()
                })
            .help(noteForFrequency(hz: 440 * pow(2, Double(k) / 12)))
    }

    private var caption: String {
        if audio.state?.samples.first(where: { $0.index == sample })?.tuningHz != nil {
            return "C4 to B5, from the sample's tuning estimated from its loop. Hold a key: KON on press, KOFF on release, the envelope at full by direct gain."
        }
        return "The sample has no loop to tune it from, so A4 is its own rate (pitch $1000)."
    }
}

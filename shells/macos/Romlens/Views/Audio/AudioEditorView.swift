import RomlensKit
import SwiftUI

/// The editor area when a sound view is open: what the views read, then
/// the view.
struct AudioEditorView: View {
    @Bindable var model: RomViewModel

    var body: some View {
        VStack(spacing: 0) {
            AudioSourceBar(model: model, audio: model.audio)
            Divider()
            if model.audio.state == nil {
                AudioUnavailable(audio: model.audio)
            } else {
                switch model.audioTab {
                case .voices: VoicesView(audio: model.audio)
                case .samples: SamplesView(audio: model.audio)
                case .aram: AramView(audio: model.audio)
                case .scope: ScopeView(audio: model.audio)
                case nil: EmptyView()
                }
            }
        }
    }
}

/// Why there is nothing to show yet.
struct AudioUnavailable: View {
    let audio: AudioModel

    var body: some View {
        ContentUnavailableView {
            Label(title, systemImage: "speaker.slash")
        } description: {
            Text(message)
        }
    }

    private var title: String {
        switch audio.source {
        case .recording: "No Sound in This Recording"
        case .rom: audio.uploadLoading ? "Tracing the Upload…" : "No Sound Driver Found"
        }
    }

    private var message: String {
        switch audio.source {
        case .recording:
            return "Recordings made before the sound layer have only the picture side. Record again with this Romlens's recorder script (Help › Save Mesen Recorder Script…)."
        case .rom:
            if audio.uploadLoading { return "Looking for the code that sends the sound driver through the four ports." }
            return audio.romProblem
                ?? "The analysis found no routine that waits for the sound CPU's $BBAA and sends it a block list. A game with another way of uploading needs a recording (with the sound layer) to show its sound."
        }
    }
}

/// The recording at its frame, or the ROM's upload and which lists are
/// laid over the driver.
struct AudioSourceBar: View {
    let model: RomViewModel
    @Bindable var audio: AudioModel

    var body: some View {
        HStack(spacing: 12) {
            Picker("Source", selection: Binding(get: { audio.source }, set: { audio.pick($0) })) {
                Text("ROM").tag(AudioModel.Source.rom)
                if audio.hasRecordingSound {
                    Text("Recording").tag(AudioModel.Source.recording)
                }
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .labelsHidden()
            Text(audio.sourceDescription)
                .font(.callout.monospaced())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer()
            TransportControls(audio: audio)
            switch audio.source {
            case .recording:
                FrameStepper(graphics: model.graphics)
            case .rom:
                if !audio.otherUploads.isEmpty {
                    Menu("Uploads") {
                        ForEach(audio.otherUploads, id: \.list) { u in
                            Toggle(isOn: included(u.list)) {
                                Text("\(formatSnesAddress(address: u.list)): \(u.blocks.count) blocks, \(u.bytes) bytes")
                            }
                        }
                    }
                    .fixedSize()
                    .help("The songs and samples a game sends the running driver: which to lay over it")
                }
            }
        }
        .controlSize(.small)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private func included(_ list: UInt32) -> Binding<Bool> {
        Binding(
            get: { audio.includedLists.contains(list) },
            set: { on in
                if on { audio.includedLists.insert(list) } else { audio.includedLists.remove(list) }
            }
        )
    }
}

/// Shared drawing and words.
enum AudioStyle {
    static func hex(_ v: some BinaryInteger, _ digits: Int) -> String {
        "$" + GraphicsStyle.hex(v, digits)
    }

    static func colour(_ kind: AramKindInfo) -> Color {
        switch kind {
        case .directPage: Color(red: 0.55, green: 0.55, blue: 0.85)
        case .io: Color(red: 0.85, green: 0.3, blue: 0.3)
        case .stack: Color(red: 0.45, green: 0.45, blue: 0.7)
        case .code: Color(red: 0.25, green: 0.55, blue: 0.95)
        case .directory: Color(red: 0.95, green: 0.75, blue: 0.2)
        case .sample: Color(red: 0.3, green: 0.75, blue: 0.4)
        case .echo: Color(red: 0.7, green: 0.4, blue: 0.85)
        case .dspData: Color(red: 0.55, green: 0.8, blue: 0.55)
        case .driverData: Color(red: 0.4, green: 0.75, blue: 0.85)
        case .boot: Color(red: 0.6, green: 0.6, blue: 0.6)
        case .other: Color(red: 0.3, green: 0.3, blue: 0.3)
        }
    }

    static func rgb(_ kind: AramKindInfo) -> (UInt8, UInt8, UInt8) {
        let c = NSColor(colour(kind)).usingColorSpace(.sRGB) ?? .gray
        return (UInt8(c.redComponent * 255), UInt8(c.greenComponent * 255), UInt8(c.blueComponent * 255))
    }
}

/// A register explained field by field, as the inspector shows a 65816
/// register write.
struct RegisterPartsView: View {
    let parts: [RegisterPartInfo]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(parts.enumerated()), id: \.offset) { _, p in
                VStack(alignment: .leading, spacing: 3) {
                    Text(p.short).font(.callout.monospaced().weight(.medium))
                    Text(p.about).font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    ForEach(Array(p.fields.enumerated()), id: \.offset) { _, f in
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Text(f.bits).font(.caption.monospaced()).foregroundStyle(.secondary)
                                .frame(width: 36, alignment: .trailing)
                            Text(f.name).font(.caption.weight(.medium))
                            if let m = f.meaning {
                                Text(m).font(.caption).foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                }
            }
        }
    }
}

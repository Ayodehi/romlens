import RomlensKit
import SwiftUI

/// The two CPUs' conversation (docs/23, A13): every port write both ways
/// around the frame, the S-CPU's matched to the 65816 code that sends that
/// value, and the uploads sent so far, block by block.
struct PortsView: View {
    @Bindable var audio: AudioModel

    var body: some View {
        switch audio.source {
        case .recording: recordingPorts
        case .rom: romPorts
        }
    }

    private var recordingPorts: some View {
        let events = audio.portEvents(around: audio.frame)
        let uploads = audio.sentUploads()
        return HSplitView {
            List {
                Section("Frames \(audio.frame > 30 ? audio.frame - 30 : 0) to \(audio.frame + 30): \(events.count) writes") {
                    ForEach(Array(events.enumerated()), id: \.offset) { _, e in
                        PortEventRow(audio: audio, event: e, current: e.frame == audio.frame)
                    }
                }
            }
            .frame(minWidth: 380, maxWidth: .infinity, maxHeight: .infinity)
            List {
                Section("Uploads sent by frame \(audio.frame)") {
                    if uploads.isEmpty {
                        Text("None yet. At power-on the SPC700's boot program waits for one: the S-CPU writes $CC and then each byte with its index, and the boot program echoes each index back.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    ForEach(Array(uploads.enumerated()), id: \.offset) { i, u in
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Upload \(i): \(u.blocks.count) blocks, \(u.blocks.reduce(0) { $0 + $1.len }) bytes" + (u.entry.map { ", then \(AudioStyle.hex($0, 4))" } ?? ", not finished"))
                                .font(.callout)
                            ForEach(Array(u.blocks.enumerated()), id: \.offset) { _, b in
                                Text("  \(AudioStyle.hex(b.aram, 4))–\(AudioStyle.hex(UInt32(b.aram) + b.len - 1, 4))  \(b.len) bytes")
                                    .font(.caption.monospaced()).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .frame(minWidth: 260, idealWidth: 320, maxWidth: 420, maxHeight: .infinity)
        }
    }

    private var romPorts: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("The ROM's driver, run by Romlens").font(.headline)
            Text("A recording holds the game's own port writes. Here, write them yourself: the driver reads a port, acts, and answers on it.")
                .font(.callout).foregroundStyle(.secondary)
            PortConsole(audio: audio)
            if let commands = audio.upload?.commands, !commands.isEmpty {
                Text("Where the game's code sends sound commands").font(.headline)
                List(Array(commands.enumerated()), id: \.offset) { _, c in
                    HStack {
                        Text("port \(c.port) = \(AudioStyle.hex(c.value, c.width == 2 ? 4 : 2))").font(.callout.monospaced())
                        Text("in \(formatSnesAddress(address: c.routine))" + (c.via.map { ", set in \(formatSnesAddress(address: $0)) and copied to the port" } ?? ""))
                            .font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("Send") { audio.send(port: c.port, value: UInt8(c.value & 0xFF)) }
                        Button("Show in ROM") { audio.showInRom?(c.at) }.buttonStyle(.link)
                    }
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

struct PortEventRow: View {
    let audio: AudioModel
    let event: PortEventInfo
    let current: Bool

    var body: some View {
        HStack(spacing: 8) {
            Text("\(event.frame)").font(.caption.monospacedDigit()).foregroundStyle(current ? .primary : .secondary)
                .frame(width: 48, alignment: .trailing)
            Text(event.fromCpu ? "S-CPU → " : "← SPC700").font(.caption.monospaced())
                .foregroundStyle(event.fromCpu ? Color.accentColor : Color.green)
                .frame(width: 70, alignment: event.fromCpu ? .leading : .trailing)
            Text("port \(event.port)  \(AudioStyle.hex(event.value, 2))").font(.callout.monospaced())
            if event.fromCpu, event.value != 0 {
                ForEach(Array(audio.commandSites(port: event.port, value: event.value).prefix(2).enumerated()), id: \.offset) { _, site in
                    Button("from \(formatSnesAddress(address: site.routine))") { audio.showInRom?(site.at) }
                        .buttonStyle(.link)
                        .font(.caption)
                        .help("The 65816 code that sends this value, found by tracing it")
                }
            }
            Spacer()
            Text("cycle \(event.spcCycle)").font(.caption2.monospacedDigit()).foregroundStyle(.tertiary)
        }
    }
}

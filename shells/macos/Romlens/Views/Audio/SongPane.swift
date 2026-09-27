import RomlensKit
import SwiftUI

/// Nintendo's N-SPC driver's song, read (docs/23, A14): the song table, the
/// song's list of blocks with the one playing marked, and one voice's
/// track as notes, lengths and commands with the byte it reads next.
struct SongPane: View {
    @Bindable var audio: AudioModel

    var body: some View {
        if let d = audio.state?.nspc {
            content(d)
        } else {
            VStack(alignment: .leading, spacing: 6) {
                Text("Song").font(.headline)
                Text("The driver is not Nintendo's N-SPC, so its songs are its own data. The piano roll reads the DSP, so it works for any driver.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(12)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    private func content(_ d: NspcDriverInfo) -> some View {
        let playing = d.playing
        let number = audio.chosenSong ?? playing?.song ?? 1
        let list = audio.nspcSong(number)
        let isPlaying = playing?.song == number
        let block = isPlaying ? playing?.block : list.first { $0.kind == .block }?.block
        let tracks = list.first { $0.block == block }?.tracks ?? []
        let track = tracks.indices.contains(audio.songVoice) ? tracks[audio.songVoice] : 0
        let now = isPlaying ? playing?.positions[audio.songVoice] : nil
        let events = track == 0 ? [] : audio.nspcTrack(track)
        return VStack(alignment: .leading, spacing: 8) {
            Text("Song").font(.headline)
            Text("\(d.dialect): its table of command lengths at \(AudioStyle.hex(d.lengthsAt, 4))\(d.songTable.map { ", \(d.songs.count) songs at \(AudioStyle.hex($0, 4))" } ?? "")")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Picker("Song", selection: Binding(get: { number }, set: { audio.chosenSong = $0 })) {
                    ForEach(Array(d.songs.enumerated()), id: \.offset) { i, s in
                        Text("\(i + 1) at \(AudioStyle.hex(s, 4))" + (playing?.song == UInt8(i + 1) ? " (playing)" : "")).tag(UInt8(i + 1))
                    }
                }
                .fixedSize()
                .help("Song n is what a game sends the driver to play it: entry n - 1 of the table")
                Picker("Voice", selection: $audio.songVoice) {
                    ForEach(0..<8, id: \.self) { v in
                        Text("Voice \(v)" + (tracks.indices.contains(v) && tracks[v] != 0 ? "" : " (silent)")).tag(v)
                    }
                }
                .fixedSize()
            }
            .controlSize(.small)
            ScrollView {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(Array(list.enumerated()), id: \.offset) { _, e in
                        Text(entryText(e))
                            .font(.caption.monospaced())
                            .foregroundStyle(e.block == block && e.kind == .block ? Color.accentColor : .primary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 120)
            Divider()
            Text(track == 0 ? "Voice \(audio.songVoice) is silent in this block." : "Voice \(audio.songVoice)'s track at \(AudioStyle.hex(track, 4))" + (now.map { ", reading \(AudioStyle.hex($0, 4)) now" } ?? ""))
                .font(.caption).foregroundStyle(.secondary)
            ScrollViewReader { reader in
                List(Array(events.enumerated()), id: \.offset) { _, e in
                    let here = now.map { $0 >= e.at && Int($0) < Int(e.at) + e.bytes.count } ?? false
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(here ? "▶" : " ").foregroundStyle(.green)
                        Text(AudioStyle.hex(e.at, 4)).foregroundStyle(.secondary)
                        Text(e.bytes.map { GraphicsStyle.hex($0, 2) }.joined(separator: " "))
                            .foregroundStyle(.secondary)
                            .frame(width: 80, alignment: .leading)
                        Text(e.text).foregroundStyle(e.kind == "note" ? .primary : .secondary)
                            .lineLimit(2)
                    }
                    .font(.caption.monospaced())
                    .id(e.at)
                }
                .listStyle(.plain)
                .onChange(of: now) { _, n in
                    if let n, let e = events.last(where: { $0.at <= n }) { reader.scrollTo(e.at, anchor: .center) }
                }
            }
        }
        .padding(12)
        .onChange(of: audio.selectedNote?.voice) { _, v in if let v { audio.songVoice = Int(v) } }
    }

    private func entryText(_ e: NspcEntryInfo) -> String {
        let at = AudioStyle.hex(e.at, 4)
        switch e.kind {
        case .block:
            let voices = e.tracks.enumerated().filter { $0.element != 0 }.map { "\($0.offset)" }.joined(separator: ",")
            return "\(at)  block \(AudioStyle.hex(e.block, 4))  voices \(voices)"
        case .repeat: return "\(at)  back to \(AudioStyle.hex(e.to, 4)), \(e.count) more"
        case .jump: return "\(at)  back to \(AudioStyle.hex(e.to, 4)) for ever"
        case .end: return "\(at)  end"
        }
    }
}

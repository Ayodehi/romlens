import RomlensKit
import SwiftUI

/// The hit list ⌘F fills. Each row shows the bytes around the match with the
/// match itself emphasised, because a hex pattern's hits are indistinguishable
/// without their context.
struct SearchResultsView: View {
    let model: RomViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(model.search.searched.isEmpty ? "Find" : "“\(model.search.searched)”")
                    .font(.headline)
                    .lineLimit(1)
                Spacer()
                Text(model.search.summary).font(.caption).foregroundStyle(.secondary)
                Button {
                    model.isResultsVisible = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                }
                .buttonStyle(.plain)
                .help("Hide results (⌘F reopens)")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            Divider()
            if model.search.hits.isEmpty {
                Text(model.search.error ?? "No matches")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(12)
            } else {
                List(selection: selection) {
                    ForEach(Array(model.search.hits.enumerated()), id: \.offset) { index, hit in
                        row(hit).tag(index)
                    }
                }
                .listStyle(.inset)
                .font(.callout.monospaced())
            }
        }
        .frame(minHeight: 120, maxHeight: 220)
    }

    private var selection: Binding<Int?> {
        Binding(
            get: { model.search.current },
            set: { if let i = $0 { model.goToHit(at: i) } }
        )
    }

    private func row(_ hit: SearchHit) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(formatFileOffset(offset: hit.fileOffset))
                .foregroundStyle(.secondary)
            Text(hit.snesAddress ?? "--:----")
                .foregroundStyle(.secondary)
            context(hit)
            Spacer(minLength: 8)
            Text(hit.regionKind)
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
    }

    /// The context bytes, with the matched run in the accent colour.
    private func context(_ hit: SearchHit) -> Text {
        let matchEnd = hit.matchStart + hit.len
        return hit.context.enumerated().reduce(Text("")) { text, pair in
            let (i, byte) = pair
            let inMatch = UInt32(i) >= hit.matchStart && UInt32(i) < matchEnd
            let piece = Text(String(format: i == 0 ? "%02X" : " %02X", byte))
            return text + (inMatch ? piece.foregroundStyle(.tint).bold() : piece)
        }
    }
}

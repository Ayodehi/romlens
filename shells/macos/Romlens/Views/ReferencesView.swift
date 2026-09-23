import RomlensKit
import SwiftUI

/// The list Find References fills, in the pane Find's results use. Each row
/// is one referring place: where it is, the routine it is in, the
/// instruction itself and what kind of reference it makes.
struct ReferencesView: View {
    let model: RomViewModel

    var body: some View {
        let refs = model.references
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("References to \(refs.targetName)")
                    .font(.headline)
                    .lineLimit(1)
                Spacer()
                Text(refs.summary).font(.caption).foregroundStyle(.secondary)
                Button {
                    model.isResultsVisible = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                }
                .buttonStyle(.plain)
                .help("Hide references")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            Divider()
            if refs.rows.isEmpty {
                Text("Nothing the analyzer found refers to \(refs.targetName).")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(12)
            } else {
                List(selection: selection) {
                    ForEach(Array(refs.rows.enumerated()), id: \.offset) { index, row in
                        self.row(row).tag(index)
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
            get: { model.references.current },
            set: { if let i = $0 { model.goToReference(at: i) } }
        )
    }

    private func row(_ row: ReferencesModel.Row) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            if model.addressStyle != .snes {
                Text(formatFileOffset(offset: row.fileOffset)).foregroundStyle(.secondary)
            }
            if model.addressStyle != .file {
                Text(row.snesAddress.map { formatSnesAddress(address: $0) } ?? "--:----")
                    .foregroundStyle(.secondary)
            }
            Text(row.routine ?? "")
                .frame(minWidth: 140, alignment: .leading)
                .lineLimit(1)
            Text(row.text)
                .lineLimit(1)
            Spacer(minLength: 8)
            Text(row.certain ? row.kindName : "\(row.kindName)?")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .help(row.certain ? "" : "The analyzer is not sure of this reference")
        }
    }
}

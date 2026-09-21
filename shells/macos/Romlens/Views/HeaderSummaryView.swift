import RomlensKit
import SwiftUI

/// Header fields and vectors as coloured chips; clicking one jumps to its
/// bytes.
struct HeaderSummaryView: View {
    let model: RomViewModel

    var body: some View {
        List {
            Section("Image") {
                summary("File", model.info.fileName)
                summary("Size", "\(model.info.byteLen) bytes, \(model.info.rowCount) rows")
                summary("Mapping", "\(model.info.mappingName)\(model.info.fastRom ? ", FastROM" : ", SlowROM")")
                summary("Header at", formatFileOffset(offset: model.info.headerOffset))
                summary("Checksum", model.info.checksumOk ? "valid (mirrored sum)" : "mismatch")
                summary("SHA-256", String(model.info.sha256.prefix(16)) + "…")
                if model.info.hasCopierHeader {
                    summary("Copier header", "512 bytes stripped")
                }
            }
            Section("Header and vectors") {
                ForEach(model.spans, id: \.id) { span in
                    Button {
                        model.jump(to: span.start)
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Circle()
                                .fill(Color(nsColor: SpanPalette.color(for: span.kind)))
                                .frame(width: 9, height: 9)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(span.name)
                                Text(span.valueText)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Text(formatFileOffset(offset: span.start))
                                .font(.caption.monospaced())
                                .foregroundStyle(.tertiary)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .listStyle(.sidebar)
    }

    @ViewBuilder
    private func summary(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(label).foregroundStyle(.secondary)
            Spacer()
            Text(value).monospaced().textSelection(.enabled)
        }
        .font(.callout)
    }
}

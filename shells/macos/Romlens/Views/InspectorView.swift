import RomlensKit
import SwiftUI

/// The selected byte's readings, or the header summary when nothing is
/// selected.
struct InspectorView: View {
    let model: RomViewModel

    var body: some View {
        Group {
            if let inspection = model.inspection {
                ByteInspectorView(model: model, byte: inspection)
            } else {
                HeaderSummaryView(model: model)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

struct ByteInspectorView: View {
    let model: RomViewModel
    let byte: ByteInterpretation

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Byte at \(formatFileOffset(offset: byte.fileOffset))")
                    .font(.headline)
                if let name = byte.spanName {
                    Label {
                        VStack(alignment: .leading) {
                            Text(name).fontWeight(.medium)
                            if let value = byte.spanValue {
                                Text(value).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    } icon: {
                        Image(systemName: "tag.fill")
                    }
                }
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
                    row("File offset", formatFileOffset(offset: byte.fileOffset))
                    if byte.diskOffset != byte.fileOffset {
                        row("On disk", formatFileOffset(offset: byte.diskOffset))
                    }
                    row("SNES address", byte.snesAddress.map { formatSnesAddress(address: $0) } ?? "unmapped")
                    row("Mirrors", byte.mirrors.map { formatSnesAddress(address: $0) }.joined(separator: ", "))
                    Divider().gridCellUnsizedAxes(.horizontal)
                    row("u8", "\(byte.valueU8)  ($\(hex(UInt32(byte.valueU8), 2)))")
                    row("i8", "\(byte.valueI8)")
                    row("u16 LE", byte.valueU16Le.map { "\($0)  ($\(hex(UInt32($0), 4)))" } ?? "—")
                    row("i16 LE", byte.valueI16Le.map { "\($0)" } ?? "—")
                    row("u24 LE", byte.valueU24Le.map { "\($0)  ($\(hex($0, 6)))" } ?? "—")
                    row("ASCII", byte.ascii.map { "\"\($0)\"" } ?? "—")
                    Divider().gridCellUnsizedAxes(.horizontal)
                    addressRow("u16 in bank", byte.u16AsAddressInBank)
                    addressRow("u24 as address", byte.u24AsSnesAddress, region: byte.pointerTargetRegion)
                    if let target = byte.pointerTargetFileOffset {
                        GridRow {
                            Text("Points to").foregroundStyle(.secondary)
                            HStack {
                                Text(formatFileOffset(offset: target)).monospaced()
                                Button("Go") { model.jump(to: target) }
                                    .controlSize(.small)
                            }
                        }
                    }
                }
                .font(.callout)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
    }

    private func hex(_ v: UInt32, _ digits: Int) -> String {
        let s = String(v, radix: 16, uppercase: true)
        return String(repeating: "0", count: max(0, digits - s.count)) + s
    }

    @ViewBuilder
    private func row(_ label: String, _ value: String) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            Text(value).monospaced().textSelection(.enabled)
        }
    }

    @ViewBuilder
    private func addressRow(_ label: String, _ address: UInt32?, region: String? = nil) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            if let address {
                HStack {
                    Text(formatSnesAddress(address: address)).monospaced()
                    if let region { Text(region).font(.caption).foregroundStyle(.secondary) }
                    if model.rom.fileOffsetFor(snesAddress: address) != nil {
                        Button("Go") { model.jump(toSnesAddress: address) }
                            .controlSize(.small)
                    }
                }
            } else {
                Text("—")
            }
        }
    }
}

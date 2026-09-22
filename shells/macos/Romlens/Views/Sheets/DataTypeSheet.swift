import RomlensKit
import SwiftUI

/// Mark as ▸ Data…, for the kinds that take a parameter.
///
/// The parameters are what turn a row of numbers into `dw SUB_808423`, so the
/// sheet explains the bank rule rather than just offering it: a sixteen-bit
/// entry names an offset and something has to supply the bank, and choosing
/// wrongly is the difference between a label and a number.
struct DataTypeSheet: View {
    let model: RomViewModel
    @Environment(\.dismiss) private var dismiss

    @State private var kind: DataKind = .table
    @State private var stride: UInt8 = 2
    @State private var bpp: UInt8 = 4
    @State private var elem: TableElem = .code
    @State private var bankChoice: BankChoice = .same
    @State private var fixedBank = "C0"

    private enum BankChoice: String, CaseIterable, Identifiable {
        case same, fixed, entry
        var id: String { rawValue }
        var title: String {
            switch self {
            case .same: "Same bank as the table"
            case .fixed: "A fixed bank"
            case .entry: "Each entry carries its own"
            }
        }
    }

    /// The kinds a person marks by hand, in the order they are reached for.
    private static let kinds: [(DataKind, String)] = [
        (.table, "Table"),
        (.pointer, "Pointer"),
        (.byte, "Byte"),
        (.word, "Word"),
        (.long, "Long"),
        (.string, "String"),
        (.graphics, "Graphics"),
        (.tilemap, "Tilemap"),
        (.palette, "Palette"),
        (.compressed, "Compressed"),
        (.struct, "Struct"),
    ]

    private var takesElem: Bool { kind == .table }
    private var takesBank: Bool {
        kind == .pointer || (kind == .table && elem != .raw)
    }
    private var takesStride: Bool { kind == .table }
    private var takesBpp: Bool { kind == .graphics }

    private var bankIsValid: Bool {
        bankChoice != .fixed || UInt8(fixedBank.trimmingCharacters(in: CharacterSet(charactersIn: "$ ")), radix: 16) != nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Mark as Data").font(.headline)
            Text(rangeDescription).font(.callout.monospaced()).foregroundStyle(.secondary)
            Form {
                Picker("Kind", selection: $kind) {
                    ForEach(Self.kinds, id: \.0) { Text($0.1).tag($0.0) }
                }
                if takesStride {
                    Picker("Bytes per entry", selection: $stride) {
                        ForEach([UInt8(1), 2, 3, 4, 8], id: \.self) { Text("\($0)").tag($0) }
                    }
                }
                if takesBpp {
                    Picker("Bitplanes", selection: $bpp) {
                        ForEach([UInt8(2), 4, 8], id: \.self) { Text("\($0) bpp").tag($0) }
                    }
                }
                if takesElem {
                    Picker("Each entry is", selection: $elem) {
                        Text("Raw bytes").tag(TableElem.raw)
                        Text("A pointer to data").tag(TableElem.pointer)
                        Text("A pointer to code").tag(TableElem.code)
                    }
                }
                if takesBank {
                    Picker("Target bank", selection: $bankChoice) {
                        ForEach(BankChoice.allCases) { Text($0.title).tag($0) }
                    }
                    if bankChoice == .fixed {
                        TextField("Bank", text: $fixedBank)
                            .font(.body.monospaced())
                            .frame(width: 80)
                    }
                }
            }
            .formStyle(.grouped)
            Text(explanation)
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(minHeight: 34, alignment: .topLeading)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Mark", action: apply)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!bankIsValid)
            }
        }
        .padding(20)
        .frame(width: 460)
    }

    private var rangeDescription: String {
        guard let range = model.highlightedRange else { return "No selection" }
        return "\(formatFileOffset(offset: range.lowerBound))  \(range.count) byte\(range.count == 1 ? "" : "s")"
    }

    private var explanation: String {
        guard takesBank else {
            return "The analyzer keeps its own guess for everything you do not mark."
        }
        switch bankChoice {
        case .same:
            return "Entries resolve in the bank the table itself is in — what a `JMP (abs,X)` dispatch does."
        case .fixed:
            return "Every entry resolves in bank $\(fixedBank.uppercased()), whatever bank the table is in."
        case .entry:
            return "Each entry carries its own bank, so entries are at least three bytes."
        }
    }

    private func apply() {
        let bank: BankRule? = {
            guard takesBank else { return nil }
            switch bankChoice {
            case .same: return .sameBank
            case .entry: return .fromEntry
            case .fixed:
                let text = fixedBank.trimmingCharacters(in: CharacterSet(charactersIn: "$ "))
                return UInt8(text, radix: 16).map { BankRule.fixed(bank: $0) }
            }
        }()
        model.mark(
            .data,
            dataKind: kind,
            stride: takesStride ? stride : nil,
            bpp: takesBpp ? bpp : nil,
            elem: takesElem ? elem : nil,
            bank: bank
        )
        dismiss()
    }
}

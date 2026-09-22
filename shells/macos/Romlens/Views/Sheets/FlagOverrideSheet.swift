import RomlensKit
import SwiftUI

/// Set Flags…: pin M, X, E, the data bank and the direct page at the
/// selected instruction. The analyzer re-runs from here with these values.
struct FlagOverrideSheet: View {
    enum Tri: Hashable {
        case unchanged, set, clear
        var value: Bool? {
            switch self {
            case .unchanged: nil
            case .set: true
            case .clear: false
            }
        }
        init(_ v: Bool?) { self = v.map { $0 ? .set : .clear } ?? .unchanged }
    }

    let model: RomViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var m: Tri = .unchanged
    @State private var x: Tri = .unchanged
    @State private var e: Tri = .unchanged
    @State private var dbr = ""
    @State private var dp = ""

    private var parsedDbr: UInt8?? {
        parse(dbr, digits: 2).map { $0.map { UInt8($0) } }
    }
    private var parsedDp: UInt16?? {
        parse(dp, digits: 4).map { $0.map { UInt16($0) } }
    }

    /// `nil` = invalid; `.some(nil)` = empty; `.some(.some(v))` = a value.
    private func parse(_ text: String, digits: Int) -> UInt32?? {
        let t = text.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "$", with: "")
        if t.isEmpty { return .some(nil) }
        guard t.count <= digits, let v = UInt32(t, radix: 16) else { return nil }
        return .some(v)
    }

    private var valid: Bool { parsedDbr != nil && parsedDp != nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Set Flags").font(.headline)
            if let address = model.selectedAddress {
                Text("at \(formatSnesAddress(address: address))").font(.callout.monospaced()).foregroundStyle(.secondary)
            }
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
                tri("M (accumulator)", $m, set: "8-bit", clear: "16-bit")
                tri("X (index)", $x, set: "8-bit", clear: "16-bit")
                tri("E (emulation)", $e, set: "emulation", clear: "native")
                GridRow {
                    Text("Data bank").foregroundStyle(.secondary)
                    TextField("$80", text: $dbr).textFieldStyle(.roundedBorder).font(.body.monospaced()).frame(width: 80)
                }
                GridRow {
                    Text("Direct page").foregroundStyle(.secondary)
                    TextField("$0000", text: $dp).textFieldStyle(.roundedBorder).font(.body.monospaced()).frame(width: 80)
                }
            }
            Text(valid ? "Leave a field unchanged to keep the analyzer's value." : "Bank and direct page are hexadecimal.")
                .font(.callout)
                .foregroundStyle(valid ? Color.secondary : Color.red)
            HStack {
                if model.flagOverride != nil {
                    Button("Clear", role: .destructive) {
                        try? model.setFlagOverride(nil)
                        dismiss()
                    }
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Apply", action: commit).keyboardShortcut(.defaultAction).disabled(!valid)
            }
        }
        .padding(20)
        .frame(width: 420)
        .onAppear {
            let f = model.flagOverride
            m = Tri(f?.m)
            x = Tri(f?.x)
            e = Tri(f?.e)
            dbr = f?.dbr.map { "$" + InspectorStyle.hex(UInt32($0), 2) } ?? ""
            dp = f?.dp.map { "$" + InspectorStyle.hex(UInt32($0), 4) } ?? ""
        }
    }

    @ViewBuilder
    private func tri(_ title: String, _ value: Binding<Tri>, set: String, clear: String) -> some View {
        GridRow {
            Text(title).foregroundStyle(.secondary)
            Picker(title, selection: value) {
                Text("Unchanged").tag(Tri.unchanged)
                Text(set).tag(Tri.set)
                Text(clear).tag(Tri.clear)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
        }
    }

    private func commit() {
        guard let dbr = parsedDbr, let dp = parsedDp else { return }
        let flags = FlagOverride(m: m.value, x: x.value, e: e.value, dbr: dbr, dp: dp)
        let empty = flags.m == nil && flags.x == nil && flags.e == nil && flags.dbr == nil && flags.dp == nil
        try? model.setFlagOverride(empty ? nil : flags)
        dismiss()
    }
}

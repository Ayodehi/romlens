import RomlensKit
import SwiftUI

/// Define Variable: name an address and give it a type, so instructions that
/// use it read `STA PlayerX` (and `STA PlayerX+1` inside it) rather than an
/// address. Opened from an instruction, it starts at the operand's address.
struct VariableSheet: View {
    @Bindable var model: RomViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var error: String?
    @FocusState private var focused: Bool

    private var draft: Binding<RomViewModel.VariableDraft> { $model.variableDraft }

    private var address: UInt32? {
        model.variableDraft.existing ?? RomViewModel.parseAddress(model.variableDraft.address)
    }

    private var validation: String? {
        let d = model.variableDraft
        guard address != nil else {
            return d.address.isEmpty ? "An address: $7E:0094, 7F8000, or $0094 for low RAM." : "Not an address; use a form like $7E:0094."
        }
        let name = d.name.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return "A name: letters, digits and underscores." }
        do {
            try validateLabelName(name: name, address: address)
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    private var size: UInt32 {
        let w: UInt32 = switch model.variableDraft.width {
        case .byte: 1
        case .word: 2
        case .long: 3
        }
        return w * UInt32(max(1, model.variableDraft.count))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(model.variableDraft.existing == nil ? "Define Variable" : "Edit Variable").font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                GridRow {
                    Text("Address")
                    if let existing = model.variableDraft.existing {
                        Text(formatSnesAddress(address: existing)).font(.body.monospaced())
                    } else {
                        TextField("$7E:0094", text: draft.address)
                            .textFieldStyle(.roundedBorder)
                            .font(.body.monospaced())
                    }
                }
                GridRow {
                    Text("Name")
                    TextField("PlayerX", text: draft.name)
                        .textFieldStyle(.roundedBorder)
                        .font(.body.monospaced())
                        .focused($focused)
                        .onSubmit(commit)
                }
                GridRow {
                    Text("Type")
                    HStack {
                        Picker("Type", selection: draft.width) {
                            Text("byte").tag(VarWidth.byte)
                            Text("word").tag(VarWidth.word)
                            Text("long").tag(VarWidth.long)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                        Stepper(value: draft.count, in: 1...4096) {
                            Text(model.variableDraft.count == 1 ? "one" : "× \(model.variableDraft.count)")
                                .monospacedDigit()
                        }
                        .help("Elements: more than one makes an array")
                    }
                }
            }
            Text(validation ?? error ?? "\(size) byte\(size == 1 ? "" : "s"); an access inside reads as name+offset.")
                .font(.callout)
                .foregroundStyle(validation == nil && error == nil ? Color.secondary : Color.red)
                .frame(minHeight: 20, alignment: .leading)
            HStack {
                if let existing = model.variableDraft.existing {
                    Button("Remove", role: .destructive) {
                        try? model.removeVariable(address: existing)
                        dismiss()
                    }
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button(model.variableDraft.existing == nil ? "Define" : "Save", action: commit)
                    .keyboardShortcut(.defaultAction)
                    .disabled(validation != nil)
            }
        }
        .padding(20)
        .frame(width: 440)
        .onAppear { focused = true }
    }

    private func commit() {
        guard validation == nil else { return }
        do {
            try model.defineVariable(model.variableDraft)
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

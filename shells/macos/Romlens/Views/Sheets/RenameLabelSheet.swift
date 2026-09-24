import RomlensKit
import SwiftUI

/// `n`: name the selected address. Validated live through the core.
struct RenameLabelSheet: View {
    let model: RomViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @FocusState private var focused: Bool

    private var validation: String? {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return nil }
        do {
            try validateLabelName(name: trimmed, address: model.selectedAddress)
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Rename Label").font(.headline)
            if let address = model.selectedAddress {
                Text(formatSnesAddress(address: address)).font(.callout.monospaced()).foregroundStyle(.secondary)
            }
            TextField(model.label?.source == .auto ? model.label!.name : "Label", text: $name)
                .textFieldStyle(.roundedBorder)
                .font(.body.monospaced())
                .focused($focused)
                .onSubmit(commit)
            Text(validation ?? "Letters, digits and underscores; up to 64 characters.")
                .font(.callout)
                .foregroundStyle(validation == nil ? Color.secondary : Color.red)
                .frame(minHeight: 20, alignment: .leading)
            HStack {
                if model.canRemoveLabel {
                    Button("Remove", role: .destructive) {
                        try? model.removeLabel()
                        dismiss()
                    }
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Rename", action: commit)
                    .keyboardShortcut(.defaultAction)
                    .disabled(validation != nil || name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 420)
        .onAppear {
            name = model.label?.source == .user ? model.label!.name : ""
            focused = true
        }
    }

    private func commit() {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard validation == nil, !trimmed.isEmpty else { return }
        try? model.setLabel(name: trimmed)
        dismiss()
    }
}

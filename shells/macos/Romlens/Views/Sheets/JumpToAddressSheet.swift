import RomlensKit
import SwiftUI

/// ⌘L. Resolves on every keystroke so the user sees `0x00041C = $80:841C`
/// or the core's message before pressing Return.
struct JumpToAddressSheet: View {
    let model: RomViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @FocusState private var focused: Bool

    private var preview: Result<ResolvedAddress, RomlensError>? {
        text.trimmingCharacters(in: .whitespaces).isEmpty ? nil : model.preview(text: text)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Jump to Address").font(.headline)
            TextField("$80:841C, 80841C or 0x41C", text: $text)
                .textFieldStyle(.roundedBorder)
                .font(.body.monospaced())
                .focused($focused)
                .onSubmit(jump)
            Group {
                switch preview {
                case .success(let r):
                    Text("\(formatFileOffset(offset: r.fileOffset)) = \(r.snesAddress.map { formatSnesAddress(address: $0) } ?? "unreachable")  ·  row \(r.row)")
                        .foregroundStyle(.secondary)
                case .failure(let error):
                    Text(error.localizedDescription).foregroundStyle(.red)
                case nil:
                    Text("File offsets start with 0x; SNES addresses with $ or bank:offset.")
                        .foregroundStyle(.tertiary)
                }
            }
            .font(.callout.monospaced())
            .frame(minHeight: 20, alignment: .leading)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Jump", action: jump)
                    .keyboardShortcut(.defaultAction)
                    .disabled({ if case .success = preview { false } else { true } }())
            }
        }
        .padding(20)
        .frame(width: 420)
        .onAppear { focused = true }
    }

    private func jump() {
        guard case .success = preview else { return }
        try? model.jump(text: text)
        dismiss()
    }
}

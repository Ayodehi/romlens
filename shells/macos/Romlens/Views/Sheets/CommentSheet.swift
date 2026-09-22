import RomlensKit
import SwiftUI

/// `;`: a line comment after the instruction or a block comment above it.
struct CommentSheet: View {
    let model: RomViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var kind: CommentKind = .line
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Comment").font(.headline)
            if let address = model.selectedAddress {
                Text(formatSnesAddress(address: address)).font(.callout.monospaced()).foregroundStyle(.secondary)
            }
            Picker("Kind", selection: $kind) {
                Text("Line").tag(CommentKind.line)
                Text("Block").tag(CommentKind.block)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .onChange(of: kind) { _, k in text = existing(k) }
            if kind == .line {
                TextField("Comment", text: $text)
                    .textFieldStyle(.roundedBorder)
                    .focused($focused)
                    .onSubmit(commit)
            } else {
                TextEditor(text: $text)
                    .font(.body)
                    .frame(height: 100)
                    .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color(nsColor: .separatorColor)))
                    .focused($focused)
            }
            HStack {
                if !existing(kind).isEmpty {
                    Button("Remove", role: .destructive) {
                        try? model.setComment(kind: kind, text: nil)
                        dismiss()
                    }
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Save", action: commit).keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 460)
        .onAppear {
            text = existing(kind)
            focused = true
        }
    }

    private func existing(_ k: CommentKind) -> String {
        (k == .line ? model.lineComment : model.blockComment)?.text ?? ""
    }

    private func commit() {
        try? model.setComment(kind: kind, text: text)
        dismiss()
    }
}

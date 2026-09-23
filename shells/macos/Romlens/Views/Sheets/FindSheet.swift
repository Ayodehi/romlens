import RomlensKit
import SwiftUI

/// ⌘F. Bytes with `??` wildcards, or text.
struct FindSheet: View {
    let model: RomViewModel
    @Environment(\.dismiss) private var dismiss
    @FocusState private var focused: Bool

    private var search: SearchModel { model.search }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Find").font(.headline)
            HStack(spacing: 8) {
                TextField(placeholder, text: Binding(get: { search.query }, set: { search.query = $0 }))
                    .textFieldStyle(.roundedBorder)
                    .font(.body.monospaced())
                    .focused($focused)
                    .onSubmit(run)
                Picker("", selection: Binding(get: { search.mode }, set: { search.mode = $0 })) {
                    ForEach(SearchModel.Mode.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(width: 130)
            }
            Toggle("Ignore case", isOn: Binding(get: { search.ignoreCase }, set: { search.ignoreCase = $0 }))
                .disabled(search.mode != .text)
                .font(.callout)
            Text(search.summary.isEmpty ? hint : search.summary)
                .font(.callout.monospaced())
                .foregroundStyle(summaryColour)
                .frame(minHeight: 20, alignment: .leading)
            HStack {
                Button("Show Results") {
                    model.resultsKind = .find
                    model.isResultsVisible = true
                    dismiss()
                }
                .disabled(!search.hasResults)
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Find", action: run)
                    .keyboardShortcut(.defaultAction)
                    .disabled(search.query.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 460)
        .onAppear { focused = true }
    }

    private var summaryColour: Color {
        if search.summary.isEmpty { return .secondary }
        return search.error == nil ? .secondary : .red
    }

    private var placeholder: String {
        search.mode == .bytes ? "78 18 ?? 5C" : "Super Metroid"
    }

    private var hint: String {
        search.mode == .bytes
            ? "Hex byte pairs; ?? matches any byte."
            : "Matched as bytes of text, not as a tile encoding."
    }

    private func run() {
        model.runSearch()
        if model.search.hasResults {
            model.isResultsVisible = true
            dismiss()
        }
    }
}

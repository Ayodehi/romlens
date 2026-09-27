import AppKit
import RomlensKit
import SwiftUI

/// Names a local, notes a routine, comments a statement, or writes a C
/// version (docs/24, U10): the same annotations the tutor makes.
struct CEditSheet: View {
    @Bindable var model: RomViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var name = ""
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title).font(.headline)
            Text(hint).font(.caption).foregroundStyle(.secondary)
            switch model.cEdit {
            case .local:
                TextField("Name", text: $text, prompt: Text("a C name, e.g. slot"))
            case .version:
                TextField("Version", text: $name, prompt: Text("Plain words"))
                TextEditor(text: $text)
                    .font(.system(.body, design: .monospaced))
                    .frame(minHeight: 260)
                    .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.secondary.opacity(0.3)))
            default:
                TextEditor(text: $text).frame(minHeight: 80)
                    .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.secondary.opacity(0.3)))
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                if canRemove {
                    Button("Remove", role: .destructive) { save(remove: true) }
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Save") { save(remove: false) }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(16)
        .frame(width: model.cEdit.map { if case .version = $0 { 560 } else { 400 } } ?? 400)
        .onAppear(perform: load)
    }

    private var wb: Workbench { model.session.workbench }

    private var title: String {
        switch model.cEdit {
        case .local(_, let l): "Name the local \(l)"
        case .note(let r): "Note on \(formatSnesAddress(address: r))"
        case .comment(let a): "C comment at \(formatSnesAddress(address: a))"
        case .version(_, let n): n == nil ? "New C version" : "Edit C version"
        case nil: ""
        }
    }

    private var hint: String {
        switch model.cEdit {
        case .local: "The C calls it this in this routine only."
        case .note: "Printed above the routine in the C: what it does, takes and returns."
        case .comment: "Printed in the C before the statement this instruction makes."
        case .version: "Your own C for this routine, kept beside the generated C and never compiled. Lines keep the anchors they had."
        case nil: ""
        }
    }

    private var canRemove: Bool {
        switch model.cEdit {
        case .local(let r, let l): wb.localNames(routine: r).contains { $0.local == l }
        case .note(let r): wb.routineNote(routine: r) != nil
        case .comment(let a): wb.cComments().contains { $0.address == a }
        case .version(_, let n): n != nil
        case nil: false
        }
    }

    private func load() {
        switch model.cEdit {
        case .local(let r, let l): text = wb.localNames(routine: r).first { $0.local == l }?.name ?? l
        case .note(let r): text = wb.routineNote(routine: r) ?? ""
        case .comment(let a): text = wb.cComments().first { $0.address == a }?.text ?? ""
        case .version(let r, let n):
            name = n ?? "My version"
            text = n.flatMap { n in wb.cVersions(routine: r).first { $0.name == n }?.version.text }
                ?? model.decompiler.result.map(\.text) ?? ""
        case nil: break
        }
    }

    private func save(remove: Bool) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let value: String? = remove || t.isEmpty ? nil : t
        let command: Command
        switch model.cEdit {
        case .local(let r, let l): command = .setLocalName(routine: r, local: l, name: value)
        case .note(let r): command = .setRoutineNote(routine: r, text: value)
        case .comment(let a): command = .setCComment(address: a, text: value)
        case .version(let r, let old):
            let kept = old.flatMap { o in wb.cVersions(routine: r).first { $0.name == o }?.version }
            let lines = UInt32(text.split(separator: "\n", omittingEmptySubsequences: false).count)
            let anchors = (kept?.anchors ?? []).filter { $0.last <= lines }
            if let old, old != name.trimmingCharacters(in: .whitespaces), !remove {
                try? model.session.execute(.setCVersion(routine: r, name: old, version: nil))
            }
            command = .setCVersion(
                routine: r, name: remove ? (old ?? name) : name.trimmingCharacters(in: .whitespaces),
                version: value == nil ? nil : CVersionInfo(text: text, author: .user, anchors: anchors))
        case nil: dismiss(); return
        }
        do {
            try model.session.execute(command)
            model.cEdit = nil
            dismiss()
        } catch {
            self.error = TutorModel.message(error)
        }
    }
}

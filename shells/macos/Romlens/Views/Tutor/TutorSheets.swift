import AppKit
import RomlensKit
import SwiftUI

/// `/resume`: the project's conversations, latest first.
struct ResumeSheet: View {
    let tutor: TutorModel
    @Environment(\.dismiss) private var dismiss
    @State private var list: [ConversationSummaryInfo] = []
    @State private var selection: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Resume a conversation").font(.headline)
            List(list, id: \.id, selection: $selection) { c in
                VStack(alignment: .leading, spacing: 2) {
                    Text(c.title).lineLimit(1)
                    Text("\(Self.date(c.updated)) · \(c.model) · \(c.turns) turns · \(String(format: "$%.3f", c.cost))")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .tag(c.id)
            }
            .frame(minHeight: 240)
            HStack {
                Button("Delete", role: .destructive) {
                    if let id = selection {
                        do {
                            try tutor.session?.deleteConversation(id: id)
                            list = tutor.conversations
                        } catch { tutor.error = TutorModel.message(error) }
                    }
                }
                .disabled(selection == nil || selection == tutor.session?.conversationId())
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Resume") {
                    if let id = selection { tutor.resume(id) }
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(selection == nil || tutor.busy)
            }
        }
        .padding(16)
        .frame(width: 480)
        .onAppear { list = tutor.conversations }
    }

    static func date(_ seconds: UInt64) -> String {
        Date(timeIntervalSince1970: TimeInterval(seconds)).formatted(date: .abbreviated, time: .shortened)
    }
}

/// `/rewind` (Esc Esc): back to an earlier question, with the tutor's
/// edits since, or either alone.
struct RewindSheet: View {
    let tutor: TutorModel
    @Environment(\.dismiss) private var dismiss
    @State private var points: [TurnInfo] = []
    @State private var selection: UInt32?
    @State private var what = RewindWhat.both
    @State private var report: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Rewind").font(.headline)
            Text("Go back to before a question. Its words come back into the composer.").font(.caption).foregroundStyle(.secondary)
            List(points.reversed(), id: \.index, selection: $selection) { t in
                Text(Self.words(t)).lineLimit(2).tag(t.index)
            }
            .frame(minHeight: 200)
            Picker("", selection: $what) {
                Text("The conversation and the tutor's edits").tag(RewindWhat.both)
                Text("The conversation only").tag(RewindWhat.conversation)
                Text("The tutor's edits only").tag(RewindWhat.edits)
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
            if let report { Text(report).font(.caption).foregroundStyle(.secondary) }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Rewind") { rewind() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(selection == nil || tutor.busy)
            }
        }
        .padding(16)
        .frame(width: 480)
        .onAppear {
            points = tutor.session?.rewindPoints() ?? []
            selection = points.last?.index
        }
    }

    static func words(_ t: TurnInfo) -> String {
        for case let .text(s) in t.blocks.reversed() where !s.hasPrefix("[") { return s }
        return "A picture"
    }

    private func rewind() {
        guard let i = selection, let s = tutor.session else { return }
        do {
            let r = try s.rewind(index: i, what: what)
            if let p = r.prompt { tutor.composer = p }
            tutor.refresh()
            tutor.rom.session.tutorEdited()
            if let e = r.edits, !e.kept.isEmpty {
                report = "Kept \(e.kept.count) edit\(e.kept.count == 1 ? "" : "s") you changed afterwards: \(e.kept.joined(separator: ", "))."
                return
            }
            dismiss()
        } catch {
            report = TutorModel.message(error)
        }
    }
}

/// `/model`: the provider, the model and its effort, from the next
/// question on.
struct ModelSheet: View {
    let tutor: TutorModel
    @Environment(\.dismiss) private var dismiss
    @State private var endpoint = "anthropic"
    @State private var model = ""
    @State private var effort = ""
    @State private var served: [String] = []

    var body: some View {
        let settings = tutor.settings
        let e = settings.endpoint(endpoint) ?? settings.defaultEndpoint
        let table = settings.tableModels(for: e)
        VStack(alignment: .leading, spacing: 10) {
            Text("Model").font(.headline)
            Form {
                Picker("Provider", selection: $endpoint) {
                    ForEach(settings.endpoints) { Text($0.name + (settings.ready($0) ? "" : " (no key)")).tag($0.id) }
                }
                if table.isEmpty {
                    if served.isEmpty {
                        TextField("Model", text: $model)
                    } else {
                        Picker("Model", selection: $model) { ForEach(served, id: \.self) { Text($0).tag($0) } }
                    }
                } else {
                    Picker("Model", selection: $model) {
                        ForEach(table, id: \.id) { Text($0.name).tag($0.id) }
                    }
                }
                let efforts = table.first { $0.id == model }?.efforts ?? []
                if !efforts.isEmpty {
                    Picker("Effort", selection: $effort) {
                        Text("The model's default").tag("")
                        ForEach(efforts, id: \.self) { Text($0).tag($0) }
                    }
                }
            }
            Text("The conversation goes on with the new model; its cache starts cold.").font(.caption).foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Use") { use(e) }.keyboardShortcut(.defaultAction).disabled(model.isEmpty || tutor.busy)
            }
        }
        .padding(16)
        .frame(width: 420)
        .onAppear {
            endpoint = tutor.session?.endpoint()?.id ?? settings.defaultEndpoint.id
            model = tutor.session?.model() ?? settings.model(for: e) ?? ""
            effort = tutor.session?.effort() ?? ""
        }
        .onChange(of: endpoint) { _, id in
            guard let n = settings.endpoint(id) else { return }
            model = settings.model(for: n) ?? ""
            served = []
            if settings.tableModels(for: n).isEmpty {
                let info = n.info
                let key = settings.keys.key(for: n.id)
                Task.detached {
                    let ids = (try? tutorListModels(endpoint: info, key: key)) ?? []
                    await MainActor.run {
                        served = ids
                        if model.isEmpty { model = ids.first ?? "" }
                    }
                }
            }
        }
    }

    private func use(_ e: TutorEndpoint) {
        let settings = tutor.settings
        settings.setModel(model, for: e)
        settings.setEffort(effort.isEmpty ? nil : effort, for: e)
        if let s = tutor.session, s.conversationId() != nil {
            do {
                try s.setModel(endpoint: e.info, model: model, effort: effort.isEmpty ? nil : effort)
            } catch { tutor.error = TutorModel.message(error) }
        } else {
            settings.defaultEndpoint = e
        }
        tutor.refresh()
        dismiss()
    }
}

struct HelpSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("The tutor").font(.headline)
            Text("It reads Romlens's analysis with its tools (the listing, the C, the graphs, registers, tiles, frames, sound) and cites what it finds; click a citation to go there in the main window.")
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 4) {
                GridRow { Text("Return").bold(); Text("Send") }
                GridRow { Text("⇧Return").bold(); Text("A new line") }
                GridRow { Text("↑ ↓").bold(); Text("Earlier questions, at the first or last line") }
                GridRow { Text("⇧⇥").bold(); Text("Read-only → Ask before edits → Accept edits") }
                GridRow { Text("Esc").bold(); Text("Stop the answer; twice to rewind") }
                GridRow { Text("Paste, drop").bold(); Text("Attach a screenshot or photo") }
            }
            .font(.callout)
            Divider()
            ForEach(TutorModel.commands.filter { $0.name != "/clear" }) { c in
                HStack(alignment: .firstTextBaseline) {
                    Text(c.name).font(.callout.monospaced()).frame(width: 90, alignment: .leading)
                    Text(c.about).font(.callout).foregroundStyle(.secondary)
                }
            }
            HStack { Spacer(); Button("Done") { dismiss() }.keyboardShortcut(.defaultAction) }
        }
        .padding(16)
        .frame(width: 460)
    }
}

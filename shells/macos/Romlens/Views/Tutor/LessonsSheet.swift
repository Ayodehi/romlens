import AppKit
import RomlensKit
import SwiftUI

/// `/lessons` and `/map` (docs/25): the student's lessons, to read again,
/// and the concept map shaded by how far they have got.
struct LessonsSheet: View {
    let tutor: TutorModel
    enum Tab: Hashable { case lessons, map }
    @State var tab: Tab
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Picker("", selection: $tab) {
                Text("Lessons").tag(Tab.lessons)
                Text("What you know").tag(Tab.map)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 280)
            switch tab {
            case .lessons: LessonLibrary(tutor: tutor)
            case .map: ConceptMap(tutor: tutor)
            }
            HStack {
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(16)
        .frame(width: 720, height: 560)
    }
}

/// The lessons, newest first, and the one picked as its card.
struct LessonLibrary: View {
    let tutor: TutorModel
    @State private var list: [LessonInfo] = []
    @State private var selection: String?

    var body: some View {
        HSplitView {
            List(list, id: \.id, selection: $selection) { l in
                VStack(alignment: .leading, spacing: 2) {
                    Text(l.title).lineLimit(2)
                    Text("\(l.levelName) · \(l.steps.count) steps · \(ResumeSheet.date(l.created))\(l.finished ? "" : " · not ended")")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .tag(l.id)
                .contextMenu {
                    Button("Delete Lesson", role: .destructive) {
                        tutor.deleteLesson(l.id)
                        list = tutor.lessons
                    }
                }
            }
            .frame(minWidth: 220, idealWidth: 260)
            .overlay {
                if list.isEmpty {
                    Text("No lessons yet. Turn on Explain, or ask with /learn.")
                        .foregroundStyle(.secondary).multilineTextAlignment(.center).padding()
                }
            }
            ScrollView {
                if let id = selection {
                    LessonCard(tutor: tutor, id: id).padding(4)
                } else {
                    Text("Pick a lesson to read it again.").foregroundStyle(.secondary).padding()
                }
            }
            .frame(minWidth: 320)
        }
        .onAppear {
            list = tutor.lessons
            selection = list.first?.id
        }
    }
}

/// The concepts grouped, each shaded by the level reached. A concept can be
/// marked known, so lessons skip what the student already has.
struct ConceptMap: View {
    let tutor: TutorModel
    @State private var learner: LearnerInfo?

    var body: some View {
        ScrollView {
            if let learner {
                VStack(alignment: .leading, spacing: 14) {
                    legend(learner)
                    ForEach(learner.groups, id: \.self) { group in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(group).font(.headline)
                            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 6)], alignment: .leading, spacing: 6) {
                                ForEach(learner.concepts.filter { $0.group == group }, id: \.id) { c in
                                    chip(c, learner)
                                }
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .onAppear { learner = tutor.learner }
    }

    private func legend(_ l: LearnerInfo) -> some View {
        HStack(spacing: 10) {
            ForEach(Array(l.levels.enumerated()), id: \.offset) { i, name in
                HStack(spacing: 4) {
                    RoundedRectangle(cornerRadius: 3).fill(Self.shade(UInt8(i + 1))).frame(width: 14, height: 10)
                    Text("\(i + 1) \(name)").font(.caption2).foregroundStyle(.secondary)
                }
            }
        }
    }

    static func shade(_ level: UInt8) -> Color {
        level == 0 ? Color.secondary.opacity(0.08) : Color.accentColor.opacity(0.12 + 0.17 * Double(level))
    }

    private func chip(_ c: ConceptInfo, _ l: LearnerInfo) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(c.name).font(.callout).lineLimit(1)
            Text(c.level == 0 ? "not yet" : "\(c.level) · \(l.levels[Int(c.level) - 1])\(c.marked ? " (marked)" : "")")
                .font(.caption2).foregroundStyle(c.level >= 4 ? Color.white : Color.secondary)
        }
        .padding(.horizontal, 8).padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 6).fill(Self.shade(c.level)))
        .help(c.line + (c.needs.isEmpty ? "" : "\nRests on: " + c.needs.joined(separator: ", ")))
        .contextMenu {
            Menu("Mark as Known") {
                ForEach(1...5, id: \.self) { n in
                    Button("\(n) · \(l.levels[n - 1])") {
                        tutor.markKnown(c.id, level: UInt8(n))
                        learner = tutor.learner
                    }
                }
            }
            if c.marked {
                Button("Clear the Mark") {
                    tutor.markKnown(c.id, level: nil)
                    learner = tutor.learner
                }
            }
        }
    }
}

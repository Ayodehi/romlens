import AppKit
import RomlensKit
import SwiftUI

/// A lesson (docs/25), one step at a time: its title and level, the step
/// ("3 of 8"), a predict question before the step's answer, Back and Next,
/// and at the end the lessons that could come next. Each step moves the main
/// window to what it is about.
struct LessonCard: View {
    let tutor: TutorModel
    let id: String

    var body: some View {
        Group {
            if let lesson = tutor.lesson(id) {
                content(lesson)
            } else {
                Label("A lesson that is no longer kept", systemImage: "book.closed")
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Color.accentColor.opacity(0.35)))
    }

    @ViewBuilder
    private func content(_ lesson: LessonInfo) -> some View {
        let count = lesson.steps.count
        let at = min(tutor.step(of: lesson.id), max(count - 1, 0))
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "book").foregroundStyle(Color.accentColor)
                Text(lesson.title).font(.headline)
                Spacer()
                Text(lesson.levelName)
                    .font(.caption)
                    .padding(.horizontal, 7).padding(.vertical, 2)
                    .background(Capsule().fill(Color.accentColor.opacity(0.15)))
                    .help("How deep the lesson goes: " + lesson.concepts.map { "\($0.name) (level \($0.level))" }.joined(separator: ", "))
            }
            if count == 0 {
                Text(lesson.finished ? "This lesson has no steps." : "The tutor is writing the first step…")
                    .font(.callout).foregroundStyle(.secondary)
            } else {
                step(lesson, at)
                controls(lesson, at)
            }
            if lesson.finished, at == count - 1, !lesson.next.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Where next").font(.caption).foregroundStyle(.secondary)
                    ForEach(lesson.next, id: \.title) { offer in
                        Button {
                            tutor.take(offer)
                        } label: {
                            Label(offer.title, systemImage: "arrow.down.right.circle")
                        }
                        .buttonStyle(.link)
                        .disabled(tutor.busy)
                        .help("A lesson at level \(offer.level)")
                    }
                }
            }
            if !lesson.thisRom {
                Text("Made with another ROM: its addresses are that game's.").font(.caption2).foregroundStyle(.tertiary)
            }
        }
    }

    @ViewBuilder
    private func step(_ lesson: LessonInfo, _ i: Int) -> some View {
        let s = lesson.steps[i]
        VStack(alignment: .leading, spacing: 8) {
            Text(s.title).font(.callout.bold())
            if let q = s.predict, !tutor.isRevealed(lesson.id, i) {
                VStack(alignment: .leading, spacing: 6) {
                    Label(q, systemImage: "questionmark.bubble")
                        .font(.callout).italic()
                    Text("Think about it, then:").font(.caption).foregroundStyle(.secondary)
                    Button("Show") { tutor.reveal(lesson.id, i) }.controlSize(.small)
                }
            } else {
                if let q = s.predict {
                    Label(q, systemImage: "questionmark.bubble").font(.caption).italic().foregroundStyle(.secondary)
                }
                MessageText(tutor: tutor, text: s.body)
                if let p = s.picture { LessonPicture(tutor: tutor, lesson: lesson.id, id: p) }
            }
            if let f = s.focus, let words = s.focusText {
                Button {
                    if let url = URL(string: f) { _ = tutor.follow(url) }
                } label: {
                    Label("Show \(words) in Romlens", systemImage: "scope")
                }
                .buttonStyle(.link)
                .font(.caption)
            }
        }
    }

    private func controls(_ lesson: LessonInfo, _ at: Int) -> some View {
        let count = lesson.steps.count
        return HStack {
            Button {
                tutor.show(step: at - 1, of: lesson)
            } label: {
                Label("Back", systemImage: "chevron.left")
            }
            .disabled(at == 0)
            Text("\(at + 1) of \(count)\(lesson.finished ? "" : "…")")
                .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            Button {
                tutor.show(step: at + 1, of: lesson)
            } label: {
                Label("Next", systemImage: "chevron.right")
            }
            .disabled(at + 1 >= count)
            .keyboardShortcut(.rightArrow, modifiers: [.command])
            Spacer()
            if at + 1 < count {
                Button("Skip to the end") { tutor.show(step: count - 1, of: lesson) }
                    .buttonStyle(.link).font(.caption)
            }
        }
        .controlSize(.small)
    }
}

/// A picture a lesson's step shows.
struct LessonPicture: View {
    let tutor: TutorModel
    let lesson: String
    let id: String

    var body: some View {
        if let data = tutor.session?.lessonPicture(lesson: lesson, picture: id), let image = NSImage(data: data) {
            Image(nsImage: image)
                .resizable()
                .interpolation(.none)
                .aspectRatio(contentMode: .fit)
                .frame(maxHeight: 280)
        }
    }
}

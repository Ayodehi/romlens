import RomlensKit
import SwiftUI

/// A quiz (docs/28): one question at a time, each answered and then shown
/// right or not with what it teaches, and at the end what it proved.
struct QuizSheet: View {
    @Bindable var tutor: TutorModel
    @Environment(\.dismiss) private var dismiss
    /// The question shown.
    @State private var at = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let quiz = tutor.quiz {
                header(quiz)
                Dots(quiz: quiz, at: at) { at = $0 }
                Divider()
                ScrollView {
                    Group {
                        if at < quiz.questions.count {
                            QuestionCard(tutor: tutor, question: quiz.questions[at]) {
                                at += 1
                                if at >= quiz.questions.count { tutor.finishQuiz() }
                            }
                            .id(quiz.questions[at].id)
                        } else {
                            Outcome(tutor: tutor, quiz: quiz)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                if let e = tutor.quizError {
                    Text(e).font(.callout).foregroundStyle(.red)
                }
                HStack {
                    if quiz.writing {
                        ProgressView().controlSize(.small)
                        Text("The tutor is writing questions about the game…")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Close") {
                        tutor.finishQuiz()
                        dismiss()
                    }
                    .keyboardShortcut(.cancelAction)
                }
            }
        }
        .padding(20)
        .frame(width: 580, height: 560)
        .onAppear {
            // Pick up where the student left off.
            if let q = tutor.quiz {
                at = q.questions.firstIndex { id in !q.results.contains { $0.question == id.id } } ?? q.questions.count
            }
        }
    }

    private func header(_ q: QuizInfo) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("\(Self.purpose(q.purpose)) \(q.conceptName), level \(q.level)").font(.title3.bold())
            Text(q.levelName).font(.callout).foregroundStyle(.secondary)
        }
    }

    static func purpose(_ p: QuizPurposeInfo) -> String {
        switch p {
        case .prove: "Proving"
        case .review: "Reviewing"
        case .practice: "Practising"
        }
    }
}

/// One dot a question: right, half, wrong, or not yet; the one shown ringed.
private struct Dots: View {
    let quiz: QuizInfo
    let at: Int
    let pick: (Int) -> Void

    var body: some View {
        HStack(spacing: 6) {
            ForEach(Array(quiz.questions.enumerated()), id: \.offset) { i, q in
                let r = quiz.results.first { $0.question == q.id }
                Circle()
                    .fill(Self.colour(r))
                    .frame(width: 12, height: 12)
                    .overlay(Circle().stroke(Color.primary.opacity(i == at ? 0.8 : 0), lineWidth: 1.5).padding(-3))
                    .onTapGesture { if r != nil { pick(i) } }
                    .help(q.prompt)
            }
        }
    }

    static func colour(_ r: AnswerResultInfo?) -> Color {
        guard let r else { return Color.secondary.opacity(0.25) }
        guard let c = r.credit else { return Color.secondary.opacity(0.6) }
        return c >= 1 ? .green : c > 0 ? .orange : .red.opacity(0.8)
    }
}

/// A question: its answer control, the hint, and once answered, the result.
private struct QuestionCard: View {
    let tutor: TutorModel
    let question: QuestionInfo
    let next: () -> Void
    @State private var choice: Int?
    @State private var text = ""
    @State private var bits = Set<UInt8>()
    @State private var hint: String?

    private var result: AnswerResultInfo? { tutor.result(question.id) }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            MessageText(tutor: tutor, text: question.prompt)
                .font(.title3)
            answerControl
                .disabled(result != nil)
            if let hint {
                Label(hint, systemImage: "lightbulb").font(.callout).foregroundStyle(.secondary)
            }
            if let r = result {
                ResultView(tutor: tutor, result: r)
                Button("Next", action: next).keyboardShortcut(.defaultAction)
            } else {
                HStack {
                    Button("Check") { tutor.answer(question.id, given) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(!ready)
                    Button("Skip") { tutor.answer(question.id, .skipped) }
                    if question.hasHint, hint == nil {
                        Button("Hint") { hint = tutor.questionHint(question.id) }
                            .help("A hint halves what the answer earns")
                    }
                }
            }
            Text(question.source).font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var answerControl: some View {
        switch question.kind {
        case .choice(let choices):
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(choices.enumerated()), id: \.offset) { i, c in
                    Button {
                        choice = i
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Image(systemName: choice == i ? "largecircle.fill.circle" : "circle")
                            Text(c).multilineTextAlignment(.leading)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        case .number(let hex):
            TextField(hex ? "A hex number, such as $2100" : "A number", text: $text)
                .textFieldStyle(.roundedBorder)
                .font(.body.monospaced())
                .frame(maxWidth: 240)
                .onSubmit { if ready { tutor.answer(question.id, given) } }
        case .bits(_, let width, _):
            HStack(spacing: 3) {
                ForEach((0..<Int(width)).reversed(), id: \.self) { b in
                    let on = bits.contains(UInt8(b))
                    Button {
                        if on { bits.remove(UInt8(b)) } else { bits.insert(UInt8(b)) }
                    } label: {
                        Text("\(b)")
                            .font(.callout.monospaced())
                            .frame(width: 24, height: 24)
                            .background(RoundedRectangle(cornerRadius: 4).fill(on ? Color.accentColor : Color.secondary.opacity(0.12)))
                            .foregroundStyle(on ? Color.white : Color.primary)
                    }
                    .buttonStyle(.plain)
                }
            }
        case .line(let lines):
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(lines.enumerated()), id: \.offset) { i, l in
                    Button {
                        choice = i
                    } label: {
                        HStack(spacing: 12) {
                            Text(TutorModel.address(l.address)).foregroundStyle(.secondary)
                            Text(l.text)
                        }
                        .font(.callout.monospaced())
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 4).fill(choice == i ? Color.accentColor.opacity(0.25) : .clear))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        case .text:
            TextEditor(text: $text)
                .font(.body)
                .frame(minHeight: 70, maxHeight: 110)
                .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.secondary.opacity(0.3)))
        }
    }

    private var ready: Bool {
        switch question.kind {
        case .choice, .line: choice != nil
        case .number, .text: !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        case .bits: !bits.isEmpty
        }
    }

    private var given: GivenInfo {
        switch question.kind {
        case .choice: .choice(index: UInt32(choice ?? 0))
        case .line: .line(index: UInt32(choice ?? 0))
        case .number: .number(text: text)
        case .bits: .bits(bits: Data(Array(bits).sorted()))
        case .text: .text(text: text)
        }
    }
}

/// Right or not, the right answer, and what it teaches.
private struct ResultView: View {
    let tutor: TutorModel
    let result: AnswerResultInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            switch result.credit {
            case .none:
                Label("The tutor is marking your answer…", systemImage: "hourglass").foregroundStyle(.secondary)
            case .some(let c) where c >= 1:
                Label("Right", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            case .some(let c) where c > 0:
                Label(result.hinted ? "Right, with the hint" : "Partly right", systemImage: "checkmark.circle").foregroundStyle(.orange)
            case .some:
                Label("Not quite: \(result.rightAnswer)", systemImage: "xmark.circle").foregroundStyle(.red)
            }
            if let f = result.feedback { Text(f).font(.callout).italic() }
            MessageText(tutor: tutor, text: result.explanation)
            ForEach(result.cite.filter { $0.hasPrefix("romlens://") }, id: \.self) { link in
                if let url = URL(string: link) {
                    Button("Show it in Romlens") { _ = tutor.follow(url) }.buttonStyle(.link).font(.caption)
                }
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color.secondary.opacity(0.08)))
    }
}

/// What the quiz came to.
private struct Outcome: View {
    let tutor: TutorModel
    let quiz: QuizInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            let o = quiz.outcome
            Text(String(format: "%g of %d", o.score, o.questions)).font(.largeTitle.bold())
            if !o.done {
                Text("Some answers are still being marked.").foregroundStyle(.secondary)
            } else if o.passed {
                Label("Proven: \(quiz.conceptName), level \(quiz.level)", systemImage: "checkmark.seal.fill")
                    .font(.title3).foregroundStyle(.green)
                Text("It comes back for a short review tomorrow, then less and less often.")
                    .foregroundStyle(.secondary)
            } else if quiz.purpose == .prove {
                Text("Not proven yet: a proof takes 4 of 5, with 3 of Romlens's questions right without a hint.")
                Button("Try again with new questions") { tutor.startQuiz(quiz.concept, level: quiz.level) }
            } else if quiz.purpose == .review {
                Text("Reviewed. What you missed comes back sooner.").foregroundStyle(.secondary)
            } else {
                Text("Practice: it proves nothing, but every right answer counts.").foregroundStyle(.secondary)
            }
        }
    }
}

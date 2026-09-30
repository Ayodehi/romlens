import RomlensKit
import SwiftUI

/// `/progress` (docs/28): what the student has proven across the map, the
/// reviews due, and, unless hidden in Settings, points, the rank, the
/// streak and achievements, with this game's milestones.
struct ProgressPane: View {
    let tutor: TutorModel
    @State private var p: ProgressInfo?

    var body: some View {
        ScrollView {
            if let p {
                VStack(alignment: .leading, spacing: 18) {
                    if tutor.settings.showProgress { score(p) }
                    groups(p)
                    due(p)
                    if tutor.settings.showProgress {
                        achievements(p.achievements.filter { !$0.game }, title: "Achievements")
                        achievements(p.achievements.filter(\.game), title: "Found in \(p.achievements.first(where: \.game)?.romTitle ?? "this game")")
                        recent(p)
                    } else {
                        Text("Points and achievements are hidden (Settings › Tutor).")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.trailing, 8)
            } else {
                Text("No record of your learning here yet.").foregroundStyle(.secondary)
            }
        }
        .onAppear {
            _ = tutor.session?.checkMilestones()
            tutor.refreshProgress()
            p = tutor.progress
        }
    }

    private func score(_ p: ProgressInfo) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("\(p.xp)").font(.system(size: 34, weight: .bold)).monospacedDigit()
                Text("points").foregroundStyle(.secondary)
                Spacer()
                VStack(alignment: .trailing, spacing: 1) {
                    Text(p.rank).font(.title3.bold())
                    if p.streak > 0 {
                        Text("\(p.streak)-day streak\(p.bestStreak > p.streak ? " · best \(p.bestStreak)" : "")")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            ProgressView(value: Double(p.corpus), total: Double(max(p.corpusMax, 1))) {
                Text("\(p.corpus) of \(p.corpusMax) levels proven across the map").font(.caption)
            }
            if let next = p.nextRank, let needs = p.nextNeeds {
                Text("Next rank, \(next): \(needs.prefix(1).lowercased() + needs.dropFirst()).")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func groups(_ p: ProgressInfo) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("What you have proven").font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 4) {
                ForEach(p.groups, id: \.name) { g in
                    GridRow {
                        Text(g.name)
                        Text("\(g.proven) of \(g.concepts) proven").foregroundStyle(.secondary)
                        Text("\(g.learned) learned").foregroundStyle(.secondary)
                        Text(g.level > 0 ? "all to level \(g.level)" : "").foregroundStyle(Color.accentColor)
                    }
                    .font(.callout)
                }
            }
        }
    }

    @ViewBuilder private func due(_ p: ProgressInfo) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Ready to review").font(.headline)
                Spacer()
                if !p.due.isEmpty {
                    Button("Review now") { tutor.startQuiz(nil, purpose: .review) }
                }
            }
            if p.due.isEmpty {
                Text("Nothing yet: what you prove comes back after a day, then less often.")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                ForEach(p.due, id: \.concept) { d in
                    Text("\(d.conceptName), level \(d.level)").font(.callout)
                }
            }
        }
    }

    private func achievements(_ list: [AchievementInfo], title: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("\(title)  \(list.filter { $0.unlocked != nil }.count) of \(list.count)").font(.headline)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 200), spacing: 8)], alignment: .leading, spacing: 8) {
                ForEach(list, id: \.id) { a in
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: a.unlocked != nil ? "star.fill" : "star")
                            .foregroundStyle(a.unlocked != nil ? Color.yellow : Color.secondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(a.title).font(.callout.bold())
                            Text(a.unlocked.map { "Earned " + ResumeSheet.date($0) } ?? a.detail)
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .opacity(a.unlocked != nil ? 1 : 0.6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    @ViewBuilder private func recent(_ p: ProgressInfo) -> some View {
        if !p.recent.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text("Latest points").font(.headline)
                ForEach(Array(p.recent.prefix(8).enumerated()), id: \.offset) { _, l in
                    HStack {
                        Text("+\(l.points)").monospacedDigit().frame(width: 44, alignment: .trailing)
                        Text(l.why)
                    }
                    .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }
}

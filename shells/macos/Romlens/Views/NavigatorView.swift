import RomlensKit
import SwiftUI

/// Labels, regions and banks; selecting a row jumps (one way).
struct NavigatorView: View {
    let model: RomViewModel

    var body: some View {
        @Bindable var nav = model.navigator
        VStack(spacing: 0) {
            Picker("Navigator", selection: $nav.tab) {
                ForEach(NavigatorModel.Tab.allCases) { tab in
                    Text(tab.title).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(8)
            Group {
                switch nav.tab {
                case .labels: labels
                case .variables: variables
                case .regions: regions
                case .banks: banks
                }
            }
            .frame(maxHeight: .infinity)
            if nav.tab != .banks {
                TextField("Filter", text: $nav.filter)
                    .textFieldStyle(.roundedBorder)
                    .padding(8)
            }
        }
        .overlay {
            if nav.isLoading && nav.labels.isEmpty {
                ProgressView().controlSize(.small)
            }
        }
    }

    private var labels: some View {
        List(model.navigator.filteredLabels, id: \.address) { label in
            Button {
                model.jump(toSnesAddress: label.address)
            } label: {
                HStack {
                    Text(label.name)
                        .font(.callout.monospaced())
                        .foregroundStyle(label.source == .auto ? .secondary : .primary)
                        .lineLimit(1)
                    Spacer()
                    if label.source == .user || label.source == .imported {
                        Text(label.source == .user ? "user" : label.origin)
                            .font(.caption2)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Capsule().fill(Color.accentColor.opacity(0.2)))
                    }
                    Text(formatSnesAddress(address: label.address))
                        .font(.caption.monospaced())
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .listStyle(.sidebar)
    }

    /// Variables are mostly RAM, which the editor cannot scroll to, so a row
    /// lists what uses the variable; double-click edits it.
    private var variables: some View {
        VStack(spacing: 0) {
            List(model.navigator.filteredVariables, id: \.address) { v in
                Button {
                    model.references.find(to: v.address, in: model.workbench)
                    model.resultsKind = .references
                    model.isResultsVisible = true
                } label: {
                    HStack {
                        Text(v.name.isEmpty ? "(unnamed)" : v.name)
                            .font(.callout.monospaced())
                            .lineLimit(1)
                        Text(v.description)
                            .font(.caption2)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Capsule().fill(Color.accentColor.opacity(0.2)))
                        Spacer()
                        Text(formatSnesAddress(address: v.address))
                            .font(.caption.monospaced())
                            .foregroundStyle(.tertiary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("\(v.memory), \(v.len) byte\(v.len == 1 ? "" : "s"). Click for what uses it; double-click to edit.")
                .simultaneousGesture(TapGesture(count: 2).onEnded { model.beginDefineVariable(at: v.address) })
                .contextMenu {
                    Button("Edit…") { model.beginDefineVariable(at: v.address) }
                    Button("Find References") {
                        model.references.find(to: v.address, in: model.workbench)
                        model.resultsKind = .references
                        model.isResultsVisible = true
                    }
                    Divider()
                    Button("Remove", role: .destructive) { try? model.removeVariable(address: v.address) }
                }
            }
            .listStyle(.sidebar)
            .overlay {
                if model.navigator.variables.isEmpty {
                    Text("No variables. Name a RAM address with Define Variable…, or right-click an instruction to name what it reads or writes.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding()
                }
            }
            HStack {
                Button {
                    model.beginDefineVariable()
                } label: {
                    Label("Define Variable…", systemImage: "plus")
                }
                .buttonStyle(.borderless)
                .controlSize(.small)
                Spacer()
            }
            .padding(.horizontal, 10)
            .padding(.top, 4)
        }
    }

    private var regions: some View {
        VStack(spacing: 0) {
            if model.navigator.regionsTruncated {
                Text("Largest \(NavigatorModel.regionLimit) of each kind")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 4)
                    .help("A classified ROM has far more regions than a list can usefully hold; the overview strip shows all of them.")
            }
            regionList
        }
    }

    private var regionList: some View {
        List(model.navigator.filteredRegions, id: \.start) { region in
            Button {
                model.jump(to: region.start)
            } label: {
                HStack {
                    Text(region.name)
                        .font(.caption)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color(nsColor: RegionPalette.color(
                            kind: region.kind == .code ? .code : .byte,
                            confidence: Double(region.confidence)
                        )).opacity(0.25)))
                    Text("\(formatFileOffset(offset: region.start)) · \(region.len) B")
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text("\(Int((region.confidence * 100).rounded()))%")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .listStyle(.sidebar)
    }

    private var banks: some View {
        List(model.navigator.banks) { bank in
            Button {
                model.jump(to: bank.fileOffset)
            } label: {
                HStack {
                    Text(String(format: "Bank $%02X", bank.bank)).font(.callout.monospaced())
                    Spacer()
                    Text("\(formatFileOffset(offset: bank.fileOffset)) · \(bank.length / 1024) KB")
                        .font(.caption.monospaced())
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .listStyle(.sidebar)
    }
}

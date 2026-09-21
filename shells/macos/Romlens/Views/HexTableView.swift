import AppKit
import SwiftUI

/// The virtualized hex table: an `NSTableView` with one column, a fixed row
/// height and `HexRowView` rows, so 196,608 rows cost nothing until they
/// scroll into view.
struct HexTableView: NSViewRepresentable {
    let model: RomViewModel

    func makeCoordinator() -> Coordinator { Coordinator(model: model) }

    func makeNSView(context: Context) -> NSScrollView {
        let coordinator = context.coordinator
        let table = HexTable()
        table.dataSource = coordinator
        table.delegate = coordinator
        table.headerView = nil
        table.usesAutomaticRowHeights = false
        table.rowHeight = model.layout.rowHeight
        table.intercellSpacing = .zero
        table.selectionHighlightStyle = .none
        table.allowsEmptySelection = true
        table.allowsMultipleSelection = false
        table.backgroundColor = .textBackgroundColor
        table.gridStyleMask = []
        table.focusRingType = .none
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("hex"))
        column.minWidth = model.layout.totalWidth
        column.maxWidth = 4096
        column.width = model.layout.totalWidth
        column.resizingMask = []
        table.addTableColumn(column)
        table.columnAutoresizingStyle = .noColumnAutoresizing
        table.onKeyCommand = { [weak coordinator] command in
            coordinator?.handle(command)
        }

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = true
        scroll.backgroundColor = .textBackgroundColor
        coordinator.table = table
        coordinator.scrollView = scroll
        return scroll
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        let c = context.coordinator
        guard let table = c.table else { return }
        // Every property read here is observed; SwiftUI re-runs this update
        // when one changes.
        let generation = model.lineGeneration
        let selected = model.selectedOffset
        let scroll = model.scrollRequest

        if c.lineGeneration != generation {
            c.lineGeneration = generation
            table.tableColumns.first?.minWidth = model.layout.totalWidth
            table.tableColumns.first?.width = model.layout.totalWidth
            table.rowHeight = model.layout.rowHeight
            table.reloadData()
        }
        if c.selectedOffset != selected {
            var rows = IndexSet()
            if let old = c.selectedOffset { rows.insert(Int(old / 16)) }
            if let new = selected { rows.insert(Int(new / 16)) }
            c.selectedOffset = selected
            let visible = table.rows(in: table.visibleRect)
            let affected = rows.filteredIndexSet { visible.contains($0) }
            if !affected.isEmpty {
                table.reloadData(forRowIndexes: affected, columnIndexes: IndexSet(integer: 0))
            }
        }
        if let scroll, c.lastScrollId != scroll.id {
            c.lastScrollId = scroll.id
            c.scroll(toRow: Int(scroll.row))
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        let model: RomViewModel
        weak var table: HexTable?
        weak var scrollView: NSScrollView?
        var lineGeneration = -1
        var selectedOffset: UInt32?
        var lastScrollId = 0

        init(model: RomViewModel) { self.model = model }

        func numberOfRows(in tableView: NSTableView) -> Int { Int(model.rowCount) }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            let view = (tableView.makeView(withIdentifier: HexRowView.identifier, owner: nil) as? HexRowView) ?? {
                let v = HexRowView()
                v.identifier = HexRowView.identifier
                return v
            }()
            let row32 = UInt32(row)
            let batch = model.batch(containingRow: row32)
            let record = batch.record(row: row32)
            let line = batch.line(row: row32, generation: model.lineGeneration, layout: model.layout)
            var selectedByte: Int?
            if let s = model.selectedOffset, s / 16 == row32 { selectedByte = Int(s % 16) }
            view.configure(record: record, line: line, layout: model.layout, palette: model.palette, selectedByte: selectedByte)
            view.onSelectByte = { [weak self] offset in
                guard let self else { return }
                self.table.map { $0.window?.makeFirstResponder($0) }
                self.model.select(offset: offset)
            }
            return view
        }

        func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }

        /// Scroll so the row sits in the middle of the visible area.
        func scroll(toRow row: Int) {
            guard let table, let scrollView else { return }
            table.scrollRowToVisible(row)
            let rowRect = table.rect(ofRow: row)
            let visibleHeight = scrollView.contentView.bounds.height
            let y = max(0, min(rowRect.midY - visibleHeight / 2, table.bounds.height - visibleHeight))
            scrollView.contentView.scroll(to: NSPoint(x: scrollView.contentView.bounds.origin.x, y: y))
            scrollView.reflectScrolledClipView(scrollView.contentView)
        }

        func handle(_ command: HexTable.KeyCommand) {
            switch command {
            case .left: model.moveSelection(by: -1)
            case .right: model.moveSelection(by: 1)
            case .up: model.moveSelection(by: -16)
            case .down: model.moveSelection(by: 16)
            case .pageUp, .pageDown:
                guard let table, let scrollView else { return }
                let visibleRows = max(1, Int(scrollView.contentView.bounds.height / table.rowHeight) - 1)
                model.moveSelection(by: (command == .pageUp ? -16 : 16) * visibleRows)
            case .home: model.jump(to: 0)
            case .end: model.jump(to: model.byteCount - 1)
            case .back: model.goBack()
            }
        }
    }
}

/// `NSTableView` that turns navigation keys into commands and accepts focus.
final class HexTable: NSTableView {
    enum KeyCommand { case left, right, up, down, pageUp, pageDown, home, end, back }

    var onKeyCommand: ((KeyCommand) -> Void)?

    override var acceptsFirstResponder: Bool { true }

    override func keyDown(with event: NSEvent) {
        let command: KeyCommand?
        switch event.specialKey {
        case .leftArrow: command = .left
        case .rightArrow: command = .right
        case .upArrow: command = .up
        case .downArrow: command = .down
        case .pageUp: command = .pageUp
        case .pageDown: command = .pageDown
        case .home: command = .home
        case .end: command = .end
        case .delete, .backspace: command = .back
        default: command = nil
        }
        if let command {
            onKeyCommand?(command)
        } else {
            super.keyDown(with: event)
        }
    }
}

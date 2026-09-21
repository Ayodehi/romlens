import AppKit
import Foundation
import RomlensKit
import Testing
@testable import Romlens

/// Drives the real window and table off screen: rows are produced through
/// the same data-source path the app uses, drawn into a bitmap, and timed
/// over a scroll through the whole image. Uses the development ROM when
/// `ROMLENS_ROM_DIR` is set, else a 1 MB padded fixture.
@MainActor
@Suite struct HexTableIntegrationTests {
    private func rom() throws -> Rom {
        if let dir = ProcessInfo.processInfo.environment["ROMLENS_ROM_DIR"] {
            let url = URL(fileURLWithPath: dir).appendingPathComponent("SuperMetroid.F8DF.sfc")
            if FileManager.default.fileExists(atPath: url.path) {
                return try Rom.open(path: url.path)
            }
        }
        var bytes = makeTestRom(mapping: .loRom)
        bytes.append(Data(repeating: 0xEA, count: (1 << 20) - bytes.count))
        return try Rom.fromBytes(bytes: bytes, name: "big.sfc")
    }

    private func table(in controller: RomWindowController) throws -> HexTable {
        func find(_ view: NSView) -> HexTable? {
            if let t = view as? HexTable { return t }
            for sub in view.subviews { if let t = find(sub) { return t } }
            return nil
        }
        let content = try #require(controller.window?.contentView)
        content.layoutSubtreeIfNeeded()
        return try #require(find(content))
    }

    @Test func windowShowsEveryRow() throws {
        let model = RomViewModel(rom: try rom())
        let controller = RomWindowController(model: model)
        controller.window?.orderFront(nil)
        let table = try table(in: controller)
        #expect(table.numberOfRows == Int(model.rowCount))
        #expect(table.rowHeight == model.layout.rowHeight)
        let width = try #require(table.tableColumns.first?.width)
        #expect(width >= model.layout.totalWidth - 1, "column wide enough for \(model.layout.totalChars) characters")

        // The visible rows are real HexRowViews built from the cache.
        let visible = table.rows(in: table.visibleRect)
        #expect(visible.length > 10)
        let first = try #require(table.view(atColumn: 0, row: 0, makeIfNecessary: true) as? HexRowView)
        #expect(first.bounds.height == model.layout.rowHeight)

        // Jump lands the header row in view and selects it.
        model.jump(to: model.info.headerOffset)
        // The representable applies scroll requests in updateNSView; drive one.
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        let headerRow = Int(model.info.headerOffset / 16)
        #expect(table.rows(in: table.visibleRect).contains(headerRow))
        #expect(model.selectedOffset == model.info.headerOffset)
        controller.window?.close()
    }

    @Test func scrollingTheWholeImageStaysUnderBudget() throws {
        let model = RomViewModel(rom: try rom())
        let controller = RomWindowController(model: model)
        controller.window?.orderFront(nil)
        let table = try table(in: controller)
        let coordinator = try #require(table.dataSource as? HexTableView.Coordinator)
        let column = table.tableColumns[0]
        let rows = Int(model.rowCount)
        let stride = 40 // rows per "frame"
        let bitmap = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(model.layout.totalWidth), pixelsHigh: Int(model.layout.rowHeight),
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        )!
        var frames = 0
        var worstFrame: TimeInterval = 0
        let start = Date()
        var row = 0
        while row < rows {
            let frameStart = Date()
            for r in row..<min(row + stride, rows) {
                let view = try #require(coordinator.tableView(table, viewFor: column, row: r) as? HexRowView)
                view.frame = NSRect(x: 0, y: 0, width: model.layout.totalWidth, height: model.layout.rowHeight)
                NSGraphicsContext.saveGraphicsState()
                NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
                view.draw(view.bounds)
                NSGraphicsContext.restoreGraphicsState()
            }
            frames += 1
            worstFrame = max(worstFrame, Date().timeIntervalSince(frameStart))
            row += stride
        }
        let total = Date().timeIntervalSince(start)
        let perRow = total / Double(rows) * 1e6
        print("hex table: \(rows) rows in \(String(format: "%.0f", total * 1000)) ms, \(String(format: "%.1f", perRow)) µs/row, worst 40-row frame \(String(format: "%.2f", worstFrame * 1000)) ms, \(model.cache.missCount) batch misses")
        // A 120 Hz frame is 8.3 ms; formatting and drawing 40 fresh rows must
        // fit with room for AppKit's own work.
        #expect(worstFrame < 0.008, "worst frame \(worstFrame * 1000) ms")
        #expect(model.cache.missCount == (rows + 255) / 256)
        controller.window?.close()
    }
}

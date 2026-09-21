import Foundation
import Observation
import RomlensKit

/// Per-document state shared by the hex table, inspector and jump sheet.
@MainActor
@Observable
final class RomViewModel {
    struct ScrollRequest: Equatable {
        let id: Int
        let row: UInt32
    }

    let rom: Rom
    let info: RomInfo
    let spans: [Span]
    let palette: SpanPalette
    let rowCount: UInt32
    @ObservationIgnored let cache: HexRowCache

    var addressStyle: AddressStyle = .both {
        didSet {
            guard addressStyle != oldValue else { return }
            layout = HexRowLayout(style: addressStyle, font: layout.font)
            lineGeneration += 1
        }
    }
    private(set) var layout: HexRowLayout
    /// Bumped whenever cached `CTLine`s must be rebuilt.
    private(set) var lineGeneration = 0

    private(set) var selectedOffset: UInt32?
    private(set) var inspection: ByteInterpretation?
    private(set) var history: [UInt32] = []
    private(set) var scrollRequest: ScrollRequest?
    var isShowingJumpSheet = false

    init(rom: Rom, cacheCapacity: Int = 64) {
        self.rom = rom
        info = rom.info()
        spans = rom.spans()
        palette = SpanPalette(spans: spans)
        rowCount = rom.rowCount()
        cache = HexRowCache(rom: rom, capacity: cacheCapacity)
        layout = HexRowLayout(style: .both)
    }

    var byteCount: UInt32 { info.byteLen }
    var canGoBack: Bool { !history.isEmpty }

    func batch(containingRow row: UInt32) -> HexBatch {
        cache.batch(containingRow: row)
    }

    /// Select a byte without scrolling (mouse, arrow keys).
    func select(offset: UInt32?) {
        guard let offset else {
            selectedOffset = nil
            inspection = nil
            return
        }
        guard offset < byteCount else { return }
        selectedOffset = offset
        inspection = rom.inspect(fileOffset: offset)
    }

    /// Move the selection by `delta` bytes, clamped, and keep it visible.
    func moveSelection(by delta: Int) {
        let current = Int(selectedOffset ?? 0)
        let next = min(max(current + delta, 0), Int(byteCount) - 1)
        select(offset: UInt32(next))
        requestScroll(toRow: UInt32(next) / 16)
    }

    /// Jump to a file offset: select it, centre it, remember where we were.
    func jump(to offset: UInt32, recordHistory: Bool = true) {
        guard offset < byteCount else { return }
        if recordHistory, let from = selectedOffset, from != offset {
            history.append(from)
            if history.count > 100 { history.removeFirst() }
        }
        select(offset: offset)
        requestScroll(toRow: offset / 16)
    }

    /// Jump to a 24-bit SNES address when it maps to ROM.
    func jump(toSnesAddress address: UInt32) {
        if let offset = rom.fileOffsetFor(snesAddress: address) {
            jump(to: offset)
        }
    }

    /// Resolve and jump to an address expression; throws the core's message.
    @discardableResult
    func jump(text: String) throws -> ResolvedAddress {
        let resolved = try rom.resolve(text: text)
        jump(to: resolved.fileOffset)
        return resolved
    }

    /// Live preview for the jump sheet: what the expression resolves to.
    func preview(text: String) -> Result<ResolvedAddress, RomlensError> {
        do {
            return .success(try rom.resolve(text: text))
        } catch let error as RomlensError {
            return .failure(error)
        } catch {
            return .failure(.BadAddress(msg: error.localizedDescription))
        }
    }

    func goBack() {
        guard let previous = history.popLast() else { return }
        jump(to: previous, recordHistory: false)
    }

    private func requestScroll(toRow row: UInt32) {
        scrollRequest = ScrollRequest(id: (scrollRequest?.id ?? 0) + 1, row: row)
    }
}

import AppKit
import UniformTypeIdentifiers
import RomlensKit
import SwiftUI

/// One window per document. Hosts the SwiftUI `DocumentView` and answers the
/// menu and context-menu actions that travel the responder chain.
final class RomWindowController: NSWindowController, NSMenuItemValidation {
    let model: RomViewModel

    init(model: RomViewModel) {
        self.model = model
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1440, height: 860),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered, defer: false
        )
        window.minSize = NSSize(width: 900, height: 480)
        window.toolbarStyle = .unified
        window.tabbingMode = .disallowed
        super.init(window: window)
        shouldCascadeWindows = true
        let hosting = NSHostingController(rootView: DocumentView(model: model))
        hosting.sceneBridgingOptions = [.toolbars]
        window.contentViewController = hosting
        window.subtitle = Self.subtitle(for: model.info)
        RecordingController.reattach(model: model)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    /// The titlebar subtitle. Short enough not to truncate at the window's
    /// minimum width, and a size in the units a person thinks in — the exact
    /// byte count is in the inspector, where there is room for it.
    static func subtitle(for info: RomInfo) -> String {
        let megabytes = Double(info.byteLen) / (1024 * 1024)
        let size = megabytes >= 1
            ? String(format: "%.3g MB", megabytes)
            : "\(info.byteLen / 1024) KB"
        return "\(info.mappingName) · \(info.fastRom ? "FastROM" : "SlowROM") · \(size)"
    }

    private var projectDocument: ProjectDocument? { document as? ProjectDocument }

    // MARK: Navigation

    @objc func jumpToAddress(_ sender: Any?) { model.activeSheet = .jump }
    @objc func find(_ sender: Any?) { model.activeSheet = .find }
    @objc func findNext(_ sender: Any?) { model.stepSearch(by: 1) }
    @objc func findPrevious(_ sender: Any?) { model.stepSearch(by: -1) }
    @objc func followReference(_ sender: Any?) { model.followReference() }
    @objc func findReferences(_ sender: Any?) { model.findReferences() }
    @objc func goBack(_ sender: Any?) { model.goBack() }
    @objc func goForward(_ sender: Any?) { model.goForward() }
    @objc func goToHeader(_ sender: Any?) { model.jump(to: model.info.headerOffset) }
    @objc func goToReset(_ sender: Any?) {
        if let offset = model.rom.fileOffsetFor(snesAddress: UInt32(model.info.emulation.reset)) {
            model.jump(to: offset)
        }
    }

    // MARK: View

    @objc func showBothAddresses(_ sender: Any?) { model.addressStyle = .both }
    @objc func showSnesAddresses(_ sender: Any?) { model.addressStyle = .snes }
    @objc func showFileOffsets(_ sender: Any?) { model.addressStyle = .file }
    @objc func showHex(_ sender: Any?) { model.editorTab = .hex }
    @objc func showDisassembly(_ sender: Any?) { model.editorTab = .disassembly }
    @objc func showBoth(_ sender: Any?) { model.editorTab = .both }
    @objc func showC(_ sender: Any?) { model.editorTab = .c }
    @objc func decompileRoutine(_ sender: Any?) { model.showDecompiled() }
    @objc func showGraph(_ sender: Any?) { model.showGraph() }
    @objc func showSource(_ sender: Any?) { model.editorTab = .source }
    @objc func showAtlas(_ sender: Any?) { model.editorTab = .atlas }
    @objc func showCompare(_ sender: Any?) { model.editorTab = .compare }
    @objc func compareWith(_ sender: Any?) { CompareController.open(model: model, window: window) }
    /// View › Show Explanations, remembered for the next window.
    @objc func toggleExplanations(_ sender: Any?) {
        model.showExplanations.toggle()
        UserDefaults.standard.set(!model.showExplanations, forKey: RomViewModel.hideExplanationsKey)
    }
    /// Zoom In, Zoom Out and Zoom to Fit act on the Graph or the Atlas,
    /// whichever shows.
    @objc func zoomGraphIn(_ sender: Any?) {
        if model.editorTab == .atlas { model.atlas.requestZoom(.zoomIn) } else { model.graph.requestZoom(.zoomIn) }
    }
    @objc func zoomGraphOut(_ sender: Any?) {
        if model.editorTab == .atlas { model.atlas.requestZoom(.zoomOut) } else { model.graph.requestZoom(.zoomOut) }
    }
    @objc func zoomGraphToFit(_ sender: Any?) {
        if model.editorTab == .atlas { model.atlas.requestZoom(.fit) } else { model.graph.requestZoom(.fit) }
    }

    /// Export C…: the routine's translation unit and the snes.h it
    /// includes, side by side.
    @objc func exportC(_ sender: Any?) {
        guard let result = model.decompiler.result, let window else { return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(result.name).c"
        panel.allowedContentTypes = [.init(filenameExtension: "c") ?? .sourceCode]
        panel.message = "snes.h is written beside it."
        panel.beginSheetModal(for: window) { response in
            guard response == .OK, let url = panel.url else { return }
            do {
                try result.text.write(to: url, atomically: true, encoding: .utf8)
                let header = url.deletingLastPathComponent().appendingPathComponent("snes.h")
                try snesHeader().write(to: header, atomically: true, encoding: .utf8)
            } catch {
                NSAlert(error: error).beginSheetModal(for: window)
            }
        }
    }
    @objc func showFrame(_ sender: Any?) { model.openGraphics(.frame) }
    @objc func showLayers(_ sender: Any?) { model.openGraphics(.layers) }
    @objc func showTileDecoder(_ sender: Any?) { model.openGraphics(.tiles) }
    @objc func showPalette(_ sender: Any?) { model.openGraphics(.palette) }
    @objc func showOam(_ sender: Any?) { model.openGraphics(.oam) }
    @objc func showTilemap(_ sender: Any?) { model.openGraphics(.tilemap) }

    @objc func openRecording(_ sender: Any?) {
        RecordingController.open(model: model, window: window)
    }

    @objc func closeRecording(_ sender: Any?) { RecordingController.close(model: model) }
    @objc func toggleLiveSession(_ sender: Any?) { LiveController.toggle(model: model, window: window) }
    @objc func importSnapshot(_ sender: Any?) { RecordingController.importSnapshot(model: model, window: window) }
    @objc func exportFrameRegion(_ sender: Any?) { RecordingController.exportFrameRegion(model: model, window: window) }
    @objc func saveRecorderScript(_ sender: Any?) { RecordingController.saveRecorderScript(window: window) }
    // Explicit animations: `DocumentView` binds these without one so the
    // framework's own re-application on window activation cannot slide the
    // content.
    @objc func toggleNavigator(_ sender: Any?) { withAnimation { model.isNavigatorVisible.toggle() } }
    @objc func toggleInspector(_ sender: Any?) { withAnimation { model.isInspectorVisible.toggle() } }
    @objc func toggleStrip(_ sender: Any?) { withAnimation { model.isStripVisible.toggle() } }
    @objc func toggleFocus(_ sender: Any?) { withAnimation { model.toggleFocus() } }
    @objc func toggleResults(_ sender: Any?) { withAnimation { model.isResultsVisible.toggle() } }

    // MARK: Editing

    @objc func undo(_ sender: Any?) { model.undo() }
    @objc func redo(_ sender: Any?) { model.redo() }
    @objc func renameLabel(_ sender: Any?) { if model.selectedOffset != nil { model.activeSheet = .renameLabel } }
    @objc func removeLabel(_ sender: Any?) { try? model.removeLabel() }
    @objc func defineVariable(_ sender: Any?) { model.beginDefineVariable() }
    @objc func editComment(_ sender: Any?) { if model.selectedOffset != nil { model.activeSheet = .comment } }
    @objc func markAsCode(_ sender: Any?) { model.mark(.code) }
    @objc func markAsData(_ sender: Any?) { model.mark(.data) }
    @objc func markAsUnknown(_ sender: Any?) { model.mark(.unknown) }
    @objc func markAsDataWithOptions(_ sender: Any?) {
        if model.highlightedRange != nil { model.activeSheet = .dataType }
    }
    @objc func markAsString(_ sender: Any?) { model.mark(.data, dataKind: .string) }
    @objc func markAsWord(_ sender: Any?) { model.mark(.data, dataKind: .word) }
    @objc func markAsPointer(_ sender: Any?) {
        model.mark(.data, dataKind: .pointer, bank: .sameBank)
    }
    @objc func markAsGraphics(_ sender: Any?) { model.mark(.data, dataKind: .graphics, bpp: 4) }
    @objc func markAsPalette(_ sender: Any?) { model.mark(.data, dataKind: .palette) }
    @objc func markAsTilemap(_ sender: Any?) { model.mark(.data, dataKind: .tilemap) }
    @objc func markAsCompressed(_ sender: Any?) { model.mark(.data, dataKind: .compressed) }
    @objc func clearMark(_ sender: Any?) { model.clearMark() }
    @objc func setFlags(_ sender: Any?) { if model.selectedOffset != nil { model.activeSheet = .flags } }

    @objc func copyAddress(_ sender: Any?) {
        guard let text = model.selectedAddressText else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    @objc func copyLine(_ sender: Any?) {
        guard let text = model.selectedLineText else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    // MARK: Export

    @objc func exportAssembly(_ sender: Any?) { export(.assembly) }
    @objc func exportAnnotations(_ sender: Any?) { export(.annotations) }
    @objc func exportSymbols(_ sender: Any?) { export(.symbols) }

    @objc func importTrace(_ sender: Any?) { importFile(.trace) }
    @objc func importSymbols(_ sender: Any?) { importFile(.symbols) }
    @objc func importDbg(_ sender: Any?) { importFile(.dbg) }

    private func importFile(_ kind: ImportController.Kind) {
        guard let document = projectDocument else { return }
        ImportController.run(kind, document: document, window: window)
    }

    private func export(_ kind: ExportController.Kind) {
        guard let document = projectDocument else { return }
        ExportController.run(kind, document: document, window: window)
    }

    // MARK: Validation

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        let hasSelection = model.selectedOffset != nil
        switch item.action {
        case #selector(showBothAddresses(_:)):
            item.state = model.addressStyle == .both ? .on : .off
        case #selector(showSnesAddresses(_:)):
            item.state = model.addressStyle == .snes ? .on : .off
        case #selector(showFileOffsets(_:)):
            item.state = model.addressStyle == .file ? .on : .off
        case #selector(showHex(_:)):
            item.state = model.graphicsTab == nil && model.editorTab == .hex ? .on : .off
        case #selector(showDisassembly(_:)):
            item.state = model.graphicsTab == nil && model.editorTab == .disassembly ? .on : .off
        case #selector(showBoth(_:)):
            item.state = model.graphicsTab == nil && model.editorTab == .both ? .on : .off
        case #selector(showC(_:)):
            item.state = model.graphicsTab == nil && model.editorTab == .c ? .on : .off
            return model.hasDisassembly
        case #selector(toggleExplanations(_:)):
            item.state = model.showExplanations ? .on : .off
            return true
        case #selector(showGraph(_:)):
            item.state = model.graphicsTab == nil && model.editorTab == .graph ? .on : .off
            return model.hasDisassembly
        case #selector(showAtlas(_:)):
            item.state = model.graphicsTab == nil && model.editorTab == .atlas ? .on : .off
        case #selector(showCompare(_:)):
            item.state = model.graphicsTab == nil && model.editorTab == .compare ? .on : .off
            return model.compare.isActive
        case #selector(showSource(_:)):
            item.state = model.graphicsTab == nil && model.editorTab == .source ? .on : .off
            return model.source.hasFiles
        case #selector(zoomGraphIn(_:)), #selector(zoomGraphOut(_:)), #selector(zoomGraphToFit(_:)):
            return model.graphicsTab == nil && (model.editorTab == .graph || model.editorTab == .atlas)
        case #selector(decompileRoutine(_:)):
            return model.hasDisassembly && model.instruction != nil
        case #selector(exportC(_:)):
            return model.decompiler.result != nil
        case #selector(showFrame(_:)):
            item.state = model.graphicsTab == .frame ? .on : .off
        case #selector(showLayers(_:)):
            item.state = model.graphicsTab == .layers ? .on : .off
        case #selector(showTileDecoder(_:)):
            item.state = model.graphicsTab == .tiles ? .on : .off
        case #selector(showPalette(_:)):
            item.state = model.graphicsTab == .palette ? .on : .off
        case #selector(showOam(_:)):
            item.state = model.graphicsTab == .oam ? .on : .off
        case #selector(showTilemap(_:)):
            item.state = model.graphicsTab == .tilemap ? .on : .off
        case #selector(closeRecording(_:)), #selector(exportFrameRegion(_:)):
            return model.graphics.hasRecording
        case #selector(toggleLiveSession(_:)):
            item.title = model.graphics.isLive ? "Stop Live Session" : "Start Live Session"
        case #selector(toggleFocus(_:)):
            item.state = model.isFocused ? .on : .off
        case #selector(toggleNavigator(_:)):
            item.title = model.isNavigatorVisible ? "Hide Navigator" : "Show Navigator"
        case #selector(toggleInspector(_:)):
            item.title = model.isInspectorVisible ? "Hide Inspector" : "Show Inspector"
        case #selector(goBack(_:)):
            return model.canGoBack
        case #selector(goForward(_:)):
            return model.canGoForward
        case #selector(undo(_:)):
            item.title = model.session.undoTitle.map { "Undo \($0)" } ?? "Undo"
            return model.session.canUndo
        case #selector(redo(_:)):
            item.title = model.session.redoTitle.map { "Redo \($0)" } ?? "Redo"
            return model.session.canRedo
        case #selector(followReference(_:)):
            return model.instruction?.targetFileOffset != nil || model.inspection?.pointerTargetFileOffset != nil
        case #selector(renameLabel(_:)), #selector(editComment(_:)), #selector(markAsCode(_:)),
             #selector(markAsData(_:)), #selector(markAsUnknown(_:)), #selector(clearMark(_:)),
             #selector(setFlags(_:)), #selector(copyAddress(_:)), #selector(copyLine(_:)),
             #selector(markAsDataWithOptions(_:)), #selector(markAsString(_:)),
             #selector(markAsWord(_:)), #selector(markAsPointer(_:)), #selector(markAsGraphics(_:)),
             #selector(markAsPalette(_:)), #selector(markAsTilemap(_:)),
             #selector(markAsCompressed(_:)):
            return hasSelection
        case #selector(findNext(_:)), #selector(findPrevious(_:)):
            return model.search.hasResults
        case #selector(removeLabel(_:)):
            if let label = model.label, model.canRemoveLabel {
                item.title = "Remove Label \(label.name)"
                return true
            }
            item.title = "Remove Label"
            return false
        case #selector(defineVariable(_:)):
            // From an instruction that reads or writes memory, name that
            // address; otherwise start from an empty sheet.
            if let at = model.operandAddress {
                let existing = model.workbench.variableContaining(snesAddress: at)
                item.title = existing.map { "Edit Variable \($0.name)…" }
                    ?? "Define Variable at \(formatSnesAddress(address: at))…"
            } else {
                item.title = "Define Variable…"
            }
            return true
        case #selector(findReferences(_:)):
            // Named for what it will look for, so a right-click on a line
            // says whose references it lists and how many there are.
            guard let target = model.referenceTarget else {
                item.title = "Find References"
                return false
            }
            item.title = "Find References to \(target.name) (\(target.count))"
            return target.count > 0
        case #selector(toggleResults(_:)):
            item.title = model.isResultsVisible ? "Hide Results" : "Show Results"
            return model.search.hasResults || model.references.hasResults
        case #selector(toggleStrip(_:)):
            item.title = model.isStripVisible ? "Hide Overview Strip" : "Show Overview Strip"
            return true
        case #selector(exportAssembly(_:)), #selector(exportAnnotations(_:)), #selector(exportSymbols(_:)):
            return model.hasDisassembly
        default:
            break
        }
        return true
    }
}

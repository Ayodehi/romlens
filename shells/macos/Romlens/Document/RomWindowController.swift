import AppKit
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
        window.subtitle = "\(model.info.mappingName)\(model.info.fastRom ? ", FastROM" : "") · \(model.info.byteLen) bytes"
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    private var projectDocument: ProjectDocument? { document as? ProjectDocument }

    // MARK: Navigation

    @objc func jumpToAddress(_ sender: Any?) { model.activeSheet = .jump }
    @objc func followReference(_ sender: Any?) { model.followReference() }
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
    @objc func toggleNavigator(_ sender: Any?) { model.isNavigatorVisible.toggle() }
    @objc func toggleInspector(_ sender: Any?) { model.isInspectorVisible.toggle() }

    // MARK: Editing

    @objc func undo(_ sender: Any?) { model.undo() }
    @objc func redo(_ sender: Any?) { model.redo() }
    @objc func renameLabel(_ sender: Any?) { if model.selectedOffset != nil { model.activeSheet = .renameLabel } }
    @objc func editComment(_ sender: Any?) { if model.selectedOffset != nil { model.activeSheet = .comment } }
    @objc func markAsCode(_ sender: Any?) { model.mark(.code) }
    @objc func markAsData(_ sender: Any?) { model.mark(.data) }
    @objc func markAsUnknown(_ sender: Any?) { model.mark(.unknown) }
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
            item.state = model.editorTab == .hex ? .on : .off
        case #selector(showDisassembly(_:)):
            item.state = model.editorTab == .disassembly ? .on : .off
        case #selector(showBoth(_:)):
            item.state = model.editorTab == .both ? .on : .off
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
             #selector(setFlags(_:)), #selector(copyAddress(_:)), #selector(copyLine(_:)):
            return hasSelection
        case #selector(exportAssembly(_:)), #selector(exportAnnotations(_:)), #selector(exportSymbols(_:)):
            return model.hasDisassembly
        default:
            break
        }
        return true
    }
}

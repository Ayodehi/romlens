import AppKit
import RomlensKit
import SwiftUI

/// One window per document. Hosts the SwiftUI `DocumentView` and answers the
/// menu actions that travel the responder chain.
final class RomWindowController: NSWindowController, NSMenuItemValidation {
    let model: RomViewModel

    init(model: RomViewModel) {
        self.model = model
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1080, height: 720),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered, defer: false
        )
        window.minSize = NSSize(width: 720, height: 400)
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

    // MARK: Menu actions

    @objc func jumpToAddress(_ sender: Any?) { model.isShowingJumpSheet = true }
    @objc func goBack(_ sender: Any?) { model.goBack() }
    @objc func goToHeader(_ sender: Any?) { model.jump(to: model.info.headerOffset) }
    @objc func goToReset(_ sender: Any?) {
        if let offset = model.rom.fileOffsetFor(snesAddress: UInt32(model.info.emulation.reset)) {
            model.jump(to: offset)
        }
    }
    @objc func showBothAddresses(_ sender: Any?) { model.addressStyle = .both }
    @objc func showSnesAddresses(_ sender: Any?) { model.addressStyle = .snes }
    @objc func showFileOffsets(_ sender: Any?) { model.addressStyle = .file }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(showBothAddresses(_:)):
            item.state = model.addressStyle == .both ? .on : .off
        case #selector(showSnesAddresses(_:)):
            item.state = model.addressStyle == .snes ? .on : .off
        case #selector(showFileOffsets(_:)):
            item.state = model.addressStyle == .file ? .on : .off
        case #selector(goBack(_:)):
            return model.canGoBack
        default:
            break
        }
        return true
    }
}

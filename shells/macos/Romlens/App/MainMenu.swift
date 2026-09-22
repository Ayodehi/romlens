import AppKit

/// The main menu, built in code so there is no nib to maintain. Actions with a
/// nil target travel the responder chain to `RomWindowController`.
enum MainMenu {
    static func build() -> NSMenu {
        let main = NSMenu()
        main.addItem(appMenu())
        main.addItem(fileMenu())
        main.addItem(editMenu())
        main.addItem(viewMenu())
        main.addItem(goMenu())
        main.addItem(windowMenu())
        main.addItem(helpMenu())
        return main
    }

    private static func submenu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        let menu = NSMenu(title: title)
        items.forEach(menu.addItem)
        item.submenu = menu
        return item
    }

    private static func item(
        _ title: String, _ action: Selector?, _ key: String = "",
        modifiers: NSEvent.ModifierFlags = .command
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = key.isEmpty ? [] : modifiers
        return item
    }

    private static func appMenu() -> NSMenuItem {
        let name = ProcessInfo.processInfo.processName
        return submenu(name, [
            item("About \(name)", #selector(AppDelegate.showAboutPanel(_:))),
            .separator(),
            item("Hide \(name)", #selector(NSApplication.hide(_:)), "h"),
            item("Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h", modifiers: [.command, .option]),
            item("Show All", #selector(NSApplication.unhideAllApplications(_:))),
            .separator(),
            item("Quit \(name)", #selector(NSApplication.terminate(_:)), "q"),
        ])
    }

    private static func fileMenu() -> NSMenuItem {
        let recent = submenu("Open Recent", [
            item("Clear Menu", #selector(NSDocumentController.clearRecentDocuments(_:))),
        ])
        let export = submenu("Export", [
            item("Assembly Listing…", #selector(RomWindowController.exportAssembly(_:))),
            item("Labels and Comments…", #selector(RomWindowController.exportAnnotations(_:))),
            item("Symbol File…", #selector(RomWindowController.exportSymbols(_:))),
        ])
        let importMenu = submenu("Import", [
            item("Execution Trace…", #selector(RomWindowController.importTrace(_:))),
            item("Symbols…", #selector(RomWindowController.importSymbols(_:))),
        ])
        return submenu("File", [
            item("Open…", #selector(NSDocumentController.openDocument(_:)), "o"),
            recent,
            .separator(),
            item("Close", #selector(NSWindow.performClose(_:)), "w"),
            item("Save", #selector(NSDocument.save(_:)), "s"),
            item("Save As…", #selector(NSDocument.saveAs(_:)), "S", modifiers: [.command, .shift]),
            item("Duplicate", #selector(NSDocument.duplicate(_:)), "s", modifiers: [.command, .shift, .option]),
            item("Revert to Saved", #selector(NSDocument.revertToSaved(_:))),
            .separator(),
            importMenu,
            export,
        ])
    }

    private static func editMenu() -> NSMenuItem {
        submenu("Edit", [
            item("Undo", #selector(RomWindowController.undo(_:)), "z"),
            item("Redo", #selector(RomWindowController.redo(_:)), "Z", modifiers: [.command, .shift]),
            .separator(),
            item("Cut", #selector(NSText.cut(_:)), "x"),
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Paste", #selector(NSText.paste(_:)), "v"),
            item("Select All", #selector(NSText.selectAll(_:)), "a"),
            .separator(),
            item("Rename Label…", #selector(RomWindowController.renameLabel(_:))),
            item("Comment…", #selector(RomWindowController.editComment(_:))),
            .separator(),
            markAsMenu(),
            item("Clear Mark", #selector(RomWindowController.clearMark(_:))),
            item("Set Flags…", #selector(RomWindowController.setFlags(_:))),
            .separator(),
            item("Copy Address", #selector(RomWindowController.copyAddress(_:)), "c", modifiers: [.command, .option]),
            item("Copy Line", #selector(RomWindowController.copyLine(_:)), "c", modifiers: [.command, .shift]),
        ])
    }

    /// Every kind reachable in one gesture, with the parameterised ones
    /// behind a sheet. The three Phase 1 items keep their wording and their
    /// place at the top, so existing muscle memory is untouched.
    private static func markAsMenu() -> NSMenuItem {
        var items = [
            item("Code", #selector(RomWindowController.markAsCode(_:))),
            item("Data", #selector(RomWindowController.markAsData(_:))),
            item("Unknown", #selector(RomWindowController.markAsUnknown(_:))),
            .separator(),
            item("Data…", #selector(RomWindowController.markAsDataWithOptions(_:))),
            .separator(),
        ]
        items.append(contentsOf: [
            item("String", #selector(RomWindowController.markAsString(_:))),
            item("Word", #selector(RomWindowController.markAsWord(_:))),
            item("Pointer", #selector(RomWindowController.markAsPointer(_:))),
            item("Graphics", #selector(RomWindowController.markAsGraphics(_:))),
            item("Palette", #selector(RomWindowController.markAsPalette(_:))),
            item("Tilemap", #selector(RomWindowController.markAsTilemap(_:))),
            item("Compressed", #selector(RomWindowController.markAsCompressed(_:))),
        ])
        return submenu("Mark as", items)
    }

    private static func viewMenu() -> NSMenuItem {
        submenu("View", [
            item("Hex", #selector(RomWindowController.showHex(_:)), "1", modifiers: [.command, .option]),
            item("Disassembly", #selector(RomWindowController.showDisassembly(_:)), "2", modifiers: [.command, .option]),
            item("Both", #selector(RomWindowController.showBoth(_:)), "3", modifiers: [.command, .option]),
            .separator(),
            item("File Offset and SNES Address", #selector(RomWindowController.showBothAddresses(_:)), "1"),
            item("SNES Address Only", #selector(RomWindowController.showSnesAddresses(_:)), "2"),
            item("File Offset Only", #selector(RomWindowController.showFileOffsets(_:)), "3"),
            .separator(),
            item("Show Navigator", #selector(RomWindowController.toggleNavigator(_:)), "0"),
            item("Show Inspector", #selector(RomWindowController.toggleInspector(_:)), "0", modifiers: [.command, .option]),
            item("Show Overview Strip", #selector(RomWindowController.toggleStrip(_:)), "0", modifiers: [.command, .shift]),
            item("Show Find Results", #selector(RomWindowController.toggleResults(_:)), "0", modifiers: [.command, .control]),
            .separator(),
            item("Enter Full Screen", #selector(NSWindow.toggleFullScreen(_:)), "f", modifiers: [.command, .control]),
        ])
    }

    private static func goMenu() -> NSMenuItem {
        submenu("Go", [
            item("Find…", #selector(RomWindowController.find(_:)), "f"),
            item("Find Next", #selector(RomWindowController.findNext(_:)), "g"),
            item("Find Previous", #selector(RomWindowController.findPrevious(_:)), "G", modifiers: [.command, .shift]),
            .separator(),
            item("Jump to Address…", #selector(RomWindowController.jumpToAddress(_:)), "l"),
            item("Follow Reference", #selector(RomWindowController.followReference(_:)), "\r"),
            item("Back", #selector(RomWindowController.goBack(_:)), "["),
            item("Forward", #selector(RomWindowController.goForward(_:)), "]"),
            .separator(),
            item("Header", #selector(RomWindowController.goToHeader(_:)), "h", modifiers: [.command, .shift]),
            item("Reset Vector", #selector(RomWindowController.goToReset(_:)), "r", modifiers: [.command, .shift]),
        ])
    }

    private static func windowMenu() -> NSMenuItem {
        let menu = submenu("Window", [
            item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"),
            item("Zoom", #selector(NSWindow.performZoom(_:))),
            .separator(),
            item("Bring All to Front", #selector(NSApplication.arrangeInFront(_:))),
        ])
        NSApp.windowsMenu = menu.submenu
        return menu
    }

    private static func helpMenu() -> NSMenuItem {
        let menu = submenu("Help", [
            item("Romlens Help", #selector(NSApplication.showHelp(_:)), "?"),
        ])
        NSApp.helpMenu = menu.submenu
        return menu
    }
}

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
        item.keyEquivalentModifierMask = modifiers
        return item
    }

    private static func appMenu() -> NSMenuItem {
        let name = ProcessInfo.processInfo.processName
        return submenu(name, [
            item("About \(name)", #selector(NSApplication.orderFrontStandardAboutPanel(_:))),
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
        return submenu("File", [
            item("Open…", #selector(NSDocumentController.openDocument(_:)), "o"),
            recent,
            .separator(),
            item("Close", #selector(NSWindow.performClose(_:)), "w"),
        ])
    }

    private static func editMenu() -> NSMenuItem {
        submenu("Edit", [
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Select All", #selector(NSText.selectAll(_:)), "a"),
        ])
    }

    private static func viewMenu() -> NSMenuItem {
        submenu("View", [
            item("File Offset and SNES Address", #selector(RomWindowController.showBothAddresses(_:)), "1"),
            item("SNES Address Only", #selector(RomWindowController.showSnesAddresses(_:)), "2"),
            item("File Offset Only", #selector(RomWindowController.showFileOffsets(_:)), "3"),
            .separator(),
            item("Enter Full Screen", #selector(NSWindow.toggleFullScreen(_:)), "f", modifiers: [.command, .control]),
        ])
    }

    private static func goMenu() -> NSMenuItem {
        submenu("Go", [
            item("Jump to Address…", #selector(RomWindowController.jumpToAddress(_:)), "l"),
            item("Back", #selector(RomWindowController.goBack(_:)), "["),
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

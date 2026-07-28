import AppKit

@MainActor
final class SpoolMenuDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = NSImage(
            systemSymbolName: "printer.fill",
            accessibilityDescription: "Spool"
        )

        let menu = NSMenu()
        let status = NSMenuItem(title: "Agent status unavailable", action: nil, keyEquivalent: "")
        status.isEnabled = false
        menu.addItem(status)
        menu.addItem(.separator())
        menu.addItem(
            withTitle: "Open Spool",
            action: #selector(openDashboard),
            keyEquivalent: "o"
        ).target = self
        menu.addItem(
            withTitle: "Quit Menu",
            action: #selector(quitMenu),
            keyEquivalent: "q"
        ).target = self
        item.menu = menu
        statusItem = item
    }

    @objc private func openDashboard() {
        guard let url = URL(string: "http://127.0.0.1:39100") else { return }
        NSWorkspace.shared.open(url)
    }

    @objc private func quitMenu() {
        // Quitting the disposable shell deliberately leaves the agent running.
        NSApplication.shared.terminate(nil)
    }
}

let application = NSApplication.shared
let delegate = SpoolMenuDelegate()
application.delegate = delegate
application.setActivationPolicy(.accessory)
application.run()


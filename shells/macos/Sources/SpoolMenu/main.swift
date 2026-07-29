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
        if dashboardURL() != nil {
            menu.addItem(.separator())
            menu.addItem(
                withTitle: "Open Spool",
                action: #selector(openDashboard),
                keyEquivalent: "o"
            ).target = self
        }
        menu.addItem(.separator())
        menu.addItem(
            withTitle: "Quit Menu",
            action: #selector(quitMenu),
            keyEquivalent: "q"
        ).target = self
        item.menu = menu
        statusItem = item
    }

    @objc private func openDashboard() {
        guard let url = dashboardURL() else { return }
        NSWorkspace.shared.open(url)
    }

    private func dashboardURL() -> URL? {
        guard
            let value = ProcessInfo.processInfo.environment["SPOOL_DASHBOARD_URL"],
            let url = URL(string: value),
            ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
            url.host != nil
        else {
            return nil
        }
        return url
    }

    @objc private func quitMenu() {
        // Quitting the disposable shell deliberately leaves the agent running.
        NSApplication.shared.terminate(nil)
    }
}

MainActor.assumeIsolated {
    let application = NSApplication.shared
    let delegate = SpoolMenuDelegate()
    application.delegate = delegate
    application.setActivationPolicy(.accessory)
    application.run()
}

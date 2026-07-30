import AppKit
import SpoolMenuCore
import SpoolProfileHost

private struct RecentJob: Sendable {
    let jobID: String
    let sequence: Int64?
    let title: String
    let state: String
}

private enum QueueLoadResult: Sendable {
    case loaded([LocalQueueJob])
    case unsupported
    case unavailable
}

private final class ProfileActionContext: NSObject {
    let printerID: String
    let profileID: String?
    let revision: UInt64?
    let markAsDefault: Bool

    init(
        printerID: String,
        profileID: String? = nil,
        revision: UInt64? = nil,
        markAsDefault: Bool = false
    ) {
        self.printerID = printerID
        self.profileID = profileID
        self.revision = revision
        self.markAsDefault = markAsDefault
    }
}

@MainActor
final class SpoolMenuDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private var statusItem: NSStatusItem?
    private let menu = NSMenu()
    private var client: LocalAPIClient?
    private var configuration: LocalAPIConfiguration?
    private var status: LocalStatus?
    private var printers: [LocalPrinter] = []
    private var recentJobs: [RecentJob] = []
    private var profilesSupported = false
    private var queueSupported = false
    private var lastError: String?
    private var isRefreshing = false
    private var refreshTask: Task<Void, Never>?
    private var actionTask: Task<Void, Never>?
    private var refreshTimer: Timer?
    private var updateCoordinator: SpoolUpdateCoordinator?

    func applicationDidFinishLaunching(_ notification: Notification) {
        do {
            let configuration = try LocalAPIConfiguration()
            self.configuration = configuration
            client = LocalAPIClient(configuration: configuration)
        } catch {
            lastError = error.localizedDescription
        }
        updateCoordinator = SpoolUpdateCoordinator(client: client)

        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = symbol(
            "printer.fill",
            description: "Piqae print node"
        )
        item.button?.toolTip = "Piqae"
        menu.delegate = self
        item.menu = menu
        statusItem = item

        rebuildMenu()
        refresh()
        refreshTimer = Timer.scheduledTimer(
            withTimeInterval: 10,
            repeats: true
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.refresh()
            }
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        refreshTask?.cancel()
        actionTask?.cancel()
        refreshTimer?.invalidate()
    }

    func menuWillOpen(_ menu: NSMenu) {
        refresh()
    }

    private func refresh() {
        guard let client, !isRefreshing else {
            rebuildMenu()
            return
        }
        isRefreshing = true
        rebuildMenu()
        refreshTask?.cancel()
        refreshTask = Task { [weak self] in
            guard let self else { return }
            defer {
                isRefreshing = false
                updateStatusIcon()
                rebuildMenu()
            }
            do {
                async let nextStatus = client.status()
                async let nextPrinters = client.printers()
                let (loadedStatus, loadedPrinters) = try await (nextStatus, nextPrinters)

                var loadedJobs: [RecentJob] = []
                var supportsQueue = loadedPrinters.isEmpty
                var queueEndpointUnsupported = false
                await withTaskGroup(of: QueueLoadResult.self) { group in
                    for printer in loadedPrinters.prefix(20) {
                        group.addTask {
                            do {
                                return .loaded(
                                    try await client.queue(printerID: printer.printerID)
                                )
                            } catch let LocalAPIError.rejected(status, _)
                                where [404, 405].contains(status)
                            {
                                return .unsupported
                            } catch {
                                return .unavailable
                            }
                        }
                    }
                    for await result in group {
                        switch result {
                        case let .loaded(jobs):
                            supportsQueue = true
                            loadedJobs.append(
                                contentsOf: jobs.map {
                                    RecentJob(
                                        jobID: $0.jobID,
                                        sequence: $0.sequence,
                                        title: $0.title,
                                        state: $0.state
                                    )
                                }
                            )
                        case .unsupported:
                            queueEndpointUnsupported = true
                        case .unavailable:
                            break
                        }
                    }
                }
                if queueEndpointUnsupported {
                    supportsQueue = false
                    loadedJobs.removeAll()
                }

                guard !Task.isCancelled else { return }
                status = loadedStatus
                updateCoordinator?.observe(status: loadedStatus)
                printers = loadedPrinters.sorted {
                    if $0.isDefault != $1.isDefault { return $0.isDefault }
                    return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
                }
                profilesSupported = loadedPrinters.allSatisfy { $0.profiles != nil }
                queueSupported = supportsQueue
                if supportsQueue {
                    recentJobs = Array(
                        loadedJobs
                            .sorted { ($0.sequence ?? 0) > ($1.sequence ?? 0) }
                            .prefix(8)
                    )
                }
                lastError = nil
            } catch is CancellationError {
                return
            } catch {
                status = nil
                updateCoordinator?.observe(status: nil)
                printers = []
                profilesSupported = false
                queueSupported = false
                recentJobs = []
                lastError = error.localizedDescription
            }
        }
    }

    private func rebuildMenu() {
        menu.removeAllItems()

        let statusTitle: String
        if let status {
            let connection = status.connection.replacingOccurrences(of: "_", with: " ").capitalized
            statusTitle = status.paused ? "Piqae — Paused" : "Piqae — \(connection)"
        } else if isRefreshing {
            statusTitle = "Piqae — Connecting…"
        } else {
            statusTitle = "Piqae — Node unavailable"
        }
        menu.addItem(informational(statusTitle, symbol: statusSymbolName()))

        if let status {
            let workspace = status.workspaceName ?? "Local node"
            menu.addItem(
                informational(
                    "\(workspace) · \(status.queuedJobs) queued · \(status.activeJobs) active"
                )
            )
        } else if let lastError {
            menu.addItem(informational(shortened(lastError)))
        }

        menu.addItem(.separator())
        addPrinterSection()
        addRecentJobsSection()

        menu.addItem(.separator())
        if let status {
            let item = menu.addItem(
                withTitle: status.paused ? "Resume Node" : "Pause Node",
                action: status.paused ? #selector(resumeAgent) : #selector(pauseAgent),
                keyEquivalent: ""
            )
            item.target = self
            item.image = symbol(
                status.paused ? "play.fill" : "pause.fill",
                description: status.paused ? "Resume" : "Pause"
            )
        }
        let refreshItem = menu.addItem(
            withTitle: isRefreshing ? "Refreshing…" : "Refresh",
            action: #selector(refreshNow),
            keyEquivalent: "r"
        )
        refreshItem.target = self
        refreshItem.isEnabled = !isRefreshing && client != nil
        refreshItem.image = symbol("arrow.clockwise", description: "Refresh")

        menu.addItem(.separator())
        addNavigationItems()
        menu.addItem(diagnosticsItem())

        menu.addItem(.separator())
        let quit = menu.addItem(
            withTitle: "Quit Piqae",
            action: #selector(quitMenu),
            keyEquivalent: "q"
        )
        quit.target = self
    }

    private func addPrinterSection() {
        menu.addItem(informational("PRINTERS (\(printers.count))"))
        if printers.isEmpty {
            menu.addItem(
                informational(status == nil ? "Node connection required" : "No printers discovered")
            )
            return
        }

        for printer in printers {
            let root = NSMenuItem(
                title: printer.name + (printer.isDefault ? " — macOS default" : ""),
                action: nil,
                keyEquivalent: ""
            )
            root.image = symbol(printerSymbol(for: printer), description: printer.state)
            let printerMenu = NSMenu()
            let state = printer.state.replacingOccurrences(of: "_", with: " ").capitalized
            let count = printer.queueCounts.map {
                " · \($0.queued) queued · \($0.active) active"
            } ?? ""
            printerMenu.addItem(informational("\(state)\(count)"))

            if let exposed = printer.exposed {
                let exposure = NSMenuItem(
                    title: exposed
                        ? "Cloud & API access on"
                        : "Allow cloud & API jobs",
                    action: #selector(toggleExposure(_:)),
                    keyEquivalent: ""
                )
                exposure.target = self
                exposure.representedObject = printer.printerID
                exposure.state = exposed ? .on : .off
                printerMenu.addItem(exposure)
                printerMenu.addItem(
                    informational(
                        exposed
                            ? "Remote jobs can use this printer’s saved profiles"
                            : "Off · this printer can only be used on this Mac"
                    )
                )
            } else {
                printerMenu.addItem(informational("Cloud availability requires an updated node"))
            }

            printerMenu.addItem(.separator())
            addProfileItems(for: printer, to: printerMenu)
            printerMenu.addItem(.separator())
            addDriverTestItem(for: printer, to: printerMenu)
            root.submenu = printerMenu
            menu.addItem(root)
        }
    }

    private func addProfileItems(for printer: LocalPrinter, to printerMenu: NSMenu) {
        let allProfiles = profilesForPrinter(printer)
        let currentDefaults = allProfiles.first {
            $0.usesCurrentPrinterDefaults == true
        }
        let profiles = allProfiles.filter {
            $0.usesCurrentPrinterDefaults != true
        }
        let captureAvailability = PrinterProfileCaptureAvailability(
            printerState: printer.state
        )
        printerMenu.addItem(informational("SAVED PROFILES (\(profiles.count))"))

        let defaultsRoot = NSMenuItem(
            title: CurrentPrinterDefaultsProfile.name,
            action: nil,
            keyEquivalent: ""
        )
        defaultsRoot.image = symbol("gearshape", description: "Current macOS printer defaults")
        let defaultsMenu = NSMenu()
        defaultsMenu.addItem(informational(CurrentPrinterDefaultsProfile.detail))
        defaultsMenu.addItem(
            informational(
                currentDefaults == nil
                    ? "Piqae will add this automatically after the next refresh"
                    : "Follows the driver’s current defaults until you save fixed settings"
            )
        )
        let saveDefaults = defaultsMenu.addItem(
            withTitle: currentDefaults == nil
                ? "Save as Default Profile…"
                : "Save Fixed Native Settings…",
            action: currentDefaults == nil
                ? #selector(addProfile(_:))
                : #selector(editProfile(_:)),
            keyEquivalent: ""
        )
        saveDefaults.target = self
        saveDefaults.representedObject = ProfileActionContext(
            printerID: printer.printerID,
            profileID: currentDefaults?.profileID,
            revision: currentDefaults?.revision,
            markAsDefault: true
        )
        saveDefaults.image = symbol("square.and.arrow.down", description: "Save printer defaults")
        saveDefaults.isEnabled =
            client != nil && status != nil && captureAvailability.canCapture
        if let recovery = captureAvailability.recoveryMessage {
            defaultsMenu.addItem(informational(recovery))
        }
        defaultsRoot.submenu = defaultsMenu
        printerMenu.addItem(defaultsRoot)

        for profile in profiles {
            let revision = profile.revision.map { " · r\($0)" } ?? ""
            let profileRoot = NSMenuItem(
                title: profile.name + (profile.isDefault == true ? " — Job default" : ""),
                action: nil,
                keyEquivalent: ""
            )
            profileRoot.image = symbol(
                profile.status == "ready" ? "checkmark.seal" : "slider.horizontal.3",
                description: profile.name
            )
            let profileMenu = NSMenu()
            let status = profile.status?
                .replacingOccurrences(of: "_", with: " ")
                .capitalized ?? "Saved"
            profileMenu.addItem(informational("\(status)\(revision)"))

            let edit = profileMenu.addItem(
                withTitle: "Edit Native Settings…",
                action: #selector(editProfile(_:)),
                keyEquivalent: ""
            )
            edit.target = self
            edit.representedObject = ProfileActionContext(
                printerID: printer.printerID,
                profileID: profile.profileID,
                revision: profile.revision
            )
            edit.image = symbol("slider.horizontal.3", description: "Edit native settings")
            edit.isEnabled = captureAvailability.canCapture

            let clone = profileMenu.addItem(
                withTitle: "Clone as New Profile…",
                action: #selector(cloneProfile(_:)),
                keyEquivalent: ""
            )
            clone.target = self
            clone.representedObject = ProfileActionContext(
                printerID: printer.printerID,
                profileID: profile.profileID,
                revision: profile.revision
            )
            clone.image = symbol("plus.square.on.square", description: "Clone profile")
            clone.isEnabled = captureAvailability.canCapture
            profileRoot.submenu = profileMenu
            printerMenu.addItem(profileRoot)
        }

        if profiles.isEmpty {
            printerMenu.addItem(
                informational(
                    profilesSupported
                        ? "No immutable profiles saved for jobs"
                        : "Profile listing requires an updated node"
                )
            )
        }

        let add = printerMenu.addItem(
            withTitle: "Add Saved Profile…",
            action: #selector(addProfile(_:)),
            keyEquivalent: ""
        )
        add.target = self
        add.representedObject = ProfileActionContext(printerID: printer.printerID)
        add.image = symbol("plus", description: "Add saved print profile")
        add.isEnabled = client != nil && status != nil && captureAvailability.canCapture
    }

    private func addDriverTestItem(for printer: LocalPrinter, to printerMenu: NSMenu) {
        let availableProfiles = profilesForPrinter(printer)
        let captureAvailability = PrinterProfileCaptureAvailability(
            printerState: printer.state
        )
        let test = NSMenuItem(
            title: "Local driver test…",
            action: #selector(confirmDriverTest(_:)),
            keyEquivalent: ""
        )
        test.target = self
        test.representedObject = printer.printerID
        test.image = symbol("doc.text", description: "Local driver test")
        test.isEnabled =
            profilesSupported
            && !availableProfiles.isEmpty
            && captureAvailability.canCapture
        printerMenu.addItem(test)
        if !profilesSupported {
            printerMenu.addItem(informational("Requires a node with print-profile support"))
        } else if availableProfiles.isEmpty {
            printerMenu.addItem(
                informational("Save printer defaults as a profile to test")
            )
        } else if let recovery = captureAvailability.recoveryMessage {
            printerMenu.addItem(informational(recovery))
        }
    }

    private func addRecentJobsSection() {
        menu.addItem(.separator())
        menu.addItem(informational("RECENT JOBS"))
        if recentJobs.isEmpty {
            menu.addItem(
                informational(
                    queueSupported ? "No recent local jobs" : "Recent jobs unavailable from node"
                )
            )
            return
        }
        for job in recentJobs.prefix(5) {
            let state = job.state.replacingOccurrences(of: "_", with: " ").capitalized
            menu.addItem(informational("\(shortened(job.title, limit: 34)) — \(state)"))
        }
    }

    private func addNavigationItems() {
        let addPrinter = menu.addItem(
            withTitle: "Add Printer…",
            action: #selector(addPrinter),
            keyEquivalent: ""
        )
        addPrinter.target = self
        addPrinter.image = symbol("plus", description: "Add printer")

        let manage = menu.addItem(
            withTitle: "Manage Printers",
            action: #selector(managePrinters),
            keyEquivalent: ""
        )
        manage.target = self
        manage.isEnabled = dashboardURL() != nil
        manage.image = symbol("printer", description: "Manage printers")

        let dashboard = menu.addItem(
            withTitle: "Open Dashboard",
            action: #selector(openDashboard),
            keyEquivalent: "o"
        )
        dashboard.target = self
        dashboard.isEnabled = dashboardURL() != nil
        dashboard.image = symbol("rectangle.3.group", description: "Dashboard")

        let updates = menu.addItem(
            withTitle: updateCoordinator?.isEnabled == true
                ? "Check for Updates…"
                : "Updates unavailable in this build",
            action: #selector(checkForUpdates(_:)),
            keyEquivalent: ""
        )
        updates.target = self
        updates.isEnabled = updateCoordinator?.canCheckForUpdates == true
        updates.image = symbol("arrow.down.circle", description: "Software updates")
    }

    private func diagnosticsItem() -> NSMenuItem {
        let item = NSMenuItem(title: "Diagnostics", action: nil, keyEquivalent: "")
        item.image = symbol("stethoscope", description: "Diagnostics")
        let diagnostics = NSMenu()
        let copy = diagnostics.addItem(
            withTitle: "Copy Diagnostics",
            action: #selector(copyDiagnostics),
            keyEquivalent: ""
        )
        copy.target = self
        let log = diagnostics.addItem(
            withTitle: "Open Node Log",
            action: #selector(openAgentLog),
            keyEquivalent: ""
        )
        log.target = self
        log.isEnabled = FileManager.default.fileExists(atPath: agentLogURL().path)
        item.submenu = diagnostics
        return item
    }

    @objc private func refreshNow() {
        refresh()
    }

    @objc private func pauseAgent() {
        performAction(successMessage: nil) { client in
            try await client.pause()
        }
    }

    @objc private func resumeAgent() {
        performAction(successMessage: nil) { client in
            try await client.resume()
        }
    }

    @objc private func toggleExposure(_ sender: NSMenuItem) {
        guard
            let printerID = sender.representedObject as? String,
            let printer = printers.first(where: { $0.printerID == printerID }),
            let exposed = printer.exposed
        else {
            return
        }
        performAction(successMessage: nil) { client in
            try await client.setExposure(printerID: printerID, exposed: !exposed)
        }
    }

    @objc private func addProfile(_ sender: NSMenuItem) {
        guard let context = sender.representedObject as? ProfileActionContext else { return }
        beginProfileCapture(context: context, operation: .create)
    }

    @objc private func editProfile(_ sender: NSMenuItem) {
        guard let context = sender.representedObject as? ProfileActionContext else { return }
        beginProfileCapture(context: context, operation: .edit)
    }

    @objc private func cloneProfile(_ sender: NSMenuItem) {
        guard let context = sender.representedObject as? ProfileActionContext else { return }
        beginProfileCapture(context: context, operation: .clone)
    }

    private func beginProfileCapture(
        context: ProfileActionContext,
        operation: LocalProfileCaptureOperation
    ) {
        guard
            let client,
            let printer = printers.first(where: { $0.printerID == context.printerID })
        else {
            return
        }
        let availability = PrinterProfileCaptureAvailability(printerState: printer.state)
        guard availability.canCapture else {
            showAlert(
                title: "Piqae cannot open this printer",
                message: availability.recoveryMessage
                    ?? "Refresh Piqae after restoring this printer in macOS Printer Settings."
            )
            return
        }
        actionTask?.cancel()
        actionTask = Task { [weak self] in
            guard let self else { return }
            updateCoordinator?.beginForegroundOperation()
            defer {
                updateCoordinator?.endForegroundOperation()
            }
            var session: LocalProfileCaptureSession?
            do {
                let openedSession = try await client.createProfileCaptureSession(
                    printerID: printer.printerID,
                    operation: operation,
                    profileID: context.profileID,
                    expectedRevision: context.revision
                )
                session = openedSession
                guard !Task.isCancelled else {
                    try? await client.cancelProfileCapture(session: openedSession)
                    return
                }

                let capturer = MacPrintProfileCapturer()
                guard let completion = try capturer.capture(
                    session: openedSession,
                    markAsDefault: context.markAsDefault
                ) else {
                    try? await client.cancelProfileCapture(session: openedSession)
                    return
                }
                let saved = try await client.completeProfileCapture(
                    session: openedSession,
                    completion: completion
                )
                showAlert(
                    title: "Profile saved",
                    message:
                        "\(saved.name) is available for \(printer.name)"
                        + (saved.revision.map { " as revision \($0)." } ?? "."),
                    style: .informational
                )
                refresh()
            } catch is CancellationError {
                if let session {
                    try? await client.cancelProfileCapture(session: session)
                }
            } catch {
                if let session {
                    try? await client.cancelProfileCapture(session: session)
                }
                showAlert(
                    title: "Piqae could not save the profile",
                    message: error.localizedDescription
                )
                refresh()
            }
        }
    }

    @objc private func confirmDriverTest(_ sender: NSMenuItem) {
        guard
            let printerID = sender.representedObject as? String,
            let printer = printers.first(where: { $0.printerID == printerID })
        else {
            return
        }
        let availableProfiles = profilesForPrinter(printer)
        guard !availableProfiles.isEmpty else { return }

        let picker = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 300, height: 26))
        for profile in availableProfiles {
            picker.addItem(
                withTitle: profile.name + (profile.isDefault == true ? " — Job default" : "")
            )
        }
        if let defaultIndex = availableProfiles.firstIndex(where: { $0.isDefault == true }) {
            picker.selectItem(at: defaultIndex)
        }

        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Print a local driver test?"
        alert.informativeText =
            "Printer: \(printer.name)\nChoose the print profile to confirm. " +
            "This A4 page validates only the local macOS driver path."
        alert.accessoryView = picker
        alert.addButton(withTitle: "Print Local Test")
        alert.addButton(withTitle: "Cancel")
        NSApplication.shared.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else { return }

        let selected = availableProfiles[picker.indexOfSelectedItem]
        performAction(successMessage: "Local driver test accepted for \(printer.name).") { client in
            _ = try await client.submitDriverTest(
                printerID: printer.printerID,
                profileID: selected.profileID
            )
        }
    }

    private func performAction(
        successMessage: String?,
        operation: @escaping @Sendable (LocalAPIClient) async throws -> Void
    ) {
        guard let client else { return }
        actionTask?.cancel()
        actionTask = Task { [weak self] in
            guard let self else { return }
            do {
                try await operation(client)
                if let successMessage {
                    showAlert(title: "Piqae", message: successMessage, style: .informational)
                }
                refresh()
            } catch is CancellationError {
                return
            } catch {
                showAlert(
                    title: "Piqae could not complete that action",
                    message: error.localizedDescription
                )
                refresh()
            }
        }
    }

    @objc private func addPrinter() {
        let candidates = [
            "x-apple.systempreferences:com.apple.Print-Scan-Settings.extension",
            "x-apple.systempreferences:com.apple.preference.printfax",
        ]
        for candidate in candidates {
            if let url = URL(string: candidate), NSWorkspace.shared.open(url) {
                return
            }
        }
        showAlert(
            title: "Open Printers & Scanners",
            message: "Open System Settings, then choose Printers & Scanners."
        )
    }

    @objc private func managePrinters() {
        guard let dashboard = dashboardURL() else { return }
        NSWorkspace.shared.open(
            dashboard.appendingPathComponent("dashboard/local", isDirectory: false)
        )
    }

    @objc private func openDashboard() {
        guard let url = dashboardURL() else { return }
        NSWorkspace.shared.open(url)
    }

    @objc private func checkForUpdates(_ sender: NSMenuItem) {
        updateCoordinator?.checkForUpdates(sender)
    }

    @objc private func copyDiagnostics() {
        var lines = [
            "Piqae menu diagnostics",
            "api=\(configuration?.baseURL.absoluteString ?? "invalid")",
            "token_file=\(configuration?.tokenFile.path ?? "invalid")",
            "connection=\(status?.connection ?? "unavailable")",
            "paused=\(status?.paused.description ?? "unknown")",
            "queued=\(status?.queuedJobs.description ?? "unknown")",
            "active=\(status?.activeJobs.description ?? "unknown")",
            "printers=\(printers.count)",
            "profiles_supported=\(profilesSupported)",
            "queue_supported=\(queueSupported)",
            "node_version=\(status?.version ?? "unknown")",
        ]
        if let lastError {
            lines.append("last_error=\(lastError)")
        }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(lines.joined(separator: "\n"), forType: .string)
    }

    @objc private func openAgentLog() {
        NSWorkspace.shared.open(agentLogURL())
    }

    @objc private func quitMenu() {
        // The node is a separate service and deliberately keeps running.
        NSApplication.shared.terminate(nil)
    }

    private func profilesForPrinter(_ printer: LocalPrinter) -> [LocalPrintProfile] {
        (printer.profiles ?? [])
            .sorted {
                if $0.isDefault == true && $1.isDefault != true { return true }
                if $1.isDefault == true && $0.isDefault != true { return false }
                return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
            }
    }

    private func updateStatusIcon() {
        let name = statusSymbolName()
        statusItem?.button?.image = symbol(
            name,
            description: "Piqae \(status?.connection ?? "offline")"
        )
        statusItem?.button?.toolTip = status.map {
            "Piqae · \($0.connection.replacingOccurrences(of: "_", with: " ").capitalized)"
        } ?? "Piqae · Node unavailable"
    }

    private func statusSymbolName() -> String {
        guard let status else { return "printer.fill" }
        if status.paused { return "pause.circle.fill" }
        switch status.connection {
        case "connected", "local_only":
            return status.printerWarnings > 0 ? "exclamationmark.triangle.fill" : "printer.fill"
        case "connecting":
            return "arrow.triangle.2.circlepath"
        case "degraded":
            return "exclamationmark.triangle.fill"
        default:
            return "printer.fill"
        }
    }

    private func printerSymbol(for printer: LocalPrinter) -> String {
        switch printer.state {
        case "idle", "ready", "online":
            return "printer.fill"
        case "printing", "spooling", "busy":
            return "printer.dotmatrix.fill"
        case "paused":
            return "pause.circle"
        case "error", "paper_out", "offline":
            return "exclamationmark.triangle"
        default:
            return "printer"
        }
    }

    private func informational(_ title: String, symbol name: String? = nil) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        if let name {
            item.image = symbol(name, description: title)
        }
        return item
    }

    private func symbol(_ name: String, description: String) -> NSImage? {
        let image = NSImage(systemSymbolName: name, accessibilityDescription: description)
        image?.isTemplate = true
        return image
    }

    private func dashboardURL() -> URL? {
        guard
            let value = ProcessInfo.processInfo.environment["SPOOL_DASHBOARD_URL"],
            let url = URL(string: value),
            ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
            url.host != nil,
            url.user == nil,
            url.password == nil
        else {
            return nil
        }
        return url
    }

    private func agentLogURL() -> URL {
        if let path = ProcessInfo.processInfo.environment["SPOOL_AGENT_LOG_FILE"], !path.isEmpty {
            return URL(fileURLWithPath: path)
        }
        return URL(fileURLWithPath: "/var/log/spool-agent.log")
    }

    private func shortened(_ value: String, limit: Int = 54) -> String {
        guard value.count > limit else { return value }
        return String(value.prefix(max(1, limit - 1))) + "…"
    }

    private func showAlert(
        title: String,
        message: String,
        style: NSAlert.Style = .warning
    ) {
        let alert = NSAlert()
        alert.alertStyle = style
        alert.messageText = title
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        NSApplication.shared.activate(ignoringOtherApps: true)
        alert.runModal()
    }
}

MainActor.assumeIsolated {
    let application = NSApplication.shared
    let delegate = SpoolMenuDelegate()
    application.delegate = delegate
    application.setActivationPolicy(.accessory)
    application.run()
}

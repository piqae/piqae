import AppKit
import Darwin
import PiqaeMenuCore
import PiqaeProfileHost

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

private func restartInstalledAgent() throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
    process.arguments = [
        "kickstart", "-k", "gui/\(getuid())/com.piqae.node.agent",
    ]
    process.standardOutput = Pipe()
    process.standardError = Pipe()
    try process.run()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
        throw NodeConnectAgentBridgeError.failed
    }
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
final class PiqaeMenuDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
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
    private var updateCoordinator: PiqaeUpdateCoordinator?
    private let connectReplayGuard = NodeConnectReplayGuard()

    func applicationDidFinishLaunching(_ notification: Notification) {
        do {
            let configuration = try LocalAPIConfiguration()
            self.configuration = configuration
            client = LocalAPIClient(configuration: configuration)
        } catch {
            lastError = error.localizedDescription
        }
        updateCoordinator = PiqaeUpdateCoordinator(client: client)
        updateCoordinator?.onPresentationChange = { [weak self] in
            self?.rebuildMenu()
        }

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

    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls.prefix(4) {
            Task { await handleConnectApplicationLink(url) }
        }
    }

    func application(
        _ application: NSApplication,
        continue userActivity: NSUserActivity,
        restorationHandler: @escaping ([any NSUserActivityRestoring]) -> Void
    ) -> Bool {
        guard userActivity.activityType == NSUserActivityTypeBrowsingWeb,
              let url = userActivity.webpageURL else { return false }
        Task { await handleConnectApplicationLink(url) }
        return true
    }

    private func handleConnectApplicationLink(_ url: URL) async {
        let link: NodeConnectApplicationLink
        do {
            link = try NodeConnectApplicationLink(url: url)
        } catch {
            showAlert(title: "Invalid Piqae connection", message: "This connection link is invalid or unsafe.")
            return
        }
        guard await connectReplayGuard.begin(link) else { return }
        var consumed = false
        defer { Task { await connectReplayGuard.finish(link, consumed: consumed) } }
        do {
            guard let client else { throw NodeConnectAgentBridgeError.unavailable }
            async let currentStatusRequest = client.status()
            async let currentPrintersRequest = client.printers()
            let (currentStatus, currentPrinters) = try await (
                currentStatusRequest,
                currentPrintersRequest
            )
            guard currentStatus.queuedJobs == 0, currentStatus.activeJobs == 0 else {
                showAlert(
                    title: "Finish current print jobs first",
                    message: "Piqae will not change connected services while jobs are queued or active."
                )
                return
            }
            guard !currentPrinters.isEmpty else {
                showAlert(
                    title: "No printers available",
                    message: "Add a local printer before connecting this service."
                )
                return
            }
            let bridge = try NodeConnectAgentBridge()
            let preview = try await Task.detached {
                try bridge.preview(capability: link.enrolmentCapability)
            }.value
            let selected = presentConnectorConsent(preview: preview, printers: currentPrinters)
            guard !selected.isEmpty else { return }
            let statusBeforeAccept = try await client.status()
            guard statusBeforeAccept.queuedJobs == 0, statusBeforeAccept.activeJobs == 0 else {
                showAlert(
                    title: "Print activity started",
                    message: "The connection was not changed because a print job started while approval was open. Try again when the queue is idle."
                )
                return
            }
            try await Task.detached {
                try bridge.accept(capability: link.enrolmentCapability, printerIDs: selected)
            }.value
            consumed = true
            do {
                try await Task.detached {
                    try restartInstalledAgent()
                }.value
            } catch {
                showAlert(
                    title: "Connected — restart Piqae",
                    message: "The connector was saved securely, but Piqae could not restart automatically. Restart Piqae to activate it."
                )
                return
            }
            showAlert(
                title: "Connected with Piqae",
                message: "\(preview.requestingServiceName ?? preview.workspaceName) can now print only to the printers you selected.",
                style: .informational
            )
            if let returnURL = preview.returnURL { NSWorkspace.shared.open(returnURL) }
        } catch {
            showAlert(
                title: "Connection not completed",
                message: error.localizedDescription
            )
        }
    }

    private func presentConnectorConsent(
        preview: NodeConnectPreview,
        printers availablePrinters: [LocalPrinter]
    ) -> [String] {
        let alert = NSAlert()
        alert.alertStyle = .informational
        let requester = preview.requestingServiceName.map { "Piqae integration \($0)" }
            ?? "Piqae workspace \(preview.workspaceName)"
        alert.messageText = "Connect \(requester)?"
        alert.informativeText = "Customer workspace \(preview.workspaceName) (\(preview.workspaceID)) requests: \(preview.requestedScopes.joined(separator: ", ")). Select the local printers it may use. You can disconnect it later."
        alert.addButton(withTitle: "Connect selected printers")
        alert.addButton(withTitle: "Cancel")
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 6
        let choices = availablePrinters.map { printer -> NSButton in
            let button = NSButton(checkboxWithTitle: printer.name, target: nil, action: nil)
            button.state = .off
            button.identifier = NSUserInterfaceItemIdentifier(printer.printerID)
            stack.addArrangedSubview(button)
            return button
        }
        alert.accessoryView = stack
        NSApplication.shared.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else { return [] }
        let selected = choices.compactMap { button in
            button.state == .on ? button.identifier?.rawValue : nil
        }
        if selected.isEmpty {
            showAlert(title: "Select a printer", message: "No printer access was granted.")
        }
        return selected
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
            let connection = status.connection == "local_only"
                ? "Local printing only"
                : status.connection.replacingOccurrences(of: "_", with: " ").capitalized
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
            printerMenu.addItem(
                informational(
                    MenuPresentation.printerActivityTitle(
                        state: printer.state,
                        queued: printer.queueCounts?.queued,
                        active: printer.queueCounts?.active
                    )
                )
            )

            if let exposed = printer.exposed {
                let exposure = NSMenuItem(
                    title: MenuPresentation.cloudAndAPIAccessTitle,
                    action: #selector(toggleExposure(_:)),
                    keyEquivalent: ""
                )
                exposure.target = self
                exposure.representedObject = printer.printerID
                exposure.state = exposed ? .on : .off
                printerMenu.addItem(exposure)
            } else {
                let exposure = informational("Cloud & API access unavailable")
                exposure.toolTip = "Update the node to manage remote access for this printer."
                printerMenu.addItem(exposure)
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
        printerMenu.addItem(
            informational(MenuPresentation.printPresetSectionTitle(count: profiles.count))
        )

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
                ? "Save as Default Preset…"
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
                withTitle: "Duplicate as New Preset…",
                action: #selector(cloneProfile(_:)),
                keyEquivalent: ""
            )
            clone.target = self
            clone.representedObject = ProfileActionContext(
                printerID: printer.printerID,
                profileID: profile.profileID,
                revision: profile.revision
            )
            clone.image = symbol("plus.square.on.square", description: "Duplicate print preset")
            clone.isEnabled = captureAvailability.canCapture
            profileRoot.submenu = profileMenu
            printerMenu.addItem(profileRoot)
        }

        if profiles.isEmpty {
            printerMenu.addItem(
                informational(
                    profilesSupported
                        ? "No print presets saved"
                        : "Print presets require an updated node"
                )
            )
        }

        let add = printerMenu.addItem(
            withTitle: "Add Print Preset…",
            action: #selector(addProfile(_:)),
            keyEquivalent: ""
        )
        add.target = self
        add.representedObject = ProfileActionContext(printerID: printer.printerID)
        add.image = symbol("plus", description: "Add print preset")
        add.isEnabled = client != nil && status != nil && captureAvailability.canCapture
    }

    private func addDriverTestItem(for printer: LocalPrinter, to printerMenu: NSMenu) {
        let availableProfiles = profilesForPrinter(printer)
        let captureAvailability = PrinterProfileCaptureAvailability(
            printerState: printer.state
        )
        let test = NSMenuItem(
            title: "Test Printer…",
            action: #selector(confirmDriverTest(_:)),
            keyEquivalent: ""
        )
        test.target = self
        test.representedObject = printer.printerID
        test.image = symbol("doc.text", description: "Test printer")
        test.isEnabled =
            profilesSupported
            && !availableProfiles.isEmpty
            && captureAvailability.canCapture
        printerMenu.addItem(test)
        if !profilesSupported {
            test.toolTip = "Update the node to test print presets."
        } else if availableProfiles.isEmpty {
            test.toolTip = "Save a print preset before testing this printer."
        } else if let recovery = captureAvailability.recoveryMessage {
            test.toolTip = recovery
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
        for job in recentJobs.prefix(3) {
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

        if dashboardURL() != nil {
            let dashboard = menu.addItem(
                withTitle: "Open Dashboard",
                action: #selector(openDashboard),
                keyEquivalent: "o"
            )
            dashboard.target = self
            dashboard.image = symbol("rectangle.3.group", description: "Dashboard")
        }

        let updates = menu.addItem(
            withTitle: updateCoordinator?.presentation.title
                ?? UpdateMenuPresentation.unavailable.title,
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
                    title: "Print preset saved",
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
                    title: "Piqae could not save the print preset",
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
        alert.messageText = "Test this printer?"
        alert.informativeText =
            "Printer: \(printer.name)\nChoose the print preset to confirm. " +
            "This A4 page validates only the local macOS driver path."
        alert.accessoryView = picker
        alert.addButton(withTitle: "Print Test Page")
        alert.addButton(withTitle: "Cancel")
        NSApplication.shared.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else { return }

        let selected = availableProfiles[picker.indexOfSelectedItem]
        performAction(successMessage: "Test page accepted for \(printer.name).") { client in
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
        let value = ProcessInfo.processInfo.environment["PIQAE_DASHBOARD_URL"]
            ?? Bundle.main.object(forInfoDictionaryKey: "PiqaeDashboardURL") as? String
        guard
            let value,
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
        if let path = ProcessInfo.processInfo.environment["PIQAE_AGENT_LOG_FILE"], !path.isEmpty {
            return URL(fileURLWithPath: path)
        }
        return URL(fileURLWithPath: "/var/log/piqae-agent.log")
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
    let delegate = PiqaeMenuDelegate()
    application.delegate = delegate
    application.setActivationPolicy(.accessory)
    application.run()
}

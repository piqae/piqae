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

@MainActor
private final class PrinterConsentSelectionController: NSObject {
    private let choices: [NSButton]
    private let allPrinters: NSButton
    private let selectedPrinters: NSButton
    var selectedPrinterViews: [NSView] = []
    weak var confirmButton: NSButton?

    init(choices: [NSButton], allPrinters: NSButton, selectedPrinters: NSButton) {
        self.choices = choices
        self.allPrinters = allPrinters
        self.selectedPrinters = selectedPrinters
    }

    @objc func selectionChanged() {
        let selectingSpecificPrinters = selectedPrinters.state == .on
        choices.forEach { $0.isEnabled = selectingSpecificPrinters }
        selectedPrinterViews.forEach { $0.isHidden = !selectingSpecificPrinters }
        confirmButton?.isEnabled = !selectingSpecificPrinters || choices.contains { $0.state == .on }
    }

    @objc func grantChanged(_ sender: NSButton) {
        allPrinters.state = sender === allPrinters ? .on : .off
        selectedPrinters.state = sender === selectedPrinters ? .on : .off
        selectionChanged()
    }
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

private final class ProfileTestContext: NSObject {
    let printerID: String
    let profileID: String
    let profileName: String

    init(printerID: String, profileID: String, profileName: String) {
        self.printerID = printerID
        self.profileID = profileID
        self.profileName = profileName
    }
}

private final class BrokerAuthorizationActionContext: NSObject {
    let request: LocalPendingBrokerAuthorization
    let approved: Bool

    init(request: LocalPendingBrokerAuthorization, approved: Bool) {
        self.request = request
        self.approved = approved
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
    private var pendingBrokerAuthorizations: [LocalPendingBrokerAuthorization] = []
    private var profilesSupported = false
    private var queueSupported = false
    private var lastError: String?
    private var isRefreshing = false
    private var refreshTask: Task<Void, Never>?
    private var actionTask: Task<Void, Never>?
    private var refreshTimer: Timer?
    private var updateCoordinator: PiqaeUpdateCoordinator?
    private var nativeComponentUpdateTask: Task<String?, Never>?
    private var hostLifecycleMonitor: PiqaeMacHostLifecycleMonitor?
    private let connectReplayGuard = NodeConnectReplayGuard()

    func applicationDidFinishLaunching(_ notification: Notification) {
        if let updater = NativeComponentUpdater() {
            nativeComponentUpdateTask = Task { [weak self] in
                let updateError = await Task.detached { () -> String? in
                    do {
                        try updater.run()
                        return nil
                    } catch {
                        return error.localizedDescription
                    }
                }.value
                if let updateError {
                    self?.lastError = updateError
                }
                return updateError
            }
        }
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
        if let client {
            let monitor = PiqaeMacHostLifecycleMonitor(
                reporter: PiqaeLocalAPIHostLifecycleReporter(client: client)
            )
            monitor.start()
            hostLifecycleMonitor = monitor
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
        hostLifecycleMonitor?.stop()
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
        if let updateError = await nativeComponentUpdateTask?.value {
            showAlert(title: "Piqae update not completed", message: updateError)
            return
        }
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
                try bridge.preview(capability: link.enrolmentCapability, controlPlaneURL: link.controlPlaneURL)
            }.value
            guard let authorization = presentConnectorConsent(
                preview: preview,
                printers: currentPrinters
            ) else { return }
            let statusBeforeAccept = try await client.status()
            guard statusBeforeAccept.queuedJobs == 0, statusBeforeAccept.activeJobs == 0 else {
                showAlert(
                    title: "Print activity started",
                    message: "The connection was not changed because a print job started while approval was open. Try again when the queue is idle."
                )
                return
            }
            guard preview.expiresAt > Date() else {
                showAlert(
                    title: "Connection expired",
                    message: "Return to the service and create a new connection, then approve it within the displayed time."
                )
                return
            }
            try await Task.detached {
                try bridge.accept(
                    capability: link.enrolmentCapability,
                    controlPlaneURL: link.controlPlaneURL,
                    authorization: authorization
                )
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
                message: authorization.grant == .allLocalPrinters
                    ? "\(preview.requestingServiceName ?? preview.workspaceName) can now use printers on this computer, including printers added later."
                    : "\(preview.requestingServiceName ?? preview.workspaceName) can now use only the printers you selected.",
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
    ) -> NodePrinterAuthorization? {
        let presentation = NodeConnectConsentPresentation(preview: preview)
        let alert = NSAlert()
        alert.alertStyle = .informational
        alert.messageText = presentation.title
        alert.informativeText = presentation.detailText
        let confirmButton = alert.addButton(withTitle: "Allow printer access")
        alert.addButton(withTitle: "Cancel")
        let allPrinters = NSButton(radioButtonWithTitle: "All printers on this computer", target: nil, action: nil)
        allPrinters.state = presentation.defaultGrant == .allLocalPrinters ? .on : .off
        let permissions = wrappingLabel(presentation.permissionsText, color: .secondaryLabelColor)
        let allDetail = wrappingLabel("Includes printers added later. Recommended for most services.", color: .secondaryLabelColor)
        allDetail.textColor = .secondaryLabelColor
        allDetail.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        let selectedPrinters = NSButton(radioButtonWithTitle: "Only selected printers", target: nil, action: nil)
        selectedPrinters.state = presentation.defaultGrant == .selectedPrinters ? .on : .off
        let selectionController = PrinterConsentSelectionController(
            choices: [],
            allPrinters: allPrinters,
            selectedPrinters: selectedPrinters
        )
        selectionController.confirmButton = confirmButton
        allPrinters.target = selectionController
        allPrinters.action = #selector(PrinterConsentSelectionController.grantChanged(_:))
        selectedPrinters.target = selectionController
        selectedPrinters.action = #selector(PrinterConsentSelectionController.grantChanged(_:))
        let accessory = NSStackView(views: [permissions, allPrinters, allDetail, selectedPrinters])
        accessory.orientation = .vertical
        accessory.alignment = .leading
        accessory.spacing = 7
        accessory.setCustomSpacing(14, after: permissions)
        sizeAccessory(accessory)
        selectionController.selectionChanged()
        alert.accessoryView = accessory
        NSApplication.shared.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        if allPrinters.state == .on {
            return NodePrinterAuthorization(grant: .allLocalPrinters)
        }
        return presentSelectedPrinterConsent(availablePrinters)
    }

    private func presentSelectedPrinterConsent(
        _ availablePrinters: [LocalPrinter]
    ) -> NodePrinterAuthorization? {
        let alert = NSAlert()
        alert.alertStyle = .informational
        alert.messageText = "Choose printers"
        alert.informativeText = "This service will be limited to the printers selected below."
        let confirmButton = alert.addButton(withTitle: "Allow selected printers")
        alert.addButton(withTitle: "Back")
        confirmButton.isEnabled = false
        let choices = availablePrinters.map { printer -> NSButton in
            let button = NSButton(checkboxWithTitle: printer.name, target: nil, action: nil)
            button.state = .off
            button.identifier = NSUserInterfaceItemIdentifier(printer.printerID)
            return button
        }
        let selectedMode = NSButton(radioButtonWithTitle: "", target: nil, action: nil)
        selectedMode.state = .on
        let selectionController = PrinterConsentSelectionController(
            choices: choices,
            allPrinters: NSButton(),
            selectedPrinters: selectedMode
        )
        selectionController.confirmButton = confirmButton
        for choice in choices {
            choice.target = selectionController
            choice.action = #selector(PrinterConsentSelectionController.selectionChanged)
        }
        let listHeight = CGFloat(choices.count) * 28
        let document = NSView(frame: NSRect(x: 0, y: 0, width: 344, height: listHeight))
        for (index, choice) in choices.enumerated() {
            choice.frame = NSRect(x: 8, y: listHeight - CGFloat(index + 1) * 28, width: 328, height: 24)
            document.addSubview(choice)
        }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 360, height: min(max(listHeight, 56), 168)))
        scroll.documentView = document
        scroll.drawsBackground = false
        scroll.borderType = .bezelBorder
        scroll.hasVerticalScroller = choices.count > 6
        alert.accessoryView = scroll
        NSApplication.shared.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        let printerIDs = choices.compactMap { button in
            button.state == .on ? button.identifier?.rawValue : nil
        }
        return NodePrinterAuthorization(grant: .selectedPrinters, printerIDs: printerIDs)
    }

    private func wrappingLabel(_ text: String, color: NSColor) -> NSTextField {
        let label = NSTextField(wrappingLabelWithString: text)
        label.textColor = color
        label.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        label.maximumNumberOfLines = 3
        label.preferredMaxLayoutWidth = 360
        return label
    }

    private func sizeAccessory(_ accessory: NSStackView) {
        accessory.frame = NSRect(x: 0, y: 0, width: 360, height: 1)
        accessory.layoutSubtreeIfNeeded()
        accessory.frame.size = NSSize(width: 360, height: ceil(accessory.fittingSize.height))
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
                let loadedAuthorizations: [LocalPendingBrokerAuthorization]
                do {
                    loadedAuthorizations = try await client.pendingBrokerAuthorizations()
                } catch let LocalAPIError.rejected(status, _) where [404, 405].contains(status) {
                    loadedAuthorizations = []
                } catch {
                    // Consent polling is additive. A transient failure must not
                    // discard otherwise fresh status, printer, and queue data.
                    loadedAuthorizations = pendingBrokerAuthorizations
                }

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
                pendingBrokerAuthorizations = loadedAuthorizations
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
                pendingBrokerAuthorizations = []
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
        addConnectionsItem()
        addApplicationAuthorizationItems()
        addUpdateItem()
        menu.addItem(diagnosticsItem())

        menu.addItem(.separator())
        let quit = menu.addItem(
            withTitle: "Quit Piqae",
            action: #selector(quitMenu),
            keyEquivalent: "q"
        )
        quit.target = self
    }

    private func addApplicationAuthorizationItems() {
        guard !pendingBrokerAuthorizations.isEmpty else { return }
        menu.addItem(.separator())
        menu.addItem(
            informational("APP ACCESS REQUESTS (\(pendingBrokerAuthorizations.count))")
        )
        for request in pendingBrokerAuthorizations {
            let root = NSMenuItem(
                title: shortened(request.application.displayName),
                action: nil,
                keyEquivalent: ""
            )
            root.image = symbol("app.badge", description: "Application access request")
            let submenu = NSMenu()
            submenu.addItem(informational("App ID: \(shortened(request.application.applicationID))"))
            if let signingIdentity = request.application.signingIdentitySHA256 {
                submenu.addItem(
                    informational("Signing identity: \(shortened(signingIdentity))")
                )
            } else {
                submenu.addItem(informational("Signing identity not supplied"))
            }
            submenu.addItem(informational("REQUESTED ACCESS"))
            for capability in request.requestedCapabilities {
                submenu.addItem(informational(capability.displayName))
            }
            submenu.addItem(.separator())
            let approve = submenu.addItem(
                withTitle: "Approve…",
                action: #selector(decideApplicationAuthorization(_:)),
                keyEquivalent: ""
            )
            approve.target = self
            approve.representedObject = BrokerAuthorizationActionContext(
                request: request,
                approved: true
            )
            let deny = submenu.addItem(
                withTitle: "Deny…",
                action: #selector(decideApplicationAuthorization(_:)),
                keyEquivalent: ""
            )
            deny.target = self
            deny.representedObject = BrokerAuthorizationActionContext(
                request: request,
                approved: false
            )
            root.submenu = submenu
            menu.addItem(root)
        }
    }

    private func addPrinterSection() {
        menu.addItem(informational("PRINTERS (\(printers.count))"))
        if printers.isEmpty {
            menu.addItem(
                informational(status == nil ? "Node connection required" : "No printers discovered")
            )
            addPrinterItem()
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

            let exposure = informational(MenuPresentation.cloudAndAPIAccessTitle)
            exposure.image = symbol("cloud", description: "Available to connected services")
            exposure.toolTip = "Printer access is selected separately when each service connects."
            printerMenu.addItem(exposure)

            printerMenu.addItem(.separator())
            addProfileItems(for: printer, to: printerMenu)
            root.submenu = printerMenu
            menu.addItem(root)
        }

        addPrinterItem()
    }

    private func addPrinterItem() {
        let addPrinter = menu.addItem(
            withTitle: "Add Printer…",
            action: #selector(addPrinter),
            keyEquivalent: ""
        )
        addPrinter.target = self
        addPrinter.image = symbol("plus", description: "Add printer")
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
        if let currentDefaults {
            defaultsMenu.addItem(.separator())
            defaultsMenu.addItem(testPresetItem(printer: printer, profile: currentDefaults))
        }
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
            profileMenu.addItem(.separator())
            let test = testPresetItem(printer: printer, profile: profile)
            test.isEnabled = captureAvailability.canCapture && profile.status != "invalid"
            profileMenu.addItem(test)
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

    private func testPresetItem(printer: LocalPrinter, profile: LocalPrintProfile) -> NSMenuItem {
        let test = NSMenuItem(
            title: MenuPresentation.testPresetTitle(profile.name),
            action: #selector(confirmPresetTest(_:)),
            keyEquivalent: ""
        )
        test.target = self
        test.representedObject = ProfileTestContext(
            printerID: printer.printerID,
            profileID: profile.profileID,
            profileName: profile.name
        )
        test.image = symbol("doc.text", description: "Test print preset")
        test.isEnabled = PrinterProfileCaptureAvailability(printerState: printer.state).canCapture
            && profile.status != "invalid"
        return test
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
        } else {
            for job in recentJobs.prefix(3) {
                let state = job.state.replacingOccurrences(of: "_", with: " ").capitalized
                menu.addItem(informational("\(shortened(job.title, limit: 34)) — \(state)"))
            }
        }
        if client != nil || dashboardURL() != nil {
            let queue = menu.addItem(
                withTitle: MenuPresentation.queueTitle,
                action: #selector(openQueue),
                keyEquivalent: "o"
            )
            queue.target = self
            queue.image = symbol("list.bullet.rectangle", description: "Full print queue")
        }
    }

    private func addUpdateItem() {
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

    private func addConnectionsItem() {
        let root = NSMenuItem(
            title: MenuPresentation.connectionsTitle,
            action: nil,
            keyEquivalent: ""
        )
        root.image = symbol("cloud", description: "Cloud connections")
        let connections = NSMenu()
        connections.addItem(
            informational(MenuPresentation.connectionStatusTitle(connection: status?.connection ?? ""))
        )
        connections.addItem(informational("Access is set separately for each service"))
        if explicitConnectionsURL() != nil {
            connections.addItem(.separator())
            let manage = connections.addItem(
                withTitle: "Manage Access & Reauthorize…",
                action: #selector(openConnections),
                keyEquivalent: ""
            )
            manage.target = self
            manage.image = symbol("person.badge.key", description: "Manage connection access")
        } else if client != nil {
            connections.addItem(.separator())
            let details = connections.addItem(
                withTitle: "View Connections…",
                action: #selector(openConnections),
                keyEquivalent: ""
            )
            details.target = self
            details.image = symbol("list.bullet.rectangle", description: "View connections")
        } else {
            connections.addItem(informational("Connection management link unavailable"))
        }
        root.submenu = connections
        menu.addItem(root)
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

    @objc private func confirmPresetTest(_ sender: NSMenuItem) {
        guard
            let context = sender.representedObject as? ProfileTestContext,
            let printer = printers.first(where: { $0.printerID == context.printerID })
        else {
            return
        }

        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Test this print preset?"
        alert.informativeText =
            "Printer: \(printer.name)\nPreset: \(context.profileName)\n" +
            "This A4 page validates only the local macOS driver path."
        alert.addButton(withTitle: "Print Test Page")
        alert.addButton(withTitle: "Cancel")
        NSApplication.shared.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let profileID = context.profileID
        let printerID = printer.printerID

        performAction(successMessage: "Test page accepted for \(printer.name).") { client in
            _ = try await client.submitDriverTest(
                printerID: printerID,
                profileID: profileID
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

    @objc private func openQueue() {
        openNodeDashboard(view: "history", fallbackURL: dashboardURL())
    }

    @objc private func openConnections() {
        if let configured = explicitConnectionsURL() {
            NSWorkspace.shared.open(configured)
        } else {
            openNodeDashboard(view: "connections", fallbackURL: dashboardURL())
        }
    }

    @objc private func decideApplicationAuthorization(_ sender: NSMenuItem) {
        guard
            let context = sender.representedObject as? BrokerAuthorizationActionContext,
            !context.request.isExpired()
        else {
            refresh()
            return
        }
        let capabilityNames = context.request.requestedCapabilities
            .map(\.displayName)
            .joined(separator: "\n• ")
        let alert = NSAlert()
        alert.alertStyle = context.approved ? .warning : .informational
        alert.messageText = context.approved
            ? "Allow \(context.request.application.displayName) to use Piqae?"
            : "Deny \(context.request.application.displayName)?"
        alert.informativeText = context.approved
            ? "Only approve an app you recognize. Requested access:\n• \(capabilityNames)"
            : "The app will not receive a local capability. It may request access again later."
        alert.addButton(withTitle: context.approved ? "Approve" : "Deny")
        alert.addButton(withTitle: "Cancel")
        NSApplication.shared.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        guard !context.request.isExpired() else {
            showAlert(
                title: "Application request expired",
                message: "Ask the application to request access again. No access was granted."
            )
            refresh()
            return
        }
        let authorizationID = context.request.authorizationID
        let approved = context.approved
        let grantedCapabilities = approved ? context.request.requestedCapabilities : []

        performAction(successMessage: approved ? "Application access approved." : "Application access denied.") {
            client in
            try await client.decideBrokerAuthorization(
                authorizationID: authorizationID,
                approved: approved,
                grantedCapabilities: grantedCapabilities
            )
        }
    }

    private func openNodeDashboard(view: String, fallbackURL: URL?) {
        guard let client else {
            if let fallbackURL { NSWorkspace.shared.open(fallbackURL) }
            return
        }
        actionTask?.cancel()
        actionTask = Task { [weak self] in
            guard let self else { return }
            do {
                let url = try await client.createDashboardSession(view: view)
                NSWorkspace.shared.open(url)
            } catch is CancellationError {
                return
            } catch {
                if let fallbackURL {
                    NSWorkspace.shared.open(fallbackURL)
                } else {
                    showAlert(
                        title: "Queue unavailable",
                        message: error.localizedDescription
                    )
                }
            }
        }
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

    private func explicitConnectionsURL() -> URL? {
        let value = ProcessInfo.processInfo.environment["PIQAE_CONNECTIONS_URL"]
            ?? Bundle.main.object(forInfoDictionaryKey: "PiqaeConnectionsURL") as? String
        guard let value else { return nil }
        guard
            let url = URL(string: value),
            ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
            url.host != nil,
            url.user == nil,
            url.password == nil
        else { return nil }
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

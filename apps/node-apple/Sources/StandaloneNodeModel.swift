import Foundation
import PiqaeNodeKit
import PiqaeNodeKitAirPrint
import SwiftUI

@MainActor
final class StandaloneNodeModel: ObservableObject {
    @Published private(set) var snapshot: PiqaeNodeSnapshot?
    @Published private(set) var history: [PiqaeJobHistoryEntry] = []
    @Published private(set) var profiles: [PiqaePrinterID: [PiqaePrintProfile]] = [:]
    @Published private(set) var started = false
    @Published private(set) var backgroundMaintenanceStatus: String
    @Published private(set) var identityConflictRevision: UInt64?
    @Published var settings: StandaloneNodeSettings
    @Published var search = ""
    @Published var notice: String?
    @Published var errorMessage: String?
    @Published var isOnboardingPresented: Bool

    let connectionPolicy = PiqaeConnectionPolicy.standaloneUserManaged
    let runtime: PiqaeNativeRuntime
    let adapter: PiqaeAirPrintAdapter?
    let node: PiqaeNode
    let lifecycle: PiqaeUIKitLifecycleCoordinator

    private let store: StandaloneNodeStore
    private let maintenance: PiqaeUIKitMaintenanceScheduler?
    private let maintenanceRegistered: Bool
    private var observationTask: Task<Void, Never>?
    private var identityRevision: UInt64

    init(store: StandaloneNodeStore = StandaloneNodeStore()) {
        let loadedSettings = store.load()
        self.store = store
        settings = loadedSettings
        identityRevision = store.identityRevision
        isOnboardingPresented = !store.isConfigured
        backgroundMaintenanceStatus = "Not registered"
        let identity = (try? PiqaeNodeIdentityConfiguration(
            displayName: loadedSettings.name,
            site: loadedSettings.site,
            location: loadedSettings.location,
            labels: loadedSettings.labels
        )) ?? (try? PiqaeNodeIdentityConfiguration(displayName: "Piqae Node"))
        let host = identity.flatMap {
            try? PiqaeHostConfiguration(
                product: .standalone,
                applicationID: "com.piqae.node",
                identity: $0,
                installedHostPolicy: .isolatedApplication,
                connectionPolicy: .standaloneUserManaged
            )
        }
        runtime = PiqaeNativeRuntime(
            configuration: host?.nativeRuntimeConfiguration(
                availability: .backgroundOpportunistic,
                localOnly: false
            ) ?? PiqaeNativeRuntimeConfiguration(
                applicationID: "com.piqae.node",
                availability: .backgroundOpportunistic,
                localOnly: false,
                nodeName: "Piqae Node",
                hostname: "ios-application-host"
            )
        )
        let selectedAdapter = try? PiqaeAirPrintAdapter(
            identityProvider: runtime,
            knownPrinterURLs: store.printerURLs()
        )
        adapter = selectedAdapter
        let selectedNode = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: .backgroundOpportunistic,
                embeddedRuntime: runtime,
                printerAdapters: selectedAdapter.map { [$0] } ?? [],
                hostLifecycleReporter: runtime
            )
        )
        node = selectedNode
        lifecycle = PiqaeUIKitLifecycleCoordinator(node: selectedNode)
        let selectedMaintenance = try? PiqaeUIKitMaintenanceScheduler(
            node: selectedNode,
            identifier: "com.piqae.node.maintenance"
        )
        maintenance = selectedMaintenance
        maintenanceRegistered = selectedMaintenance?.register() == true
        backgroundMaintenanceStatus = maintenanceRegistered
            ? "Registered; iOS controls timing"
            : "Unavailable in this process"
    }

    deinit { observationTask?.cancel() }

    func start() async {
        guard !started else { return }
        do {
            try await node.start()
            started = true
            lifecycle.startObserving()
            if maintenanceRegistered {
                do {
                    try maintenance?.schedule(
                        earliestBeginDate: Date().addingTimeInterval(15 * 60)
                    )
                    backgroundMaintenanceStatus = "Requested; iOS controls timing"
                } catch {
                    backgroundMaintenanceStatus = "Registered; request not accepted"
                }
            }
            let stream = await node.observe()
            observationTask = Task { [weak self] in
                for await snapshot in stream {
                    guard !Task.isCancelled else { return }
                    self?.snapshot = snapshot
                }
            }
            await reconcilePendingIdentityUpdate()
            await refreshAll()
        } catch {
            errorMessage = Self.message(error)
        }
    }

    func saveIdentity() async {
        do {
            let identity = try PiqaeNodeIdentityConfiguration(
                displayName: settings.name,
                site: settings.site,
                location: settings.location,
                labels: settings.labels
            )
            if started {
                let updated = try await node.identity.update(.init(
                    expectedRevision: identityRevision,
                    identity: identity
                ))
                try await runtime.updateEnrollmentNodeName(updated.identity.displayName)
                identityRevision = updated.revision
                identityConflictRevision = nil
                store.save(updated.identity, revision: updated.revision)
                store.markIdentityUpdatePending(false)
            } else {
                try await runtime.updateEnrollmentNodeName(identity.displayName)
                store.save(identity, revision: identityRevision)
                store.markIdentityUpdatePending(true)
            }
            isOnboardingPresented = false
            notice = started
                ? "Node details saved. Connected workspaces will reconcile independently."
                : "Node details saved locally. Each new connection uses this node name."
        } catch let PiqaeNativeRuntimeError.nodeIdentityRevisionConflict(currentRevision) {
            identityRevision = currentRevision
            identityConflictRevision = currentRevision
            store.saveIdentityRevision(currentRevision)
        } catch {
            errorMessage = Self.message(error)
        }
    }

    func refreshAll() async {
        guard started else { return }
        do {
            try await node.printers.refresh()
            history = try await node.jobs.history(offset: 0, limit: 200).jobs
            let printers = try await node.printers.list()
            var loaded: [PiqaePrinterID: [PiqaePrintProfile]] = [:]
            for printer in printers {
                loaded[printer.id] = try await node.profiles.list(for: printer.id)
            }
            profiles = loaded
            errorMessage = nil
        } catch {
            errorMessage = Self.message(error)
        }
    }

    func addAirPrintPrinter() async {
        guard let adapter else {
            errorMessage = "The native Preview runtime is not linked in this build."
            return
        }
        do {
            guard let selected = try await PiqaeAirPrintPicker.selectPrinter() else { return }
            let safe = try StandaloneNodeStore.safePrinterURL(selected)
            try await adapter.register(printerURL: safe)
            try store.addPrinterURL(safe)
            try await node.printers.refresh()
        } catch {
            errorMessage = Self.message(error)
        }
    }

    func connect(authority: String, invitation: String) async -> Bool {
        guard let url = URL(string: authority) else {
            errorMessage = "Enter a valid HTTPS Piqae server address."
            return false
        }
        do {
            try connectionPolicy.validateAuthority(url)
            let cloud = try PiqaeCloudConfiguration(
                authorityURL: url,
                invitation: PiqaeSensitiveString(invitation)
            )
            _ = try await node.connections.connect(cloud)
            notice = "Connection added. The invitation was not retained."
            return true
        } catch {
            errorMessage = Self.message(error)
            return false
        }
    }

    func disconnect(_ connection: PiqaeConnection) async {
        do {
            try await node.connections.disconnect(connection.id)
            notice = "Connection removed from this node."
        } catch {
            errorMessage = Self.message(error)
        }
    }

    func handleBackgroundPush(collapseID: String) async -> Bool {
        await lifecycle.handleBackgroundPush(collapseID: collapseID) == .reconciled
    }

    private func reconcilePendingIdentityUpdate() async {
        guard store.isIdentityUpdatePending else { return }
        do {
            let identity = try PiqaeNodeIdentityConfiguration(
                displayName: settings.name,
                site: settings.site,
                location: settings.location,
                labels: settings.labels
            )
            let updated = try await node.identity.update(.init(
                expectedRevision: identityRevision,
                identity: identity
            ))
            identityRevision = updated.revision
            identityConflictRevision = nil
            store.save(updated.identity, revision: updated.revision)
            store.markIdentityUpdatePending(false)
        } catch let PiqaeNativeRuntimeError.nodeIdentityRevisionConflict(currentRevision) {
            identityRevision = currentRevision
            identityConflictRevision = currentRevision
            store.saveIdentityRevision(currentRevision)
        } catch {
            errorMessage = Self.message(error)
        }
    }

    var visibleHistory: [PiqaeJobHistoryEntry] {
        guard !search.isEmpty else { return history }
        return history.filter {
            $0.title.localizedCaseInsensitiveContains(search)
                || $0.state.localizedCaseInsensitiveContains(search)
                || $0.jobID.rawValue.localizedCaseInsensitiveContains(search)
        }
    }

    private static func message(_ error: Error) -> String {
        (error as? LocalizedError)?.errorDescription ?? "The operation could not be completed."
    }
}

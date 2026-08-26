import Foundation
import PiqaeNodeKit

/// Compatibility attachment for the currently shipped loopback local API.
/// The stable SDK boundary is `PiqaeInstalledNodeIPC`; this implementation can
/// move to the versioned Unix-domain-socket broker without changing app code.
public struct PiqaeMacInstalledNodeIPC: PiqaeInstalledNodeIPC {
    private let statusProvider: @Sendable () async throws -> LocalStatus
    private let printersProvider: @Sendable () async throws -> [LocalPrinter]
    private let profilesProvider: @Sendable (String) async throws -> [LocalPrintProfile]

    public init(client: LocalAPIClient) {
        statusProvider = { try await client.status() }
        printersProvider = { try await client.printers() }
        profilesProvider = { printerID in try await client.profiles(printerID: printerID) }
    }

    init(
        status: @escaping @Sendable () async throws -> LocalStatus,
        printers: @escaping @Sendable () async throws -> [LocalPrinter],
        profiles: @escaping @Sendable (String) async throws -> [LocalPrintProfile] = { _ in [] }
    ) {
        statusProvider = status
        printersProvider = printers
        profilesProvider = profiles
    }

    public func probe() async -> PiqaeInstalledNodeProbe {
        do {
            _ = try await statusProvider()
            return .init(state: .available(protocolVersion: 1))
        } catch {
            return .init(state: .unavailable)
        }
    }

    public func snapshot() async throws -> PiqaeNodeSnapshot {
        async let statusRequest = statusProvider()
        async let printerRequest = printersProvider()
        let (status, printers) = try await (statusRequest, printerRequest)
        let now = Date()
        let connection = mapConnection(status)
        return PiqaeNodeSnapshot(
            installationID: status.agentID.map { .init(rawValue: $0) },
            hostMode: .userAgent,
            availability: .continuousWhileAwake,
            phase: status.paused ? .suspended : mapPhase(status.connection),
            connections: [connection],
            printers: printers.map { mapPrinter($0, now: now) },
            lastUpdatedAt: now
        )
    }

    public func profiles(for printerID: PiqaePrinterID) async throws -> [PiqaePrintProfile] {
        try await profilesProvider(printerID.rawValue).map { profile in
            PiqaePrintProfile(
                id: .init(rawValue: profile.profileID),
                printerID: printerID,
                name: profile.name,
                revision: profile.revision ?? 0,
                isDefault: profile.isDefault ?? false
            )
        }
    }

    private func mapConnection(_ status: LocalStatus) -> PiqaeConnection {
        if status.connection == "local_only" { return .localOnly }
        let state: PiqaeConnectionState
        switch status.connection {
        case "connected": state = .connected
        case "unauthorized", "needs_reauthorization": state = .needsReauthorization
        case "connecting": state = .connecting
        default: state = .offline
        }
        return PiqaeConnection(
            id: .init(rawValue: "installed_node_connection"),
            authorityURL: nil,
            workspaceName: status.workspaceName,
            state: state
        )
    }

    private func mapPhase(_ connection: String) -> PiqaeNodePhase {
        switch connection {
        case "connected", "local_only": .ready
        case "connecting": .starting
        default: .degraded
        }
    }

    private func mapPrinter(_ printer: LocalPrinter, now: Date) -> PiqaePrinter {
        let state: PiqaePrinterState
        switch printer.state.lowercased() {
        case "idle", "online", "available": state = .available
        case "printing", "processing", "busy": state = .busy
        case "paused": state = .paused
        case "offline", "unavailable": state = .offline
        default: state = .unknown
        }
        let queue = printer.queueCounts.map { counts in
            let total = min(UInt64(UInt32.max), UInt64(counts.queued) + UInt64(counts.active))
            return PiqaeQueueObservation(
                piqaeOwned: UInt32(total),
                observedAt: now,
                freshUntil: now.addingTimeInterval(5)
            )
        }
        return PiqaePrinter(
            id: .init(rawValue: printer.printerID),
            adapterID: "piqae.installed-node",
            nativeID: printer.nativeID ?? printer.printerID,
            displayName: printer.name,
            state: state,
            queue: queue,
            observedAt: now,
            freshUntil: now.addingTimeInterval(5)
        )
    }
}

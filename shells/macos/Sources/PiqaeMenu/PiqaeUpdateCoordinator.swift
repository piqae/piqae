import Foundation
import Sparkle
import PiqaeMenuCore

@MainActor
final class PiqaeUpdateCoordinator: NSObject, SPUUpdaterDelegate {
    private static let pollInterval: Duration = .seconds(2)

    private let client: LocalAPIClient?
    private(set) var updaterController: SPUStandardUpdaterController?
    private var latestStatus: LocalStatus?
    private var foregroundOperationCount = 0
    private var handoffTask: Task<Void, Never>?
    private var availableVersion: String?

    var onPresentationChange: (() -> Void)?

    var isEnabled: Bool {
        updaterController != nil
    }

    var canCheckForUpdates: Bool {
        presentation.canOpenUpdater
            && (updaterController?.updater.canCheckForUpdates ?? false)
    }

    var presentation: UpdateMenuPresentation {
        guard updaterController != nil else {
            return .unavailable
        }
        if handoffTask != nil, let availableVersion {
            return .waitingForIdle(version: availableVersion)
        }
        if let availableVersion {
            return .available(version: availableVersion)
        }
        return .readyToCheck
    }

    init(client: LocalAPIClient?, bundle: Bundle = .main) {
        self.client = client
        super.init()

        guard Self.hasTrustedUpdateConfiguration(bundle: bundle) else {
            return
        }
        updaterController = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: self,
            userDriverDelegate: nil
        )
    }

    deinit {
        handoffTask?.cancel()
    }

    func observe(status: LocalStatus?) {
        latestStatus = status
    }

    func beginForegroundOperation() {
        foregroundOperationCount += 1
    }

    func endForegroundOperation() {
        foregroundOperationCount = max(0, foregroundOperationCount - 1)
    }

    func checkForUpdates(_ sender: Any?) {
        updaterController?.checkForUpdates(sender)
    }

    func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
        availableVersion = item.displayVersionString
        onPresentationChange?()
    }

    func updaterDidNotFindUpdate(_ updater: SPUUpdater) {
        availableVersion = nil
        onPresentationChange?()
    }

    func updater(
        _ updater: SPUUpdater,
        shouldPostponeRelaunchForUpdate item: SUAppcastItem,
        untilInvokingBlock installHandler: @escaping () -> Void
    ) -> Bool {
        let readiness = UpdateHandoffReadiness(
            status: latestStatus,
            foregroundOperation: foregroundOperationCount > 0
        )
        guard !readiness.canReplaceNativeComponents else {
            return false
        }

        handoffTask?.cancel()
        handoffTask = Task { @MainActor [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                if let client {
                    latestStatus = try? await client.status()
                }
                let currentReadiness = UpdateHandoffReadiness(
                    status: latestStatus,
                    foregroundOperation: foregroundOperationCount > 0
                )
                if currentReadiness.canReplaceNativeComponents {
                    installHandler()
                    return
                }
                try? await Task.sleep(for: Self.pollInterval)
            }
        }
        onPresentationChange?()
        return true
    }

    func updater(_ updater: SPUUpdater, didAbortWithError error: any Error) {
        handoffTask?.cancel()
        handoffTask = nil
        onPresentationChange?()
    }

    private static func hasTrustedUpdateConfiguration(bundle: Bundle) -> Bool {
        guard
            bundle.object(forInfoDictionaryKey: "PiqaeUpdatesEnabled") as? Bool == true,
            let feedValue = bundle.object(forInfoDictionaryKey: "SUFeedURL") as? String,
            let feedURL = URL(string: feedValue),
            feedURL.scheme?.lowercased() == "https",
            feedURL.host != nil,
            feedURL.user == nil,
            feedURL.password == nil,
            feedURL.fragment == nil,
            let publicKey = bundle.object(forInfoDictionaryKey: "SUPublicEDKey") as? String,
            Data(base64Encoded: publicKey)?.count == 32
        else {
            return false
        }
        return true
    }
}

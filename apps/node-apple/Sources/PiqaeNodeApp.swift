import SwiftUI
import UIKit

@main
struct PiqaeNodeApp: App {
    @UIApplicationDelegateAdaptor(PiqaeNodeAppDelegate.self) private var appDelegate
    @StateObject private var model = StandaloneNodeModel()

    var body: some Scene {
        WindowGroup {
            StandaloneNodeRootView(model: model)
                .preferredColorScheme(nil)
                .task {
                    appDelegate.install(model: model)
                    await model.start()
                    if model.started {
                        appDelegate.modelDidStart()
                    } else {
                        appDelegate.modelStartFailed()
                    }
                }
        }
    }
}

@MainActor
final class PiqaeNodeAppDelegate: NSObject, UIApplicationDelegate {
    private struct PendingHint {
        var completions: [(UIBackgroundFetchResult) -> Void]
        var timeout: Task<Void, Never>?
        var worker: Task<Void, Never>?
    }

    private weak var model: (any StandaloneWakeHandling)?
    private var modelIsReady = false
    private var pendingOrder: [String] = []
    private var pending: [String: PendingHint] = [:]
    private let wakeDeadlineSeconds: Double
    private let maximumPendingHints: Int
    private let maximumCompletionsPerHint: Int

    override convenience init() {
        self.init(
            wakeDeadlineSeconds: 20,
            maximumPendingHints: 32,
            maximumCompletionsPerHint: 8
        )
    }

    init(
        wakeDeadlineSeconds: Double,
        maximumPendingHints: Int,
        maximumCompletionsPerHint: Int = 8
    ) {
        self.wakeDeadlineSeconds = max(0.01, wakeDeadlineSeconds)
        self.maximumPendingHints = max(1, maximumPendingHints)
        self.maximumCompletionsPerHint = max(1, maximumCompletionsPerHint)
        super.init()
    }

    func install(model: any StandaloneWakeHandling) {
        self.model = model
    }

    func modelDidStart() {
        modelIsReady = true
        drainPendingHints()
    }

    func modelStartFailed() {
        modelIsReady = false
        let collapseIDs = pendingOrder
        for collapseID in collapseIDs { finish(collapseID, result: .noData) }
    }

    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any],
        fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        guard let collapseID = StandaloneWakeHintEnvelope.collapseID(from: userInfo) else {
            completionHandler(.noData)
            return
        }
        enqueue(collapseID: collapseID, completion: completionHandler)
    }

    private func enqueue(
        collapseID: String,
        completion: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        if var current = pending[collapseID] {
            guard current.completions.count < maximumCompletionsPerHint else {
                completion(.noData)
                return
            }
            current.completions.append(completion)
            pending[collapseID] = current
            return
        }
        guard pending.count < maximumPendingHints else {
            completion(.noData)
            return
        }
        pendingOrder.append(collapseID)
        var item = PendingHint(completions: [completion], timeout: nil, worker: nil)
        item.timeout = Task { @MainActor [weak self] in
            guard let self else { return }
            try? await Task.sleep(for: .seconds(self.wakeDeadlineSeconds))
            guard !Task.isCancelled else { return }
            self.finish(collapseID, result: .noData)
        }
        pending[collapseID] = item
        if modelIsReady { start(collapseID) }
    }

    private func drainPendingHints() {
        for collapseID in pendingOrder { start(collapseID) }
    }

    private func start(_ collapseID: String) {
        guard var item = pending[collapseID], item.worker == nil, model != nil else { return }
        item.worker = Task { @MainActor [weak self] in
            guard let self, let model = self.model else { return }
            let reconciled = await model.handleBackgroundPush(collapseID: collapseID)
            guard !Task.isCancelled else { return }
            self.finish(collapseID, result: reconciled ? .newData : .noData)
        }
        pending[collapseID] = item
    }

    private func finish(_ collapseID: String, result: UIBackgroundFetchResult) {
        guard let item = pending.removeValue(forKey: collapseID) else { return }
        pendingOrder.removeAll { $0 == collapseID }
        item.timeout?.cancel()
        item.worker?.cancel()
        for completion in item.completions { completion(result) }
    }
}

@MainActor
protocol StandaloneWakeHandling: AnyObject {
    func handleBackgroundPush(collapseID: String) async -> Bool
}

extension StandaloneNodeModel: StandaloneWakeHandling {}

/// The standalone host accepts only an opaque reconciliation hint. Documents,
/// job identifiers, printer identifiers, workspace data, and visible alerts do
/// not belong in a Piqae background notification.
enum StandaloneWakeHintEnvelope {
    static func collapseID(from userInfo: [AnyHashable: Any]) -> String? {
        let topLevelKeys = Set(userInfo.keys.compactMap { $0 as? String })
        guard topLevelKeys.count == userInfo.count,
            topLevelKeys.isSubset(of: ["aps", "piqae_wake_hint"]),
            let rawCollapseID = userInfo["piqae_wake_hint"] as? String,
            let aps = userInfo["aps"] as? [String: Any],
            Set(aps.keys).isSubset(of: ["content-available"]),
            (aps["content-available"] as? NSNumber)?.intValue == 1
        else {
            return nil
        }
        let collapseID = rawCollapseID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !collapseID.isEmpty, collapseID.utf8.count <= 128,
            collapseID.unicodeScalars.allSatisfy({
                !CharacterSet.controlCharacters.contains($0)
            })
        else { return nil }
        return collapseID
    }
}

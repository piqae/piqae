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
                }
        }
    }
}

@MainActor
final class PiqaeNodeAppDelegate: NSObject, UIApplicationDelegate {
    private weak var model: StandaloneNodeModel?

    func install(model: StandaloneNodeModel) {
        self.model = model
    }

    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any],
        fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        let collapseID = (userInfo["piqae_wake_hint"] as? String) ?? "remote-change"
        Task { @MainActor [weak self] in
            guard let self, let model = self.model else {
                completionHandler(.noData)
                return
            }
            let reconciled = await model.handleBackgroundPush(collapseID: collapseID)
            completionHandler(reconciled ? .newData : .noData)
        }
    }
}

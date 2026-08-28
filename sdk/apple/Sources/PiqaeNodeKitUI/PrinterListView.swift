import PiqaeNodeKit
import SwiftUI

@MainActor
public final class PiqaeNodeViewModel: ObservableObject {
    @Published public private(set) var snapshot: PiqaeNodeSnapshot?
    @Published public private(set) var errorMessage: String?

    public let node: PiqaeNode
    private var observationTask: Task<Void, Never>?
    private var isStarting = false

    public init(node: PiqaeNode) {
        self.node = node
    }

    deinit { observationTask?.cancel() }

    public func start() async {
        guard observationTask == nil, !isStarting else { return }
        isStarting = true
        defer { isStarting = false }
        do {
            try await node.start()
            let stream = await node.observe()
            observationTask = Task { [weak self] in
                for await snapshot in stream {
                    guard !Task.isCancelled else { return }
                    self?.snapshot = snapshot
                }
            }
        } catch {
            errorMessage = (error as? LocalizedError)?.errorDescription
                ?? "Piqae could not start."
        }
    }

    public func refresh() async {
        do {
            try await node.printers.refresh()
            errorMessage = nil
        } catch {
            errorMessage = (error as? LocalizedError)?.errorDescription
                ?? "Printers could not be refreshed."
        }
    }

    public func dismissError() {
        errorMessage = nil
    }
}

/// Optional low-code printer inventory. Apps can instead observe `PiqaeNode`
/// and build a completely custom interface from the same services.
public struct PiqaePrinterListView: View {
    @StateObject private var model: PiqaeNodeViewModel

    public init(node: PiqaeNode) {
        _model = StateObject(wrappedValue: PiqaeNodeViewModel(node: node))
    }

    public var body: some View {
        Group {
            if let snapshot = model.snapshot {
                List {
                    Section {
                        LabeledContent("Availability", value: availabilityTitle(snapshot.availability))
                        LabeledContent("Mode", value: hostModeTitle(snapshot.hostMode))
                    } header: {
                        Text("Piqae node")
                    }

                    Section("Printers") {
                        if snapshot.printers.isEmpty {
                            VStack(alignment: .leading, spacing: 6) {
                                Label("No printers available", systemImage: "printer")
                                    .font(.headline)
                                Text(emptyMessage(snapshot))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        } else {
                            ForEach(snapshot.printers) { printer in
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(printer.displayName)
                                    Text(printerDetail(printer))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                .accessibilityElement(children: .combine)
                            }
                        }
                    }
                }
                .refreshable { await model.refresh() }
            } else {
                ProgressView("Starting Piqae…")
            }
        }
        .task { await model.start() }
        .alert(
            "Piqae needs attention",
            isPresented: Binding(
                get: { model.errorMessage != nil },
                set: { if !$0 { model.dismissError() } }
            )
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(model.errorMessage ?? "")
        }
    }

    private func emptyMessage(_ snapshot: PiqaeNodeSnapshot) -> String {
        #if os(iOS)
        "Select an AirPrint printer or configure a certified printer adapter in this app."
        #else
        snapshot.hostMode == .attachedClient
            ? "Open the installed Piqae node to configure a printer."
            : "Configure a printer adapter for this embedded node."
        #endif
    }

    private func printerDetail(_ printer: PiqaePrinter) -> String {
        var parts = [printer.state.rawValue.replacingOccurrences(of: "_", with: " ")]
        if let model = printer.model { parts.append(model) }
        if let queue = printer.queue {
            let total = queue.piqaeOwned + queue.external + queue.unknown
            if total > 0 { parts.append("\(total) queued") }
        }
        return parts.joined(separator: " · ")
    }

    private func availabilityTitle(_ availability: PiqaeNodeAvailabilityClass) -> String {
        availability.rawValue.replacingOccurrences(of: "_", with: " ").capitalized
    }

    private func hostModeTitle(_ hostMode: PiqaeNodeHostMode) -> String {
        hostMode.rawValue.replacingOccurrences(of: "_", with: " ").capitalized
    }
}

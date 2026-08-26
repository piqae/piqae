import PiqaeNodeKit
import SwiftUI

struct StandaloneNodeRootView: View {
    @ObservedObject var model: StandaloneNodeModel

    var body: some View {
        TabView {
            NavigationStack { OverviewView(model: model) }
                .tabItem { Label("Overview", systemImage: "gauge.with.dots.needle.67percent") }
            NavigationStack { PrintersView(model: model) }
                .tabItem { Label("Printers", systemImage: "printer") }
            NavigationStack { QueueView(model: model) }
                .tabItem { Label("Queue", systemImage: "list.number") }
            NavigationStack { HistoryView(model: model) }
                .tabItem { Label("History", systemImage: "clock.arrow.circlepath") }
            NavigationStack { ConnectionsView(model: model) }
                .tabItem { Label("Connections", systemImage: "link") }
            NavigationStack { NodeSettingsView(model: model) }
                .tabItem { Label("Node", systemImage: "gearshape") }
        }
        .sheet(isPresented: $model.isOnboardingPresented) {
            OnboardingView(model: model)
                .interactiveDismissDisabled()
        }
        .alert(
            "Piqae needs attention",
            isPresented: Binding(
                get: { !model.isOnboardingPresented && model.errorMessage != nil },
                set: { if !$0 { model.errorMessage = nil } }
            )
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(model.errorMessage ?? "")
        }
        .overlay(alignment: .top) {
            if let notice = model.notice {
                Text(notice)
                    .font(.callout)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(.regularMaterial, in: Capsule())
                    .accessibilityAddTraits(.isStaticText)
                    .onTapGesture { model.notice = nil }
                    .padding()
            }
        }
    }
}

private struct OverviewView: View {
    @ObservedObject var model: StandaloneNodeModel

    var body: some View {
        List {
            Section("Node") {
                LabeledContent("Node name", value: model.settings.name)
                if !model.settings.site.isEmpty {
                    LabeledContent("Site", value: model.settings.site)
                }
                if !model.settings.location.isEmpty {
                    LabeledContent("Location", value: model.settings.location)
                }
                LabeledContent("Runtime", value: phase)
                LabeledContent("Availability", value: availability)
            }
            Section("At a glance") {
                LabeledContent("Printers", value: "\(model.snapshot?.printers.count ?? 0)")
                LabeledContent("Connections", value: "\(model.snapshot?.connections.count ?? 0)")
                LabeledContent("Retained jobs", value: "\(model.history.count)")
            }
            Section("Background delivery") {
                Label("Opportunistic on iPhone and iPad", systemImage: "moon.zzz")
                Text("Piqae retries when the app is foregrounded, receives a permitted background hint, or iOS grants maintenance time. Force-quit, suspended, powered-off, or unreachable devices are unavailable routes.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
        .navigationTitle("Piqae Node")
        .toolbar { refreshButton(model) }
    }

    private var phase: String {
        model.snapshot?.phase.rawValue.replacingOccurrences(of: "_", with: " ").capitalized
            ?? (model.started ? "Starting" : "Stopped")
    }

    private var availability: String {
        model.snapshot?.availability.rawValue.replacingOccurrences(of: "_", with: " ").capitalized
            ?? "Background opportunistic"
    }
}

private struct PrintersView: View {
    @ObservedObject var model: StandaloneNodeModel

    var body: some View {
        List {
            if model.snapshot?.printers.isEmpty != false {
                EmptyState(
                    title: "No printers selected",
                    systemImage: "printer",
                    message: "Choose an AirPrint printer or install a reviewed vendor adapter."
                )
            } else {
                ForEach(model.snapshot?.printers ?? []) { printer in
                    NavigationLink {
                        PrinterDetailView(printer: printer, profiles: model.profiles[printer.id] ?? [])
                    } label: {
                        VStack(alignment: .leading, spacing: 5) {
                            Text(printer.displayName).font(.headline)
                            Text(printerSummary(printer))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        }
        .navigationTitle("Printers")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    Task { await model.addAirPrintPrinter() }
                } label: {
                    Label("Add printer", systemImage: "plus")
                }
            }
            refreshButton(model)
        }
    }

    private func printerSummary(_ printer: PiqaePrinter) -> String {
        let profileCount = model.profiles[printer.id]?.count ?? 0
        let stock = printer.loadedMedia == nil ? "stock not reported" : "stock observed"
        return "\(printer.state.rawValue.capitalized) · \(profileCount) profiles · \(stock)"
    }
}

private struct PrinterDetailView: View {
    let printer: PiqaePrinter
    let profiles: [PiqaePrintProfile]

    var body: some View {
        List {
            Section("Printer") {
                LabeledContent("Status", value: printer.state.rawValue.capitalized)
                LabeledContent("Adapter", value: printer.adapterID)
                if let model = printer.model { LabeledContent("Model", value: model) }
                if let location = printer.location { LabeledContent("Printer location", value: location) }
                LabeledContent("Last observed", value: printer.observedAt.formatted())
            }
            Section("Profiles") {
                if profiles.isEmpty {
                    Text("No profiles reported by this adapter.")
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(profiles) { profile in
                        LabeledContent(profile.name, value: profile.isDefault ? "Default" : "Ready")
                    }
                }
            }
            Section("Media and stock") {
                if let media = printer.loadedMedia {
                    LabeledContent("Loaded media", value: media.media.displayName)
                    LabeledContent("Confidence", value: media.confidence.formatted(.percent))
                } else {
                    Text("Not reported. This is different from an empty or out-of-stock tray.")
                        .foregroundStyle(.secondary)
                }
                if printer.capabilities.supportedMedia.isEmpty {
                    LabeledContent("Supported media", value: "Not reported")
                } else {
                    ForEach(printer.capabilities.supportedMedia) { media in
                        Text(media.displayName)
                    }
                }
            }
            Section("Queue occupancy") {
                if let queue = printer.queue {
                    LabeledContent("Piqae jobs", value: "\(queue.piqaeOwned)")
                    LabeledContent("External jobs", value: "\(queue.external)")
                    LabeledContent("Unclassified jobs", value: "\(queue.unknown)")
                } else {
                    Text("Native queue counts are not reported by this adapter.")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .navigationTitle(printer.displayName)
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct QueueView: View {
    @ObservedObject var model: StandaloneNodeModel

    private var active: [PiqaeJobHistoryEntry] {
        model.history.filter {
            !["completed_reported", "failed_terminal", "cancelled", "delivery_uncertain"]
                .contains($0.state)
        }
    }

    var body: some View {
        List {
            if active.isEmpty {
                EmptyState(
                    title: "Queue is clear",
                    systemImage: "checkmark.circle",
                    message: "Durable jobs waiting for or crossing native handoff appear here."
                )
            } else {
                ForEach(active) { job in JobRow(job: job) }
            }
        }
        .navigationTitle("Queue")
        .toolbar { refreshButton(model) }
    }
}

private struct HistoryView: View {
    @ObservedObject var model: StandaloneNodeModel

    var body: some View {
        List {
            if model.visibleHistory.isEmpty {
                EmptyState(
                    title: model.search.isEmpty ? "No retained print history" : "No matching jobs",
                    systemImage: "clock.arrow.circlepath",
                    message: model.search.isEmpty
                        ? "Completed and uncertain native handoffs appear here."
                        : "Try another title, state, or job ID."
                )
            } else {
                ForEach(model.visibleHistory) { job in JobRow(job: job) }
            }
        }
        .navigationTitle("Print history")
        .searchable(text: $model.search, prompt: "Title, state, or job ID")
        .toolbar { refreshButton(model) }
    }
}

private struct JobRow: View {
    let job: PiqaeJobHistoryEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(job.title).font(.headline)
            Text(job.state.replacingOccurrences(of: "_", with: " ").capitalized)
                .font(.caption)
                .foregroundStyle(.secondary)
            if let createdAt = job.createdAt {
                Text(createdAt.formatted()).font(.caption2).foregroundStyle(.tertiary)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

private struct EmptyState: View {
    let title: String
    let systemImage: String
    let message: String

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: systemImage).font(.largeTitle).foregroundStyle(.secondary)
            Text(title).font(.headline)
            Text(message).font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 36)
        .accessibilityElement(children: .combine)
    }
}

private struct ConnectionsView: View {
    @ObservedObject var model: StandaloneNodeModel
    @State private var authority = "https://api.piqae.com"
    @State private var invitation = ""
    @State private var isAdding = false

    var body: some View {
        List {
            Section {
                if model.snapshot?.connections.isEmpty != false {
                    Text("No cloud or self-hosted workspaces are connected.")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.snapshot?.connections ?? []) { connection in
                    VStack(alignment: .leading, spacing: 5) {
                        Text(connection.workspaceName ?? "Connected workspace").font(.headline)
                        Text(connection.authorityURL?.host ?? "Local")
                            .font(.caption).foregroundStyle(.secondary)
                        Text(connection.state.rawValue.replacingOccurrences(of: "_", with: " ").capitalized)
                            .font(.caption)
                        if connection.state != .localOnly {
                            Button("Disconnect", role: .destructive) {
                                Task { await model.disconnect(connection) }
                            }
                        }
                    }
                }
            } header: {
                Text("Who can use this node")
            } footer: {
                Text("Each connection has isolated credentials and cloud state, while all connections share this node's one durable local queue.")
            }
        }
        .navigationTitle("Connections")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { isAdding = true } label: { Label("Add connection", systemImage: "plus") }
            }
        }
        .sheet(isPresented: $isAdding) {
            NavigationStack {
                Form {
                    Section("Piqae server") {
                        TextField("HTTPS authority", text: $authority)
                            .textInputAutocapitalization(.never)
                            .keyboardType(.URL)
                    }
                    Section("One-time invitation") {
                        SecureField("Invitation", text: $invitation)
                            .textInputAutocapitalization(.never)
                        Text("The invitation is exchanged by the durable node runtime and is never retained by this screen.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                .navigationTitle("Add connection")
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { invitation = ""; isAdding = false }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Connect") {
                            Task {
                                if await model.connect(authority: authority, invitation: invitation) {
                                    invitation = ""
                                    isAdding = false
                                }
                            }
                        }
                        .disabled(invitation.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
        }
    }
}

private struct NodeSettingsView: View {
    @ObservedObject var model: StandaloneNodeModel

    var body: some View {
        Form {
            IdentityFields(settings: $model.settings)
            Section("Runtime diagnostics") {
                LabeledContent("Host product", value: "Standalone")
                LabeledContent("Queue authority", value: "Shared Rust runtime")
                LabeledContent(
                    "Native runtime",
                    value: PiqaeNativeRuntime.linkedLibraryAvailable ? "Linked" : "Unavailable in this build"
                )
                LabeledContent("Remote wake", value: "Best effort")
                LabeledContent("Maintenance", value: model.backgroundMaintenanceStatus)
                LabeledContent("After force quit", value: "Unavailable")
                LabeledContent("Last snapshot", value: model.snapshot?.lastUpdatedAt.formatted() ?? "None")
            }
            Section {
                Button("Save node details") { Task { await model.saveIdentity() } }
            } footer: {
                Text("Piqae does not infer or upload the logged-in user, contacts, postal address, device serial number, or advertising identifier.")
            }
        }
        .navigationTitle("Node")
    }
}

private struct OnboardingView: View {
    @ObservedObject var model: StandaloneNodeModel

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Label("One durable queue", systemImage: "externaldrive.connected.to.line.below")
                    Label("Many isolated connections", systemImage: "link")
                    Label("Background availability reported honestly", systemImage: "moon.zzz")
                } header: {
                    Text("Set up this iPhone or iPad as a Piqae node")
                }
                IdentityFields(settings: $model.settings)
                Section {
                    Button("Create node") { Task { await model.saveIdentity() } }
                        .buttonStyle(.borderedProminent)
                } footer: {
                    Text("The suggested name is generic on iOS. You can rename the node at any time.")
                }
            }
            .navigationTitle("Welcome to Piqae")
        }
    }
}

private struct IdentityFields: View {
    @Binding var settings: StandaloneNodeSettings

    var body: some View {
        Section("Node details") {
            TextField("Node name", text: $settings.name)
            TextField("Site (optional)", text: $settings.site)
            TextField("Location (optional)", text: $settings.location)
            TextField(
                "Labels, separated by commas",
                text: Binding(
                    get: { settings.labels.joined(separator: ", ") },
                    set: { value in
                        settings.labels = value.split(separator: ",", omittingEmptySubsequences: true)
                            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                    }
                )
            )
        }
    }
}

@ToolbarContentBuilder
private func refreshButton(_ model: StandaloneNodeModel) -> some ToolbarContent {
    ToolbarItem(placement: .secondaryAction) {
        Button { Task { await model.refreshAll() } } label: {
            Label("Refresh", systemImage: "arrow.clockwise")
        }
    }
}

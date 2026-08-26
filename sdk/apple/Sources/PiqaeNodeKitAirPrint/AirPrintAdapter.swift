import Foundation
import PiqaeNodeKit

#if os(iOS)
import UIKit

public actor PiqaeAirPrintAdapter: PiqaePrinterAdapter {
    public nonisolated let adapterID = "apple.airprint"
    public nonisolated let descriptor = PiqaePrinterAdapterDescriptor(
        id: "apple.airprint",
        displayName: "AirPrint",
        version: "1",
        transports: [.airPrint, .ipp],
        portableOptions: [.orientation],
        supportsProfiles: false
    )
    public nonisolated let runtimeFingerprint = PiqaeAdapterFingerprint(
        platform: .iosAirPrint,
        adapterID: "apple.airprint",
        adapterVersion: "1"
    )
    private var knownPrinterURLs: Set<URL>
    private let identityProvider: any PiqaeOpaqueIdentityProvider

    /// AirPrint doesn't expose silent network enumeration. Supply printers the
    /// user selected with `PiqaeAirPrintPicker`, then retain their URLs in the
    /// host app and register them again at next launch.
    public init(
        identityProvider: any PiqaeOpaqueIdentityProvider,
        knownPrinterURLs: [URL] = []
    ) throws {
        self.identityProvider = identityProvider
        self.knownPrinterURLs = try Set(
            knownPrinterURLs.map { try PiqaeAirPrintEndpoint.canonicalize($0).route }
        )
    }

    public func register(printerURL: URL) throws {
        knownPrinterURLs.insert(try PiqaeAirPrintEndpoint.canonicalize(printerURL).route)
    }

    public func forget(printerURL: URL) {
        guard let route = try? PiqaeAirPrintEndpoint.canonicalize(printerURL).route else { return }
        knownPrinterURLs.remove(route)
    }

    public func discoverPrinters() async throws -> [PiqaePrinter] {
        var printers: [PiqaePrinter] = []
        for url in knownPrinterURLs.sorted(by: { $0.absoluteString < $1.absoluteString }) {
            let id = try await id(for: url)
            printers.append(await Self.contact(url, id: id))
        }
        return printers
    }

    public func validate(_ request: PiqaePrintRequest, for printer: PiqaePrinter) async throws {
        guard request.intent.copies == 1 else {
            throw PiqaeNodeError.unsupportedOperation(
                "Direct AirPrint submission supports one copy per SDK job."
            )
        }
        guard request.intent.media == nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "AirPrint cannot pin a portable media name without presenting the system print UI."
            )
        }
        guard
            request.intent.cut == nil
                || request.intent.cut == PiqaePortablePrintIntent.Cut.none
        else {
            throw PiqaeNodeError.unsupportedOperation(
                "Cutter intent requires a certified printer adapter."
            )
        }
        guard request.intent.density == nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "Density intent requires a certified printer adapter."
            )
        }
        guard request.profileID == nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "AirPrint doesn't expose route-bound native profiles."
            )
        }
        if case .raw = request.content {
            throw PiqaeNodeError.unsupportedOperation(
                "Raw printer languages require a certified printer adapter."
            )
        }
    }

    public func submit(
        _ request: PiqaePrintRequest,
        to printer: PiqaePrinter
    ) async throws -> PiqaeJobReceipt {
        try await validate(request, for: printer)
        var matchingURL: URL?
        for url in knownPrinterURLs where try await id(for: url).rawValue == printer.nativeID {
            matchingURL = url
            break
        }
        guard let url = matchingURL else {
            throw PiqaeNodeError.printerNotFound(printer.id)
        }
        // Idempotency belongs to the durable shared node runtime. The adapter
        // performs one native handoff for the durable attempt it receives.
        return try await Self.submitOnMainActor(request, printerURL: url)
    }

    @MainActor
    private static func contact(_ url: URL, id printerID: PiqaePrinterID) async -> PiqaePrinter {
        let printer = UIPrinter(url: url)
        let available = await printer.contactPrinter()
        let now = Date()
        return PiqaePrinter(
            id: printerID,
            adapterID: "apple.airprint",
            adapterFingerprint: PiqaeAdapterFingerprint(
                platform: .iosAirPrint,
                adapterID: "apple.airprint",
                adapterVersion: "1",
                deviceFamily: printer.makeAndModel
            ),
            // The adapter retains the routable URL. Inventory exposes only a
            // stable digest so cloud projections cannot recover a local host.
            nativeID: printerID.rawValue,
            displayName: available ? printer.displayName : (url.host ?? "AirPrint printer"),
            model: available ? printer.makeAndModel : nil,
            location: available ? printer.displayLocation : nil,
            state: available ? .available : .offline,
            capabilities: PiqaePrinterCapabilities(
                color: available ? printer.supportsColor : nil,
                duplex: available ? printer.supportsDuplex : nil,
                cutter: nil,
                portableRevision: available ? 1 : 0,
                nativeRevision: nil
            ),
            observedAt: now,
            freshUntil: now.addingTimeInterval(30)
        )
    }

    @MainActor
    private static func submitOnMainActor(
        _ request: PiqaePrintRequest,
        printerURL: URL
    ) async throws -> PiqaeJobReceipt {
        await PiqaeUIKitPrintGate.shared.acquire()
        defer { PiqaeUIKitPrintGate.shared.release() }
        let controller = UIPrintInteractionController.shared
        let info = UIPrintInfo(dictionary: nil)
        info.jobName = request.title
        info.outputType = .general
        if let orientation = request.intent.orientation {
            info.orientation = orientation == .portrait ? .portrait : .landscape
        }
        controller.printInfo = info
        switch request.content {
        case let .pdf(data), let .image(data, _):
            controller.printingItem = data
        case .raw:
            throw PiqaeNodeError.unsupportedOperation(
                "Raw printer languages require a certified printer adapter."
            )
        }

        let accepted = try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Bool, Error>) in
            let oneShot = PiqaeUIKitOneShot(continuation)
            let began = controller.print(to: UIPrinter(url: printerURL)) { _, completed, error in
                Task { @MainActor in
                    if let error { oneShot.resume(throwing: error) }
                    else { oneShot.resume(returning: completed) }
                }
            }
            if !began {
                oneShot.resume(
                    throwing: PiqaeNodeError.submissionRejected(
                        "The AirPrint subsystem did not begin the handoff."
                    )
                )
            }
        }
        guard accepted else {
            throw PiqaeNodeError.submissionRejected(
                "AirPrint did not accept the print job."
            )
        }
        return PiqaeJobReceipt(
            jobID: .init(rawValue: "job_apple_\(UUID().uuidString.lowercased())"),
            nativeJobID: nil,
            handoffState: .acceptedBySpooler,
            acceptedAt: Date()
        )
    }

    private func id(for url: URL) async throws -> PiqaePrinterID {
        let endpoint = try PiqaeAirPrintEndpoint.canonicalize(url)
        let evidence = try await identityProvider.deriveOpaqueID(
            namespace: "apple.airprint.physical-destination.v1",
            canonicalIdentity: endpoint.identityInput
        )
        guard
            evidence.hasPrefix("pid_"), evidence.utf8.count >= 20,
            evidence.utf8.count <= 128,
            evidence.unicodeScalars.allSatisfy({
                CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "_-")).contains($0)
            })
        else {
            throw PiqaeNodeError.invalidConfiguration(
                "The shared runtime returned invalid printer identity evidence."
            )
        }
        return PiqaePrinterID(rawValue: "apr_\(evidence)")
    }
}

@MainActor
public enum PiqaeAirPrintPicker {
    public static func selectPrinter() async throws -> URL? {
        await PiqaeUIKitPrintGate.shared.acquire()
        defer { PiqaeUIKitPrintGate.shared.release() }
        let picker = UIPrinterPickerController(initiallySelectedPrinter: nil)
        return try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<URL?, Error>) in
            let oneShot = PiqaeUIKitOneShot(continuation)
            let presented = picker.present(animated: true) { controller, selected, error in
                Task { @MainActor in
                    if let error { oneShot.resume(throwing: error) }
                    else {
                        let selectedURL = selected ? controller.selectedPrinter?.url : nil
                        oneShot.resume(returning: selectedURL)
                    }
                }
            }
            if !presented {
                oneShot.resume(
                    throwing: PiqaeNodeError.unsupportedOperation(
                        "The AirPrint picker is already presented."
                    )
                )
            }
        }
    }
}
#else
/// AirPrint execution is an iOS/iPadOS capability. macOS applications attach
/// to the installed Piqae node so the system driver remains authoritative.
public enum PiqaeAirPrintPlatformSupport: Sendable {
    case unavailableOnMacOS
}
#endif

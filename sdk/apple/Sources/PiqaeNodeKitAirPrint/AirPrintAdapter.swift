import Foundation
import PiqaeNodeKit

#if os(iOS)
import CryptoKit
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
    private var knownPrinterURLs: Set<URL>
    private var receiptsByIdempotencyKey: [String: PiqaeJobReceipt] = [:]

    /// AirPrint doesn't expose silent network enumeration. Supply printers the
    /// user selected with `PiqaeAirPrintPicker`, then retain their URLs in the
    /// host app and register them again at next launch.
    public init(knownPrinterURLs: [URL] = []) {
        self.knownPrinterURLs = Set(knownPrinterURLs)
    }

    public func register(printerURL: URL) throws {
        guard ["ipp", "ipps"].contains(printerURL.scheme?.lowercased()) else {
            throw PiqaeNodeError.invalidConfiguration(
                "AirPrint printers must use an ipp or ipps URL."
            )
        }
        knownPrinterURLs.insert(printerURL)
    }

    public func forget(printerURL: URL) {
        knownPrinterURLs.remove(printerURL)
    }

    public func discoverPrinters() async throws -> [PiqaePrinter] {
        var printers: [PiqaePrinter] = []
        for url in knownPrinterURLs.sorted(by: { $0.absoluteString < $1.absoluteString }) {
            printers.append(await Self.contact(url))
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
        if let prior = receiptsByIdempotencyKey[request.idempotencyKey] { return prior }
        try await validate(request, for: printer)
        guard let url = knownPrinterURLs.first(where: { Self.id(for: $0) == printer.id }) else {
            throw PiqaeNodeError.printerNotFound(printer.id)
        }
        let receipt = try await Self.submitOnMainActor(request, printerURL: url)
        receiptsByIdempotencyKey[request.idempotencyKey] = receipt
        return receipt
    }

    @MainActor
    private static func contact(_ url: URL) async -> PiqaePrinter {
        let printer = UIPrinter(url: url)
        let available = await printer.contactPrinter()
        let now = Date()
        let printerID = id(for: url)
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
            let began = controller.print(to: UIPrinter(url: printerURL)) { _, completed, error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume(returning: completed) }
            }
            if !began {
                continuation.resume(
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

    private nonisolated static func id(for url: URL) -> PiqaePrinterID {
        let digest = SHA256.hash(data: Data(url.absoluteString.utf8))
        let encoded = Data(digest).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
        return PiqaePrinterID(rawValue: "apr_\(encoded)")
    }
}

@MainActor
public enum PiqaeAirPrintPicker {
    public static func selectPrinter() async throws -> URL? {
        let picker = UIPrinterPickerController(initiallySelectedPrinter: nil)
        return try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<URL?, Error>) in
            let presented = picker.present(animated: true) { controller, selected, error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume(returning: selected ? controller.selectedPrinter?.url : nil) }
            }
            if !presented {
                continuation.resume(
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

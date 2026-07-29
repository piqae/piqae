import AppKit
import ApplicationServices
import Foundation
import SpoolMenuCore

@MainActor
public final class MacPrintProfileCapturer {
    public init() {}

    /// Presents the manufacturer's real print-panel panes without creating an
    /// NSPrintOperation. A successful return captures settings only; this path
    /// has no document, graphics context, or spooler submission.
    public func capture(
        session: LocalProfileCaptureSession
    ) throws -> LocalProfileCaptureCompletion? {
        let printer = try resolvePrinter(session: session)
        let printInfo = (NSPrintInfo.shared.copy() as? NSPrintInfo) ?? NSPrintInfo()
        printInfo.printer = printer
        printInfo.setUpPrintOperationDefaultValues()
        if let seed = session.nativeConfiguration {
            let initialConfiguration = try MacPrintProfileSerializer.configuration(from: seed)
            try MacPrintProfileSerializer.restore(initialConfiguration, into: printInfo)
            guard printInfo.printer.name == printer.name else {
                throw MacPrintProfileCaptureError.invalidStoredConfiguration
            }
        }

        let profileName = suggestedName(session: session)
        let accessory = ProfileAccessoryController(
            profileName: profileName,
            stockID: session.stockID,
            safeOverrides: session.safeOverrides ?? ["copies"]
        )
        let panel = NSPrintPanel()
        panel.setDefaultButtonTitle("Save Profile")
        panel.jobStyleHint = .allPresets
        panel.options = [
            .showsCopies,
            .showsPaperSize,
            .showsOrientation,
            .showsScaling,
            .showsPageSetupAccessory,
        ]
        panel.addAccessoryController(accessory)

        NSApplication.shared.activate(ignoringOtherApps: true)
        guard panel.runModal(with: printInfo) == NSApplication.ModalResponse.OK.rawValue else {
            return nil
        }
        guard !accessory.profileName.isEmpty else {
            throw MacPrintProfileCaptureError.invalidProfileName
        }

        let selectedName = printInfo.printer.name
        guard selectedName == printer.name else {
            throw MacPrintProfileCaptureError.printerChanged(
                expected: printer.name,
                selected: selectedName
            )
        }

        let nativeID = session.nativeID ?? printer.name
        let nativeConfiguration = try MacPrintProfileSerializer.capture(printInfo: printInfo)
        let nativeBlob = try MacPrintProfileSerializer.nativeBlob(from: nativeConfiguration)
        let uri = deviceURI(nativeID: nativeID)
        return LocalProfileCaptureCompletion(
            name: accessory.profileName,
            nativeDigest: MacPrintProfileSerializer.digest(of: nativeBlob),
            nativeBlob: nativeBlob,
            driverFingerprint: LocalMacDriverFingerprint(
                driverName: printer.type.rawValue.isEmpty
                    ? "macOS PrintCore"
                    : printer.type.rawValue,
                architecture: Self.architecture,
                nativeQueueID: nativeID,
                deviceFingerprint: uri.map {
                    MacPrintProfileSerializer.digest(of: Data($0.utf8))
                }
            ),
            summary: pageSummary(printInfo: printInfo),
            stockID: accessory.stockID,
            safeOverrides: accessory.safeOverrides
        )
    }

    private func resolvePrinter(session: LocalProfileCaptureSession) throws -> NSPrinter {
        let candidates = [session.nativeID, session.printerName]
            .compactMap { $0 }
            .filter { !$0.isEmpty }
        for candidate in candidates {
            if let printer = NSPrinter(name: candidate) {
                return printer
            }
        }
        throw MacPrintProfileCaptureError.printerUnavailable(session.printerName)
    }

    private func suggestedName(session: LocalProfileCaptureSession) -> String {
        let base = session.profileName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        switch session.operation {
        case .create:
            return base.isEmpty ? "\(session.printerName) profile" : base
        case .edit:
            return base
        case .clone:
            return base.isEmpty ? "" : "\(base) copy"
        }
    }

    private func pageSummary(printInfo: NSPrintInfo) -> LocalProfileSummary {
        let page = printInfo.paperSize
        let imageable = printInfo.imageablePageBounds
        let pointsToMM = 25.4 / 72.0
        return LocalProfileSummary(
            paper: printInfo.paperName?.rawValue,
            dimensionsMM: [
                Double(page.width) * pointsToMM,
                Double(page.height) * pointsToMM,
            ],
            details: [
                "localized_paper": printInfo.localizedPaperName ?? "",
                "orientation": printInfo.orientation == .landscape ? "landscape" : "portrait",
                "scaling_factor": String(describing: printInfo.scalingFactor),
                "imageable_area_points":
                    "\(imageable.origin.x),\(imageable.origin.y),"
                    + "\(imageable.width),\(imageable.height)",
            ]
        )
    }

    private func deviceURI(nativeID: String) -> String? {
        guard let printer = PMPrinterCreateFromPrinterID(nativeID as CFString) else {
            return nil
        }
        defer { PMRelease(UnsafeRawPointer(printer)) }
        var uri: Unmanaged<CFURL>?
        guard PMPrinterCopyDeviceURI(printer, &uri) == noErr else {
            return nil
        }
        guard let value = uri?.takeRetainedValue() else { return nil }
        return (value as URL).absoluteString
    }

    private static var architecture: String {
        #if arch(arm64)
        "arm64"
        #elseif arch(x86_64)
        "x86_64"
        #else
        "unknown"
        #endif
    }
}

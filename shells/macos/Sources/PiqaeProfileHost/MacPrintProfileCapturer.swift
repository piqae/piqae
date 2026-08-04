import AppKit
import ApplicationServices
import Foundation
import PiqaeMenuCore

@MainActor
public final class MacPrintProfileCapturer {
    public init() {}

    /// Presents the manufacturer's real print-panel panes without creating an
    /// NSPrintOperation. A successful return captures settings only; this path
    /// has no document, graphics context, or spooler submission.
    public func capture(
        session: LocalProfileCaptureSession,
        markAsDefault: Bool = false
    ) throws -> LocalProfileCaptureCompletion? {
        let destination = try resolvePrinter(session: session)
        let printer = destination.printer
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

        let profileName = suggestedName(
            session: session,
            printerDisplayName: printer.name,
            markAsDefault: markAsDefault
        )
        let accessory = ProfileAccessoryController(
            profileName: profileName,
            stockID: session.stockID,
            safeOverrides: session.safeOverrides ?? ["copies"]
        )
        let panel = NSPrintPanel()
        panel.setDefaultButtonTitle("Save Preset")
        // A job-style hint opts into Apple's simplified accordion panel. On
        // current macOS that interface scrolls section-by-section, which is a
        // poor fit for inspecting complex vendor driver panes. Nil selects the
        // standard Print panel and preserves continuous scrolling/navigation.
        panel.jobStyleHint = nil
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

        let nativeID = destination.nativeID
        let nativeConfiguration = try MacPrintProfileSerializer.capture(printInfo: printInfo)
        let nativeBlob = try MacPrintProfileSerializer.nativeBlob(from: nativeConfiguration)
        let uri = deviceURI(nativeID: nativeID)
        return LocalProfileCaptureCompletion(
            name: accessory.profileName,
            isDefault: markAsDefault,
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

    private func resolvePrinter(
        session: LocalProfileCaptureSession
    ) throws -> (printer: NSPrinter, nativeID: String) {
        let destinations = try MacPrinterDestinationResolver.current()
        guard
            let identity = MacPrinterDestinationResolver.select(
                nativeID: session.nativeID,
                printerName: session.printerName,
                from: destinations
            ),
            let printer = NSPrinter(name: identity.displayName)
        else {
            throw MacPrintProfileCaptureError.printerUnavailable(session.printerName)
        }
        return (printer, identity.nativeID)
    }

    private func suggestedName(
        session: LocalProfileCaptureSession,
        printerDisplayName: String,
        markAsDefault: Bool
    ) -> String {
        let base = session.profileName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        switch session.operation {
        case .create:
            if !base.isEmpty { return base }
            return markAsDefault ? "Default" : "\(printerDisplayName) profile"
        case .edit:
            if base == CurrentPrinterDefaultsProfile.name {
                return "Default"
            }
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

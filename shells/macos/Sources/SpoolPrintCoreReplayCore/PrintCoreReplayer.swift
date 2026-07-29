import AppKit
import ApplicationServices
import Foundation
import PDFKit
import SpoolProfileHost

@MainActor
public enum PrintCoreReplayer {
    public static func replay(_ request: PrintCoreReplayRequest) throws -> PrintCoreReplayResponse {
        let validated = try PrintCoreReplayValidator.validate(request)
        NSApplication.shared.setActivationPolicy(.prohibited)
        guard let printer = NSPrinter(name: request.printerNativeID) else {
            throw PrintCoreReplayError.failure(
                code: "printer_unavailable",
                message: "the exact macOS printer destination is unavailable",
                retryable: true
            )
        }
        guard let document = PDFDocument(url: URL(fileURLWithPath: request.pdfPath)),
            document.pageCount > 0
        else {
            throw PrintCoreReplayError.failure(
                code: "invalid_pdf",
                message: "content is not a readable PDF document"
            )
        }
        if let range = validated.pageRange,
            range.upperBound > UInt32(document.pageCount)
        {
            throw PrintCoreReplayError.failure(
                code: "invalid_print_option",
                message: "requested page range exceeds the PDF page count"
            )
        }

        // Always work with a private copy. This never writes NSPrintInfo.shared
        // or lpoptions, and therefore cannot mutate driver defaults.
        let printInfo = (NSPrintInfo.shared.copy() as? NSPrintInfo) ?? NSPrintInfo()
        printInfo.printer = printer
        printInfo.setUpPrintOperationDefaultValues()
        do {
            try MacPrintProfileSerializer.restore(validated.configuration, into: printInfo)
        } catch {
            throw PrintCoreReplayError.failure(
                code: "native_profile_invalid",
                message: "PrintCore could not restore the captured native profile"
            )
        }

        // The opaque profile may contain stale destination metadata. The job
        // is always rebound to the exact queue from the executor request.
        printInfo.printer = printer
        guard printInfo.printer.name == request.printerNativeID else {
            throw PrintCoreReplayError.failure(
                code: "profile_destination_mismatch",
                message: "PrintCore did not retain the requested printer destination"
            )
        }
        try apply(
            options: request.portableOptions,
            pageRange: validated.pageRange,
            to: printInfo
        )
        let settings = OpaquePointer(printInfo.pmPrintSettings())
        try check(
            PMPrintSettingsSetJobName(settings, request.jobTitle as CFString),
            operation: "setting the print job title"
        )
        printInfo.jobDisposition = .spool

        let scalingMode: PDFPrintScalingMode =
            request.portableOptions.fitToPage == true ? .pageScaleToFit : .pageScaleNone
        guard
            let operation = document.printOperation(
                for: printInfo,
                scalingMode: scalingMode,
                autoRotate: false
            )
        else {
            throw PrintCoreReplayError.failure(
                code: "print_operation_unavailable",
                message: "PDFKit could not create a print operation"
            )
        }
        operation.showsPrintPanel = false
        operation.showsProgressPanel = false

        // From this point a false return can follow an OS handoff, so it must
        // be reported as ambiguous and must not be retried automatically.
        guard operation.run() else {
            throw PrintCoreReplayError.failure(
                code: "print_handoff_ambiguous",
                message: "macOS did not confirm the print handoff",
                handoffMayHaveSucceeded: true
            )
        }
        return PrintCoreReplayResponse(ok: true)
    }

    public static func apply(
        options: PortablePrintOptions,
        pageRange: ClosedRange<UInt32>?,
        to printInfo: NSPrintInfo
    ) throws {
        let settings = OpaquePointer(printInfo.pmPrintSettings())
        if let copies = options.copies {
            try check(PMSetCopies(settings, copies, false), operation: "setting copies")
        }
        if let collate = options.collate {
            try check(PMSetCollate(settings, collate), operation: "setting collation")
        }
        if let duplex = options.duplex {
            let mode: PMDuplexMode = switch duplex {
            case .oneSided: UInt32(kPMDuplexNone)
            case .longEdge: UInt32(kPMDuplexNoTumble)
            case .shortEdge: UInt32(kPMDuplexTumble)
            }
            try check(PMSetDuplex(settings, mode), operation: "setting duplex")
        }
        if let pageRange {
            try check(
                PMSetPageRange(settings, pageRange.lowerBound, pageRange.upperBound),
                operation: "setting the available page range"
            )
            try check(
                PMSetFirstPage(settings, pageRange.lowerBound, false),
                operation: "setting the first page"
            )
            try check(
                PMSetLastPage(settings, pageRange.upperBound, false),
                operation: "setting the last page"
            )
        }
        // Synchronize the portable PrintCore fields before applying AppKit
        // paper/orientation overrides; synchronizing afterwards would replace
        // those requested per-job values with the restored profile values.
        printInfo.updateFromPMPrintSettings()
        if let paper = options.paper {
            guard !paper.isEmpty, paper.utf8.count <= 256 else {
                throw PrintCoreReplayError.failure(
                    code: "invalid_print_option",
                    message: "paper is empty or too long"
                )
            }
            let paperName = NSPrinter.PaperName(paper)
            guard printInfo.printer.pageSize(forPaper: paperName) != .zero else {
                throw PrintCoreReplayError.failure(
                    code: "invalid_print_option",
                    message: "paper is not supported by the bound printer"
                )
            }
            printInfo.paperName = paperName
        }
        if let rotation = options.rotate {
            printInfo.orientation = rotation == .degrees90 ? .landscape : .portrait
        }
    }

    private static func check(_ status: OSStatus, operation: String) throws {
        guard status == noErr else {
            throw PrintCoreReplayError.failure(
                code: "printcore_settings_failed",
                message: "\(operation) failed with PrintCore status \(status)"
            )
        }
    }
}

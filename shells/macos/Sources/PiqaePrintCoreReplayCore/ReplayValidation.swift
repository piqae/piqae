import AppKit
import ApplicationServices
import Foundation
import PiqaeMenuCore
import PiqaeProfileHost

public struct ValidatedPrintCoreReplay {
    public let request: PrintCoreReplayRequest
    public let configuration: LocalMacNativeConfiguration
    public let pageRange: ClosedRange<UInt32>?
}

public enum PrintCoreReplayError: Error, Equatable {
    case failure(
        code: String,
        message: String,
        retryable: Bool = false,
        handoffMayHaveSucceeded: Bool = false
    )

    public var response: PrintCoreReplayResponse {
        switch self {
        case let .failure(code, message, retryable, handoffMayHaveSucceeded):
            PrintCoreReplayResponse(
                ok: false,
                code: code,
                message: Self.bounded(message),
                retryable: retryable,
                handoffMayHaveSucceeded: handoffMayHaveSucceeded
            )
        }
    }

    private static func bounded(_ value: String) -> String {
        let cleaned = value.unicodeScalars.map {
            CharacterSet.controlCharacters.contains($0) ? " " : Character($0)
        }
        return String(cleaned.prefix(512))
    }
}

public enum PrintCoreReplayValidator {
    public static let maximumRequestBytes = 2 * 1024 * 1024
    public static let maximumPDFBytes: UInt64 = 512 * 1024 * 1024
    public static let maximumTitleCharacters = 512

    public static func validate(
        _ request: PrintCoreReplayRequest,
        checkPDFPath: Bool = true
    ) throws -> ValidatedPrintCoreReplay {
        guard
            !request.printerNativeID.isEmpty,
            request.printerNativeID.utf8.count <= 512,
            !request.printerNativeID.unicodeScalars.contains(where: {
                CharacterSet.controlCharacters.contains($0)
            })
        else {
            throw failure("invalid_printer", "printer_native_id is empty or invalid")
        }
        guard
            !request.jobTitle.isEmpty,
            request.jobTitle.count <= maximumTitleCharacters,
            !request.jobTitle.unicodeScalars.contains(where: {
                CharacterSet.controlCharacters.contains($0)
            })
        else {
            throw failure("invalid_job_title", "job_title is empty or invalid")
        }
        guard
            request.nativeProfile.kind == "macos_printcore",
            request.nativeProfile.schemaVersion == 1
        else {
            throw failure(
                "native_profile_schema_unsupported",
                "only macos_printcore schema 1 profiles can be replayed"
            )
        }
        guard
            !request.nativeProfile.blob.isEmpty,
            request.nativeProfile.blob.count <= MacPrintProfileSerializer
                .maximumNativeConfigurationBytes
        else {
            throw failure(
                "native_profile_too_large",
                "native profile is empty or exceeds the 1 MiB replay limit"
            )
        }
        guard
            constantTimeEqual(
                MacPrintProfileSerializer.digest(of: request.nativeProfile.blob),
                request.nativeProfile.digest
            )
        else {
            throw failure(
                "native_profile_digest_mismatch",
                "native profile digest does not match its immutable payload"
            )
        }

        let seed = LocalNativeProfileSeed(
            kind: request.nativeProfile.kind,
            schemaVersion: UInt32(request.nativeProfile.schemaVersion),
            digest: request.nativeProfile.digest,
            nativeBlob: request.nativeProfile.blob
        )
        let configuration: LocalMacNativeConfiguration
        do {
            configuration = try MacPrintProfileSerializer.configuration(from: seed)
        } catch {
            throw failure(
                "native_profile_invalid",
                "native profile is not a valid macOS PrintCore capture"
            )
        }

        try enforceOverrideAllowlist(
            options: request.portableOptions,
            allowed: Set(request.safeOverrides)
        )
        let range = try parsePageRange(request.portableOptions.pages)
        if let copies = request.portableOptions.copies, copies == 0 || copies > 9_999 {
            throw failure("invalid_print_option", "copies must be between 1 and 9999")
        }
        if let nup = request.portableOptions.nup, nup == 0 {
            throw failure("invalid_print_option", "nup must be greater than zero")
        }

        if checkPDFPath {
            try validatePDFPath(request.pdfPath)
        }
        return ValidatedPrintCoreReplay(
            request: request,
            configuration: configuration,
            pageRange: range
        )
    }

    public static func enforceOverrideAllowlist(
        options: PortablePrintOptions,
        allowed: Set<SafePrintOverride>
    ) throws {
        guard options.nativeOptions.isEmpty else {
            throw failure(
                "profile_override_not_allowed",
                "native_options cannot override an immutable native profile"
            )
        }
        let requested: [(Bool, SafePrintOverride)] = [
            (options.bin != nil, .bin),
            (options.collate != nil, .collate),
            (options.color != nil, .color),
            (options.copies != nil, .copies),
            (options.dpi != nil, .dpi),
            (options.duplex != nil, .duplex),
            (options.fitToPage != nil, .fitToPage),
            (options.media != nil, .media),
            (options.nup != nil, .nup),
            (options.pages != nil, .pages),
            (options.paper != nil, .paper),
            (options.rotate != nil, .rotate),
        ]
        for (isRequested, field) in requested where isRequested && !allowed.contains(field) {
            throw failure(
                "profile_override_not_allowed",
                "profile does not allow \(field.rawValue) to be changed per job"
            )
        }

        // These fields cannot be expressed through stable, documented
        // AppKit/PrintCore APIs without replacing driver-owned keys. Keeping
        // them unsupported is safer than silently degrading an exact profile.
        for (isRequested, field) in [
            (options.bin != nil, SafePrintOverride.bin),
            (options.color != nil, .color),
            (options.dpi != nil, .dpi),
            (options.media != nil, .media),
            (options.nup != nil, .nup),
        ] where isRequested {
            throw failure(
                "profile_override_unsupported",
                "\(field.rawValue) is not a stable macOS PrintCore job override"
            )
        }
        if let rotation = options.rotate,
            ![PortableRotation.degrees0, .degrees90].contains(rotation)
        {
            throw failure(
                "profile_override_unsupported",
                "macOS PrintCore can safely override only 0 or 90 degree orientation"
            )
        }
    }

    public static func parsePageRange(_ value: String?) throws -> ClosedRange<UInt32>? {
        guard let value else { return nil }
        let parts = value.split(separator: "-", omittingEmptySubsequences: false)
        guard
            (1...2).contains(parts.count),
            let first = UInt32(parts[0]),
            first > 0
        else {
            throw failure(
                "invalid_print_option",
                "pages must be one page or one ascending range such as 2-5"
            )
        }
        let last = parts.count == 2 ? UInt32(parts[1]) : first
        guard let last, last >= first else {
            throw failure(
                "invalid_print_option",
                "pages must be one page or one ascending range such as 2-5"
            )
        }
        return first...last
    }

    private static func validatePDFPath(_ path: String) throws {
        guard !path.isEmpty, path.utf8.count <= 4_096 else {
            throw failure("invalid_content_path", "pdf_path is empty or too long")
        }
        let url = URL(fileURLWithPath: path)
        guard url.path == path, url.path.hasPrefix("/") else {
            throw failure("invalid_content_path", "pdf_path must be an absolute file path")
        }
        let values: URLResourceValues
        do {
            values = try url.resourceValues(forKeys: [
                .isRegularFileKey,
                .fileSizeKey,
                .isReadableKey,
            ])
        } catch {
            throw failure("content_unavailable", "PDF file is unavailable", retryable: true)
        }
        guard values.isRegularFile == true, values.isReadable != false else {
            throw failure("content_unavailable", "PDF path is not a readable regular file")
        }
        guard let size = values.fileSize, size > 0, UInt64(size) <= maximumPDFBytes else {
            throw failure(
                "invalid_pdf",
                "PDF is empty or exceeds the 512 MiB replay limit"
            )
        }
    }

    private static func constantTimeEqual(_ left: String, _ right: String) -> Bool {
        let left = Array(left.utf8)
        let right = Array(right.utf8)
        guard left.count == right.count else { return false }
        return zip(left, right).reduce(0) { $0 | ($1.0 ^ $1.1) } == 0
    }

    private static func failure(
        _ code: String,
        _ message: String,
        retryable: Bool = false
    ) -> PrintCoreReplayError {
        .failure(code: code, message: message, retryable: retryable)
    }
}

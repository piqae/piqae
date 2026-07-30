import AppKit
import PiqaeMenuCore
import PiqaePrintCoreReplayCore
import PiqaeProfileHost
import XCTest

final class ReplayValidationTests: XCTestCase {
    func testRejectsAnOverrideThatProfileDidNotAllow() throws {
        let request = try makeRequest(
            options: PortablePrintOptions(copies: 2),
            safeOverrides: []
        )

        XCTAssertThrowsError(
            try PrintCoreReplayValidator.validate(request, checkPDFPath: false)
        ) { error in
            XCTAssertEqual(
                error as? PrintCoreReplayError,
                .failure(
                    code: "profile_override_not_allowed",
                    message: "profile does not allow copies to be changed per job"
                )
            )
        }
    }

    func testPermitsOnlyStableDocumentedPrintCoreOverrides() throws {
        let request = try makeRequest(
            options: PortablePrintOptions(
                collate: true,
                copies: 2,
                duplex: .longEdge,
                fitToPage: true,
                pages: "2-5",
                paper: "iso-a4",
                rotate: .degrees90
            ),
            safeOverrides: [
                .collate, .copies, .duplex, .fitToPage, .pages, .paper, .rotate,
            ]
        )

        let validated = try PrintCoreReplayValidator.validate(
            request,
            checkPDFPath: false
        )

        XCTAssertEqual(validated.pageRange, 2...5)
    }

    func testRejectsUnsupportedDriverKeyOverrideEvenWhenAllowlisted() throws {
        let request = try makeRequest(
            options: PortablePrintOptions(media: "labels"),
            safeOverrides: [.media]
        )

        XCTAssertThrowsError(
            try PrintCoreReplayValidator.validate(request, checkPDFPath: false)
        ) { error in
            XCTAssertEqual(
                error as? PrintCoreReplayError,
                .failure(
                    code: "profile_override_unsupported",
                    message: "media is not a stable macOS PrintCore job override"
                )
            )
        }
    }

    func testRejectsDigestMismatchBeforeRestoringPrintCoreObjects() throws {
        let valid = try makeRequest()
        let request = PrintCoreReplayRequest(
            printerNativeID: valid.printerNativeID,
            pdfPath: valid.pdfPath,
            jobTitle: valid.jobTitle,
            nativeProfile: PrintCoreNativeProfile(
                kind: valid.nativeProfile.kind,
                schemaVersion: valid.nativeProfile.schemaVersion,
                digest: "sha256:" + String(repeating: "0", count: 64),
                blob: valid.nativeProfile.blob
            )
        )

        XCTAssertThrowsError(
            try PrintCoreReplayValidator.validate(request, checkPDFPath: false)
        ) { error in
            guard case .failure(let code, _, _, _) = error as? PrintCoreReplayError else {
                return XCTFail("unexpected error \(error)")
            }
            XCTAssertEqual(code, "native_profile_digest_mismatch")
        }
    }

    func testPageRangeRejectsDisjointAndDescendingForms() throws {
        XCTAssertThrowsError(try PrintCoreReplayValidator.parsePageRange("1,3"))
        XCTAssertThrowsError(try PrintCoreReplayValidator.parsePageRange("4-2"))
        XCTAssertEqual(try PrintCoreReplayValidator.parsePageRange("7"), 7...7)
    }

    @MainActor
    func testApplyChangesPrivatePrintInfoWithoutChangingSharedDefaults() throws {
        let sharedCopies =
            NSPrintInfo.shared.printSettings[NSPrintInfo.SettingKey("NSCopies")] as? Int
        let info = (NSPrintInfo.shared.copy() as? NSPrintInfo) ?? NSPrintInfo()

        try PrintCoreReplayer.apply(
            options: PortablePrintOptions(copies: 3, rotate: .degrees90),
            pageRange: 1...2,
            to: info
        )

        XCTAssertEqual(info.orientation, .landscape)
        XCTAssertEqual(
            NSPrintInfo.shared.printSettings[NSPrintInfo.SettingKey("NSCopies")] as? Int,
            sharedCopies
        )
    }

    private func makeRequest(
        options: PortablePrintOptions = .init(),
        safeOverrides: [SafePrintOverride] = []
    ) throws -> PrintCoreReplayRequest {
        let configuration = try MacPrintProfileSerializer.capture(printInfo: NSPrintInfo())
        let blob = try MacPrintProfileSerializer.nativeBlob(from: configuration)
        return PrintCoreReplayRequest(
            printerNativeID: "Unit Test Printer",
            pdfPath: "/tmp/unit-test.pdf",
            jobTitle: "Unit test",
            nativeProfile: PrintCoreNativeProfile(
                kind: "macos_printcore",
                schemaVersion: 1,
                digest: MacPrintProfileSerializer.digest(of: blob),
                blob: blob
            ),
            portableOptions: options,
            safeOverrides: safeOverrides
        )
    }
}

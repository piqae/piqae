import XCTest
@testable import SpoolProfileHost

final class MacPrinterDestinationResolverTests: XCTestCase {
    private let destinations = [
        MacPrinterDestinationIdentity(
            nativeID: "HP_Color_LaserJet_MFP",
            displayName: "HP Color LaserJet MFP"
        ),
        MacPrinterDestinationIdentity(
            nativeID: "Kyocera_ECOSYS_P3150dn",
            displayName: "Kyocera ECOSYS P3150dn"
        ),
    ]

    func testResolvesExactNativeDestinationIDToDisplayIdentity() {
        XCTAssertEqual(
            MacPrinterDestinationResolver.select(
                nativeID: "HP_Color_LaserJet_MFP",
                printerName: "HP_Color_LaserJet_MFP",
                from: destinations
            ),
            destinations[0]
        )
    }

    func testDoesNotFuzzilyMapMissingQueueToSimilarPrinter() {
        XCTAssertNil(
            MacPrinterDestinationResolver.select(
                nativeID: "Kyocera_ECOSYS_P2040dn",
                printerName: "Kyocera_ECOSYS_P2040dn",
                from: destinations
            )
        )
    }

    func testAuthoritativeNativeIDDoesNotFallBackToDisplayName() {
        XCTAssertNil(
            MacPrinterDestinationResolver.select(
                nativeID: "removed-queue",
                printerName: "HP Color LaserJet MFP",
                from: destinations
            )
        )
    }

    func testDisplayNameFallbackRequiresOneExactMatchAndNoNativeID() {
        XCTAssertEqual(
            MacPrinterDestinationResolver.select(
                nativeID: nil,
                printerName: "Kyocera ECOSYS P3150dn",
                from: destinations
            ),
            destinations[1]
        )
        XCTAssertNil(
            MacPrinterDestinationResolver.select(
                nativeID: nil,
                printerName: "kyocera ecosys p3150dn",
                from: destinations
            )
        )
    }
}

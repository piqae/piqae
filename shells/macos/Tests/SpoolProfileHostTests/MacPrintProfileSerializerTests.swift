import AppKit
import SpoolMenuCore
import XCTest
@testable import SpoolProfileHost

final class MacPrintProfileSerializerTests: XCTestCase {
    func testPropertyListRoundTripPreservesDriverValues() throws {
        let input: NSDictionary = [
            "com_vendor_print_Feature": "HeavyStock",
            "copies": 3,
            "nested": ["white_toner": true],
            "binary": Data([0x00, 0x7f, 0xff]),
        ]

        let data = try MacPrintProfileSerializer.propertyListData(from: input)
        let restored = try MacPrintProfileSerializer.propertyList(from: data)

        XCTAssertEqual(restored["com_vendor_print_Feature"] as? String, "HeavyStock")
        XCTAssertEqual(restored["copies"] as? Int, 3)
        XCTAssertEqual(restored["binary"] as? Data, Data([0x00, 0x7f, 0xff]))
    }

    func testCaptureAndRestorePrintCoreRepresentations() throws {
        let source = NSPrintInfo()
        source.paperSize = NSSize(width: 420, height: 595)
        source.scalingFactor = 0.75
        source.printSettings[NSPrintInfo.SettingKey("com_spool_test")] = "preserved"

        let captured = try MacPrintProfileSerializer.capture(printInfo: source)
        XCTAssertEqual(captured.kind, "macos_printcore")
        XCTAssertFalse(captured.pmPrintSettings.isEmpty)
        XCTAssertFalse(captured.pmPageFormat.isEmpty)
        XCTAssertEqual(
            String(decoding: captured.propertyListPrintSettings.prefix(8), as: UTF8.self),
            "bplist00"
        )
        _ = try MacPrintProfileSerializer.propertyList(
            from: captured.propertyListPrintSettings
        )

        let restored = NSPrintInfo()
        try MacPrintProfileSerializer.restore(captured, into: restored)

        XCTAssertEqual(restored.printSettings[NSPrintInfo.SettingKey("com_spool_test")] as? String, "preserved")
        XCTAssertEqual(restored.scalingFactor, 0.75, accuracy: 0.001)
    }

    func testNativeEnvelopeRoundTripVerifiesDigest() throws {
        let configuration = LocalMacNativeConfiguration(
            propertyListPrintSettings: Data("plist".utf8),
            pmPrintSettings: Data("settings".utf8),
            pmPageFormat: Data("page".utf8)
        )
        let blob = try MacPrintProfileSerializer.nativeBlob(from: configuration)
        let digest = MacPrintProfileSerializer.digest(of: blob)
        let seedData = try JSONSerialization.data(withJSONObject: [
            "kind": "macos_printcore",
            "schema_version": 1,
            "digest": digest,
            "native_blob_base64": blob.base64EncodedString(),
        ])
        let seed = try JSONDecoder().decode(LocalNativeProfileSeed.self, from: seedData)

        XCTAssertEqual(
            try MacPrintProfileSerializer.configuration(from: seed),
            configuration
        )
    }

    func testRejectsNonDictionaryStoredPropertyList() throws {
        let data = try PropertyListSerialization.data(
            fromPropertyList: ["not", "a", "dictionary"],
            format: .binary,
            options: 0
        )
        XCTAssertThrowsError(try MacPrintProfileSerializer.propertyList(from: data)) {
            XCTAssertEqual(
                $0 as? MacPrintProfileCaptureError,
                .invalidStoredConfiguration
            )
        }
    }

    @MainActor
    func testProfileAccessoryKeepsEditableFieldsAtUsableSize() {
        let controller = ProfileAccessoryController(profileName: "A4 colour")
        let accessory = controller.view
        accessory.layoutSubtreeIfNeeded()

        let fields = accessory.descendants.compactMap { view -> NSTextField? in
            guard let field = view as? NSTextField, field.isEditable else { return nil }
            return field
        }
        XCTAssertEqual(fields.count, 2)
        for field in fields {
            XCTAssertGreaterThanOrEqual(field.frame.width, 260)
            XCTAssertGreaterThan(field.frame.height, 20)
        }
        XCTAssertEqual(controller.preferredContentSize, NSSize(width: 460, height: 116))
    }
}

private extension NSView {
    var descendants: [NSView] {
        subviews + subviews.flatMap(\.descendants)
    }
}

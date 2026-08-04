import AppKit
import ApplicationServices
import CryptoKit
import Foundation
import PiqaeMenuCore

public enum MacPrintProfileCaptureError: Error, LocalizedError, Equatable {
    case printerUnavailable(String)
    case printerChanged(expected: String, selected: String)
    case invalidProfileName
    case invalidPropertyList
    case printCoreFailure(operation: String, status: OSStatus)
    case invalidStoredConfiguration
    case captureTooLarge

    public var errorDescription: String? {
        switch self {
        case let .printerUnavailable(name):
            "The macOS printer “\(name)” is no longer available."
        case let .printerChanged(expected, selected):
            "This print preset belongs to “\(expected)”. The panel selected “\(selected)” instead."
        case .invalidProfileName:
            "Enter a name for this print preset."
        case .invalidPropertyList:
            "The printer driver returned settings that cannot be stored safely."
        case let .printCoreFailure(operation, status):
            "\(operation) failed with PrintCore status \(status)."
        case .invalidStoredConfiguration:
            "The saved macOS print preset could not be restored."
        case .captureTooLarge:
            "The printer driver settings exceed Piqae’s local capture limit."
        }
    }
}

public enum MacPrintProfileSerializer {
    public static let maximumNativeConfigurationBytes = 1024 * 1024

    public static func propertyListData(from settings: NSDictionary) throws -> Data {
        let canonical = try canonicalPropertyList(settings)
        guard PropertyListSerialization.propertyList(canonical, isValidFor: .binary) else {
            throw MacPrintProfileCaptureError.invalidPropertyList
        }
        return try PropertyListSerialization.data(
            fromPropertyList: canonical,
            format: .binary,
            options: 0
        )
    }

    public static func propertyList(from data: Data) throws -> NSDictionary {
        let object = try PropertyListSerialization.propertyList(
            from: data,
            options: [],
            format: nil
        )
        guard let dictionary = object as? NSDictionary else {
            throw MacPrintProfileCaptureError.invalidStoredConfiguration
        }
        return dictionary
    }

    public static func capture(printInfo: NSPrintInfo) throws -> LocalMacNativeConfiguration {
        let printSettings = try printSettingsData(OpaquePointer(printInfo.pmPrintSettings()))
        let pageFormat = try pageFormatData(OpaquePointer(printInfo.pmPageFormat()))
        // Ask PrintCore for its representations before serializing the AppKit
        // mirror. Some third-party drivers lazily normalize the settings
        // dictionary while producing their native representation.
        let propertyList = try propertyListData(from: printInfo.printSettings as NSDictionary)
        let totalSize = propertyList.count + printSettings.count + pageFormat.count
        guard totalSize <= maximumNativeConfigurationBytes else {
            throw MacPrintProfileCaptureError.captureTooLarge
        }
        return LocalMacNativeConfiguration(
            propertyListPrintSettings: propertyList,
            pmPrintSettings: printSettings,
            pmPageFormat: pageFormat
        )
    }

    public static func restore(
        _ configuration: LocalMacNativeConfiguration,
        into printInfo: NSPrintInfo
    ) throws {
        guard
            configuration.kind == "macos_printcore",
            configuration.schemaVersion == 1,
            configuration.propertyListPrintSettings.count
                + configuration.pmPrintSettings.count
                + configuration.pmPageFormat.count <= maximumNativeConfigurationBytes
        else {
            throw MacPrintProfileCaptureError.invalidStoredConfiguration
        }

        let dictionary = try propertyList(from: configuration.propertyListPrintSettings)
        printInfo.printSettings.removeAllObjects()
        for (key, value) in dictionary {
            guard let key = key as? String else {
                throw MacPrintProfileCaptureError.invalidStoredConfiguration
            }
            printInfo.printSettings[NSPrintInfo.SettingKey(key)] = value
        }

        var restoredPrintSettings: PMPrintSettings?
        let printStatus = PMPrintSettingsCreateWithDataRepresentation(
            configuration.pmPrintSettings as CFData,
            &restoredPrintSettings
        )
        guard printStatus == noErr, let restoredPrintSettings else {
            throw MacPrintProfileCaptureError.printCoreFailure(
                operation: "Restoring print settings",
                status: printStatus
            )
        }
        defer { PMRelease(UnsafeRawPointer(restoredPrintSettings)) }
        let copyPrintStatus = PMCopyPrintSettings(
            restoredPrintSettings,
            OpaquePointer(printInfo.pmPrintSettings())
        )
        guard copyPrintStatus == noErr else {
            throw MacPrintProfileCaptureError.printCoreFailure(
                operation: "Applying print settings",
                status: copyPrintStatus
            )
        }
        printInfo.updateFromPMPrintSettings()

        var restoredPageFormat: PMPageFormat?
        let pageStatus = PMPageFormatCreateWithDataRepresentation(
            configuration.pmPageFormat as CFData,
            &restoredPageFormat
        )
        guard pageStatus == noErr, let restoredPageFormat else {
            throw MacPrintProfileCaptureError.printCoreFailure(
                operation: "Restoring page format",
                status: pageStatus
            )
        }
        defer { PMRelease(UnsafeRawPointer(restoredPageFormat)) }
        let copyPageStatus = PMCopyPageFormat(
            restoredPageFormat,
            OpaquePointer(printInfo.pmPageFormat())
        )
        guard copyPageStatus == noErr else {
            throw MacPrintProfileCaptureError.printCoreFailure(
                operation: "Applying page format",
                status: copyPageStatus
            )
        }
        printInfo.updateFromPMPageFormat()
    }

    public static func nativeBlob(
        from configuration: LocalMacNativeConfiguration
    ) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let blob = try encoder.encode(configuration)
        guard blob.count <= maximumNativeConfigurationBytes else {
            throw MacPrintProfileCaptureError.captureTooLarge
        }
        return blob
    }

    public static func configuration(
        from seed: LocalNativeProfileSeed
    ) throws -> LocalMacNativeConfiguration {
        guard
            seed.kind == "macos_printcore",
            seed.schemaVersion == 1,
            seed.nativeBlob.count <= maximumNativeConfigurationBytes,
            digest(of: seed.nativeBlob) == seed.digest
        else {
            throw MacPrintProfileCaptureError.invalidStoredConfiguration
        }
        do {
            return try JSONDecoder().decode(
                LocalMacNativeConfiguration.self,
                from: seed.nativeBlob
            )
        } catch {
            throw MacPrintProfileCaptureError.invalidStoredConfiguration
        }
    }

    public static func digest(of data: Data) -> String {
        "sha256:" + SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    private static func printSettingsData(_ settings: PMPrintSettings) throws -> Data {
        var data: Unmanaged<CFData>?
        let status = PMPrintSettingsCreateDataRepresentation(
            settings,
            &data,
            kPMDataFormatXMLMinimal
        )
        guard status == noErr, let data else {
            throw MacPrintProfileCaptureError.printCoreFailure(
                operation: "Serializing print settings",
                status: status
            )
        }
        return data.takeRetainedValue() as Data
    }

    private static func pageFormatData(_ pageFormat: PMPageFormat) throws -> Data {
        var data: Unmanaged<CFData>?
        let status = PMPageFormatCreateDataRepresentation(
            pageFormat,
            &data,
            kPMDataFormatXMLMinimal
        )
        guard status == noErr, let data else {
            throw MacPrintProfileCaptureError.printCoreFailure(
                operation: "Serializing page format",
                status: status
            )
        }
        return data.takeRetainedValue() as Data
    }

    private static func canonicalPropertyList(_ value: Any) throws -> Any {
        if let dictionary = value as? NSDictionary {
            var result: [String: Any] = [:]
            for (key, nestedValue) in dictionary {
                let stringKey: String
                if let key = key as? String {
                    stringKey = key
                } else {
                    throw MacPrintProfileCaptureError.invalidPropertyList
                }
                result[stringKey] = try canonicalPropertyList(nestedValue)
            }
            return result
        }
        if let array = value as? NSArray {
            return try array.map(canonicalPropertyList)
        }
        guard
            value is String
                || value is NSNumber
                || value is Data
                || value is Date
        else {
            throw MacPrintProfileCaptureError.invalidPropertyList
        }
        return value
    }
}

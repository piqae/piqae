import AppKit
import ApplicationServices

public struct MacPrinterDestinationIdentity: Equatable, Sendable {
    public let nativeID: String
    public let displayName: String

    public init(nativeID: String, displayName: String) {
        self.nativeID = nativeID
        self.displayName = displayName
    }
}

public enum MacPrinterDestinationResolver {
    /// Resolves only exact current identities. A supplied native destination ID
    /// is authoritative and never falls back to a similar display name.
    public static func select(
        nativeID: String?,
        printerName: String,
        from destinations: [MacPrinterDestinationIdentity]
    ) -> MacPrinterDestinationIdentity? {
        if let nativeID, !nativeID.isEmpty {
            let matches = destinations.filter { $0.nativeID == nativeID }
            return matches.count == 1 ? matches[0] : nil
        }
        let matches = destinations.filter { $0.displayName == printerName }
        return matches.count == 1 ? matches[0] : nil
    }

    static func current() throws -> [MacPrinterDestinationIdentity] {
        var unmanagedList: Unmanaged<CFArray>?
        let status = PMServerCreatePrinterList(nil, &unmanagedList)
        guard status == noErr, let list = unmanagedList?.takeRetainedValue() else {
            throw MacPrintProfileCaptureError.printCoreFailure(
                operation: "Listing printers",
                status: status
            )
        }

        var destinations: [MacPrinterDestinationIdentity] = []
        destinations.reserveCapacity(CFArrayGetCount(list))
        for index in 0 ..< CFArrayGetCount(list) {
            let value = CFArrayGetValueAtIndex(list, index)
            guard
                let printer = OpaquePointer(value),
                let unmanagedID = PMPrinterGetID(printer),
                let unmanagedName = PMPrinterGetName(printer)
            else {
                continue
            }
            destinations.append(
                MacPrinterDestinationIdentity(
                    nativeID: unmanagedID.takeUnretainedValue() as String,
                    displayName: unmanagedName.takeUnretainedValue() as String
                )
            )
        }
        return destinations
    }
}

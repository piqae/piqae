import CryptoKit
import Foundation

guard CommandLine.arguments.count == 4 else {
    FileHandle.standardError.write(
        Data("usage: verify-update-signature <archive> <public-key-base64> <signature-base64>\n".utf8)
    )
    exit(2)
}

let archiveURL = URL(fileURLWithPath: CommandLine.arguments[1])
guard
    let publicKeyData = Data(base64Encoded: CommandLine.arguments[2]),
    let signatureData = Data(base64Encoded: CommandLine.arguments[3]),
    publicKeyData.count == 32,
    signatureData.count == 64
else {
    FileHandle.standardError.write(Data("invalid Ed25519 public key or signature\n".utf8))
    exit(2)
}

do {
    let publicKey = try Curve25519.Signing.PublicKey(rawRepresentation: publicKeyData)
    let archive = try Data(contentsOf: archiveURL, options: [.mappedIfSafe])
    guard publicKey.isValidSignature(signatureData, for: archive) else {
        FileHandle.standardError.write(Data("Sparkle archive signature verification failed\n".utf8))
        exit(1)
    }
} catch {
    FileHandle.standardError.write(Data("could not verify Sparkle archive: \(error)\n".utf8))
    exit(1)
}

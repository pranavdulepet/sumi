// Publish with the release maintainer's local Keychain key. The secret never leaves Keychain on disk.
import CryptoKit
import Foundation
import Security

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}
let args = Array(CommandLine.arguments.dropFirst())
guard args.count == 2 || (args.count == 3 && args.last == "--initialize") else {
    fail("Usage: swift scripts/sign-catalog.swift payload.json models.json [--initialize]")
}
let base: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
    kSecAttrService as String: "studio.sumi.release.catalog", kSecAttrAccount as String: "publisher"]
var result: CFTypeRef?
var lookup = base
lookup[kSecReturnData as String] = true
let status = SecItemCopyMatching(lookup as CFDictionary, &result)
let key: Curve25519.Signing.PrivateKey
if status == errSecSuccess, let data = result as? Data {
    key = try Curve25519.Signing.PrivateKey(rawRepresentation: data)
} else if status == errSecItemNotFound && args.last == "--initialize" {
    key = Curve25519.Signing.PrivateKey()
    var entry = base
    entry[kSecValueData as String] = key.rawRepresentation
    entry[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
    guard SecItemAdd(entry as CFDictionary, nil) == errSecSuccess else { fail("Could not store the publishing key.") }
} else {
    fail("Could not read the publishing key. Initialize it once on the release Mac.")
}
let payload = try Data(contentsOf: URL(fileURLWithPath: args[0]))
guard let modelList = try JSONSerialization.jsonObject(with: payload) as? [String: Any],
      modelList["revision"] is Int, modelList["models"] is [[String: Any]] else { fail("Invalid catalog.") }
let signature = try key.signature(for: payload)
let envelope = ["payload": payload.base64EncodedString(), "signature": signature.base64EncodedString()]
try JSONSerialization.data(withJSONObject: envelope, options: [.prettyPrinted, .sortedKeys])
    .write(to: URL(fileURLWithPath: args[1]), options: .atomic)
print(key.publicKey.rawRepresentation.base64EncodedString())

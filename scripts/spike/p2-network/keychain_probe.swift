// keychain_probe.swift — §2.1 / §10 "keychain behaviour for an ad-hoc-signed
// macOS build", measured with user interaction DISABLED, so nothing prompts:
// a call that would have shown a dialog fails with errSecInteractionNotAllowed
// (-25308) instead.
//
//   keychain_probe add <service>     login (file) keychain item, created by THIS binary
//   keychain_probe read <service>    read it back
//   keychain_probe delete <service>  delete it
//   keychain_probe dpk               data-protection keychain add (needs an entitlement?)
//   keychain_probe se                Secure Enclave: persistent key (needs an entitlement?),
//                                    ephemeral SecKey, CryptoKit key; P-256 sign latency
//
// Build two binaries with different cdhashes (one with -D VARIANT_B) and
// ad-hoc sign both: that is what an app update looks like to the keychain.
// Spike probe; not product code.
import CryptoKit
import Foundation
import Security

#if VARIANT_B
let variant = "B"
#else
let variant = "A"
#endif

func code(_ s: OSStatus) -> String {
    let msg = (SecCopyErrorMessageString(s, nil) as String?) ?? "?"
    return "\(s) (\(msg))"
}

// Legacy file-keychain ACL dialogs are SecurityAgent prompts; this switch
// turns every one of them into errSecInteractionNotAllowed.
SecKeychainSetUserInteractionAllowed(false)

let args = CommandLine.arguments
let mode = args.count > 1 ? args[1] : ""
let service = args.count > 2 ? args[2] : "worktrees-spike-p2-keychain"
print("variant \(variant), mode \(mode)")

func base(_ extra: [CFString: Any] = [:]) -> [CFString: Any] {
    var q: [CFString: Any] = [
        kSecClass: kSecClassGenericPassword,
        kSecAttrService: service,
        kSecAttrAccount: "probe",
        kSecUseAuthenticationUI: kSecUseAuthenticationUIFail,
    ]
    for (k, v) in extra { q[k] = v }
    return q
}

switch mode {
case "add":
    let s = SecItemAdd(base([kSecValueData: Data("spike".utf8)]) as CFDictionary, nil)
    print("SecItemAdd(login keychain) -> \(code(s))")
case "read":
    var out: CFTypeRef?
    let s = SecItemCopyMatching(base([kSecReturnData: true]) as CFDictionary, &out)
    let got = (out as? Data).map { String(decoding: $0, as: UTF8.self) } ?? "-"
    print("SecItemCopyMatching(login keychain, data) -> \(code(s)) value=\(got)")
case "delete":
    let s = SecItemDelete(base() as CFDictionary)
    print("SecItemDelete -> \(code(s))")
case "dpk":
    let q = base([kSecValueData: Data("spike".utf8), kSecUseDataProtectionKeychain: true])
    let s = SecItemAdd(q as CFDictionary, nil)
    print("SecItemAdd(data-protection keychain) -> \(code(s))")
    if s == errSecSuccess { print("cleanup SecItemDelete -> \(code(SecItemDelete(base([kSecUseDataProtectionKeychain: true]) as CFDictionary)))") }
case "se":
    // 1. A persistent Secure Enclave key: stored in the keychain.
    var err: Unmanaged<CFError>?
    let tag = Data("worktrees-spike-p2-se".utf8)
    let persistent: [CFString: Any] = [
        kSecAttrKeyType: kSecAttrKeyTypeECSECPrimeRandom, kSecAttrKeySizeInBits: 256,
        kSecAttrTokenID: kSecAttrTokenIDSecureEnclave,
        kSecPrivateKeyAttrs: [kSecAttrIsPermanent: true, kSecAttrApplicationTag: tag] as [CFString: Any],
    ]
    if let k = SecKeyCreateRandomKey(persistent as CFDictionary, &err) {
        print("SE persistent SecKey: CREATED \(k) — deleting")
        let d: [CFString: Any] = [kSecClass: kSecClassKey, kSecAttrApplicationTag: tag]
        print("  cleanup -> \(code(SecItemDelete(d as CFDictionary)))")
    } else {
        print("SE persistent SecKey: FAILED \(err!.takeRetainedValue())")
    }
    // 2. An ephemeral Secure Enclave key: never stored.
    err = nil
    let ephemeral: [CFString: Any] = [
        kSecAttrKeyType: kSecAttrKeyTypeECSECPrimeRandom, kSecAttrKeySizeInBits: 256,
        kSecAttrTokenID: kSecAttrTokenIDSecureEnclave,
        kSecPrivateKeyAttrs: [kSecAttrIsPermanent: false] as [CFString: Any],
    ]
    if let k = SecKeyCreateRandomKey(ephemeral as CFDictionary, &err) {
        let msg = Data(repeating: 7, count: 32)
        let t = Date()
        var ok = 0
        for _ in 0..<50 {
            if SecKeyCreateSignature(k, .ecdsaSignatureMessageX962SHA256, msg as CFData, nil) != nil { ok += 1 }
        }
        print(String(format: "SE ephemeral SecKey: CREATED; %d/50 signatures, %.2f ms each", ok, Date().timeIntervalSince(t) * 1000 / 50))
    } else {
        print("SE ephemeral SecKey: FAILED \(err!.takeRetainedValue())")
    }
    // 3. CryptoKit: the key is an SE-wrapped blob the caller stores itself.
    do {
        let k = try SecureEnclave.P256.Signing.PrivateKey()
        let msg = Data(repeating: 7, count: 32)
        let t = Date()
        for _ in 0..<50 { _ = try k.signature(for: msg) }
        print(String(format: "CryptoKit SecureEnclave.P256 key: CREATED (blob %d bytes); %.2f ms per signature",
                     k.dataRepresentation.count, Date().timeIntervalSince(t) * 1000 / 50))
        let back = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: k.dataRepresentation)
        print("  reloaded from its blob: \(back.publicKey.rawRepresentation == k.publicKey.rawRepresentation)")
    } catch {
        print("CryptoKit SecureEnclave.P256 key: FAILED \(error)")
    }
case "ck-save":
    // An update is a new cdhash: save a CryptoKit SE key's blob from one
    // build, then load and sign with it from the other.
    do {
        let k = try SecureEnclave.P256.Signing.PrivateKey()
        try k.dataRepresentation.write(to: URL(fileURLWithPath: service))
        try k.publicKey.rawRepresentation.write(to: URL(fileURLWithPath: service + ".pub"))
        print("saved SE key blob to \(service)")
    } catch { print("ck-save FAILED \(error)") }
case "ck-load":
    do {
        let blob = try Data(contentsOf: URL(fileURLWithPath: service))
        let pub = try Data(contentsOf: URL(fileURLWithPath: service + ".pub"))
        let k = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
        let msg = Data("challenge".utf8)
        let sig = try k.signature(for: msg)
        let ok = try P256.Signing.PublicKey(rawRepresentation: pub).isValidSignature(sig, for: msg)
        print("loaded SE key blob saved by the other build; signature verifies with the saved public key: \(ok)")
    } catch { print("ck-load FAILED \(error)") }
default:
    print("usage: keychain_probe add|read|delete <service> | dpk | se | ck-save|ck-load <file>")
}

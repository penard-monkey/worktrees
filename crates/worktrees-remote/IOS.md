# The iOS signer contract

What `worktrees-remote` asks of the iOS app's two Secure Enclave keys, and
what the Swift plugin has to do to answer. The iOS repository depends on this
crate by git, pinned to a commit, so this file and `src/sign.rs` are a
contract: a change to either is a change to both repositories.

The crate never holds an iPhone's key. It asks a [`DeviceSigner`] for two
things, the key's public half and a signature, and nothing else. Every place
the crate needs a private key goes through that trait: the TLS client
certificate and its handshake signature (`tls`), pairing (`pair`), the pipe
(`pipe`, `call`), and a step-up proof (`stepup`). The shim implements the
trait by calling the Swift plugin.

## Two keys per pairing

Both keys are made at pairing, after the link has been read (Phase 2 §2.2
step 3), with one pair of keys per pairing. Pairing again makes new keys.

| | Device key | Step-up key |
|---|---|---|
| Created with | `kSecAttrTokenIDSecureEnclave`, `kSecAttrKeyTypeECSECPrimeRandom`, 256 bits, permanent and tagged; access control `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` with `.privateKeyUsage` | The same, with `.privateKeyUsage` and `.userPresence` |
| Signs | The TLS client certificate (once per dial) and the TLS 1.3 handshake's CertificateVerify (once per dial). | `challenge ‖ exporter` for `stepup_prove` (Phase 2 §2.5), once per step-up. A step-up holds for 5 minutes on its connection. |
| Prompts | Never. It is asked on the TLS handshake's own task, so it must answer without waiting for a person. | Face ID, or the passcode, on every signature. |
| Is asked on | A tokio worker, inside rustls, synchronously. | Tokio's blocking pool (`StepUp::prove`), or the caller's own thread (`StepUp::prove_blocking`). Never the main thread. |
| Is it enrolled? | Its SPKI's SHA-256 is the device's pin. | Its SPKI is sent at pairing as `stepup_spki`. Pairing never asks it to sign, so pairing shows no Face ID. |

## `spki_der()`

Return the DER SubjectPublicKeyInfo of a P-256 key with an uncompressed point:
91 bytes, `sign::P256_SPKI_PREFIX` followed by `04 ‖ X ‖ Y`.

`SecKeyCopyExternalRepresentation(SecKeyCopyPublicKey(key))` gives the 65-byte
point (`04 ‖ X ‖ Y`), not an SPKI. Pass it through `sign::p256_spki(point)`,
which adds the prefix (and refuses anything that is not a 65-byte uncompressed
point). Return the same bytes on every call, so cache them.

## `sign(message)`

```swift
SecKeyCreateSignature(key, .ecdsaSignatureMessageX962SHA256, message as CFData, &error)
```

This is ECDSA P-256 over the SHA-256 of `message`, returned as an ASN.1 DER
`Ecdsa-Sig-Value`. The plugin hashes nothing itself: `message` is the message,
never a digest. Two near misses produce well-formed output that is wrong:

- `.ecdsaSignatureDigestX962SHA256` treats its input as a digest. Handed the
  message, it signs the wrong thing.
- CryptoKit's `SecureEnclave.P256.Signing.PrivateKey` gives a signature whose
  `rawRepresentation` is the 64-byte `r ‖ s`. Use its `derRepresentation`.

The crate verifies every signature against the key's own SPKI before it uses
it, with the same check serve runs (`ring`, ECDSA P-256 / SHA-256, ASN.1). A
signature that fails the check is `SignError::Invalid`, and nothing is sent.
For the device key that happens when the certificate is made, before anything
is dialled, so a wrong format never spends a pairing window's one attempt or
knocks on the host's listener. For the step-up key it never spends the
challenge.

## Errors: `SignError`

| What happened | Return | What the person sees |
|---|---|---|
| The person dismissed Face ID or the passcode sheet | `SignError::Cancelled` | The step-up says it was cancelled on this device: nothing was sent, and the act was not done. The connection is untouched, and the challenge lapses by itself. |
| Anything else: the phone is locked (`errSecInteractionNotAllowed`), the key is gone (`errSecItemNotFound`, after a restore to a new phone), authentication failed | `SignError::Failed(<words>)`, including the OSStatus | "the key could not sign: …", escaped by `printable` before it is shown |

The error a cancel produces is Phase 0's A4 to MEASURE, not this file's to
assume. The candidates are `errSecUserCanceled` (−128) in
`NSOSStatusErrorDomain`, and `LAError.userCancel` (−2), `.systemCancel` (−4)
or `.appCancel` (−9) in `LAErrorDomain`. Map every one A4 sees on a dismissed
prompt to `Cancelled`, and nothing else. `.systemCancel` is what the system
does when the app leaves the foreground, and to the person that is a cancel as
well.

`Invalid` is the crate's verdict. A signer never returns it.

## What the shim does with them

```rust
use std::sync::Arc;
use worktrees_remote::{pair, pipe, sign::Keys, stepup};

let keys = Keys { device: Arc::new(device), stepup: Some(Arc::new(stepup)) };

// Pairing: nothing is kept by the crate, and the app keeps the record in the
// Keychain beside its key tags.
let rec = pair::pair_keys(link, name, pair::Platform::Ios, &keys, now, &mut on_event).await?;

// A connection: the page gets the session token, and the shim keeps the
// step-up handle with the connection.
let p = pipe::open(&rec, &keys, wait).await?;
let token = p.session_token().to_string();
let su = p.stepup().expect("a pairing with a step-up key has one");
let (tx, rx) = p.split();

// A step-up: the page sent `stepup_challenge` and hands the shim the
// challenge it got back.
let challenge = stepup::parse_challenge(&b64).ok_or("not a challenge")?;
match su.prove(&challenge).await {
    Ok(proof) => proof.args(), // the page sends `stepup_prove` with these
    Err(sign::SignError::Cancelled) => /* answer a cancel, and send nothing */,
    Err(e) => /* answer the failure */,
}
```

For an iPad, pass `Platform::Ipados`. `Platform::this()` cannot tell the two
apart, because one target builds for both.

## Costs, and what Phase 0 still has to prove

- **Two device-key signatures per dial**, the certificate's and the
  handshake's, because the certificate is made fresh at each dial. A3 measures
  the signature. If it is slow, the crate can make the certificate once per
  key and keep it, and nothing in this contract changes.
- **A1**: rustls with this signer from the shim. That covers whether the
  plugin can be called synchronously from inside rustls's `Signer::sign`
  (with `run_mobile_plugin`, or with an exported C function), and whether a
  development build signed with team `M882BTE3A3` carries the Secure Enclave
  and its keychain entitlement.
- **A4**: Face ID raised by a `.userPresence` signature asked from a tokio
  blocking thread, its latency, and the exact error on a cancel (above).

[`DeviceSigner`]: src/sign.rs

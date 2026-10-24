# The iOS signer contract

What `worktrees-remote` asks of the iOS app's two Secure Enclave keys, what
the Swift plugin has to do to answer, and how the app keeps the hosts it
pairs with. The iOS repository depends on this
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
| The person dismissed Face ID or the passcode sheet: `LAError.userCancel` (`com.apple.LocalAuthentication` −2), or `errSecUserCanceled` (`NSOSStatusErrorDomain` −128) | `SignError::Cancelled` | The step-up says it was cancelled on this device: nothing was sent, and the act was not done. The connection is untouched, and the challenge lapses by itself. |
| Anything else: the phone is locked (`errSecInteractionNotAllowed`), the key is gone (`errSecItemNotFound`, after a restore to a new phone), authentication failed | `SignError::Failed(<words>)`, naming the domain and the code | "the key could not sign: …", escaped by `printable` before it is shown |

**The cancel is measured.** Phase 0's A4 dismissed Face ID on an iPhone, and
`SecKeyCreateSignature` failed with `LAError.userCancel`:
`com.apple.LocalAuthentication`, code −2. `errSecUserCanceled` (−128) is the
Security framework's own cancel, and it stays mapped too. Do not write the
mapping in Swift: hand the `CFError`'s domain, code and description to
`SignError::from_apple(domain, code, description)`, which holds both and has
a test. Every other code is `Failed`, with the domain and the code in its
words. That includes `.systemCancel` (−4, the app leaving the foreground) and
`.appCancel` (−9), which nobody has measured on a dismissal. If one of them
turns up on a cancel, it joins the list there.

`Invalid` is the crate's verdict. A signer never returns it.

## What the shim does with them

```rust
use std::sync::Arc;
use worktrees_remote::{hosts::HostStore, pair, reach, sign::Keys, stepup};

let keys = Keys { device: Arc::new(device), stepup: Some(Arc::new(stepup)) };
let store = HostStore::new(&app_support_dir); // "Keeping the hosts", below

// Pairing: the crate keeps no key. The app has its keys already, and keeps
// the record second.
let rec = pair::pair_keys(link, name, pair::Platform::Ios, &keys, now, &mut on_event).await?;
store.add_record(&rec)?;

// A connection: at the kept address, or where the host moved, and the record
// follows. The page gets the session token, and the shim keeps the step-up
// handle with the connection.
let reached = reach::open(&store, &rec, &keys, &env, wait, &mut log).await?;
let p = reached.pipe;
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

## Keeping the hosts

The keys stay in the Keychain. The records go in this crate's file store,
`hosts::HostStore`, over a directory the app owns. It is the file a desktop
keeps (`remote-hosts.json`, version 1, written under a lock and replaced
atomically), so `reach` can move a record when its host moves, and every
check made on reading the file back applies on the phone too. Do not write
the file from Swift: nothing but `HostStore` writes it.

- **Keep.** `store.add_record(&rec)` keeps the record that `pair_keys`
  returned, and only the record. It writes no key file, and the record says
  `"key": "external"`. Keep the keys first and the record second, as a
  desktop does, so the store never holds a record whose keys were not kept.
  `rec.id` is this device's name for the pairing, fixed for its life and
  chosen when the host answers. The app ties its two key tags to it; how is
  the app's choice. `add_record` is an upsert by `rec.id`, and the most hosts
  a device keeps is `hosts::MAX_HOSTS`.
- **Connect.** `reach::open(&store, &rec, &keys, &env, wait, &mut log)`
  tries the kept address. If nothing answers there, or another key does, it
  looks on the LAN and keeps a new address only once a pinned open to it has
  succeeded (`HostStore::readdress`, under the store's lock). The record
  stays `"key": "external"`.
- **An address code.** `reach::parse_code(text)` reads it, then
  `reach::adopt_code(&store, &rec, &keys, &code, &env, wait)` follows it. A
  code for another host's pin is refused before `keys` is asked anything,
  even its public half, and before anything is dialled.
- **Forget.** `store.forget(&rec.id)` removes the record, and the choice if
  it was chosen. For a record kept by `add_record` it deletes nothing, so a
  file someone put where a key file would be is left alone. The app then
  deletes its two keys itself.
- `store.key_for(&rec)` refuses a record kept by `add_record`. Nothing in the
  crate reads a key for it.

**Where.** Use a directory in Application Support. Caches can be purged by
the system, and that would lose every pairing. Each write makes a new file
and renames it over the old one, so a data protection class set on the
file alone is lost at the next write. Set `NSFileProtectionComplete` as the
app's default data protection (the Data Protection entitlement), which every
new file gets. Under that class the file cannot be read while the phone is
locked, so `list` and `get` fail then with the system's error. The device key
cannot sign then either (`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`), so
nothing connects while the phone is locked anyway. If a move cannot be
written, for any reason, `reach::open` still connects at the new address, and
`log` says that the record was not moved.

**Finding a moved host on the LAN needs a browser.** `reach::Env::platform()`
has none on iOS yet: mDNS browsing is built for macOS only. With it,
`reach::open` tries the kept address and then says it could not look. The
app can put its own `discover::Browser` in `reach::Env`: Bonjour for
`discover::SERVICE` (`_worktrees._tcp`) through the Swift plugin, handing
over each resolved advert as a `discover::Sighting`. Every sighting is then
judged by `discover::candidate` before anything is dialled, as on a Mac.
Without a browser, the address code is the way back.

## Costs, and what Phase 0 still has to prove

- **Two device-key signatures per connection**, the certificate's and the
  handshake's. `reach::open` makes the certificate once per call, however many
  addresses it tries (the kept one, and up to four on the LAN), and each
  handshake adds one. A3 measures the signature. If it is slow, the crate can
  keep the certificate across calls, and nothing in this contract changes.
- **A1**: rustls with this signer from the shim. That covers whether the
  plugin can be called synchronously from inside rustls's `Signer::sign`
  (with `run_mobile_plugin`, or with an exported C function), and whether a
  development build signed with team `M882BTE3A3` carries the Secure Enclave
  and its keychain entitlement.
- **A4**: the cancel is measured (above). Still to confirm: Face ID raised by
  a `.userPresence` signature asked from a tokio blocking thread, and its
  latency.

[`DeviceSigner`]: src/sign.rs

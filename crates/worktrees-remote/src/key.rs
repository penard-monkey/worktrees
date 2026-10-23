//! A desktop device's key (Phase 2 §2.1): P-256, in a `0600` PKCS#8 (DER)
//! file in the client's own config dir. One per pairing (§2.2 step 3 makes the
//! keys after the link is read), so pairing again after a revoke is a new
//! key, never the tombstoned one.
//!
//! The file is the credential, so a key file anyone but its owner can read —
//! or a link where the file should be — is refused, not tightened: it may
//! already have been read, and the remedy is to pair again. The page never
//! sees it: only the shim and the CLI hold a `DeviceKey`, and its `Debug`
//! prints the pin alone.
//!
//! A `DeviceKey` is one [`DeviceSigner`] among others, and the rest of the
//! crate sees it only as one: the certificate it presents is made at each
//! dial through the trait (`tls::device_cert`), and its handshake signature
//! too. Its bytes go nowhere but its own file — this module is the only one
//! that holds a private key's bytes, which a test reads the sources to keep
//! true. On iOS the key is the Secure Enclave's, behind the same trait.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use rcgen::{KeyPair, PublicKeyData, PKCS_ECDSA_P256_SHA256};

use crate::sign::{DeviceSigner, Keys, SignError};
use crate::tls::pin_of_spki;

/// The largest key file read: a P-256 PKCS#8 is under 150 bytes.
const MAX_KEY_FILE: u64 = 4096;

pub struct DeviceKey {
    pair: KeyPair,
    spki: Vec<u8>,
    pin: String,
}

impl std::fmt::Debug for DeviceKey {
    /// Never the key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceKey").field("pin", &self.pin).finish()
    }
}

impl DeviceKey {
    /// A new P-256 key.
    pub fn generate() -> Result<DeviceKey, String> {
        let pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|e| format!("a device key: {e}"))?;
        Self::from_pair(pair)
    }

    fn from_pair(pair: KeyPair) -> Result<DeviceKey, String> {
        if pair.algorithm() != &PKCS_ECDSA_P256_SHA256 {
            return Err("a device key is P-256".into());
        }
        let spki = pair.subject_public_key_info();
        Ok(DeviceKey { pair, pin: pin_of_spki(&spki), spki })
    }

    /// The key at `path`: a regular file, not a link, owned by this user and
    /// readable by nobody else, holding a P-256 PKCS#8.
    pub fn load(path: &Path) -> Result<DeviceKey, String> {
        let show = path.display();
        let mut f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| format!("the device key {show}: {e}"))?;
        let md = f.metadata().map_err(|e| format!("the device key {show}: {e}"))?;
        if !md.is_file() {
            return Err(format!("the device key {show} is not a regular file"));
        }
        // SAFETY: geteuid has no preconditions.
        if md.uid() != unsafe { libc::geteuid() } || md.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "the device key {show} is readable by someone else (mode {:o}); it is not used. Forget this host and pair again.",
                md.permissions().mode() & 0o777
            ));
        }
        if md.len() > MAX_KEY_FILE {
            return Err(format!("the device key {show} is not a key"));
        }
        let mut der = Vec::new();
        f.read_to_end(&mut der).map_err(|e| format!("the device key {show}: {e}"))?;
        let pair = KeyPair::try_from(der.as_slice()).map_err(|e| format!("the device key {show} does not parse: {e}"))?;
        Self::from_pair(pair)
    }

    /// Write the key to `path`, `0600`, through `fsx::write_atomic`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        worktrees_core::fsx::write_atomic(path, &self.pair.serialize_der(), Some(fs::Permissions::from_mode(0o600)))
            .map_err(|e| format!("the device key {}: {e}", path.display()))
    }

    /// SHA-256 of the SPKI, lowercase hex: what the host's record holds.
    pub fn pin(&self) -> &str {
        &self.pin
    }

    /// The DER SubjectPublicKeyInfo.
    pub fn spki(&self) -> &[u8] {
        &self.spki
    }

}

/// A desktop's keys: this file, and no step-up key (§2.1).
impl From<DeviceKey> for Keys {
    fn from(key: DeviceKey) -> Keys {
        Keys::device_only(std::sync::Arc::new(key))
    }
}

impl DeviceSigner for DeviceKey {
    fn spki_der(&self) -> Vec<u8> {
        self.spki.clone()
    }

    /// `ring`'s ECDSA P-256 / SHA-256, ASN.1 DER — through rcgen, which holds
    /// the key: the signature rustls's own key provider made for this file
    /// before the trait existed.
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignError> {
        rcgen::SigningKey::sign(&self.pair, message).map_err(|e| SignError::Failed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wt-remote-key-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_key_round_trips_through_a_0600_file_and_its_certificate_carries_it() {
        let d = dir("roundtrip");
        let k = DeviceKey::generate().unwrap();
        let p = d.join("k.key");
        k.save(&p).unwrap();
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        let back = DeviceKey::load(&p).unwrap();
        assert_eq!(back.pin(), k.pin());
        assert_eq!(crate::tls::pin_of_cert(&crate::tls::device_cert(&back).unwrap()).as_deref(), Some(k.pin()), "the certificate is over this key");
        assert!(!format!("{back:?}").contains(&crate::tls::hex(&k.pair.serialize_der())), "Debug never prints the key");
        let _ = fs::remove_dir_all(&d);
    }

    /// The to-be-signed part of a DER certificate: the first element of its
    /// outer SEQUENCE.
    fn tbs(cert: &[u8]) -> &[u8] {
        let len_at = |b: &[u8], i: usize| -> (usize, usize) {
            match b[i] {
                n if n < 0x80 => (n as usize, i + 1),
                0x81 => (b[i + 1] as usize, i + 2),
                0x82 => (((b[i + 1] as usize) << 8) | b[i + 2] as usize, i + 3),
                n => panic!("a length of {n:#x}"),
            }
        };
        assert_eq!(cert[0], 0x30);
        let (_, inner) = len_at(cert, 1);
        assert_eq!(cert[inner], 0x30);
        let (n, body) = len_at(cert, inner + 1);
        &cert[inner..body + n]
    }

    /// Desktop behaviour is unchanged by the trait (the brief's rule): the
    /// certificate a key file presents, now made through [`DeviceSigner`], is
    /// rcgen's certificate from the key's bytes, signature aside (ECDSA is
    /// randomized) — and both signatures verify, DER, over the same part.
    #[test]
    fn the_file_keys_certificate_through_the_trait_is_the_one_its_bytes_made() {
        let k = DeviceKey::generate().unwrap();
        let through = crate::tls::device_cert(&k).unwrap();
        let from_bytes = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap().self_signed(&k.pair).unwrap();
        assert_eq!(tbs(&through), tbs(from_bytes.der()), "the to-be-signed part, byte for byte");
        let sig = k.sign(b"m").unwrap();
        assert!(crate::sign::verifies(&k.spki_der(), b"m", &sig), "an ASN.1 DER signature over the message");
        assert_eq!(k.spki_der(), k.spki(), "one public half");
    }

    /// `text` without its `#[cfg(test)]` items — a test module's fixtures (a
    /// software key standing in for the Secure Enclave) are not the crate's
    /// paths. Each gated item is dropped on its own, to its closing `}` at
    /// column 0 (or its `;`), never by cutting the file at the first one:
    /// `tls.rs` has live code after its test module.
    fn live_lines(text: &str) -> String {
        let mut out = String::new();
        let mut lines = text.lines();
        while let Some(l) = lines.next() {
            if l.trim() != "#[cfg(test)]" {
                out.push_str(l);
                out.push('\n');
                continue;
            }
            for item in lines.by_ref() {
                if item == "}" || (item.ends_with(';') && !item.starts_with(' ')) {
                    break;
                }
            }
        }
        out
    }

    #[test]
    fn the_scan_drops_each_test_item_and_keeps_what_follows() {
        let text = "a\n#[cfg(test)]\nuse x::pkcs8;\nb\n#[cfg(test)]\nmod tests {\n    pkcs8\n}\nc\n";
        assert_eq!(live_lines(text), "a\nb\nc\n");
    }

    /// The private-bytes boundary, held by reading the sources: outside this
    /// module, nothing in the crate names a private key's bytes or a way to
    /// hand bytes to rustls, so every path that dials — pairing, the pipe, a
    /// call, a step-up — can only ask a [`DeviceSigner`] to sign. (The type
    /// system holds the rest: those paths take `sign::Keys`, which is trait
    /// objects.)
    #[test]
    fn only_the_key_file_holds_private_bytes() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let banned = ["PrivateKeyDer", "PrivatePkcs8KeyDer", "with_client_auth_cert", "CertifiedKey::from_der", "load_private_key", "serialize_der", "KeyPair::", "pkcs8"];
        let mut seen = Vec::new();
        for e in fs::read_dir(&src).unwrap() {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if name == "key.rs" || p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            let text = fs::read_to_string(&p).unwrap();
            let live = live_lines(&text);
            for b in banned {
                if live.contains(b) {
                    seen.push(format!("{name}: {b}"));
                }
            }
            seen.push(format!("read {name}"));
        }
        let read = seen.iter().filter(|s| s.starts_with("read ")).count();
        assert!(read >= 8, "the scan read the crate's modules: {seen:?}");
        seen.retain(|s| !s.starts_with("read "));
        assert!(seen.is_empty(), "private key bytes outside key.rs: {seen:?}");
    }

    /// §2.1: a key file someone else can read is refused, never tightened, and
    /// a link where the key should be is refused too.
    #[test]
    fn a_loose_or_linked_key_file_is_refused() {
        let d = dir("loose");
        let k = DeviceKey::generate().unwrap();
        let p = d.join("k.key");
        k.save(&p).unwrap();
        for mode in [0o640, 0o604, 0o660] {
            fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
            let e = DeviceKey::load(&p).unwrap_err();
            assert!(e.contains("readable by someone else"), "{mode:o}: {e}");
            assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, mode, "not tightened");
        }
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        let l = d.join("link.key");
        std::os::unix::fs::symlink(&p, &l).unwrap();
        assert!(DeviceKey::load(&l).is_err(), "a link is not followed");
        fs::write(d.join("junk.key"), b"not a key").unwrap();
        fs::set_permissions(d.join("junk.key"), fs::Permissions::from_mode(0o600)).unwrap();
        assert!(DeviceKey::load(&d.join("junk.key")).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}

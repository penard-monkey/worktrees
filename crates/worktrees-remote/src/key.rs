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
//! The certificate the device presents is made from the key at each dial
//! (self-signed, no names): the server pins the SPKI, never the certificate.
//! On iOS the key is the Secure Enclave's instead, handed to rustls as a
//! signer rather than as bytes (the hardware spike's A1); nothing here
//! assumes how it signs beyond [`crate::tls::client_config`]'s inputs.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use rcgen::{CertificateParams, KeyPair, PublicKeyData, PKCS_ECDSA_P256_SHA256};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::tls::pin_of_spki;

/// The largest key file read: a P-256 PKCS#8 is under 150 bytes.
const MAX_KEY_FILE: u64 = 4096;

pub struct DeviceKey {
    pkcs8: Vec<u8>,
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
        Self::from_pair(&pair)
    }

    fn from_pair(pair: &KeyPair) -> Result<DeviceKey, String> {
        if pair.algorithm() != &PKCS_ECDSA_P256_SHA256 {
            return Err("a device key is P-256".into());
        }
        let spki = pair.subject_public_key_info();
        Ok(DeviceKey { pkcs8: pair.serialize_der(), pin: pin_of_spki(&spki), spki })
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
        Self::from_pair(&pair)
    }

    /// Write the key to `path`, `0600`, through `fsx::write_atomic`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        worktrees_core::fsx::write_atomic(path, &self.pkcs8, Some(fs::Permissions::from_mode(0o600)))
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

    /// The key for rustls.
    pub fn private_der(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.pkcs8.clone()))
    }

    /// A self-signed certificate over this key, naming nobody.
    pub fn cert(&self) -> Result<CertificateDer<'static>, String> {
        let pair = KeyPair::try_from(self.pkcs8.as_slice()).map_err(|e| e.to_string())?;
        let cert = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?.self_signed(&pair).map_err(|e| e.to_string())?;
        Ok(cert.der().clone())
    }

    /// A client config pinning `pin` and presenting this key.
    pub fn client_config(&self, pin: &str) -> Result<std::sync::Arc<rustls::ClientConfig>, String> {
        crate::tls::client_config(pin, self.cert()?, self.private_der())
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
        assert_eq!(crate::tls::pin_of_cert(&back.cert().unwrap()).as_deref(), Some(k.pin()), "the certificate is over this key");
        assert!(!format!("{back:?}").contains(&hex_of(&k.pkcs8)), "Debug never prints the key");
        let _ = fs::remove_dir_all(&d);
    }

    fn hex_of(b: &[u8]) -> String {
        crate::tls::hex(b)
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

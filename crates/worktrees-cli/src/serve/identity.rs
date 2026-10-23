//! The server's identity (Phase 2 §2.1, §2.5): a P-256 key and a self-signed
//! certificate in the config dir, whose SPKI SHA-256 is the PIN every paired
//! device holds, and beside them the discovery id the mDNS advert will carry
//! (§4.7: 128 random bits, derived from nothing public).
//!
//! `server-identity.key` (PKCS#8, DER), `server-identity.crt` (DER) and
//! `server-identity.discovery` (26 base32 characters), each `0600`, written
//! through `fsx::write_atomic` under `server-identity.lock`. Created the first
//! time something needs them (a pairing window), then reused across launches:
//! a pin must survive a restart, unlike the loopback token.
//!
//! The certificate carries no claim worth checking — no host name, no
//! owner, validity from 1975 to 4096 — because the pin IS the identity: a
//! device compares the key and nothing else (§4.3). It names nobody, so a
//! passive observer of a TLS 1.3 handshake learns nothing from it either.
//!
//! A key file anyone but the owner can read is refused, not tightened: it may
//! already have been read, and the remedy is a rotation, which costs one
//! pairing per device and is the honest price of a suspected compromise
//! (§2.5) — there is no signed handover, since the old key would sign it.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PublicKeyData, PKCS_ECDSA_P256_SHA256};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use super::devices::{base32, pin_of};
use super::servefile;

pub const KEY_FILE: &str = "server-identity.key";
pub const CERT_FILE: &str = "server-identity.crt";
pub const DISCOVERY_FILE: &str = "server-identity.discovery";

/// What the certificate is issued to. A constant: nothing about the host.
const SUBJECT: &str = "worktrees serve";

/// The server identity, loaded.
#[derive(Clone)]
pub struct Identity {
    key_der: Vec<u8>,
    cert_der: CertificateDer<'static>,
    pin: String,
    discovery_id: String,
}

impl std::fmt::Debug for Identity {
    /// Never the key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("pin", &self.pin).field("discovery_id", &self.discovery_id).finish()
    }
}

impl Identity {
    /// The pin: SHA-256 of the certificate's SubjectPublicKeyInfo, lowercase hex.
    pub fn pin(&self) -> &str {
        &self.pin
    }

    pub fn discovery_id(&self) -> &str {
        &self.discovery_id
    }

    pub fn cert(&self) -> CertificateDer<'static> {
        self.cert_der.clone()
    }

    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone()))
    }

    /// The identity `dir` holds, or `None` if it has none yet. Refused: files
    /// that do not parse, a key and certificate that do not belong together,
    /// a key that is not P-256, and a key file with any group or world bit.
    pub fn load(dir: &Path) -> Result<Option<Identity>, String> {
        let (kp, cp) = (dir.join(KEY_FILE), dir.join(CERT_FILE));
        let key = read_private(&kp)?;
        let cert = read_private(&cp)?;
        let (key, cert) = match (key, cert) {
            (None, None) => return Ok(None),
            (Some(k), Some(c)) => (k, c),
            _ => return Err(broken("one of its two files is missing")),
        };
        let pair = KeyPair::try_from(key.as_slice()).map_err(|e| broken(&format!("{KEY_FILE} does not parse: {e}")))?;
        if pair.algorithm() != &PKCS_ECDSA_P256_SHA256 {
            return Err(broken(&format!("{KEY_FILE} is not a P-256 key")));
        }
        let cert = CertificateDer::from(cert);
        let spki = rustls::server::ParsedCertificate::try_from(&cert)
            .map_err(|e| broken(&format!("{CERT_FILE} does not parse: {e}")))?
            .subject_public_key_info();
        if spki.as_ref() != pair.subject_public_key_info().as_slice() {
            return Err(broken("the key and the certificate do not belong together"));
        }
        let discovery_id = match read_private(&dir.join(DISCOVERY_FILE))? {
            Some(b) => String::from_utf8(b).ok().map(|s| s.trim().to_string()).filter(|s| is_discovery_id(s)).ok_or_else(|| broken(&format!("{DISCOVERY_FILE} is not a discovery id")))?,
            None => {
                let id = new_discovery_id();
                write_private(&dir.join(DISCOVERY_FILE), format!("{id}\n").as_bytes())?;
                id
            }
        };
        Ok(Some(Identity { pin: pin_of(spki.as_ref()), key_der: key, cert_der: cert, discovery_id }))
    }

    /// The identity, created on first need (§2.1). Under the lock, so two
    /// first needs make one identity.
    pub fn ensure(dir: &Path) -> Result<Identity, String> {
        servefile::with_lock(dir, "server-identity", || {
            if let Some(id) = Identity::load(dir)? {
                return Ok(id);
            }
            let id = generate()?;
            save(dir, &id)?;
            Ok(id)
        })
    }

    /// A new key, certificate and discovery id in place of the old (§2.5:
    /// `identity_rotate`). Every paired device must pair again: the pin each
    /// holds no longer matches. A broken identity can always be rotated.
    pub fn rotate(dir: &Path) -> Result<Identity, String> {
        servefile::with_lock(dir, "server-identity", || {
            let id = generate()?;
            save(dir, &id)?;
            Ok(id)
        })
    }
}

fn broken(why: &str) -> String {
    format!("the server identity is unusable ({why}); rotate it to make a new one, and pair every device again")
}

/// A fresh P-256 key and its self-signed certificate.
fn generate() -> Result<Identity, String> {
    let pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|e| format!("could not make a key: {e}"))?;
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, SUBJECT);
    params.distinguished_name = dn;
    let cert = params.self_signed(&pair).map_err(|e| format!("could not make a certificate: {e}"))?;
    let spki = pair.subject_public_key_info();
    Ok(Identity { pin: pin_of(&spki), key_der: pair.serialize_der(), cert_der: cert.der().clone(), discovery_id: new_discovery_id() })
}

/// The three files, each `0600`. The key last: a crash part-way leaves a
/// pair that does not match, which `load` refuses rather than serves.
fn save(dir: &Path, id: &Identity) -> Result<(), String> {
    write_private(&dir.join(DISCOVERY_FILE), format!("{}\n", id.discovery_id).as_bytes())?;
    write_private(&dir.join(CERT_FILE), id.cert_der.as_ref())?;
    write_private(&dir.join(KEY_FILE), &id.key_der)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    worktrees_core::fsx::write_atomic(path, bytes, Some(fs::Permissions::from_mode(0o600))).map_err(|e| format!("{}: {e}", path.display()))
}

/// A file of the identity: `None` if absent; refused if it is not a regular
/// file or anyone but the owner may read or write it.
fn read_private(path: &PathBuf) -> Result<Option<Vec<u8>>, String> {
    let m = match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
        Ok(m) => m,
    };
    if !m.file_type().is_file() {
        return Err(broken(&format!("{} is not a regular file", path.display())));
    }
    if m.permissions().mode() & 0o077 != 0 {
        return Err(broken(&format!("{} is readable by others (mode {:o})", path.display(), m.permissions().mode() & 0o777)));
    }
    fs::read(path).map(Some).map_err(|e| format!("{}: {e}", path.display()))
}

/// 128 random bits, lowercase base32: 26 characters (§4.7).
pub fn new_discovery_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the OS refused entropy");
    base32(&b)
}

pub fn is_discovery_id(s: &str) -> bool {
    s.len() == 26 && s.bytes().all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-identity-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn an_identity_is_made_once_owner_only_and_reused() {
        let d = dir("make");
        assert!(Identity::load(&d).unwrap().is_none(), "none until something needs it");
        let a = Identity::ensure(&d).unwrap();
        for f in [KEY_FILE, CERT_FILE, DISCOVERY_FILE] {
            assert_eq!(mode(&d.join(f)), 0o600, "{f}");
        }
        assert!(devices_pin(&a));
        assert!(is_discovery_id(a.discovery_id()), "{}", a.discovery_id());
        let b = Identity::ensure(&d).unwrap();
        assert_eq!((a.pin(), a.discovery_id()), (b.pin(), b.discovery_id()), "a pin survives a restart");
        // The pin is the certificate's own SPKI hash, as a device computes it.
        let spki = rustls::server::ParsedCertificate::try_from(&b.cert()).unwrap().subject_public_key_info();
        assert_eq!(pin_of(spki.as_ref()), a.pin());
        let _ = fs::remove_dir_all(&d);
    }

    fn devices_pin(id: &Identity) -> bool {
        super::super::devices::is_pin(id.pin())
    }

    #[test]
    fn a_rotation_changes_the_pin_and_the_discovery_id() {
        let d = dir("rotate");
        let a = Identity::ensure(&d).unwrap();
        let b = Identity::rotate(&d).unwrap();
        assert_ne!(a.pin(), b.pin());
        assert_ne!(a.discovery_id(), b.discovery_id(), "the discovery id rotates with the key (§2.1)");
        assert_eq!(Identity::load(&d).unwrap().unwrap().pin(), b.pin());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_broken_or_exposed_identity_is_refused_and_rotation_repairs_it() {
        let d = dir("broken");
        let a = Identity::ensure(&d).unwrap();
        // A key readable by the group: it may have been read.
        fs::set_permissions(d.join(KEY_FILE), fs::Permissions::from_mode(0o640)).unwrap();
        assert!(Identity::load(&d).unwrap_err().contains("readable by others"));
        assert!(Identity::ensure(&d).is_err(), "never served, never silently replaced");
        fs::set_permissions(d.join(KEY_FILE), fs::Permissions::from_mode(0o600)).unwrap();
        // A certificate from another key.
        let other = generate().unwrap();
        write_private(&d.join(CERT_FILE), other.cert_der.as_ref()).unwrap();
        assert!(Identity::load(&d).unwrap_err().contains("do not belong together"));
        // Garbage, and half an identity.
        write_private(&d.join(KEY_FILE), b"not a key").unwrap();
        assert!(Identity::load(&d).unwrap_err().contains("does not parse"));
        fs::remove_file(d.join(CERT_FILE)).unwrap();
        assert!(Identity::load(&d).unwrap_err().contains("missing"));
        let b = Identity::rotate(&d).unwrap();
        assert_ne!(b.pin(), a.pin());
        assert_eq!(Identity::load(&d).unwrap().unwrap().pin(), b.pin());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_certificate_names_nothing_about_the_host() {
        let id = generate().unwrap();
        let der = id.cert_der.as_ref();
        let mut buf = [0u8; 256];
        // SAFETY: a stack buffer and its length.
        let host = if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
            String::from_utf8_lossy(&buf[..buf.iter().position(|b| *b == 0).unwrap_or(buf.len())]).into_owned()
        } else {
            String::new()
        };
        let user = std::env::var("USER").unwrap_or_default();
        for word in [host.as_str(), user.as_str()].into_iter().filter(|w| w.len() > 2) {
            assert!(!der.windows(word.len()).any(|w| w == word.as_bytes()), "the certificate carries {word:?}");
        }
        assert!(der.windows(SUBJECT.len()).any(|w| w == SUBJECT.as_bytes()));
        assert!(format!("{id:?}").contains(id.pin()) && !format!("{id:?}").contains("key_der"), "Debug never prints the key");
    }
}

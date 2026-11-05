//! The team's APNs key: imported at the console, stored `0600`, read back by
//! nothing but the sender, and never logged.
//!
//! A `.p8` is PKCS#8 PEM around a P-256 key. `ring` signs ES256 with it, and
//! its parser is the import's whole validity check: what it refuses never
//! reaches the file. Its errors carry no key bytes and neither do ours.

use std::path::{Path, PathBuf};

use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use serde::{Deserialize, Serialize};
use worktrees_api::files::b64_encode;

use crate::serve::stepup::b64_decode;

pub const FILE: &str = "apns-key.json";

/// A `.p8` is a few hundred bytes. Past this it is not one, and is not read.
const MAX_P8: u64 = 8 * 1024;

/// Apple's key ids and team ids: ten uppercase letters and digits.
fn apple_id_ok(s: &str) -> bool {
    s.len() == 10 && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// As the file holds it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    key_id: String,
    team_id: String,
    /// The PKCS#8 DER, standard base64.
    pkcs8: String,
}

/// The key, loaded. No `Debug` that could print it, no `Serialize`.
pub struct ApnsKey {
    pub key_id: String,
    pub team_id: String,
    signer: EcdsaKeyPair,
    rng: SystemRandom,
}

impl std::fmt::Debug for ApnsKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApnsKey(..)")
    }
}

impl ApnsKey {
    fn from_der(key_id: String, team_id: String, der: &[u8]) -> Result<ApnsKey, String> {
        let rng = SystemRandom::new();
        let signer = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, der, &rng).map_err(|_| "not a P-256 key in PKCS#8".to_string())?;
        Ok(ApnsKey { key_id, team_id, signer, rng })
    }

    /// An ES256 signature over `msg`: `r‖s`, 64 bytes.
    pub fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, String> {
        self.signer.sign(&self.rng, msg).map(|s| s.as_ref().to_vec()).map_err(|_| "could not sign".to_string())
    }
}

/// The DER inside a `.p8`'s PEM armour.
fn der_of_pem(pem: &str) -> Result<Vec<u8>, String> {
    let body: String = pem.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("-----")).collect();
    if !pem.contains("-----BEGIN PRIVATE KEY-----") || body.is_empty() {
        return Err("this is not a .p8 (no PRIVATE KEY block)".into());
    }
    b64_decode(&body).ok_or_else(|| "this is not a .p8 (the block is not base64)".to_string())
}

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE)
}

/// Read a regular file, refusing a link at the name.
fn read_regular(p: &Path, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let m = f.metadata().map_err(|e| format!("{}: {e}", p.display()))?;
    if !m.is_file() {
        return Err(format!("{} is not a regular file", p.display()));
    }
    if m.len() > max {
        return Err(format!("{} is too large", p.display()));
    }
    let mut b = Vec::new();
    f.take(max).read_to_end(&mut b).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(b)
}

/// The console's import. `p8` is a path the person at the Mac typed; only the
/// parsed key reaches `<config_dir>/apns-key.json`, `0600`, through
/// `fsx::write_atomic`. Nothing is written if any of it is wrong, and no
/// message carries the file's contents.
pub fn import(config_dir: &Path, p8: &Path, key_id: &str, team_id: &str) -> Result<ApnsKey, String> {
    use std::os::unix::fs::PermissionsExt;
    if !apple_id_ok(key_id) {
        return Err("key_id is Apple's ten-character key id".into());
    }
    if !apple_id_ok(team_id) {
        return Err("team_id is Apple's ten-character team id".into());
    }
    let bytes = read_regular(p8, MAX_P8)?;
    let pem = std::str::from_utf8(&bytes).map_err(|_| "this is not a .p8 (not text)".to_string())?;
    let der = der_of_pem(pem)?;
    let key = ApnsKey::from_der(key_id.to_string(), team_id.to_string(), &der)?;
    let stored = Stored { key_id: key_id.to_string(), team_id: team_id.to_string(), pkcs8: b64_encode(&der) };
    let out = serde_json::to_vec(&stored).map_err(|e| e.to_string())?;
    worktrees_core::fsx::write_atomic(&path(config_dir), &out, Some(std::fs::Permissions::from_mode(0o600))).map_err(|e| format!("{FILE}: {e}"))?;
    Ok(key)
}

/// The stored key, or `None` when there is none. A file that cannot be used
/// says so in the log and counts as none: pushes wait for a good import.
pub fn load(config_dir: &Path) -> Option<ApnsKey> {
    let p = path(config_dir);
    if std::fs::symlink_metadata(&p).is_err() {
        return None;
    }
    let parsed = read_regular(&p, MAX_P8 * 2).and_then(|b| {
        let s: Stored = serde_json::from_slice(&b).map_err(|_| format!("{FILE} does not parse"))?;
        let der = b64_decode(&s.pkcs8).ok_or_else(|| format!("{FILE}: not base64"))?;
        ApnsKey::from_der(s.key_id, s.team_id, &der)
    });
    match parsed {
        Ok(k) => Some(k),
        Err(e) => {
            worktrees_api::log::warn(&format!("serve: push: {e}: import the key again"));
            None
        }
    }
}

pub fn remove(config_dir: &Path) -> Result<bool, String> {
    match std::fs::remove_file(path(config_dir)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{FILE}: {e}")),
    }
}

/// A throwaway key as Apple's `.p8` lays one out, for tests. Never Apple's.
#[cfg(test)]
pub fn test_p8() -> (String, Vec<u8>) {
    let rng = SystemRandom::new();
    let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, doc.as_ref(), &rng).unwrap();
    use ring::signature::KeyPair;
    let public = pair.public_key().as_ref().to_vec();
    let b = b64_encode(doc.as_ref());
    let lines: Vec<&str> = b.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
    (format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", lines.join("\n")), public)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-push-key-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_import_is_stored_0600_and_comes_back_signing() {
        let d = dir("ok");
        let p8 = d.join("AuthKey.p8");
        let (pem, public) = test_p8();
        std::fs::write(&p8, &pem).unwrap();
        let k = import(&d, &p8, "ABC123DEFG", "M882BTE3A3").unwrap();
        assert_eq!(std::fs::metadata(path(&d)).unwrap().permissions().mode() & 0o777, 0o600);
        let again = load(&d).unwrap();
        assert_eq!((again.key_id.as_str(), again.team_id.as_str()), ("ABC123DEFG", "M882BTE3A3"));
        let sig = again.sign(b"m").unwrap();
        assert_eq!(sig.len(), 64, "ES256 is r‖s");
        ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, &public).verify(b"m", &sig).unwrap();
        assert_eq!(format!("{k:?}"), "ApnsKey(..)");
        assert!(remove(&d).unwrap());
        assert!(load(&d).is_none());
        assert!(!remove(&d).unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Every refusal writes nothing, and none repeats what it read. A wrong
    /// id is refused before the file is opened at all.
    #[test]
    fn a_bad_import_writes_nothing_and_says_nothing_of_the_file() {
        let d = dir("bad");
        let p8 = d.join("x.p8");
        let (pem, _) = test_p8();
        std::fs::write(&p8, &pem).unwrap();
        assert!(import(&d, &p8, "short", "M882BTE3A3").unwrap_err().contains("key_id"));
        assert!(import(&d, &p8, "ABC123DEFG", "lower12345").unwrap_err().contains("team_id"));
        let marker = "SECRETMARKERSECRETMARKER";
        for bad in ["not pem at all ".to_string() + marker, format!("-----BEGIN PRIVATE KEY-----\n{marker}!\n-----END PRIVATE KEY-----\n"), format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", b64_encode(marker.as_bytes()))] {
            std::fs::write(&p8, &bad).unwrap();
            let e = import(&d, &p8, "ABC123DEFG", "M882BTE3A3").unwrap_err();
            assert!(!e.contains(marker), "{e}");
            assert!(std::fs::symlink_metadata(path(&d)).is_err(), "nothing was written for {e}");
        }
        std::fs::write(&p8, vec![b'x'; MAX_P8 as usize + 1]).unwrap();
        assert!(import(&d, &p8, "ABC123DEFG", "M882BTE3A3").unwrap_err().contains("too large"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A link at either name is refused, not followed: a stored key cannot be
    /// read through one, and an import cannot be pointed at one by a repo.
    #[test]
    fn a_link_is_refused_at_both_names() {
        let d = dir("link");
        let (pem, _) = test_p8();
        let real = d.join("real.p8");
        std::fs::write(&real, &pem).unwrap();
        let link = d.join("link.p8");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(import(&d, &link, "ABC123DEFG", "M882BTE3A3").is_err());
        import(&d, &real, "ABC123DEFG", "M882BTE3A3").unwrap();
        let moved = d.join("moved.json");
        std::fs::rename(path(&d), &moved).unwrap();
        std::os::unix::fs::symlink(&moved, path(&d)).unwrap();
        assert!(load(&d).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}

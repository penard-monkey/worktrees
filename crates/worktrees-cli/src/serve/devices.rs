//! `<config_dir>/devices.json` — the principals and the paired devices this
//! engine serves, and the [`authz::Registry`] every grant check reads.
//!
//! Server Edition Phase 2 §2.1, §2.5 and §2.8 (`docs/proposals/
//! server-edition-phase2.md`). The file is `{version, principals, devices}`,
//! mode `0600`, and serve is its only writer: every change is a
//! read-modify-write under `devices.json.lock` (`servefile::with_lock`), the
//! new bytes through `fsx::write_atomic`. It lives in the config dir the
//! engine was started with, never beside the project registry, so a sandbox
//! or dev-stack serve cannot accept the user's real phone (§2.1).
//!
//! **One principal, keyed on two.** Phase 2 has exactly one principal, the
//! owner, whose id is the reserved constant `owner` and whose grant is never
//! read from the file (§2.8): a corrupt or deleted file cannot lock the
//! console out. Every device names its principal, and every rule that names
//! another device names it within the caller's principal. The loader treats
//! any other principal as corruption; the second-principal tests build one in
//! memory to prove the rules hold for it anyway.
//!
//! **Refused, never clobbered.** A file that does not parse, names another
//! principal, carries an unknown field or capability, or has a newer
//! `version` refuses EVERY device and is never overwritten: every write is
//! refused with a message saying so, and the user at the host — whose
//! identity never came from the file — repairs or removes it (§5.2).
//!
//! **Decided once.** An expiry is recorded as `disabled` when it fires and
//! never recomputed from the clock at each check (§2.5), so setting the
//! host's clock back re-enables nothing. Per-request lookups read the record
//! as it stands and nothing else; the expiry timer, the TLS verifier
//! ([`DeviceRegistry::admit`]) and the host poll are what mark it.
//!
//! **Typed events.** Every change is a [`DeviceEvent`] handed to the
//! subscribed hooks after the lock is released: `conn::Clients` acts on the
//! ones that reduce a device (§3.3), and the audit log (slice 3) records them
//! (§5.1) and raises the alerts §5.3 lists. Nothing here formats a line.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use worktrees_core::authz::{self, Caller, Capability, Caps, Device, DeviceId, Grant, PrincipalId, Projects, Registry};

use super::servefile;

pub const FILE: &str = "devices.json";

/// The file format this build writes and reads. A file with a higher one
/// was written by a newer build and is refused rather than narrowed.
pub const VERSION: u32 = 1;

/// Enrolled devices at most (§2.4, §6). Disabled ones count; revoked
/// tombstones do not.
pub const MAX_DEVICES: usize = 16;

/// The default expiry for every preset: disabled after 30 days unused
/// (§2.5, §11.1 N3).
pub const DEFAULT_IDLE_DAYS: u32 = 30;

/// The longest idle expiry the host can choose. `"never"` exists for longer.
pub const MAX_IDLE_DAYS: u32 = 3650;

/// `last_seen` is kept in memory and written at most this often, and at exit
/// (§2.5).
pub const SEEN_FLUSH_SECS: i64 = 600;

/// The longest device name, in characters (§2.2 step 4).
pub const NAME_MAX: usize = 64;

// ── the record ───────────────────────────────────────────────────────────────

/// What kind of machine a device is. Closed: the platform decides whether
/// the device holds a presence-bound step-up key (§2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Ipados,
    Macos,
    Linux,
}

impl Platform {
    pub fn parse(s: &str) -> Option<Platform> {
        serde_json::from_value(Value::from(s)).ok()
    }

    /// The device enrolls a second, presence-bound key (Face ID or the
    /// passcode) for its drive-class acts (§2.1, §2.5). A desktop has none
    /// while macOS builds are ad-hoc signed, so its drive-class acts rest on
    /// one factor, and the device list says so.
    pub fn has_presence_key(self) -> bool {
        matches!(self, Platform::Ios | Platform::Ipados)
    }
}

/// When a device stops being served (§2.5, §11 Q7): chosen on the host at
/// pairing and edited from the host console only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    /// Disabled once N days pass with no completed handshake.
    IdleDays(u32),
    /// Disabled at this moment (unix seconds; RFC 3339 in the file).
    Until(i64),
    /// Never. Allowed, and the device list marks it.
    Never,
}

impl Default for Expiry {
    fn default() -> Self {
        Expiry::IdleDays(DEFAULT_IDLE_DAYS)
    }
}

impl Expiry {
    /// `{"idle_days": N}`, `{"until": "<RFC 3339>"}` or `"never"`, strictly.
    pub fn from_value(v: &Value) -> Result<Expiry, String> {
        const SHAPE: &str = r#"an expiry is {"idle_days": N}, {"until": "<YYYY-MM-DDTHH:MM:SSZ>"} or "never""#;
        match v {
            Value::String(s) if s == "never" => Ok(Expiry::Never),
            Value::Object(m) if m.len() == 1 => {
                if let Some(n) = m.get("idle_days") {
                    let n = n
                        .as_u64()
                        .filter(|n| (1..=u64::from(MAX_IDLE_DAYS)).contains(n))
                        .ok_or_else(|| format!("idle_days must be a whole number from 1 to {MAX_IDLE_DAYS}"))?;
                    return Ok(Expiry::IdleDays(n as u32));
                }
                if let Some(u) = m.get("until") {
                    return u.as_str().and_then(parse_rfc3339).map(Expiry::Until).ok_or_else(|| SHAPE.to_string());
                }
                Err(SHAPE.to_string())
            }
            _ => Err(SHAPE.to_string()),
        }
    }

    /// When a device whose idle clock started at `idle_from` is to be
    /// disabled, and why; `None` for never.
    pub fn deadline(&self, idle_from: i64) -> Option<(i64, DisabledReason)> {
        match *self {
            Expiry::IdleDays(n) => Some((idle_from.saturating_add(i64::from(n) * 86_400), DisabledReason::Idle)),
            Expiry::Until(at) => Some((at, DisabledReason::Until)),
            Expiry::Never => None,
        }
    }

    /// `self` keeps a device longer than `other` does — or the two cannot be
    /// compared (an idle rule against a date), which counts as longer: the
    /// question is only ever asked to catch a record that says MORE than its
    /// evidence (§5.2).
    pub fn later_than(&self, other: &Expiry) -> bool {
        match (self, other) {
            (Expiry::Never, Expiry::Never) => false,
            (Expiry::Never, _) => true,
            (_, Expiry::Never) => false,
            (Expiry::IdleDays(a), Expiry::IdleDays(b)) => a > b,
            (Expiry::Until(a), Expiry::Until(b)) => a > b,
            _ => true,
        }
    }
}

impl Serialize for Expiry {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Expiry::Never => s.serialize_str("never"),
            Expiry::IdleDays(n) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("idle_days", n)?;
                m.end()
            }
            Expiry::Until(at) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("until", &rfc3339(*at))?;
                m.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Expiry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Expiry::from_value(&Value::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`, UTC, the one spelling written.
pub fn rfc3339(epoch: i64) -> String {
    format!("{}Z", worktrees_api::util::fmt_utc(epoch).replacen(' ', "T", 1))
}

/// The inverse of [`rfc3339`], and nothing looser: a string that does not
/// round-trip (a 30 February, a fraction, an offset) is refused, never
/// repaired.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let shape = b.len() == 20
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'Z',
            _ => c.is_ascii_digit(),
        });
    if !shape {
        return None;
    }
    let at = worktrees_core::sysclock::parse_iso8601(s)?;
    (rfc3339(at) == s).then_some(at)
}

/// Why a device is not being served although it was never revoked (§2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisabledReason {
    /// Its idle expiry ran out.
    Idle,
    /// Its `until` passed.
    Until,
    /// Its record says more than the evidence behind it (§5.2).
    Unrecorded,
    /// Paused by hand on the host (`device_disable`).
    Console,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disabled {
    pub at: i64,
    pub reason: DisabledReason,
}

/// A person (§2.1). Phase 2 has exactly one, the owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalRecord {
    pub id: PrincipalId,
    pub name: String,
    pub caps: Caps,
    pub projects: Projects,
    pub created_at: i64,
}

impl PrincipalRecord {
    fn owner(now: i64) -> PrincipalRecord {
        PrincipalRecord { id: PrincipalId::owner(), name: "owner".into(), caps: Caps::ALL, projects: Projects::All, created_at: now }
    }

    pub fn grant(&self) -> Grant {
        Grant { capabilities: self.caps, projects: self.projects.clone() }
    }
}

/// One paired device (§2.1). `id` is random, never derived from the key;
/// `principal` is set at pairing and never changes. A revoked record is kept
/// as a tombstone, for the audit trail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRecord {
    pub id: DeviceId,
    pub principal: PrincipalId,
    pub name: String,
    pub platform: Platform,
    /// The pin of the device's TLS key: SHA-256 of its SPKI, lowercase hex.
    pub spki_sha256: String,
    /// The pin of its presence-bound step-up key, on a platform that has one.
    pub stepup_spki_sha256: Option<String>,
    pub stepup_required: bool,
    pub caps: Caps,
    pub projects: Projects,
    pub expiry: Expiry,
    pub enrolled_at: i64,
    pub last_seen: Option<i64>,
    pub last_addr: Option<String>,
    pub disabled: Option<Disabled>,
    pub revoked_at: Option<i64>,
    /// When the host last re-enabled it: re-enabling restarts the idle clock
    /// from that moment (§2.5) without pretending the device was seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reenabled_at: Option<i64>,
}

impl DeviceRecord {
    pub fn grant(&self) -> Grant {
        Grant { capabilities: self.caps, projects: self.projects.clone() }
    }

    pub fn subject(&self) -> Subject {
        Subject { principal: self.principal.clone(), device: self.id.clone() }
    }

    /// Served: neither revoked nor disabled.
    pub fn is_live(&self) -> bool {
        self.revoked_at.is_none() && self.disabled.is_none()
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    /// Where its idle clock started: the last completed handshake, the
    /// enrollment, or the last re-enable, whichever is latest.
    pub fn idle_from(&self) -> i64 {
        [Some(self.enrolled_at), self.last_seen, self.reenabled_at].into_iter().flatten().max().unwrap_or(0)
    }

    /// What §5.1's checkpoint keeps and §5.2 compares.
    pub fn security(&self) -> SecurityFields {
        SecurityFields {
            principal: self.principal.clone(),
            device: self.id.clone(),
            spki_sha256: self.spki_sha256.clone(),
            stepup_spki_sha256: self.stepup_spki_sha256.clone(),
            stepup_required: self.stepup_required,
            caps: self.caps,
            projects: self.projects.clone(),
            expiry: self.expiry,
            disabled: self.disabled,
            revoked_at: self.revoked_at,
        }
    }
}

/// The security fields of one device record (§5.1's checkpoint): what a
/// same-user edit of `devices.json` would want to change, and what §5.2's
/// cross-check compares with the evidence behind it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityFields {
    pub principal: PrincipalId,
    pub device: DeviceId,
    pub spki_sha256: String,
    pub stepup_spki_sha256: Option<String>,
    pub stepup_required: bool,
    pub caps: Caps,
    pub projects: Projects,
    pub expiry: Expiry,
    pub disabled: Option<Disabled>,
    pub revoked_at: Option<i64>,
}

impl SecurityFields {
    /// What `self` — a record as found — claims beyond `evidence`, the last
    /// state serve wrote or the audit chain set (§5.2), or `None` if nothing.
    /// Narrower is fine: only MORE is a record that should not exist.
    pub fn exceeds(&self, evidence: &SecurityFields) -> Option<&'static str> {
        if self.principal != evidence.principal {
            return Some("principal");
        }
        if self.spki_sha256 != evidence.spki_sha256 {
            return Some("key");
        }
        if self.stepup_spki_sha256 != evidence.stepup_spki_sha256 {
            return Some("step-up key");
        }
        if evidence.revoked_at.is_some() && self.revoked_at.is_none() {
            return Some("revocation");
        }
        if !self.caps.within(evidence.caps) {
            return Some("capabilities");
        }
        if !self.projects.within(&evidence.projects) {
            return Some("projects");
        }
        if self.expiry.later_than(&evidence.expiry) {
            return Some("expiry");
        }
        if evidence.stepup_required && !self.stepup_required {
            return Some("step-up");
        }
        if evidence.disabled.is_some() && self.disabled.is_none() {
            return Some("re-enabled");
        }
        None
    }
}

/// The whole file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevicesFile {
    pub version: u32,
    pub principals: Vec<PrincipalRecord>,
    pub devices: Vec<DeviceRecord>,
}

impl DevicesFile {
    /// A file with the owner and no device: what a missing file reads as, and
    /// what the first write creates (§2.1: the owner is created with the file).
    pub fn new(now: i64) -> DevicesFile {
        DevicesFile { version: VERSION, principals: vec![PrincipalRecord::owner(now)], devices: Vec::new() }
    }

    /// Parse and validate, as Phase 2 reads it: exactly one principal, the
    /// owner with everything; every device the owner's. Anything else is
    /// corruption (§2.8, §5.2), refused rather than handled.
    pub fn parse(bytes: &[u8]) -> Result<DevicesFile, String> {
        let f: DevicesFile = serde_json::from_slice(bytes).map_err(|e| format!("{FILE} does not parse: {e}"))?;
        if f.version != VERSION {
            return Err(format!("{FILE} is version {}; this build reads version {VERSION}", f.version));
        }
        match f.principals.as_slice() {
            [p] if p.id.is_owner() && p.caps == Caps::ALL && p.projects.is_all() => {}
            _ => return Err(format!("{FILE} must name exactly one principal, the owner, with every capability and \"*\"")),
        }
        // With the owner the only principal, `check` refuses a device that
        // names any other.
        f.check()?;
        Ok(f)
    }

    /// The rules every file obeys, whatever its principals: unique ids, a key
    /// on at most one unrevoked device, well-formed pins, every device under a
    /// principal that exists and no wider than it.
    fn check(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for p in &self.principals {
            if !ids.insert(p.id.as_str().to_string()) {
                return Err(format!("{FILE}: principal {} is listed twice", p.id));
            }
        }
        let mut ids = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for d in &self.devices {
            if !ids.insert(d.id.as_str().to_string()) {
                return Err(format!("{FILE}: device {} is listed twice", d.id));
            }
            if !is_pin(&d.spki_sha256) || d.stepup_spki_sha256.as_deref().is_some_and(|p| !is_pin(p)) {
                return Err(format!("{FILE}: device {} has a malformed key pin", d.id));
            }
            if !d.is_revoked() && !keys.insert(d.spki_sha256.clone()) {
                return Err(format!("{FILE}: one key is enrolled on two devices ({})", d.id));
            }
            let p = self.principal(&d.principal).ok_or_else(|| format!("{FILE}: device {} names no principal it has", d.id))?;
            authz::check_device_grant(&p.grant(), &d.grant()).map_err(|e| format!("{FILE}: device {}: {e}", d.id))?;
        }
        Ok(())
    }

    pub fn principal(&self, id: &PrincipalId) -> Option<&PrincipalRecord> {
        self.principals.iter().find(|p| &p.id == id)
    }

    pub fn device(&self, id: &DeviceId) -> Option<&DeviceRecord> {
        self.devices.iter().find(|d| &d.id == id)
    }

    fn device_mut(&mut self, id: &DeviceId) -> Result<&mut DeviceRecord, String> {
        self.devices.iter_mut().find(|d| &d.id == id).ok_or_else(|| format!("no device {id}"))
    }

    /// The devices that count toward [`MAX_DEVICES`]: every one not revoked.
    pub fn enrolled(&self) -> usize {
        self.devices.iter().filter(|d| !d.is_revoked()).count()
    }

    fn principal_grant(&self, id: &PrincipalId) -> Option<Grant> {
        if id.is_owner() {
            return Some(Grant::everything());
        }
        self.principal(id).map(PrincipalRecord::grant)
    }
}

/// SHA-256, as 64 lowercase hex characters.
pub fn is_pin(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// The pin of a DER SubjectPublicKeyInfo: its SHA-256, lowercase hex.
pub fn pin_of(spki_der: &[u8]) -> String {
    hex(&Sha256::digest(spki_der))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A device name as the host dialog will show it (§2.2 step 4): 1 to 64
/// characters, no control characters, and nothing invisible or
/// direction-changing that could make one device pass for another — the
/// bidirectional overrides, isolates and marks, zero-width characters, the
/// line and paragraph separators, the soft hyphen. Refused, never stripped.
pub fn name_ok(name: &str) -> Result<(), String> {
    let n = name.chars().count();
    if n == 0 || name.trim().is_empty() {
        return Err("a device name cannot be empty".into());
    }
    if n > NAME_MAX {
        return Err(format!("a device name is at most {NAME_MAX} characters"));
    }
    let invisible = |c: char| {
        c.is_control()
            || matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{FEFF}')
            || ('\u{200B}'..='\u{200F}').contains(&c)
            || ('\u{2028}'..='\u{202E}').contains(&c)
            || ('\u{2060}'..='\u{2069}').contains(&c)
    };
    if name.chars().any(invisible) {
        return Err("a device name cannot hold control, invisible or direction-changing characters".into());
    }
    Ok(())
}

/// A fresh device id: `d_` and 128 random bits in lowercase base32. Random,
/// never derived from the key (§2.1).
pub fn new_device_id() -> DeviceId {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the OS refused entropy");
    DeviceId::new(format!("d_{}", base32(&b)))
}

/// RFC 4648 base32, lowercase, unpadded.
pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

// ── events ───────────────────────────────────────────────────────────────────

/// A device, named within its principal (§5.1's `detail.subject`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Subject {
    pub principal: PrincipalId,
    pub device: DeviceId,
}

/// Who made a change: the host console, a paired device, or serve itself (an
/// expiry, the cross-check).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Actor {
    Console,
    Device { principal: PrincipalId, device: DeviceId },
    Serve,
}

impl Actor {
    pub fn of(c: &Caller) -> Actor {
        match c.device() {
            Device::Console => Actor::Console,
            Device::Paired(d) => Actor::Device { principal: c.principal().clone(), device: d.clone() },
        }
    }
}

/// Why a device's handshake was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectRefusal {
    /// No device holds this key.
    Unknown,
    Revoked,
    Disabled,
    /// Its expiry ran out at this handshake; it is disabled from now on.
    Expired,
    /// `devices.json` cannot be read, so no device is served.
    Unreadable,
}

/// One change to the registry, or one refusal it decided. The audit log's
/// device records (§5.1) and the alerts of §5.3 are built from these by the
/// connection layer; `conn::Clients` acts on [`DeviceEvent::ends`] and
/// [`DeviceEvent::reduces`] (§3.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DeviceEvent {
    /// Paired (§2.2 step 7). Alert.
    Enrolled { subject: Subject, name: String, platform: Platform, fields: SecurityFields },
    /// Revoked, by whom. The record stays as a tombstone.
    Revoked { subject: Subject, by: Actor },
    /// Disabled, and why. An expiry is an alert.
    Disabled { subject: Subject, reason: DisabledReason, by: Actor },
    /// Re-enabled on the host.
    Enabled { subject: Subject },
    ExpiryChanged { subject: Subject, from: Expiry, to: Expiry },
    /// Step-up turned off (a widening) or back on.
    StepupChanged { subject: Subject, required: bool },
    /// Capabilities or projects changed. `widened` = the console added
    /// something; `reduced` = something was taken away.
    ScopeChanged { subject: Subject, by: Actor, from: Grant, to: Grant, widened: bool, reduced: bool },
    /// A revoked, disabled, expired or unknown key tried to connect. The
    /// first three are an alert (§5.3); unknown keys are aggregated.
    ConnectRefused { subject: Option<Subject>, spki_sha256: String, why: ConnectRefusal },
    /// `devices.json` cannot be read: every device is refused and the file is
    /// left as it is. Alert.
    Unreadable { why: String },
    /// The file changed under serve and was cross-checked (§5.2); each record
    /// it had to correct is its own `Disabled { reason: unrecorded }` or
    /// `Revoked` event.
    Reloaded,
    /// The server identity was rotated on the host (§2.5): a new pin, and every
    /// device revoked, each its own `Revoked` event.
    IdentityRotated { pin: String },
}

impl DeviceEvent {
    /// The device whose live connections must close now: revoked or disabled
    /// (§2.5).
    pub fn ends(&self) -> Option<&DeviceId> {
        match self {
            DeviceEvent::Revoked { subject, .. } | DeviceEvent::Disabled { subject, .. } => Some(&subject.device),
            _ => None,
        }
    }

    /// The device whose open channels must be checked against its narrowed
    /// grant (§3.3: a reduction ends each channel it no longer covers).
    pub fn reduces(&self) -> Option<&DeviceId> {
        match self {
            DeviceEvent::ScopeChanged { subject, reduced: true, .. } => Some(&subject.device),
            _ => None,
        }
    }

    /// Worth a host notification (§5.3).
    pub fn alerts(&self) -> bool {
        match self {
            DeviceEvent::Enrolled { .. } | DeviceEvent::Unreadable { .. } | DeviceEvent::IdentityRotated { .. } => true,
            DeviceEvent::Disabled { reason, .. } => *reason != DisabledReason::Console,
            DeviceEvent::ConnectRefused { why, .. } => *why != ConnectRefusal::Unknown,
            _ => false,
        }
    }
}

/// A subscriber to [`DeviceEvent`]s. Called after the registry's locks are
/// released, in subscription order, on the thread that made the change.
pub type Hook = Arc<dyn Fn(&DeviceEvent) + Send + Sync>;

// ── the registry ─────────────────────────────────────────────────────────────

/// What a new device is enrolled with (§2.2 step 7).
#[derive(Clone, Debug)]
pub struct Enrollment {
    pub principal: PrincipalId,
    pub name: String,
    pub platform: Platform,
    pub spki_sha256: String,
    pub stepup_spki_sha256: Option<String>,
    pub grant: Grant,
    pub expiry: Expiry,
    pub addr: Option<String>,
}

/// A device as a listing shows it: the record, with `last_seen` as serve
/// knows it (it is written at most every ten minutes).
pub type DeviceView = DeviceRecord;

struct State {
    /// The file, or why it cannot be read.
    file: Result<DevicesFile, String>,
    /// SHA-256 of the bytes last read or written (`None`: no file on disk).
    hash: Option<[u8; 32]>,
    /// What serve itself last wrote or vouched for, per device — the evidence
    /// a changed file is cross-checked against (§5.2). `None` until serve
    /// knows: at boot an existing file has no evidence but the audit chain's,
    /// which [`DeviceRegistry::cross_check`] takes.
    evidence: Option<HashMap<DeviceId, SecurityFields>>,
}

/// The device registry: `devices.json`, in memory and on disk.
pub struct DeviceRegistry {
    /// The config dir; `None` keeps everything in memory (tests).
    dir: Option<PathBuf>,
    state: RwLock<State>,
    /// `last_seen` and `last_addr` not yet written (§2.5).
    seen: Mutex<HashMap<DeviceId, (i64, Option<String>)>>,
    last_flush: AtomicI64,
    generation: AtomicU64,
    hooks: Mutex<Vec<Hook>>,
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

fn evidence_of(f: &DevicesFile) -> HashMap<DeviceId, SecurityFields> {
    f.devices.iter().map(|d| (d.id.clone(), d.security())).collect()
}

/// The file's bytes, `None` for absent. Anything but a regular file is
/// unreadable: a FIFO at the name would hang the read.
fn read_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
        Ok(m) if !m.file_type().is_file() => return Err(format!("{} is not a regular file", path.display())),
        Ok(_) => {}
    }
    fs::read(path).map(Some).map_err(|e| format!("{}: {e}", path.display()))
}

impl DeviceRegistry {
    /// The registry of `config_dir`, as its file stands. A missing file is
    /// the owner and no devices; an unreadable one refuses every device until
    /// it is fixed (call [`DeviceRegistry::startup`] once the hooks are in, so
    /// the alert reaches them).
    pub fn open(config_dir: &Path) -> DeviceRegistry {
        let reg = DeviceRegistry::with_dir(Some(config_dir.to_path_buf()));
        let _ = reg.locked(|| {
            let mut st = reg.state.write().unwrap();
            // Whatever is on disk now, this run of serve did not write: it is
            // taken as it stands until the audit chain's cross-check runs.
            // With no file at all there is nothing to vouch for, and every
            // device that appears later must have come through serve.
            st.evidence = None;
            reg.refresh_locked(&mut st, &mut Vec::new());
            if st.hash.is_none() && st.file.is_ok() {
                st.evidence = Some(HashMap::new());
            }
            Ok(())
        });
        reg
    }

    /// An empty registry that never touches the disk.
    pub fn in_memory() -> DeviceRegistry {
        DeviceRegistry::with_dir(None)
    }

    fn with_dir(dir: Option<PathBuf>) -> DeviceRegistry {
        DeviceRegistry {
            dir,
            state: RwLock::new(State { file: Ok(DevicesFile::new(worktrees_core::sysclock::now_epoch())), hash: None, evidence: Some(HashMap::new()) }),
            seen: Mutex::new(HashMap::new()),
            last_flush: AtomicI64::new(worktrees_core::sysclock::now_epoch()),
            generation: AtomicU64::new(1),
            hooks: Mutex::new(Vec::new()),
        }
    }

    /// A registry over `file` as given, skipping Phase 2's owner-only rule —
    /// the second-principal fixtures of §2.8, which no production path can
    /// build.
    #[cfg(test)]
    pub fn from_file_for_test(file: DevicesFile) -> DeviceRegistry {
        file.check().expect("a fixture file obeys the general rules");
        let reg = DeviceRegistry::in_memory();
        {
            let mut st = reg.state.write().unwrap();
            st.evidence = Some(evidence_of(&file));
            st.file = Ok(file);
        }
        reg
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(FILE))
    }

    /// Hand every future event to `hook`.
    pub fn subscribe(&self, hook: Hook) {
        self.hooks.lock().unwrap().push(hook);
    }

    fn fire(&self, events: &[DeviceEvent]) {
        if events.is_empty() {
            return;
        }
        let hooks = self.hooks.lock().unwrap().clone();
        for e in events {
            for h in &hooks {
                h(e);
            }
        }
    }

    /// Hand an event that is not a record change to the hooks: one the host's
    /// acts around the registry make (a rotation, a pairing window).
    pub fn announce(&self, e: DeviceEvent) {
        self.fire(&[e]);
    }

    /// Once at boot, after the hooks: an unreadable file's alert, and every
    /// expiry that ran out while serve was down.
    pub fn startup(&self, now: i64) {
        let why = self.state.read().unwrap().file.as_ref().err().cloned();
        if let Some(why) = why {
            self.fire(&[DeviceEvent::Unreadable { why }]);
            return;
        }
        let _ = self.check_expiry(now);
    }

    /// The file can be read. While it cannot, every device is refused.
    pub fn readable(&self) -> Result<(), String> {
        self.state.read().unwrap().file.as_ref().map(|_| ()).map_err(|why| unreadable(why))
    }

    /// Run `f` under the file lock, against the freshest state.
    fn locked<T>(&self, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        match &self.dir {
            Some(d) => servefile::with_lock(d, FILE, f),
            None => f(),
        }
    }

    /// Re-read the file if it is not what serve last read or wrote. A change
    /// nobody here made is cross-checked against the evidence (§5.2) and the
    /// corrections written back; an unreadable file is kept as it is and
    /// refuses every device.
    fn refresh_locked(&self, st: &mut State, events: &mut Vec<DeviceEvent>) {
        let Some(path) = self.path() else { return };
        let bytes = match read_file(&path) {
            Ok(b) => b,
            Err(why) => {
                if st.file.as_ref().err() != Some(&why) {
                    st.file = Err(why.clone());
                    st.hash = None;
                    self.generation.fetch_add(1, Ordering::SeqCst);
                    events.push(DeviceEvent::Unreadable { why });
                }
                return;
            }
        };
        let hash = bytes.as_deref().map(sha256);
        // Unchanged since serve last read or wrote it — a corrupt file
        // included, which is reported once, not at every look.
        if hash == st.hash && (hash.is_some() || st.file.is_ok()) {
            return;
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        let Some(bytes) = bytes else {
            // Removed: the owner and no devices. Anything a deleted file
            // held is gone, which only ever narrows.
            st.file = Ok(DevicesFile::new(worktrees_core::sysclock::now_epoch()));
            st.hash = None;
            st.evidence = Some(HashMap::new());
            return;
        };
        let parsed = match DevicesFile::parse(&bytes) {
            Ok(f) => f,
            Err(why) => {
                st.file = Err(why.clone());
                st.hash = hash;
                events.push(DeviceEvent::Unreadable { why });
                return;
            }
        };
        st.hash = hash;
        // No evidence of serve's own (an existing file at boot): taken as it
        // stands, until the audit chain's cross-check runs.
        let Some(ev) = st.evidence.clone() else {
            st.file = Ok(parsed);
            return;
        };
        // Serve knows what it wrote: whatever says more is corrected.
        let mut fixed = parsed;
        let mut corrections = Vec::new();
        correct(&mut fixed, &ev, worktrees_core::sysclock::now_epoch(), &mut corrections);
        events.push(DeviceEvent::Reloaded);
        if corrections.is_empty() {
            st.evidence = Some(evidence_of(&fixed));
            st.file = Ok(fixed);
            return;
        }
        events.extend(corrections);
        if let Err(why) = self.write_locked(st, &fixed) {
            worktrees_api::log::error(&format!("serve: {FILE}: could not write the cross-check's corrections: {why}"));
            st.evidence = Some(evidence_of(&fixed));
            st.file = Ok(fixed);
        }
    }

    /// Serialize and replace the file, `0600`. The caller holds the lock.
    fn write_locked(&self, st: &mut State, next: &DevicesFile) -> Result<(), String> {
        if let Some(path) = self.path() {
            use std::os::unix::fs::PermissionsExt;
            let mut bytes = serde_json::to_vec_pretty(next).map_err(|e| e.to_string())?;
            bytes.push(b'\n');
            worktrees_core::fsx::write_atomic(&path, &bytes, Some(fs::Permissions::from_mode(0o600)))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            st.hash = Some(sha256(&bytes));
        }
        st.evidence = Some(evidence_of(next));
        st.file = Ok(next.clone());
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// One change: read the freshest file under the lock, apply `f` to a copy,
    /// write it if it changed, then hand the events to the hooks. An `Err`
    /// from `f` changes nothing (a reload's own events still go out).
    fn mutate<T>(&self, f: impl FnOnce(&mut DevicesFile, &mut Vec<DeviceEvent>) -> Result<T, String>) -> Result<T, String> {
        let mut reloaded = Vec::new();
        let mut events = Vec::new();
        let out = self.locked(|| {
            let mut st = self.state.write().unwrap();
            self.refresh_locked(&mut st, &mut reloaded);
            let cur = st.file.as_ref().map_err(|why| unreadable(why))?.clone();
            let mut next = cur.clone();
            let t = f(&mut next, &mut events)?;
            if next != cur {
                self.write_locked(&mut st, &next)?;
            }
            Ok(t)
        });
        if out.is_err() {
            events.clear();
        }
        reloaded.extend(events);
        self.fire(&reloaded);
        out
    }

    /// Re-read the file if it changed on disk. The serve poll calls it, so a
    /// hand edit is cross-checked without waiting for the next write.
    pub fn refresh(&self) {
        let mut events = Vec::new();
        let _ = self.locked(|| {
            let mut st = self.state.write().unwrap();
            self.refresh_locked(&mut st, &mut events);
            Ok(())
        });
        self.fire(&events);
    }

    /// The registry as it stands, `last_seen` included. `None` while the file
    /// is unreadable.
    pub fn snapshot(&self) -> Option<DevicesFile> {
        let mut f = self.state.read().unwrap().file.clone().ok()?;
        let seen = self.seen.lock().unwrap();
        for d in &mut f.devices {
            if let Some((at, addr)) = seen.get(&d.id) {
                if d.last_seen.is_none_or(|s| s < *at) {
                    d.last_seen = Some(*at);
                    d.last_addr = addr.clone();
                }
            }
        }
        Some(f)
    }

    /// The devices `of` may see: every device of that principal (§3.3, a
    /// listing is filtered to the caller's principal), tombstones included.
    pub fn list(&self, of: &PrincipalId) -> Result<Vec<DeviceView>, String> {
        let f = self.snapshot().ok_or_else(|| self.readable().unwrap_err())?;
        Ok(f.devices.into_iter().filter(|d| &d.principal == of).collect())
    }

    /// One device of `of`'s, as a listing shows it.
    pub fn get(&self, of: &PrincipalId, id: &DeviceId) -> Option<DeviceView> {
        self.snapshot()?.devices.into_iter().find(|d| &d.id == id && &d.principal == of)
    }

    /// Every unrevoked device's security fields: §5.1's checkpoint.
    pub fn checkpoint(&self) -> Vec<SecurityFields> {
        let st = self.state.read().unwrap();
        st.file.as_ref().map(|f| f.devices.iter().filter(|d| !d.is_revoked()).map(DeviceRecord::security).collect()).unwrap_or_default()
    }

    // ── changes ──────────────────────────────────────────────────────────────

    /// Enroll a device (§2.2 step 7, the host's Allow). Refused past
    /// [`MAX_DEVICES`], for a key already on an unrevoked device, for a grant
    /// wider than its principal's (§2.8), and for a step-up key on the wrong
    /// platform: an iPhone or iPad enrolls one, a desktop has none.
    pub fn enroll(&self, e: Enrollment, now: i64) -> Result<DeviceRecord, String> {
        name_ok(&e.name)?;
        if !is_pin(&e.spki_sha256) || e.stepup_spki_sha256.as_deref().is_some_and(|p| !is_pin(p)) {
            return Err("a key pin is 64 lowercase hex characters".into());
        }
        match (e.platform.has_presence_key(), &e.stepup_spki_sha256) {
            (true, None) => return Err("an iPhone or iPad enrolls a step-up key with its device key".into()),
            (false, Some(_)) => return Err("a desktop device has no presence-bound step-up key to enroll".into()),
            _ => {}
        }
        if let Expiry::Until(at) = e.expiry {
            if at <= now {
                return Err("the expiry date has already passed".into());
            }
        }
        self.mutate(|f, events| {
            let pg = f.principal_grant(&e.principal).ok_or_else(|| format!("no principal {}", e.principal))?;
            authz::check_device_grant(&pg, &e.grant)?;
            if f.enrolled() >= MAX_DEVICES {
                return Err(format!("{MAX_DEVICES} devices are enrolled; revoke one before pairing another"));
            }
            if f.devices.iter().any(|d| !d.is_revoked() && d.spki_sha256 == e.spki_sha256) {
                return Err("this device's key is already enrolled".into());
            }
            let mut id = new_device_id();
            while f.device(&id).is_some() {
                id = new_device_id();
            }
            let rec = DeviceRecord {
                id,
                principal: e.principal.clone(),
                name: e.name.clone(),
                platform: e.platform,
                spki_sha256: e.spki_sha256.clone(),
                stepup_spki_sha256: e.stepup_spki_sha256.clone(),
                // On by default (§11 Q8), for every device that holds the key:
                // inert while it has no drive-class capability, and already
                // in force the day the console widens it to one.
                stepup_required: e.platform.has_presence_key(),
                caps: e.grant.capabilities,
                projects: e.grant.projects.clone(),
                expiry: e.expiry,
                enrolled_at: now,
                last_seen: None,
                last_addr: e.addr.clone(),
                disabled: None,
                revoked_at: None,
                reenabled_at: None,
            };
            events.push(DeviceEvent::Enrolled { subject: rec.subject(), name: rec.name.clone(), platform: rec.platform, fields: rec.security() });
            f.devices.push(rec.clone());
            Ok(rec)
        })
    }

    /// Revoke `id` (§2.5). The console may revoke any device; a device may
    /// always revoke ITSELF; a device holding `admin` may revoke another device
    /// of its own principal, and nothing else. Idempotent: `Ok(false)` for a
    /// tombstone. A device of another principal is answered as a missing one.
    pub fn revoke(&self, by: &Caller, by_grant: &Grant, id: &DeviceId, now: i64) -> Result<bool, String> {
        self.mutate(|f, events| {
            let d = f.device_mut(id)?;
            check_reach(by, by_grant, d, Reach::Revoke)?;
            if d.is_revoked() {
                return Ok(false);
            }
            d.revoked_at = Some(now);
            events.push(DeviceEvent::Revoked { subject: d.subject(), by: Actor::of(by) });
            Ok(true)
        })
    }

    /// The console's "turn off every device but this one" (§2.5): revoke every
    /// unrevoked device except `keep`. The revoked ids.
    pub fn revoke_all_others(&self, keep: Option<&DeviceId>, now: i64) -> Result<Vec<DeviceId>, String> {
        self.mutate(|f, events| {
            if let Some(k) = keep {
                f.device_mut(k)?;
            }
            let mut out = Vec::new();
            for d in f.devices.iter_mut().filter(|d| !d.is_revoked() && Some(&d.id) != keep) {
                d.revoked_at = Some(now);
                events.push(DeviceEvent::Revoked { subject: d.subject(), by: Actor::Console });
                out.push(d.id.clone());
            }
            Ok(out)
        })
    }

    /// Pause a device by hand on the host (`device_disable`): refused exactly
    /// as a revoked one is, but kept with its keys and scope. `Ok(false)` if
    /// it already was.
    pub fn disable(&self, id: &DeviceId, now: i64) -> Result<bool, String> {
        self.mutate(|f, events| disable_in(f, id, DisabledReason::Console, Actor::Console, now, events))
    }

    /// Re-enable a disabled device, on the host only (§2.5, §11 Q7). Its idle
    /// clock restarts now. A revoked device is never re-enabled.
    pub fn enable(&self, id: &DeviceId, now: i64) -> Result<bool, String> {
        self.mutate(|f, events| {
            let d = f.device_mut(id)?;
            if d.is_revoked() {
                return Err(format!("device {id} is revoked; pair it again instead"));
            }
            if d.disabled.is_none() {
                return Ok(false);
            }
            // An `until` that has already passed would fire again at once.
            if let Expiry::Until(at) = d.expiry {
                if at <= now {
                    return Err(format!("device {id}'s expiry date has passed; set a new expiry before re-enabling it"));
                }
            }
            d.disabled = None;
            d.reenabled_at = Some(now);
            events.push(DeviceEvent::Enabled { subject: d.subject() });
            Ok(true)
        })
    }

    /// Change a device's expiry, on the host only (§2.5): an `admin` device may
    /// not, not even to shorten one.
    pub fn set_expiry(&self, id: &DeviceId, expiry: Expiry) -> Result<bool, String> {
        self.mutate(|f, events| {
            let d = f.device_mut(id)?;
            if d.is_revoked() {
                return Err(format!("device {id} is revoked"));
            }
            if d.expiry == expiry {
                return Ok(false);
            }
            events.push(DeviceEvent::ExpiryChanged { subject: d.subject(), from: d.expiry, to: expiry });
            d.expiry = expiry;
            Ok(true)
        })
    }

    /// Turn a device's step-up on or off, on the host only. Off removes a
    /// factor, so it is a widening: audited, and shown for as long as it stays
    /// off (§2.5). Only a device that holds a step-up key can have it on.
    pub fn set_stepup(&self, id: &DeviceId, required: bool) -> Result<bool, String> {
        self.mutate(|f, events| {
            let d = f.device_mut(id)?;
            if d.is_revoked() {
                return Err(format!("device {id} is revoked"));
            }
            if required && d.stepup_spki_sha256.is_none() {
                return Err(format!("device {id} enrolled no step-up key"));
            }
            if d.stepup_required == required {
                return Ok(false);
            }
            d.stepup_required = required;
            events.push(DeviceEvent::StepupChanged { subject: d.subject(), required });
            Ok(true)
        })
    }

    /// Set a device's capabilities and projects from the HOST CONSOLE, the
    /// one place a scope may widen (§3.2) — never past its principal's grant.
    pub fn set_scope(&self, id: &DeviceId, grant: Grant) -> Result<bool, String> {
        self.mutate(|f, events| {
            let pg = {
                let d = f.device(id).ok_or_else(|| format!("no device {id}"))?;
                f.principal_grant(&d.principal).ok_or_else(|| format!("no principal {}", d.principal))?
            };
            authz::check_device_grant(&pg, &grant)?;
            let d = f.device_mut(id)?;
            if d.is_revoked() {
                return Err(format!("device {id} is revoked"));
            }
            Ok(change_scope(d, grant, Actor::Console, events))
        })
    }

    /// Narrow ANOTHER device of the caller's principal (§3.2): a device with
    /// `admin` may take capabilities or projects away from a peer, never add
    /// one, and never change itself. The console narrows with
    /// [`DeviceRegistry::set_scope`].
    pub fn narrow(&self, by: &Caller, by_grant: &Grant, id: &DeviceId, grant: Grant) -> Result<bool, String> {
        self.mutate(|f, events| {
            let d = f.device_mut(id)?;
            check_reach(by, by_grant, d, Reach::Narrow)?;
            if d.is_revoked() {
                return Err(format!("device {id} is revoked"));
            }
            if !grant.within(&d.grant()) {
                return Err("a device can only narrow another device's scope; widening is the host's".into());
            }
            Ok(change_scope(d, grant, Actor::of(by), events))
        })
    }

    /// A completed, authorized handshake (§2.5): `last_seen` moves, in memory,
    /// and reaches the file at most every [`SEEN_FLUSH_SECS`] and at exit. A
    /// refused handshake never comes here, so a disabled device cannot keep
    /// itself alive by knocking.
    pub fn seen(&self, id: &DeviceId, addr: Option<String>, now: i64) {
        self.seen.lock().unwrap().insert(id.clone(), (now, addr));
        self.flush_due(now);
    }

    /// [`DeviceRegistry::flush_seen`], if the last write is ten minutes old.
    pub fn flush_due(&self, now: i64) {
        if now - self.last_flush.load(Ordering::SeqCst) >= SEEN_FLUSH_SECS {
            self.flush_seen(now);
        }
    }

    /// Write every pending `last_seen`. The ten-minute tick and the exit path.
    pub fn flush_seen(&self, now: i64) {
        let pending = std::mem::take(&mut *self.seen.lock().unwrap());
        self.last_flush.store(now, Ordering::SeqCst);
        if pending.is_empty() {
            return;
        }
        let r = self.mutate(|f, _| {
            for d in &mut f.devices {
                if let Some((at, addr)) = pending.get(&d.id) {
                    if d.last_seen.is_none_or(|s| s < *at) {
                        d.last_seen = Some(*at);
                        d.last_addr = addr.clone();
                    }
                }
            }
            Ok(())
        });
        if let Err(e) = r {
            worktrees_api::log::warn(&format!("serve: {FILE}: last-seen not written: {e}"));
        }
    }

    /// Disable every live device whose expiry has run out at `now` (§2.5),
    /// once, as `disabled: {at, reason}`. The ids disabled.
    pub fn check_expiry(&self, now: i64) -> Result<Vec<DeviceId>, String> {
        let due = self.due(now);
        if due.is_empty() {
            return Ok(Vec::new());
        }
        self.mutate(|f, events| {
            let mut out = Vec::new();
            for (id, reason) in &due {
                if disable_in(f, id, *reason, Actor::Serve, now, events)? {
                    out.push(id.clone());
                }
            }
            Ok(out)
        })
    }

    /// Live devices whose expiry has run out at `now`, `last_seen` as serve
    /// knows it.
    fn due(&self, now: i64) -> Vec<(DeviceId, DisabledReason)> {
        let Some(f) = self.snapshot() else { return Vec::new() };
        f.devices
            .iter()
            .filter(|d| d.is_live())
            .filter_map(|d| d.expiry.deadline(d.idle_from()).filter(|(at, _)| now >= *at).map(|(_, r)| (d.id.clone(), r)))
            .collect()
    }

    /// The earliest moment a live device expires: when the expiry timer next
    /// has work.
    pub fn next_deadline(&self) -> Option<i64> {
        let f = self.snapshot()?;
        f.devices.iter().filter(|d| d.is_live()).filter_map(|d| d.expiry.deadline(d.idle_from()).map(|(at, _)| at)).min()
    }

    /// The handshake check (§2.5, §4.3): the device holding the key pinned as
    /// `spki_sha256`, if it may connect now. A device whose expiry ran out is
    /// disabled HERE, at the knock, and refused. Every refusal of a known
    /// device is an event (§5.3: "a revoked or disabled device trying to
    /// connect").
    pub fn admit(&self, spki_sha256: &str, now: i64) -> Result<Caller, ConnectRefusal> {
        let found = {
            let st = self.state.read().unwrap();
            match &st.file {
                Err(_) => Err(ConnectRefusal::Unreadable),
                Ok(f) => Ok(f
                    .devices
                    .iter()
                    .filter(|d| d.spki_sha256 == spki_sha256)
                    .max_by_key(|d| (!d.is_revoked(), d.enrolled_at))
                    .cloned()),
            }
        };
        let refuse = |subject: Option<Subject>, why| {
            self.fire(&[DeviceEvent::ConnectRefused { subject, spki_sha256: spki_sha256.to_string(), why }]);
            Err(why)
        };
        let d = match found {
            Err(why) => return refuse(None, why),
            Ok(None) => return refuse(None, ConnectRefusal::Unknown),
            Ok(Some(d)) => d,
        };
        if d.is_revoked() {
            return refuse(Some(d.subject()), ConnectRefusal::Revoked);
        }
        if d.disabled.is_some() {
            return refuse(Some(d.subject()), ConnectRefusal::Disabled);
        }
        if self.due(now).iter().any(|(id, _)| id == &d.id) {
            let _ = self.check_expiry(now);
            return refuse(Some(d.subject()), ConnectRefusal::Expired);
        }
        Ok(Caller::paired(d.principal, d.id))
    }

    /// §5.2's cross-check against the audit chain: `evidence` is, per device,
    /// the security fields its last `enrolled`, scope, expiry and step-up
    /// records set (the newest checkpoint onward). A device with no evidence
    /// is disabled as `unrecorded`, and so is one whose record says more than
    /// its evidence; a tombstone edited back to life is revoked again. The
    /// devices corrected. The audit lane wires this at boot.
    pub fn cross_check(&self, evidence: &HashMap<DeviceId, SecurityFields>, now: i64) -> Result<Vec<DeviceId>, String> {
        self.mutate(|f, events| {
            let before = events.len();
            correct(f, evidence, now, events);
            Ok(events[before..].iter().filter_map(|e| e.ends().cloned()).collect())
        })
    }
}

/// What [`check_reach`] is asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    Revoke,
    Narrow,
}

/// May `by` act on `d` (§2.5, §3.2, §2.8)? The console reaches every device
/// of the owner. A device reaches another device only within its own
/// principal — anything else reads as missing — and only with `admin`. A
/// device may always revoke itself, and never narrows itself.
fn check_reach(by: &Caller, by_grant: &Grant, d: &DeviceRecord, what: Reach) -> Result<(), String> {
    if &d.principal != by.principal() {
        return Err(format!("no device {}", d.id));
    }
    match by.device() {
        Device::Console => Ok(()),
        Device::Paired(me) if me == &d.id => match what {
            Reach::Revoke => Ok(()),
            Reach::Narrow => Err("a device cannot change its own scope".into()),
        },
        Device::Paired(_) if by_grant.allows(Capability::Admin) => Ok(()),
        Device::Paired(_) => Err("acting on another device needs `admin`".into()),
    }
}

fn disable_in(f: &mut DevicesFile, id: &DeviceId, reason: DisabledReason, by: Actor, now: i64, events: &mut Vec<DeviceEvent>) -> Result<bool, String> {
    let d = f.device_mut(id)?;
    if d.is_revoked() {
        return Err(format!("device {id} is revoked"));
    }
    if d.disabled.is_some() {
        return Ok(false);
    }
    d.disabled = Some(Disabled { at: now, reason });
    events.push(DeviceEvent::Disabled { subject: d.subject(), reason, by });
    Ok(true)
}

fn change_scope(d: &mut DeviceRecord, grant: Grant, by: Actor, events: &mut Vec<DeviceEvent>) -> bool {
    let from = d.grant();
    if from == grant {
        return false;
    }
    let widened = !grant.within(&from);
    let reduced = !from.within(&grant);
    d.caps = grant.capabilities;
    d.projects = grant.projects.clone();
    events.push(DeviceEvent::ScopeChanged { subject: d.subject(), by, from, to: grant, widened, reduced });
    true
}

/// Correct every device of `f` that says more than `evidence` (§5.2): a
/// revocation undone is restored, anything else disabled as `unrecorded`.
fn correct(f: &mut DevicesFile, evidence: &HashMap<DeviceId, SecurityFields>, now: i64, events: &mut Vec<DeviceEvent>) {
    for d in &mut f.devices {
        let why = match evidence.get(&d.id) {
            None => Some("no record"),
            Some(ev) => d.security().exceeds(ev),
        };
        let Some(why) = why else { continue };
        worktrees_api::log::warn(&format!("serve: {FILE}: device {} says more than its evidence ({why})", d.id));
        if let Some(at) = evidence.get(&d.id).and_then(|ev| ev.revoked_at) {
            if d.revoked_at.is_none() {
                d.revoked_at = Some(at);
                events.push(DeviceEvent::Revoked { subject: d.subject(), by: Actor::Serve });
            }
            continue;
        }
        if d.is_revoked() || d.disabled.is_some_and(|x| x.reason == DisabledReason::Unrecorded) {
            continue;
        }
        d.disabled = Some(Disabled { at: now, reason: DisabledReason::Unrecorded });
        events.push(DeviceEvent::Disabled { subject: d.subject(), reason: DisabledReason::Unrecorded, by: Actor::Serve });
    }
}

fn unreadable(why: &str) -> String {
    format!("{why} — every device is refused, and the file is left as it is until it is fixed or removed")
}

impl Registry for DeviceRegistry {
    fn principal_grant(&self, principal: &PrincipalId) -> Option<Grant> {
        let st = self.state.read().unwrap();
        st.file.as_ref().ok()?.principal(principal).map(PrincipalRecord::grant)
    }

    fn device_grant(&self, principal: &PrincipalId, device: &DeviceId) -> Option<Grant> {
        let st = self.state.read().unwrap();
        let d = st.file.as_ref().ok()?.device(device)?;
        (&d.principal == principal && d.is_live()).then(|| d.grant())
    }

    fn device_principal(&self, device: &DeviceId) -> Option<PrincipalId> {
        let st = self.state.read().unwrap();
        st.file.as_ref().ok()?.device(device).map(|d| d.principal.clone())
    }

    fn stepup_required(&self, principal: &PrincipalId, device: &DeviceId) -> bool {
        let st = self.state.read().unwrap();
        match st.file.as_ref().ok().and_then(|f| f.device(device)) {
            Some(d) if &d.principal == principal => d.stepup_required,
            _ => true,
        }
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use Capability::*;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-devices-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn pin(n: u8) -> String {
        hex(&[n; 32])
    }

    fn grant(caps: &[Capability], projects: Projects) -> Grant {
        Grant { capabilities: Caps::of(caps), projects }
    }

    fn enrollment(name: &str, key: u8, platform: Platform, g: Grant) -> Enrollment {
        Enrollment {
            principal: PrincipalId::owner(),
            name: name.into(),
            platform,
            spki_sha256: pin(key),
            stepup_spki_sha256: platform.has_presence_key().then(|| pin(key.wrapping_add(100))),
            grant: g,
            expiry: Expiry::default(),
            addr: None,
        }
    }

    fn phone(reg: &DeviceRegistry, name: &str, key: u8, caps: &[Capability]) -> DeviceRecord {
        reg.enroll(enrollment(name, key, Platform::Ios, grant(caps, Projects::only(["/r/a"]))), NOW).unwrap()
    }

    fn caller(d: &DeviceRecord) -> Caller {
        Caller::paired(d.principal.clone(), d.id.clone())
    }

    /// Every event a registry hands its hooks, in order.
    fn recorder(reg: &DeviceRegistry) -> Arc<Mutex<Vec<DeviceEvent>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        reg.subscribe(Arc::new(move |e| s.lock().unwrap().push(e.clone())));
        seen
    }

    #[test]
    fn a_missing_file_is_the_owner_and_no_devices_and_the_first_write_is_0600() {
        let d = dir("fresh");
        let reg = DeviceRegistry::open(&d);
        let f = reg.snapshot().unwrap();
        assert_eq!(f.principals.len(), 1);
        assert!(f.principals[0].id.is_owner());
        assert!(f.devices.is_empty());
        assert!(!d.join(FILE).exists(), "reading creates nothing");
        let p = phone(&reg, "Phone", 1, &[Read]);
        let mode = fs::metadata(d.join(FILE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "devices.json is {mode:o}");
        // A second registry over the same dir reads what the first wrote.
        let again = DeviceRegistry::open(&d);
        assert_eq!(again.device_grant(&PrincipalId::owner(), &p.id), Some(p.grant()));
        let _ = fs::remove_dir_all(&d);
    }

    /// §5.2 / §2.8: a file serve cannot vouch for refuses EVERY device and is
    /// left byte for byte as it was — never "repaired" by the next write.
    #[test]
    fn an_unreadable_file_refuses_every_device_and_is_never_overwritten() {
        let d = dir("corrupt");
        let reg = DeviceRegistry::open(&d);
        let p = phone(&reg, "Phone", 1, &[Read]);
        let good: Value = serde_json::from_slice(&fs::read(d.join(FILE)).unwrap()).unwrap();
        let mut bad: Vec<(&str, Vec<u8>)> = vec![("not json", b"{\"version\": 1, ".to_vec())];
        let mut edit = |why: &'static str, f: &dyn Fn(&mut Value)| {
            let mut v = good.clone();
            f(&mut v);
            bad.push((why, serde_json::to_vec(&v).unwrap()));
        };
        edit("newer version", &|v| v["version"] = json!(2));
        edit("a second principal", &|v| v["principals"].as_array_mut().unwrap().push(json!({"id": "p_bob", "name": "bob", "caps": ["read"], "projects": "*", "created_at": 1})));
        edit("an owner with less", &|v| v["principals"][0]["caps"] = json!(["read"]));
        edit("no owner", &|v| v["principals"] = json!([]));
        edit("an unknown field", &|v| v["devices"][0]["admin_override"] = json!(true));
        edit("an unknown capability", &|v| v["devices"][0]["caps"] = json!(["read", "root"]));
        edit("a device of another principal", &|v| v["devices"][0]["principal"] = json!("p_bob"));
        edit("a duplicate device", &|v| {
            let dev = v["devices"][0].clone();
            v["devices"].as_array_mut().unwrap().push(dev);
        });
        edit("a malformed pin", &|v| v["devices"][0]["spki_sha256"] = json!("ABC"));
        edit("an until that is not RFC 3339", &|v| v["devices"][0]["expiry"] = json!({"until": "2027-02-30T00:00:00Z"}));
        for (why, bytes) in bad {
            fs::write(d.join(FILE), &bytes).unwrap();
            let reg = DeviceRegistry::open(&d);
            let events = recorder(&reg);
            reg.startup(NOW);
            assert!(matches!(events.lock().unwrap().first(), Some(DeviceEvent::Unreadable { .. })), "{why}: the alert");
            assert_eq!(reg.device_grant(&PrincipalId::owner(), &p.id), None, "{why}: the device is refused");
            assert_eq!(reg.admit(&p.spki_sha256, NOW), Err(ConnectRefusal::Unreadable), "{why}");
            let e = reg.disable(&p.id, NOW).unwrap_err();
            assert!(e.contains("every device is refused"), "{why}: {e}");
            assert!(reg.enroll(enrollment("New", 9, Platform::Macos, grant(&[Read], Projects::All)), NOW).is_err(), "{why}");
            reg.flush_seen(NOW + DAY);
            assert_eq!(fs::read(d.join(FILE)).unwrap(), bytes, "{why}: the file is never overwritten");
            // The console is the owner whatever the file says.
            assert_eq!(authz::effective(&reg, &Caller::console()), Grant::everything(), "{why}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn sixteen_devices_count_the_disabled_and_not_the_revoked() {
        let reg = DeviceRegistry::in_memory();
        let all: Vec<_> = (0..16u8).map(|i| phone(&reg, &format!("D{i}"), i, &[Read])).collect();
        let over = reg.enroll(enrollment("17th", 99, Platform::Macos, grant(&[Read], Projects::All)), NOW).unwrap_err();
        assert!(over.contains("16 devices"), "{over}");
        reg.disable(&all[0].id, NOW).unwrap();
        assert!(reg.enroll(enrollment("17th", 99, Platform::Macos, grant(&[Read], Projects::All)), NOW).is_err(), "a disabled device still counts");
        reg.revoke(&Caller::console(), &Grant::everything(), &all[1].id, NOW).unwrap();
        reg.enroll(enrollment("17th", 99, Platform::Macos, grant(&[Read], Projects::All)), NOW).expect("a tombstone does not count");
    }

    #[test]
    fn one_key_is_enrolled_on_one_unrevoked_device() {
        let reg = DeviceRegistry::in_memory();
        let a = phone(&reg, "A", 1, &[Read]);
        let e = reg.enroll(enrollment("B", 1, Platform::Ios, grant(&[Read], Projects::All)), NOW).unwrap_err();
        assert!(e.contains("already enrolled"), "{e}");
        reg.revoke(&Caller::console(), &Grant::everything(), &a.id, NOW).unwrap();
        let b = reg.enroll(enrollment("B", 1, Platform::Ios, grant(&[Read], Projects::All)), NOW).unwrap();
        // The handshake finds the live record, not the tombstone.
        assert_eq!(reg.admit(&pin(1), NOW), Ok(caller(&b)));
    }

    /// §2.1, §2.5, §11 Q8: an iPhone or iPad enrolls a presence-bound key and
    /// has step-up on; a desktop has neither.
    #[test]
    fn the_platform_decides_the_step_up_key_and_its_default() {
        let reg = DeviceRegistry::in_memory();
        let ios = phone(&reg, "iPhone", 1, &[Read, Stream]);
        assert!(ios.stepup_required && ios.stepup_spki_sha256.is_some());
        assert!(reg.stepup_required(&PrincipalId::owner(), &ios.id));
        let mac = reg.enroll(enrollment("Mac", 2, Platform::Macos, grant(&[Read, Stream], Projects::All)), NOW).unwrap();
        assert!(!mac.stepup_required);
        let mut no_key = enrollment("iPad", 3, Platform::Ipados, grant(&[Read], Projects::All));
        no_key.stepup_spki_sha256 = None;
        assert!(reg.enroll(no_key, NOW).unwrap_err().contains("step-up key"));
        let mut extra = enrollment("Linux", 4, Platform::Linux, grant(&[Read], Projects::All));
        extra.stepup_spki_sha256 = Some(pin(5));
        assert!(reg.enroll(extra, NOW).is_err());
        // Off is the console's act, and on needs a key to be on.
        assert!(reg.set_stepup(&ios.id, false).unwrap());
        assert!(!reg.stepup_required(&PrincipalId::owner(), &ios.id));
        assert!(reg.set_stepup(&mac.id, true).unwrap_err().contains("no step-up key"));
        // A device the registry cannot vouch for never has step-up waived.
        assert!(reg.stepup_required(&PrincipalId::owner(), &DeviceId::new("ghost")));
    }

    #[test]
    fn a_name_cannot_pass_itself_off_as_another() {
        for ok in ["Bret's iPhone", "Mac mini (office)", "ラップトップ", &"x".repeat(64)] {
            assert!(name_ok(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            "", "   ", &"x".repeat(65), "Phone\n", "Phone\u{7}", "Phone\u{85}", "Bret\u{202E}enohP", "Bret\u{2066}x\u{2069}",
            "i\u{200B}Phone", "Phone\u{200F}", "Phone\u{2028}", "\u{FEFF}Phone", "Pho\u{00AD}ne", "\u{061C}Phone",
        ] {
            assert!(name_ok(bad).is_err(), "{bad:?}");
        }
    }

    /// §2.5: the console revokes any device; a device always revokes itself;
    /// an `admin` device revokes a peer of its principal; nothing else.
    #[test]
    fn who_may_revoke_whom() {
        let reg = DeviceRegistry::in_memory();
        let me = phone(&reg, "Me", 1, &[Read]);
        let peer = phone(&reg, "Peer", 2, &[Read]);
        let admin = phone(&reg, "Admin", 3, &[Read, Admin]);
        let g = |d: &DeviceRecord| authz::effective(&reg, &caller(d));
        let e = reg.revoke(&caller(&me), &g(&me), &peer.id, NOW).unwrap_err();
        assert!(e.contains("admin"), "{e}");
        assert!(reg.device_grant(&PrincipalId::owner(), &peer.id).is_some());
        assert!(reg.revoke(&caller(&admin), &g(&admin), &peer.id, NOW).unwrap());
        assert_eq!(reg.device_grant(&PrincipalId::owner(), &peer.id), None, "the next request is refused");
        assert!(!reg.revoke(&Caller::console(), &Grant::everything(), &peer.id, NOW).unwrap(), "idempotent");
        assert!(reg.revoke(&caller(&me), &g(&me), &me.id, NOW).unwrap(), "a device may always revoke itself");
        assert!(reg.enable(&peer.id, NOW).unwrap_err().contains("revoked"), "a tombstone is never re-enabled");
        assert_eq!(reg.admit(&peer.spki_sha256, NOW), Err(ConnectRefusal::Revoked), "the next handshake fails");
        let f = reg.snapshot().unwrap();
        assert!(f.device(&peer.id).unwrap().revoked_at.is_some(), "kept as a tombstone");
    }

    /// §2.8's fixture: a second principal, which only a test can build. Its
    /// `admin` device cannot touch the owner's devices — they read as missing
    /// — and no device's grant is set wider than its principal's.
    #[test]
    fn a_second_principal_reaches_only_its_own_devices_and_never_past_its_grant() {
        let bob = PrincipalId::new("p_bob");
        let mut f = DevicesFile::new(NOW);
        f.principals.push(PrincipalRecord { id: bob.clone(), name: "bob".into(), caps: Caps::of(&[Read, Admin]), projects: Projects::only(["/r/b"]), created_at: NOW });
        let reg = DeviceRegistry::from_file_for_test(f);
        let mine = phone(&reg, "Mine", 1, &[Read]);
        let mut e = enrollment("Bob's", 2, Platform::Macos, grant(&[Read, Admin], Projects::only(["/r/b"])));
        e.principal = bob.clone();
        let bobs = reg.enroll(e.clone(), NOW).unwrap();
        let bob_caller = caller(&bobs);
        let bob_grant = authz::effective(&reg, &bob_caller);
        assert!(bob_grant.allows(Admin));
        // Looked up under the wrong principal, a device has no grant at all.
        assert_eq!(reg.device_grant(&PrincipalId::owner(), &bobs.id), None);
        assert_eq!(authz::effective(&reg, &Caller::paired(PrincipalId::owner(), bobs.id.clone())), Grant::nothing());
        let r = reg.revoke(&bob_caller, &bob_grant, &mine.id, NOW).unwrap_err();
        assert_eq!(r, format!("no device {}", mine.id), "another principal's device reads as missing");
        let r = reg.narrow(&bob_caller, &bob_grant, &mine.id, grant(&[], Projects::only(Vec::<String>::new()))).unwrap_err();
        assert_eq!(r, format!("no device {}", mine.id));
        assert!(reg.device_grant(&PrincipalId::owner(), &mine.id).is_some(), "untouched");
        // Wider than bob: refused at enrollment and at a scope change.
        let mut wide = e.clone();
        wide.spki_sha256 = pin(3);
        wide.grant = grant(&[Read, Write], Projects::only(["/r/b"]));
        assert!(reg.enroll(wide.clone(), NOW).unwrap_err().contains("wider than its principal"));
        wide.grant = grant(&[Read], Projects::All);
        assert!(reg.enroll(wide, NOW).unwrap_err().contains("wider than its principal"));
        assert!(reg.set_scope(&bobs.id, grant(&[Read, Stream], Projects::only(["/r/b"]))).unwrap_err().contains("wider than its principal"));
        // The owner's console reaches only the owner's devices, by principal.
        assert_eq!(reg.revoke(&Caller::console(), &Grant::everything(), &bobs.id, NOW).unwrap_err(), format!("no device {}", bobs.id));
        assert_eq!(reg.list(&PrincipalId::owner()).unwrap().len(), 1, "a listing is the caller's principal's");
    }

    /// §3.2: an `admin` device takes away from a peer, never adds, never
    /// changes itself; only the console widens.
    #[test]
    fn only_the_console_widens_and_an_admin_device_only_narrows_a_peer() {
        let reg = DeviceRegistry::in_memory();
        let events = recorder(&reg);
        let admin = phone(&reg, "Admin", 1, &[Read, Write, Admin]);
        let peer = reg.enroll(enrollment("Peer", 2, Platform::Macos, grant(&[Read, Write], Projects::only(["/r/a", "/r/b"]))), NOW).unwrap();
        let ag = authz::effective(&reg, &caller(&admin));
        let wider = grant(&[Read, Write, Stream], Projects::only(["/r/a", "/r/b"]));
        assert!(reg.narrow(&caller(&admin), &ag, &peer.id, wider.clone()).unwrap_err().contains("widening is the host's"));
        assert!(reg.narrow(&caller(&admin), &ag, &peer.id, grant(&[Read, Write], Projects::All)).is_err(), "a list to \"*\" is a widening");
        assert!(reg.narrow(&caller(&admin), &ag, &admin.id, grant(&[Read], Projects::only(["/r/a"]))).unwrap_err().contains("own scope"));
        events.lock().unwrap().clear();
        assert!(reg.narrow(&caller(&admin), &ag, &peer.id, grant(&[Read], Projects::only(["/r/a"]))).unwrap());
        assert_eq!(reg.device_grant(&PrincipalId::owner(), &peer.id), Some(grant(&[Read], Projects::only(["/r/a"]))));
        match &events.lock().unwrap()[..] {
            [DeviceEvent::ScopeChanged { widened: false, reduced: true, by: Actor::Device { .. }, .. }] => {}
            other => panic!("{other:?}"),
        }
        events.lock().unwrap().clear();
        assert!(reg.set_scope(&peer.id, wider.clone()).unwrap());
        match &events.lock().unwrap()[..] {
            [DeviceEvent::ScopeChanged { widened: true, reduced: false, by: Actor::Console, .. }] => {}
            other => panic!("{other:?}"),
        }
        assert!(!reg.set_scope(&peer.id, wider).unwrap(), "no change, no event");
    }

    /// §2.5: idle days count from the last completed handshake, the
    /// enrollment or a re-enable; `until` is a date; the expiry is decided
    /// ONCE and recorded, so a clock set back re-enables nothing.
    #[test]
    fn an_expiry_is_decided_once_and_recorded() {
        let reg = DeviceRegistry::in_memory();
        let events = recorder(&reg);
        let idle = phone(&reg, "Idle", 1, &[Read]);
        let dated = reg.enroll(Enrollment { expiry: Expiry::Until(NOW + 5 * DAY), ..enrollment("Dated", 2, Platform::Macos, grant(&[Read], Projects::All)) }, NOW).unwrap();
        let never = reg.enroll(Enrollment { expiry: Expiry::Never, ..enrollment("Never", 3, Platform::Macos, grant(&[Read], Projects::All)) }, NOW).unwrap();
        assert_eq!(reg.next_deadline(), Some(NOW + 5 * DAY));
        reg.seen(&idle.id, Some("100.64.0.7".into()), NOW + 10 * DAY);
        assert_eq!(reg.check_expiry(NOW + 5 * DAY - 1).unwrap(), Vec::<DeviceId>::new());
        assert_eq!(reg.check_expiry(NOW + 5 * DAY).unwrap(), vec![dated.id.clone()]);
        assert_eq!(reg.next_deadline(), Some(NOW + 40 * DAY), "30 days from the handshake at day 10");
        assert!(reg.check_expiry(NOW + 40 * DAY - 1).unwrap().is_empty());
        assert_eq!(reg.check_expiry(NOW + 40 * DAY).unwrap(), vec![idle.id.clone()]);
        assert!(reg.check_expiry(NOW + 9000 * DAY).unwrap().is_empty(), "never is never");
        let f = reg.snapshot().unwrap();
        assert_eq!(f.device(&idle.id).unwrap().disabled, Some(Disabled { at: NOW + 40 * DAY, reason: DisabledReason::Idle }));
        assert_eq!(f.device(&dated.id).unwrap().disabled.unwrap().reason, DisabledReason::Until);
        assert!(f.device(&never.id).unwrap().disabled.is_none());
        assert!(events.lock().unwrap().iter().filter(|e| matches!(e, DeviceEvent::Disabled { by: Actor::Serve, .. })).all(DeviceEvent::alerts));
        // Back in time: still disabled — the record decides, not the clock.
        assert!(reg.check_expiry(NOW).unwrap().is_empty());
        assert_eq!(reg.device_grant(&PrincipalId::owner(), &idle.id), None);
        assert_eq!(reg.admit(&idle.spki_sha256, NOW), Err(ConnectRefusal::Disabled));
        // Re-enabling restarts the idle clock from that moment.
        assert!(reg.enable(&idle.id, NOW + 50 * DAY).unwrap());
        assert_eq!(reg.next_deadline(), Some(NOW + 80 * DAY));
        assert!(reg.enable(&dated.id, NOW + 50 * DAY).unwrap_err().contains("set a new expiry"));
        reg.set_expiry(&dated.id, Expiry::IdleDays(7)).unwrap();
        assert!(reg.enable(&dated.id, NOW + 50 * DAY).unwrap());
    }

    /// The handshake check: a known, live key is the device's caller; every
    /// other key is refused, and a known one's refusal is an event (§5.3).
    #[test]
    fn admit_answers_the_handshake_from_the_live_set() {
        let reg = DeviceRegistry::in_memory();
        let events = recorder(&reg);
        let p = phone(&reg, "Phone", 1, &[Read]);
        assert_eq!(reg.admit(&p.spki_sha256, NOW), Ok(caller(&p)));
        assert_eq!(reg.admit(&pin(77), NOW), Err(ConnectRefusal::Unknown));
        // Past its idle window with no tick in between: disabled AT the knock.
        assert_eq!(reg.admit(&p.spki_sha256, NOW + 31 * DAY), Err(ConnectRefusal::Expired));
        assert_eq!(reg.snapshot().unwrap().device(&p.id).unwrap().disabled.unwrap().reason, DisabledReason::Idle);
        assert_eq!(reg.admit(&p.spki_sha256, NOW + 31 * DAY), Err(ConnectRefusal::Disabled));
        let ev = events.lock().unwrap().clone();
        let refusals: Vec<_> = ev.iter().filter_map(|e| match e {
            DeviceEvent::ConnectRefused { why, subject, .. } => Some((*why, subject.is_some())),
            _ => None,
        }).collect();
        assert_eq!(refusals, vec![(ConnectRefusal::Unknown, false), (ConnectRefusal::Expired, true), (ConnectRefusal::Disabled, true)]);
        assert!(ev.iter().any(|e| matches!(e, DeviceEvent::Disabled { reason: DisabledReason::Idle, .. })));
    }

    /// §2.5: `last_seen` lives in memory and reaches the file at most every
    /// ten minutes, and at exit.
    #[test]
    fn last_seen_is_written_every_ten_minutes_and_at_exit() {
        let d = dir("seen");
        let reg = DeviceRegistry::open(&d);
        let p = phone(&reg, "Phone", 1, &[Read]);
        let on_disk = || DevicesFile::parse(&fs::read(d.join(FILE)).unwrap()).unwrap().device(&p.id).unwrap().last_seen;
        reg.flush_seen(NOW);
        reg.seen(&p.id, Some("192.168.1.5".into()), NOW + 60);
        assert_eq!(on_disk(), None, "not yet");
        assert_eq!(reg.get(&PrincipalId::owner(), &p.id).unwrap().last_seen, Some(NOW + 60), "but serve knows");
        reg.seen(&p.id, Some("192.168.1.5".into()), NOW + SEEN_FLUSH_SECS);
        assert_eq!(on_disk(), Some(NOW + SEEN_FLUSH_SECS), "ten minutes on");
        reg.seen(&p.id, None, NOW + SEEN_FLUSH_SECS + 5);
        reg.flush_seen(NOW + SEEN_FLUSH_SECS + 6);
        assert_eq!(on_disk(), Some(NOW + SEEN_FLUSH_SECS + 5), "at exit");
        let _ = fs::remove_dir_all(&d);
    }

    /// §5.2, while serve runs: a hand edit of `devices.json` that says MORE
    /// than serve wrote is corrected — a new or widened device disabled as
    /// `unrecorded`, a tombstone brought back revoked again — and a narrowing
    /// is simply taken.
    #[test]
    fn a_hand_edit_that_says_more_is_disabled_as_unrecorded() {
        let d = dir("edit");
        let reg = DeviceRegistry::open(&d);
        let events = recorder(&reg);
        let a = phone(&reg, "A", 1, &[Read]);
        let b = phone(&reg, "B", 2, &[Read, Write]);
        let gone = phone(&reg, "Gone", 3, &[Read]);
        reg.revoke(&Caller::console(), &Grant::everything(), &gone.id, NOW).unwrap();
        let mut v: Value = serde_json::from_slice(&fs::read(d.join(FILE)).unwrap()).unwrap();
        let idx = |v: &Value, id: &DeviceId| v["devices"].as_array().unwrap().iter().position(|x| x["id"] == json!(id.as_str())).unwrap();
        let (ia, ib, ig) = (idx(&v, &a.id), idx(&v, &b.id), idx(&v, &gone.id));
        v["devices"][ia]["caps"] = json!(["read", "stream"]);
        v["devices"][ib]["caps"] = json!(["read"]);
        v["devices"][ig]["revoked_at"] = Value::Null;
        let mut forged = v["devices"][ia].clone();
        forged["id"] = json!("d_forged");
        forged["spki_sha256"] = json!(pin(50));
        forged["caps"] = json!(["read"]);
        v["devices"].as_array_mut().unwrap().push(forged);
        fs::write(d.join(FILE), serde_json::to_vec(&v).unwrap()).unwrap();
        events.lock().unwrap().clear();
        reg.refresh();
        let f = reg.snapshot().unwrap();
        let reason = |id: &str| f.device(&DeviceId::new(id)).unwrap().disabled.map(|x| x.reason);
        assert_eq!(reason(a.id.as_str()), Some(DisabledReason::Unrecorded), "widened");
        assert_eq!(reason("d_forged"), Some(DisabledReason::Unrecorded), "no record behind it");
        assert_eq!(reason(b.id.as_str()), None, "a narrowing is taken");
        assert_eq!(reg.device_grant(&PrincipalId::owner(), &b.id), Some(grant(&[Read], Projects::only(["/r/a"]))));
        assert!(f.device(&gone.id).unwrap().revoked_at.is_some(), "revoked again");
        // And the corrections are on disk, so the next reader sees them too.
        let disk = DevicesFile::parse(&fs::read(d.join(FILE)).unwrap()).unwrap();
        assert_eq!(disk.device(&a.id).unwrap().disabled.map(|x| x.reason), Some(DisabledReason::Unrecorded));
        let ev = events.lock().unwrap().clone();
        assert!(matches!(ev.first(), Some(DeviceEvent::Reloaded)), "{ev:?}");
        assert_eq!(ev.iter().filter_map(DeviceEvent::ends).count(), 3, "{ev:?}");
        let _ = fs::remove_dir_all(&d);
    }

    /// At boot serve has no evidence of its own about a file it finds, so it
    /// takes it as it stands until the audit chain's cross-check runs (§5.2);
    /// that check then corrects anything its chain does not vouch for.
    #[test]
    fn boot_takes_the_file_as_it_stands_until_the_audit_cross_check() {
        let d = dir("boot");
        let a = phone(&DeviceRegistry::open(&d), "A", 1, &[Read]);
        let b = phone(&DeviceRegistry::open(&d), "B", 2, &[Read, Write]);
        let reg = DeviceRegistry::open(&d);
        assert!(reg.device_grant(&PrincipalId::owner(), &a.id).is_some());
        assert!(reg.device_grant(&PrincipalId::owner(), &b.id).is_some());
        let mut chain = HashMap::new();
        chain.insert(a.id.clone(), a.security());
        let mut narrower = b.security();
        narrower.caps = Caps::of(&[Read]);
        chain.insert(b.id.clone(), narrower);
        let mut fixed = reg.cross_check(&chain, NOW).unwrap();
        fixed.sort();
        assert_eq!(fixed, vec![b.id.clone()], "b says more than its chain");
        let c = phone(&reg, "C", 3, &[Read]);
        assert!(reg.cross_check(&chain, NOW).unwrap() == vec![c.id.clone()], "no enrolled record behind c");
        assert!(reg.device_grant(&PrincipalId::owner(), &a.id).is_some());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_generation_moves_on_a_change_and_not_on_a_no_op() {
        let reg = DeviceRegistry::in_memory();
        let g0 = reg.generation();
        let p = phone(&reg, "P", 1, &[Read]);
        let g1 = reg.generation();
        assert!(g1 > g0);
        assert!(!reg.disable(&p.id, NOW).unwrap() || reg.generation() > g1);
        let g2 = reg.generation();
        assert!(!reg.disable(&p.id, NOW).unwrap());
        assert_eq!(reg.generation(), g2, "a no-op changes nothing");
        let _ = reg.device_grant(&PrincipalId::owner(), &p.id);
        assert_eq!(reg.generation(), g2, "a read changes nothing");
    }

    #[test]
    fn an_expiry_has_three_spellings_and_no_others() {
        assert_eq!(serde_json::to_value(Expiry::default()).unwrap(), json!({"idle_days": 30}));
        assert_eq!(serde_json::to_value(Expiry::Never).unwrap(), json!("never"));
        assert_eq!(serde_json::to_value(Expiry::Until(1_800_000_000)).unwrap(), json!({"until": "2027-01-15T08:00:00Z"}));
        for good in [json!({"idle_days": 1}), json!({"idle_days": 3650}), json!("never"), json!({"until": "2024-02-29T23:59:59Z"})] {
            let e = Expiry::from_value(&good).unwrap();
            assert_eq!(serde_json::to_value(e).unwrap(), good);
        }
        for bad in [
            json!({"idle_days": 0}), json!({"idle_days": 3651}), json!({"idle_days": -1}), json!({"idle_days": 1.5}), json!("Never"),
            json!({"until": "2027-01-15T08:00:00"}), json!({"until": "2027-01-15T08:00:00+00:00"}), json!({"until": "2027-01-15T08:00:00.5Z"}),
            json!({"until": "2023-02-29T00:00:00Z"}), json!({"until": "2027-1-15T08:00:00Z"}), json!({"idle_days": 3, "until": "2027-01-15T08:00:00Z"}),
            json!({}), json!(30), json!(null),
        ] {
            assert!(Expiry::from_value(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn base32_is_rfc_4648_lowercase_unpadded() {
        // RFC 4648 §10's vectors, lowercased and unpadded.
        for (i, o) in [("", ""), ("f", "my"), ("fo", "mzxq"), ("foo", "mzxw6"), ("foob", "mzxw6yq"), ("fooba", "mzxw6ytb"), ("foobar", "mzxw6ytboi")] {
            assert_eq!(base32(i.as_bytes()), o);
        }
        let id = new_device_id();
        assert!(id.as_str().starts_with("d_") && id.as_str().len() == 28, "{id}");
        assert_ne!(new_device_id(), id);
    }
}

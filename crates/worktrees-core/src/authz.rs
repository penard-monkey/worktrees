//! Who is asking, and what they may do: the types every grant check keys on.
//!
//! Server Edition Phase 2 (`docs/proposals/server-edition-phase2.md` §2.8,
//! §3.1–§3.3). A request reaches serve from a CALLER, which is a principal (a
//! person) on a device. What the caller may do is the principal's grant
//! INTERSECTED with the device's, evaluated on every request and never
//! stored: a capability and a project scope each side. Phase 2 has one
//! principal, the owner, and one device the host knows without a registry,
//! the console (whoever holds serve.json's token on the loopback listener).
//! Paired devices arrive with the device registry (slice 5), which implements
//! [`Registry`]; until then [`NoDevices`] answers "unknown" for every device,
//! which refuses everything.
//!
//! The types are here, in core, and dependency-free on purpose: the path
//! classifier (`pathclass`) maps a path class to the [`Capability`] it needs,
//! serve's dispatcher resolves objects against a [`Grant`], and the audit log
//! and the resource limits key on [`Caller`] — three crates, one vocabulary.
//! Keep the shapes stable; later slices build on them.
//!
//! Nothing in this module reads a file or the clock.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ── capabilities ─────────────────────────────────────────────────────────────

/// What a grant can allow. A closed enum (§3.1), so a new capability is a
/// compile error everywhere a `match` decides on one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    /// Engine reads inside the scope.
    Read,
    /// Mutations of declared state and places inside the scope; ordinary
    /// files; agent launch under host policy.
    Write,
    /// Answering a prompt an agent raised, with one of its own choices. The
    /// verb is Phase 3; the capability exists now so device records need no
    /// migration.
    Answer,
    /// Text that becomes an agent's instructions (R7, §7).
    Instruct,
    /// Terminal attaches and dock shells: everything a shell can do.
    Stream,
    /// Host-wide configuration that cannot widen agent autonomy.
    Admin,
}

impl Capability {
    pub const ALL: [Capability; 6] =
        [Capability::Read, Capability::Write, Capability::Answer, Capability::Instruct, Capability::Stream, Capability::Admin];

    /// The wire and file spelling: `read`, `write`, …
    pub fn name(self) -> &'static str {
        match self {
            Capability::Read => "read",
            Capability::Write => "write",
            Capability::Answer => "answer",
            Capability::Instruct => "instruct",
            Capability::Stream => "stream",
            Capability::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Capability> {
        Capability::ALL.into_iter().find(|c| c.name() == s)
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of capabilities. Serialized as a list of names, in [`Capability::ALL`]
/// order; an unknown name in a file is an error, not a silent drop, so a
/// grant written by a newer build is refused rather than narrowed by accident
/// into something nobody chose.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Caps(u8);

impl Caps {
    pub const NONE: Caps = Caps(0);
    pub const ALL: Caps = Caps(0b11_1111);

    pub fn of(caps: &[Capability]) -> Caps {
        Caps(caps.iter().fold(0, |b, c| b | c.bit()))
    }

    pub fn has(self, c: Capability) -> bool {
        self.0 & c.bit() != 0
    }

    pub fn intersect(self, other: Caps) -> Caps {
        Caps(self.0 & other.0)
    }

    /// Every capability here is also in `outer`.
    pub fn within(self, outer: Caps) -> bool {
        self.0 & !outer.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Capability> {
        Capability::ALL.into_iter().filter(move |c| self.has(*c))
    }
}

impl fmt::Debug for Caps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter().map(Capability::name)).finish()
    }
}

impl Serialize for Caps {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for Caps {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Caps::of(&Vec::<Capability>::deserialize(d)?))
    }
}

/// Reading or writing an object. A path's class (§7.1) needs a different
/// capability for each, so the classifier is asked with one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Read,
    Write,
}

/// What an object needs ON TOP OF its row's capability — the path classifier's
/// answer for one class and one [`Access`] (§7.1: "the class can RAISE the
/// capability the row needs"). Both must hold: a `write_file` to `AGENTS.md`
/// needs the row's `write` and the class's `instruct`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Requirement {
    /// Nothing beyond the row's capability (a plain path).
    Row,
    /// This capability as well.
    Cap(Capability),
    /// The console only: no paired device, whatever it holds.
    Console,
    /// Nobody, the console included (`.git/**` for writing).
    Never,
}

impl Requirement {
    /// A caller holding `caps` meets this, on top of the row's capability.
    /// `console`: the caller is the console, which alone meets
    /// [`Requirement::Console`].
    pub fn met_by(self, caps: Caps, console: bool) -> bool {
        match self {
            Requirement::Row => true,
            Requirement::Cap(c) => caps.has(c),
            Requirement::Console => console,
            Requirement::Never => false,
        }
    }
}

// ── scope ────────────────────────────────────────────────────────────────────

/// Which projects a grant reaches (§3.2): every registered project, including
/// ones added later, or a list of canonical project roots in the registry's
/// own keys. Projects, not places: places are short-lived and their slugs get
/// reused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Projects {
    /// `"*"`.
    All,
    /// Canonical roots, compared exactly with the registry's keys.
    Only(BTreeSet<String>),
}

impl Projects {
    pub fn only<I: IntoIterator<Item = S>, S: Into<String>>(roots: I) -> Projects {
        Projects::Only(roots.into_iter().map(Into::into).collect())
    }

    pub fn is_all(&self) -> bool {
        matches!(self, Projects::All)
    }

    /// `"*"` ∩ X is X; a list ∩ a list is the roots in both. Never stored:
    /// a device whose projects are `"*"` under a principal with a list has
    /// that list, and a project added later does not reach it through the
    /// device's `"*"` (§2.8).
    pub fn intersect(&self, other: &Projects) -> Projects {
        match (self, other) {
            (Projects::All, x) | (x, Projects::All) => x.clone(),
            (Projects::Only(a), Projects::Only(b)) => Projects::Only(a.intersection(b).cloned().collect()),
        }
    }

    /// The registry key `root` is in scope. Exact: `/r/a` does not cover
    /// `/r/ab`, and a path INSIDE a project is a question for
    /// [`Projects::covers_path`].
    pub fn covers_root(&self, root: &str) -> bool {
        match self {
            Projects::All => true,
            Projects::Only(set) => set.contains(root),
        }
    }

    /// `path` (canonical) is a project root in scope or lies under one. Lexical,
    /// component-wise: `/r/a/x` is under `/r/a`, `/r/ab` is not. A caller
    /// holding a non-canonical path canonicalizes first; this does no I/O.
    pub fn covers_path(&self, path: &str) -> bool {
        match self {
            Projects::All => true,
            Projects::Only(set) => set.iter().any(|r| std::path::Path::new(path).starts_with(r)),
        }
    }

    /// Every root here is also reachable through `outer`.
    pub fn within(&self, outer: &Projects) -> bool {
        match (self, outer) {
            (_, Projects::All) => true,
            (Projects::All, Projects::Only(_)) => false,
            (Projects::Only(a), Projects::Only(b)) => a.is_subset(b),
        }
    }
}

impl Serialize for Projects {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Projects::All => s.serialize_str("*"),
            Projects::Only(set) => s.collect_seq(set),
        }
    }
}

impl<'de> Deserialize<'de> for Projects {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Star(String),
            List(Vec<String>),
        }
        match Raw::deserialize(d)? {
            Raw::Star(s) if s == "*" => Ok(Projects::All),
            Raw::Star(s) => Err(serde::de::Error::custom(format!("projects must be \"*\" or a list of roots, not {s:?}"))),
            Raw::List(v) => Ok(Projects::only(v)),
        }
    }
}

// ── grants ───────────────────────────────────────────────────────────────────

/// Capabilities and a project scope: what one side of the (principal,
/// device) pair allows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub capabilities: Caps,
    pub projects: Projects,
}

impl Grant {
    /// Every capability, scope `"*"`: the owner's grant, and the console's.
    pub fn everything() -> Grant {
        Grant { capabilities: Caps::ALL, projects: Projects::All }
    }

    /// No capability and no project: what an unknown principal or device gets.
    pub fn nothing() -> Grant {
        Grant { capabilities: Caps::NONE, projects: Projects::Only(BTreeSet::new()) }
    }

    pub fn intersect(&self, other: &Grant) -> Grant {
        Grant {
            capabilities: self.capabilities.intersect(other.capabilities),
            projects: self.projects.intersect(&other.projects),
        }
    }

    pub fn allows(&self, c: Capability) -> bool {
        self.capabilities.has(c)
    }

    /// No wider than `outer` on either axis.
    pub fn within(&self, outer: &Grant) -> bool {
        self.capabilities.within(outer.capabilities) && self.projects.within(&outer.projects)
    }
}

/// The rule a device-record writer applies (§2.8, §3.2): a device's grant may
/// never be set wider than its principal's. The per-request intersection
/// enforces it anyway; this is the refusal at the moment of writing, so a
/// record never SAYS more than it can do.
pub fn check_device_grant(principal: &Grant, device: &Grant) -> Result<(), String> {
    if device.within(principal) {
        return Ok(());
    }
    Err("a device's grant cannot be wider than its principal's".to_string())
}

// ── identities ───────────────────────────────────────────────────────────────

/// A person. Phase 2 has exactly one, the owner (the host account's user),
/// whose id is the reserved constant [`PrincipalId::OWNER`] and is never
/// looked up — a corrupt or deleted device file cannot lock the console out
/// or split the audit trail under a new id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrincipalId(String);

impl PrincipalId {
    pub const OWNER: &'static str = "owner";

    pub fn owner() -> PrincipalId {
        PrincipalId(Self::OWNER.to_string())
    }

    /// Any other principal. Phase 2 creates none; the device registry's loader
    /// treats one in its file as corruption (§2.8), and tests build one to
    /// prove the rules hold for it.
    pub fn new(id: impl Into<String>) -> PrincipalId {
        PrincipalId(id.into())
    }

    pub fn is_owner(&self) -> bool {
        self.0 == Self::OWNER
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrincipalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A paired device's id, as the device registry assigns it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn new(id: impl Into<String>) -> DeviceId {
        DeviceId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which device a request came through.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "id")]
pub enum Device {
    /// The host console: a loopback client that passed handshake v1 with
    /// serve.json's token — the desktop app, the CLI, a dev stack.
    Console,
    /// A device that completed the remote handshake with its enrolled key.
    Paired(DeviceId),
}

/// Which listener accepted the socket. Derived from the device, never stored
/// beside it: the console is exactly the loopback listener's caller, and a
/// paired device is exactly the remote listener's (§3.3). Resource limits
/// key their budgets on it (§6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Listener {
    Loopback,
    Remote,
}

/// Who a request is from: a principal on a device. Set by the LISTENER that
/// accepted the socket and fixed for the connection's life — never derived
/// from a header, an `Origin` or a peer address (§3.3). The fields are private
/// so the only way to a `Console` caller is [`Caller::console`], which is
/// always the owner.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Caller {
    principal: PrincipalId,
    device: Device,
}

impl Caller {
    /// The loopback listener's caller: the owner, on the console.
    pub fn console() -> Caller {
        Caller { principal: PrincipalId::owner(), device: Device::Console }
    }

    /// The remote listener's caller: the device whose key completed the
    /// handshake, with the principal its record names.
    pub fn paired(principal: PrincipalId, device: DeviceId) -> Caller {
        Caller { principal, device: Device::Paired(device) }
    }

    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn is_console(&self) -> bool {
        self.device == Device::Console
    }

    pub fn listener(&self) -> Listener {
        match self.device {
            Device::Console => Listener::Loopback,
            Device::Paired(_) => Listener::Remote,
        }
    }
}

impl fmt::Display for Caller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.device {
            Device::Console => write!(f, "{}@console", self.principal),
            Device::Paired(d) => write!(f, "{}@{}", self.principal, d),
        }
    }
}

// ── the registry seam ────────────────────────────────────────────────────────

/// Where principals' and devices' grants come from. Slice 5's device registry
/// implements it over `devices.json`; tests implement it with fixtures. Read
/// on EVERY request (§3.3: a narrowed scope or a revoke applies to the very
/// next request on a socket already open), so an implementation keeps the
/// lookup cheap.
///
/// The owner is never asked about: its grant is [`Grant::everything`] by
/// definition. A `None` is "unknown, revoked, disabled or expired" and grants
/// nothing.
pub trait Registry: Send + Sync {
    /// A non-owner principal's grant.
    fn principal_grant(&self, principal: &PrincipalId) -> Option<Grant>;
    /// A device's own grant, looked up WITHIN `principal`: a device of another
    /// principal is `None`, the same as one that does not exist.
    fn device_grant(&self, principal: &PrincipalId, device: &DeviceId) -> Option<Grant>;
    /// The principal a device belongs to, whatever its state. What a
    /// `device(id)` object resolves through: a device of another principal is
    /// answered exactly as a missing one is (§3.3).
    fn device_principal(&self, device: &DeviceId) -> Option<PrincipalId>;
    /// Whether the device's drive-class acts (`stream`, `instruct`, `answer`,
    /// `admin`) need a fresh step-up proof on the connection (§2.5, §11 Q8).
    /// A device the registry cannot vouch for answers `true`: no grant
    /// reaches it anyway, and not knowing must never waive a factor.
    fn stepup_required(&self, principal: &PrincipalId, device: &DeviceId) -> bool;
    /// Moves on every change to any record (§3.3). A connection compares it
    /// with the value it last reconciled its channels at, so "nothing changed"
    /// costs one atomic load.
    fn generation(&self) -> u64;
    /// A device's display name as it stands NOW, looked up WITHIN `principal`
    /// as its grant is, whatever its state: a revoked device keeps the name
    /// it was revoked under, so what it did is still named. What serve names
    /// the device in a place's provenance stamps with (`scope::stamps`).
    /// `None` by default: a registry with no names names nothing.
    fn device_name(&self, _principal: &PrincipalId, _device: &DeviceId) -> Option<String> {
        None
    }
}

/// Phase 2 slice 1's registry: no paired devices exist, so every device and
/// every non-owner principal is unknown. The console is unaffected — it is
/// never looked up.
pub struct NoDevices;

impl Registry for NoDevices {
    fn principal_grant(&self, _: &PrincipalId) -> Option<Grant> {
        None
    }
    fn device_grant(&self, _: &PrincipalId, _: &DeviceId) -> Option<Grant> {
        None
    }
    fn device_principal(&self, _: &DeviceId) -> Option<PrincipalId> {
        None
    }
    fn stepup_required(&self, _: &PrincipalId, _: &DeviceId) -> bool {
        true
    }
    fn generation(&self) -> u64 {
        0
    }
}

/// What `caller` may do right now: the principal's grant intersected with the
/// device's (§2.8). The console's device grant is everything, so the owner on
/// the console has everything; an unknown principal or device has nothing.
pub fn effective(reg: &dyn Registry, caller: &Caller) -> Grant {
    let principal = if caller.principal.is_owner() {
        Grant::everything()
    } else {
        reg.principal_grant(&caller.principal).unwrap_or_else(Grant::nothing)
    };
    let device = match &caller.device {
        Device::Console => Grant::everything(),
        Device::Paired(id) => reg.device_grant(&caller.principal, id).unwrap_or_else(Grant::nothing),
    };
    principal.intersect(&device)
}

/// Whether `caller` may name `device` as an object (`device(id)`): only a
/// device of the caller's own principal (§2.8, §3.3).
pub fn owns_device(reg: &dyn Registry, caller: &Caller, device: &DeviceId) -> bool {
    reg.device_principal(device).is_some_and(|p| &p == caller.principal())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A registry built in the test: principals and devices with grants, each
    /// device under one principal.
    #[derive(Default)]
    struct Fixture {
        principals: HashMap<PrincipalId, Grant>,
        devices: HashMap<DeviceId, (PrincipalId, Grant)>,
    }

    impl Registry for Fixture {
        fn principal_grant(&self, p: &PrincipalId) -> Option<Grant> {
            self.principals.get(p).cloned()
        }
        fn device_grant(&self, p: &PrincipalId, d: &DeviceId) -> Option<Grant> {
            self.devices.get(d).filter(|(owner, _)| owner == p).map(|(_, g)| g.clone())
        }
        fn device_principal(&self, d: &DeviceId) -> Option<PrincipalId> {
            self.devices.get(d).map(|(p, _)| p.clone())
        }
        fn stepup_required(&self, _: &PrincipalId, _: &DeviceId) -> bool {
            false
        }
        fn generation(&self) -> u64 {
            0
        }
    }

    fn grant(caps: &[Capability], projects: Projects) -> Grant {
        Grant { capabilities: Caps::of(caps), projects }
    }

    use Capability::*;

    #[test]
    fn the_console_is_the_owner_with_everything() {
        let c = Caller::console();
        assert!(c.principal().is_owner());
        assert!(c.is_console());
        assert_eq!(c.listener(), Listener::Loopback);
        assert_eq!(effective(&NoDevices, &c), Grant::everything());
    }

    #[test]
    fn an_unknown_device_gets_nothing() {
        let c = Caller::paired(PrincipalId::owner(), DeviceId::new("ghost"));
        assert_eq!(c.listener(), Listener::Remote);
        let g = effective(&NoDevices, &c);
        assert!(Capability::ALL.iter().all(|cap| !g.allows(*cap)), "{g:?}");
        assert!(!g.projects.covers_root("/r"));
    }

    /// The owner's grant is everything, so the owner's device has exactly its
    /// own grant: nothing behaves differently with one principal.
    #[test]
    fn an_owner_device_has_its_own_grant() {
        let mut reg = Fixture::default();
        let d = grant(&[Read], Projects::only(["/r/a"]));
        reg.devices.insert(DeviceId::new("phone"), (PrincipalId::owner(), d.clone()));
        assert_eq!(effective(&reg, &Caller::paired(PrincipalId::owner(), DeviceId::new("phone"))), d);
    }

    /// §2.8's first second-principal fixture: a device grant WIDER than its
    /// principal's (more capabilities, `"*"` against a list) is cut down to
    /// the principal's on every request, and refused when written.
    #[test]
    fn a_second_principals_device_never_exceeds_the_principal() {
        let bob = PrincipalId::new("bob");
        let mut reg = Fixture::default();
        let bobs = grant(&[Read], Projects::only(["/r/a"]));
        let wide = grant(&[Read, Write, Stream], Projects::All);
        reg.principals.insert(bob.clone(), bobs.clone());
        reg.devices.insert(DeviceId::new("bob-phone"), (bob.clone(), wide.clone()));
        let g = effective(&reg, &Caller::paired(bob.clone(), DeviceId::new("bob-phone")));
        assert_eq!(g, bobs, "the pair's grant is the intersection");
        assert!(!g.allows(Write) && !g.allows(Stream));
        assert!(g.projects.covers_root("/r/a") && !g.projects.covers_root("/r/b"));
        assert!(!g.projects.is_all(), "a device's \"*\" under a listed principal is that list, not \"*\"");
        assert!(check_device_grant(&bobs, &wide).is_err(), "a wider device grant is refused at write time");
        assert!(check_device_grant(&bobs, &grant(&[Read], Projects::only(["/r/a"]))).is_ok());
        assert!(check_device_grant(&bobs, &grant(&[Read], Projects::only(["/r/a", "/r/b"]))).is_err());
    }

    /// §2.8's second fixture, at the type level: a device is an object only to
    /// its own principal. Another principal's device answers as a missing one,
    /// and so does a device-grant lookup made under the wrong principal.
    #[test]
    fn a_device_of_another_principal_is_not_an_object_for_the_caller() {
        let bob = PrincipalId::new("bob");
        let mut reg = Fixture::default();
        reg.principals.insert(bob.clone(), Grant::everything());
        reg.devices.insert(DeviceId::new("bob-phone"), (bob.clone(), Grant::everything()));
        reg.devices.insert(DeviceId::new("my-phone"), (PrincipalId::owner(), Grant::everything()));
        let me = Caller::paired(PrincipalId::owner(), DeviceId::new("my-phone"));
        assert!(owns_device(&reg, &me, &DeviceId::new("my-phone")));
        assert!(!owns_device(&reg, &me, &DeviceId::new("bob-phone")));
        assert!(!owns_device(&reg, &me, &DeviceId::new("nobody")));
        assert!(!owns_device(&reg, &Caller::console(), &DeviceId::new("bob-phone")), "not even the console");
        // A caller claiming bob's device under the owner gets nothing.
        let forged = Caller::paired(PrincipalId::owner(), DeviceId::new("bob-phone"));
        assert_eq!(effective(&reg, &forged), Grant::nothing());
    }

    #[test]
    fn projects_intersect_and_cover_as_section_2_8_says() {
        let a = Projects::only(["/r/a", "/r/b"]);
        let b = Projects::only(["/r/b", "/r/c"]);
        assert_eq!(Projects::All.intersect(&a), a);
        assert_eq!(a.intersect(&Projects::All), a);
        assert_eq!(a.intersect(&b), Projects::only(["/r/b"]));
        assert!(Projects::All.intersect(&Projects::All).is_all());
        assert!(a.covers_path("/r/a") && a.covers_path("/r/a/.worktrees/x"));
        assert!(!a.covers_path("/r/ab") && !a.covers_path("/r") && !a.covers_root("/r/a/x"));
        assert!(a.within(&Projects::All) && !Projects::All.within(&a));
        assert!(Projects::only(["/r/b"]).within(&a) && !b.within(&a));
    }

    #[test]
    fn grants_round_trip_through_json_as_the_device_file_will_hold_them() {
        let g = grant(&[Write, Read], Projects::only(["/r/a"]));
        let v = serde_json::to_value(&g).unwrap();
        assert_eq!(v, serde_json::json!({"capabilities": ["read", "write"], "projects": ["/r/a"]}));
        assert_eq!(serde_json::from_value::<Grant>(v).unwrap(), g);
        let all: Grant = serde_json::from_value(serde_json::json!({"capabilities": ["read"], "projects": "*"})).unwrap();
        assert!(all.projects.is_all());
        assert!(serde_json::from_value::<Grant>(serde_json::json!({"capabilities": ["root"], "projects": "*"})).is_err());
        assert!(serde_json::from_value::<Grant>(serde_json::json!({"capabilities": [], "projects": "all"})).is_err());
        assert_eq!(serde_json::to_value(Caller::console()).unwrap(), serde_json::json!({"principal": "owner", "device": {"kind": "console"}}));
    }

    #[test]
    fn a_requirement_is_met_by_the_capability_it_names_and_console_by_the_console_alone() {
        let drive = Caps::of(&[Read, Write, Instruct, Stream]);
        assert!(Requirement::Row.met_by(Caps::NONE, false));
        assert!(Requirement::Cap(Stream).met_by(drive, false) && !Requirement::Cap(Stream).met_by(Caps::of(&[Read]), false));
        assert!(!Requirement::Console.met_by(Caps::ALL, false), "every capability is still not the console");
        assert!(Requirement::Console.met_by(Caps::ALL, true));
        assert!(!Requirement::Never.met_by(Caps::ALL, true), "nor is the console enough for never");
    }

    #[test]
    fn capabilities_have_one_spelling_each() {
        for c in Capability::ALL {
            assert_eq!(Capability::parse(c.name()), Some(c));
            assert_eq!(serde_json::to_value(c).unwrap(), serde_json::json!(c.name()));
        }
        assert_eq!(Caps::of(&Capability::ALL), Caps::ALL);
        assert!(Caps::of(&[Read]).within(Caps::of(&[Read, Write])));
        assert!(!Caps::of(&[Admin]).within(Caps::of(&[Read, Write])));
    }
}

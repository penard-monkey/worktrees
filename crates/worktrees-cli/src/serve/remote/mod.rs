//! Remote access, the host's half (Server Edition Phase 2 slice 6; the
//! proposal's §4.1, §4.5, §6, §9.3): off by default, turned on only by a
//! console act, and one act from off again.
//!
//! [`RemoteAccess`] is the one per serve. It owns the opt-in
//! (`serve-remote.json`, [`file`]), the address watch (`netwatch::Watch`,
//! slice 6a's rules) on its own 3 s poll, the live listener ([`listener`]),
//! and where pairing windows open (`Pairing::set_bind`). The watch decides;
//! this performs: a pause closes the listener, every device connection and
//! any pairing window at once; a resume binds a NEW socket (the old one still
//! held the port, and on Linux kept serving a re-added address, §12.4 T2); a
//! move does both and persists the new address.
//!
//! **The kill switch** is [`RemoteAccess::disable`] (`remote_disable`): the
//! listener, every device socket, the pairing window and the opt-in, all at
//! once.
//!
//! **Fail closed on the audit log** (§5.1): no remote listener binds while
//! the writer is unhealthy. Until the audit log (#534) is in this branch,
//! that is the [`Gate`] seam, whose stand-in is always healthy and writes the
//! lifecycle records to serve.log.

pub mod advert;
pub mod file;
pub mod listener;
pub mod peer;
pub mod sock;
#[cfg(test)]
mod tests;

use std::net::{IpAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use worktrees_api::log::{info, warn};
use worktrees_api::Ctx;
use worktrees_core::netclass;

use super::conn::Clients;
use super::devices::{DeviceEvent, DeviceRegistry};
use super::identity::Identity;
use super::netwatch::{self, Action, Binding, Question, Snapshot, Watch, Why};
use super::pairing::{Bind, Pairing};
use super::tls;
use file::Saved;
use listener::{Deps, RemoteListener};
use peer::Rule;

/// How often the address watch looks (§4.1: the host poll's 3 s).
pub const POLL: Duration = Duration::from_secs(3);

/// The audit log's half of remote access (§5.1), as a seam until #534 is in
/// this branch: whether a record can be written right now, and the lifecycle
/// records themselves.
pub trait Gate: Send + Sync {
    /// The writer is healthy: a remote listener may bind.
    fn healthy(&self) -> bool;
    /// One lifecycle record: enabled, disabled, started, paused, resumed,
    /// moved, asked.
    fn record(&self, event: &'static str, detail: Value);
}

/// The audit log is the gate: its health is §5.1's, and each lifecycle
/// event is a record in it (and a serve.log line, for the person reading it).
impl Gate for super::audit::Audit {
    fn healthy(&self) -> bool {
        super::audit::Audit::healthy(self)
    }
    fn record(&self, event: &'static str, detail: Value) {
        info(&format!("serve: remote: {event} {detail}"));
        self.remote(event, detail);
    }
}

/// Always healthy, records to serve.log only: the tests' gate where the
/// audit log is beside the point.
#[cfg(test)]
pub struct NoAuditYet;

#[cfg(test)]
impl Gate for NoAuditYet {
    fn healthy(&self) -> bool {
        true
    }
    fn record(&self, event: &'static str, detail: Value) {
        info(&format!("serve: remote: {event} {detail}"));
    }
}

/// One registration with the OS's own mDNS responder (§4.7): the service
/// `_worktrees._tcp` named by the host's discovery id (instance and TXT
/// `id`, beside `v=1`), its SRV naming the REMOTE listener's port (never the
/// pairing port) at a host name of serve's own, `wt-<id>.local`, whose one
/// address record is the bound address, all on the bound interface alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advert {
    pub id: String,
    pub ip: IpAddr,
    pub port: u16,
    pub index: u32,
}

/// What serve asks of the OS's responder: register one advert (answering
/// the host name the SRV names), and withdraw whatever is registered —
/// freeing the registration, not only going quiet, because Avahi announces
/// a live one again by itself when its address returns (§12.4 T2).
pub trait Advertiser: Send + Sync {
    fn register(&self, ad: &Advert) -> Result<String, String>;
    fn withdraw(&self);
    /// A problem the responder reported after `register` returned (a name
    /// conflict found late): the advert is gone for this episode.
    fn trouble(&self) -> Option<String> {
        None
    }
}

/// The OS's responder, where serve has a backend for it (`advert.rs`).
pub fn platform_advertiser() -> Arc<dyn Advertiser> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(advert::DnsSd::default())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(NoAdvertiser("mDNS registration is not built for this platform yet; devices follow a moved host with the address code"))
    }
}

/// No responder to ask: discovery is off, the status says why, and the
/// address code is how a device follows a moved host (§4.7).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct NoAdvertiser(pub &'static str);

impl Advertiser for NoAdvertiser {
    fn register(&self, _: &Advert) -> Result<String, String> {
        Err(self.0.to_string())
    }
    fn withdraw(&self) {}
}

/// Whether a bind may be advertised at all (§4.7): LAN mode only — an
/// overlay is never advertised, and mDNS would not cross the tunnel — and
/// never `100.64.0.0/10` on an interface that is not a tunnel, which may be a
/// carrier's or a hotspot's shared space (§11.1 N1).
fn advert_eligible(rule: &Rule) -> Result<(), &'static str> {
    if rule.point_to_point {
        return Err("an overlay bind is never advertised");
    }
    if netclass::classify(rule.ip) == Ok(netclass::Class::Cgnat) {
        return Err("100.64.0.0/10 on an interface that is not a tunnel may be shared with strangers, so it is not advertised");
    }
    Ok(())
}

/// The advert, as the status reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Advertising {
    /// Not listening, or switched off.
    #[default]
    Off,
    Up(String),
    /// This bind is never advertised, or the responder said no.
    Not(String),
}

/// How the watch reads the host's interfaces; the tests hand in their own.
pub type Reader = Box<dyn Fn() -> std::io::Result<Snapshot> + Send + Sync>;

#[derive(Default)]
struct State {
    /// The opt-in, as the file holds it.
    saved: Option<Saved>,
    /// The file is there and cannot be read as one.
    unreadable: Option<String>,
    watch: Option<Watch>,
    /// What the watch says to listen on.
    want: Option<Binding>,
    live: Option<Live>,
    /// Why the listener is down, while it is.
    paused: Option<Why>,
    question: Option<Question>,
    /// A bind the watch wants that did not happen (a port taken, the audit
    /// log), said once per episode.
    error: Option<String>,
    /// Shown beside the state: shared address space, an untied socket.
    warning: Option<String>,
    advert: Advertising,
}

struct Live {
    listener: RemoteListener,
    binding: Binding,
    rule: Rule,
}

/// Remote access for one serve.
pub struct RemoteAccess {
    dir: PathBuf,
    rt: tokio::runtime::Handle,
    deps: Arc<Deps>,
    pairing: Arc<Pairing>,
    gate: Arc<dyn Gate>,
    advertiser: Arc<dyn Advertiser>,
    read: Reader,
    st: Mutex<State>,
    /// Who manages this serve's life (`--owner`): "app" when the desktop app
    /// does, so it can say that quitting it ends remote access (§9.3).
    owner: Mutex<Option<String>>,
    /// The audit log was found unhealthy and has not been seen healthy since.
    audit_down: std::sync::atomic::AtomicBool,
}

impl RemoteAccess {
    /// Remote access for the config dir `dir`, as its opt-in file says: off,
    /// or the bind it records, which the first poll takes up (the host has
    /// not looked at this network, so nothing is learned from it,
    /// `Watch::restored`). Inside the runtime its listeners will run on.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dir: PathBuf,
        ctx: Ctx,
        clients: Arc<Clients>,
        devices: Arc<DeviceRegistry>,
        pairing: Arc<Pairing>,
        audit: Arc<super::audit::Audit>,
        gate: Arc<dyn Gate>,
        advertiser: Arc<dyn Advertiser>,
        read: Reader,
    ) -> Arc<RemoteAccess> {
        let rt = tokio::runtime::Handle::current();
        let mut st = State::default();
        match file::read(&dir) {
            Ok(Some(saved)) => {
                if let (true, Some(r)) = (saved.on, &saved.record) {
                    st.watch = Some(Watch::restored(r.clone()));
                }
                st.saved = Some(saved);
            }
            Ok(None) => {}
            Err(e) => {
                warn(&format!("serve: remote access stays off: {e}"));
                st.unreadable = Some(e);
            }
        }
        let deps = Arc::new(Deps::new(ctx, clients, devices.clone(), audit));
        let me = Arc::new(RemoteAccess { dir, rt, deps, pairing, gate, advertiser, read, st: Mutex::new(st), owner: Mutex::new(None), audit_down: Default::default() });
        // A rotated identity is a new pin: the next handshake presents it.
        let weak = Arc::downgrade(&me);
        devices.subscribe(Arc::new(move |e: &DeviceEvent| {
            if let (DeviceEvent::IdentityRotated { .. }, Some(me)) = (e, weak.upgrade()) {
                *me.deps.tls.lock().unwrap() = None;
            }
        }));
        me
    }

    /// §5.1, the device half: may a device's non-read request be served
    /// now? `false` while the audit log cannot take its record. Said once
    /// per episode.
    pub fn audit_healthy(&self) -> bool {
        let ok = self.gate.healthy();
        if self.audit_down.swap(!ok, std::sync::atomic::Ordering::SeqCst) == ok {
            if ok {
                info("serve: the audit log takes records again; devices' writes are served");
            } else {
                warn("serve: the audit log cannot take a record; every non-read request from a device is refused until it can (§5.1)");
            }
        }
        ok
    }

    /// This serve's owner, as `--owner` named it.
    pub fn set_owner(&self, owner: Option<String>) {
        *self.owner.lock().unwrap() = owner;
    }

    /// The watch's own poll, every [`POLL`], on the blocking pool.
    pub fn start_watch(self: &Arc<Self>) {
        let me = self.clone();
        self.rt.spawn(async move {
            loop {
                tokio::time::sleep(POLL).await;
                let me = me.clone();
                let _ = tokio::task::spawn_blocking(move || me.tick()).await;
            }
        });
    }

    /// One poll: read the interfaces (only while remote access is on), and
    /// act on what the watch makes of them.
    pub fn tick(&self) {
        self.deps.tick(worktrees_core::sysclock::now_epoch());
        if self.st.lock().unwrap().watch.is_none() {
            return;
        }
        match (self.read)() {
            Ok(snap) => self.poll(Instant::now(), &snap),
            Err(e) => warn(&format!("serve: remote: interfaces unreadable: {e}")),
        }
    }

    fn poll(&self, now: Instant, snap: &Snapshot) {
        let before = self.status_value();
        {
            let mut st = self.st.lock().unwrap();
            let Some(w) = st.watch.as_mut() else { return };
            let actions = w.poll(now, snap);
            let record = w.record().clone();
            for a in actions {
                self.act(&mut st, a);
            }
            if let Some(saved) = st.saved.as_mut().filter(|s| s.record.as_ref() != Some(&record)) {
                saved.record = Some(record);
                if let Err(e) = file::write(&self.dir, saved) {
                    warn(&format!("serve: remote: {e}"));
                }
            }
            self.ensure_bound(&mut st, snap);
        }
        let after = self.status_value();
        if after != before {
            self.deps.ctx.emit("remote:changed", after);
        }
    }

    fn act(&self, st: &mut State, a: Action) {
        match a {
            Action::Start(b) => {
                st.want = Some(b);
                st.paused = None;
            }
            Action::Resume(b) => {
                self.gate.record("remote_resumed", json!({ "address": b.ip.to_string(), "iface": b.iface }));
                st.want = Some(b);
                st.paused = None;
                st.question = None;
            }
            Action::Pause { why, starting } => {
                self.unbind(st, "remote access paused: this host's address changed");
                if starting {
                    info(&format!("serve: remote access is paused at startup: {}", describe(&why)));
                }
                self.gate.record("remote_paused", json!({ "why": describe(&why), "starting": starting }));
                st.want = None;
                st.paused = Some(why);
            }
            Action::Move { from, to } => {
                self.unbind(st, "remote access moved to this host's new address");
                self.gate.record("bind_moved", json!({ "from": from.to_string(), "to": to.ip.to_string(), "by": "serve" }));
                st.want = Some(to);
                st.paused = None;
                st.question = None;
            }
            Action::Ask(q) => {
                self.gate.record("remote_asked", json!({ "question": question_text(&q), "alert": q.alerts() }));
                st.question = Some(q);
            }
            Action::Retract => st.question = None,
            // The record is completed by the watch itself; `poll` persists it.
            Action::Learn(_) => {}
        }
    }

    /// Close the listener, every device connection and any pairing window,
    /// and withdraw the advert.
    fn unbind(&self, st: &mut State, why: &'static str) {
        self.unadvertise(st);
        if let Some(live) = st.live.take() {
            live.listener.stop();
        }
        self.pairing.set_bind(None);
        let n = self.deps.clients.end_every_device(why);
        if n > 0 {
            info(&format!("serve: remote: {n} device connection(s) closed: {why}"));
        }
    }

    /// Bind what the watch wants, if nothing is bound: on the audit log's
    /// say-so, with the pairing windows opening there too.
    fn ensure_bound(&self, st: &mut State, snap: &Snapshot) {
        let Some(want) = st.want.clone() else { return };
        if st.live.as_ref().is_some_and(|l| l.binding == want) {
            return;
        }
        if let Some(live) = st.live.take() {
            live.listener.stop();
        }
        let failed = |st: &mut State, e: String| {
            if st.error.as_deref() != Some(e.as_str()) {
                warn(&format!("serve: remote access is not listening: {e}"));
            }
            st.error = Some(e);
        };
        if !self.gate.healthy() {
            return failed(st, "the audit log cannot be written, and no remote listener runs without it (§5.1)".into());
        }
        let Some(held) = snap.ifaces.iter().find(|i| i.name == want.iface).and_then(|i| i.holds(want.ip).map(|a| (i, *a))) else {
            return failed(st, format!("{} does not hold {}", want.iface, want.ip));
        };
        let rule = Rule { ip: want.ip, prefix: held.1.prefix, point_to_point: held.0.point_to_point };
        if self.deps.tls.lock().unwrap().is_none() {
            let cfg = Identity::ensure(&self.dir).and_then(|id| tls::server_config(&id, Arc::new(tls::DeviceVerifier::new(self.deps.devices.clone()))));
            match cfg {
                Ok(c) => *self.deps.tls.lock().unwrap() = Some(c),
                Err(e) => return failed(st, e),
            }
        }
        let port = st.saved.as_ref().map_or(0, |s| s.port);
        let started = {
            let _in = self.rt.enter();
            listener::start(self.deps.clone(), want.ip, port, want.index, rule)
        };
        let l = match started {
            Ok(l) => l,
            Err(e) => return failed(st, format!("could not listen on {}: {e}", sock::public(want.ip, port))),
        };
        if let Some(saved) = st.saved.as_mut().filter(|s| s.port == 0) {
            saved.port = l.at.port();
            if let Err(e) = file::write(&self.dir, saved) {
                warn(&format!("serve: remote: {e}"));
            }
        }
        info(&format!("serve: remote access listening on {} ({})", l.at, want.iface));
        self.gate.record("remote_started", json!({ "address": l.at.to_string(), "iface": want.iface, "version": env!("CARGO_PKG_VERSION") }));
        self.pairing.set_bind(Some(Bind { ip: want.ip, remote: l.at, index: want.index, rule: Some(rule) }));
        st.error = None;
        st.warning = l.warning.clone().or_else(|| st.warning.take());
        let ad = (l.at.port(), want.index, rule);
        st.live = Some(Live { listener: l, binding: want, rule });
        self.advertise(st, ad.0, ad.1, &ad.2);
    }

    /// Register the advert for the live bind, when this bind may be
    /// advertised and the host's switch is on (§4.7).
    fn advertise(&self, st: &mut State, port: u16, index: u32, rule: &Rule) {
        // The host's switch first: off means nothing goes to the responder.
        if !st.saved.as_ref().is_some_and(|s| s.advertise) {
            st.advert = Advertising::Off;
            return;
        }
        if let Err(why) = advert_eligible(rule) {
            st.advert = Advertising::Not(why.into());
            return;
        }
        let id = match Identity::ensure(&self.dir) {
            Ok(i) => i.discovery_id().to_string(),
            Err(e) => {
                st.advert = Advertising::Not(e);
                return;
            }
        };
        st.advert = match self.advertiser.register(&Advert { id, ip: rule.ip, port, index }) {
            Ok(host) => {
                info(&format!("serve: remote: advertised on the LAN as {host}"));
                self.gate.record("advert_registered", json!({ "host": host, "address": rule.ip.to_string() }));
                Advertising::Up(host)
            }
            Err(e) => {
                info(&format!("serve: remote: not advertised: {e}"));
                Advertising::Not(e)
            }
        };
    }

    fn unadvertise(&self, st: &mut State) {
        if let Advertising::Up(host) = std::mem::take(&mut st.advert) {
            self.advertiser.withdraw();
            self.gate.record("advert_withdrawn", json!({ "host": host }));
        }
    }

    /// `remote_enable` (§4.1): the host chooses `address` (on `iface`, or the
    /// one interface that holds it). The classifier runs at parse and again
    /// against the interface as it is now; the network is fingerprinted; the
    /// opt-in is written; and the first poll binds at once. A choice made
    /// while paused on a question is the question's "Use this address".
    pub fn enable(&self, address: &str, iface: Option<String>, port: Option<u16>, advertise: Option<bool>) -> Result<Value, String> {
        if !self.gate.healthy() {
            return Err("the audit log cannot be written; remote access stays off until it can (§5.1)".into());
        }
        // The classifier first (§4.1): a public address is `not-private`,
        // never "not on this host" — the refusal says what is wrong with it.
        if let Ok(ip) = address.split('%').next().unwrap_or_default().parse::<IpAddr>() {
            netclass::classify(ip).map_err(|e| addr_err(e.into()))?;
        }
        let snap = (self.read)().map_err(|e| format!("could not read this host's interfaces: {e}"))?;
        let iface = match iface {
            Some(i) => i,
            None => holder(address, &snap)?,
        };
        let want = netclass::parse_bind(address, &iface).map_err(addr_err)?;
        let bound = netclass::at_bind(&want, &snap.ifaces).map_err(addr_err)?;
        // The enable-time datagram, then a fresh read for the fingerprint.
        nudge_router(&bound, &snap);
        let snap = (self.read)().map_err(|e| format!("could not read this host's interfaces: {e}"))?;
        let record = netwatch::Record::enable(&bound, &snap);
        let mut st = self.st.lock().unwrap();
        let port = port.unwrap_or_else(|| st.saved.as_ref().map_or(0, |s| s.port));
        // The advert's switch: as given, else as the host last left it, else
        // on — §4.7: "It starts on, as §11.1 N2 decided".
        let advertise = advertise.or(st.saved.as_ref().map(|s| s.advertise)).unwrap_or(true);
        let saved = Saved { record: Some(record.clone()), port, advertise, on: true };
        file::write(&self.dir, &saved)?;
        self.unbind(&mut st, "remote access was moved by the host");
        let warning = bound.shared_space.then(|| shared_space_warning(bound.ip, &bound.iface));
        *st = State { saved: Some(saved), watch: Some(Watch::enabled(record, Instant::now())), warning, ..State::default() };
        self.gate.record("remote_enabled", json!({ "address": bound.ip.to_string(), "iface": bound.iface, "port": port }));
        drop(st);
        self.poll(Instant::now(), &snap);
        let st = self.st.lock().unwrap();
        if let (Some(e), None) = (&st.error, &st.live) {
            return Err(e.clone());
        }
        drop(st);
        Ok(self.status_value())
    }

    /// `remote_disable`, the kill switch (§4.1, ASI10): the listener, every
    /// device socket and the pairing window, at once, and the opt-in marked
    /// off (its port kept for the next time). An unreadable opt-in is removed.
    pub fn disable(&self) -> Result<Value, String> {
        let mut st = self.st.lock().unwrap();
        self.unbind(&mut st, "remote access was turned off on the host");
        let was_on = st.saved.as_ref().is_some_and(|s| s.on) || st.unreadable.is_some();
        let kept = st.saved.take().map(|s| Saved { on: false, ..s });
        let unreadable = st.unreadable.is_some();
        *st = State { saved: kept.clone(), ..State::default() };
        drop(st);
        match (&kept, unreadable) {
            (Some(s), false) => file::write(&self.dir, s)?,
            _ => file::remove(&self.dir)?,
        }
        if was_on {
            self.gate.record("remote_disabled", json!({}));
            info("serve: remote access turned off");
        }
        let v = self.status_value();
        self.deps.ctx.emit("remote:changed", v.clone());
        Ok(v)
    }

    /// serve is stopping: the advert withdrawn (its goodbye goes out while
    /// the address is still here), every device told why, the listener and
    /// any pairing window closed. The opt-in stays as it is, so the next
    /// serve takes remote access up again.
    pub fn shutdown(&self) {
        let mut st = self.st.lock().unwrap();
        let was = st.live.is_some();
        self.unbind(&mut st, "serve is stopping on the host");
        if was {
            self.gate.record("remote_stopped", json!({}));
        }
    }

    /// `remote_advertise` (§4.7 D6): the console's switch for the advert,
    /// at any time — off before remote access is ever on means it is never
    /// advertised, not for an instant. Kept in the opt-in for every enable
    /// after; acts on a live bind at once.
    pub fn set_advertise(&self, on: bool) -> Result<Value, String> {
        let mut st = self.st.lock().unwrap();
        if st.unreadable.is_some() {
            return Err("serve-remote.json is unreadable; `worktrees serve remote off` removes it".into());
        }
        let saved = st.saved.get_or_insert(Saved { record: None, port: 0, advertise: true, on: false });
        saved.advertise = on;
        file::write(&self.dir, saved)?;
        self.gate.record("advert_switched", json!({ "on": on }));
        match (on, st.live.as_ref().map(|l| (l.listener.at.port(), l.binding.index))) {
            (false, _) => self.unadvertise(&mut st),
            (true, Some((port, index))) if !matches!(st.advert, Advertising::Up(_)) => {
                let rule = st.live.as_ref().map(|l| l.rule).expect("live");
                self.advertise(&mut st, port, index, &rule);
            }
            _ => {}
        }
        drop(st);
        let v = self.status_value();
        self.deps.ctx.emit("remote:changed", v.clone());
        Ok(v)
    }

    /// The addresses `remote_enable` would take now (`remote_candidates`):
    /// the host app's "Turn on" picker, read from the interfaces as they are
    /// (`netclass::candidates`, which keeps `at_bind`'s rules), in the order a
    /// person should meet them, each with its mode and its shared-space
    /// warning in the words `remote_enable` would say it.
    pub fn candidates(&self) -> Result<Value, String> {
        let snap = (self.read)().map_err(|e| format!("could not read this host's interfaces: {e}"))?;
        let list: Vec<Value> = netclass::candidates(&snap.ifaces)
            .into_iter()
            .map(|c| {
                json!({
                    "address": c.ip.to_string(),
                    "prefix": c.prefix,
                    "iface": c.iface,
                    "class": c.class.name(),
                    "mode": if c.point_to_point { "overlay" } else { "lan" },
                    "warning": c.shared_space.then(|| shared_space_warning(c.ip, &c.iface)),
                })
            })
            .collect();
        Ok(Value::Array(list))
    }

    /// What remote access is doing, for the console (`remote_status`): with
    /// the host's name, which the pairing dialog's label names ("can run
    /// commands on <host> as you", §1.1) before any window gives it.
    pub fn status_value(&self) -> Value {
        let mut v = self.status_inner();
        v["owner"] = json!(*self.owner.lock().unwrap());
        v["host"] = json!(self.pairing.host());
        v
    }

    fn status_inner(&self) -> Value {
        let st = self.st.lock().unwrap();
        let pin = Identity::load(&self.dir).ok().flatten().map(|i| i.pin().to_string());
        if let Some(e) = &st.unreadable {
            return json!({ "state": "unreadable", "error": e });
        }
        let Some((saved, record)) = st.saved.as_ref().filter(|s| s.on).and_then(|s| s.record.as_ref().map(|r| (s, r))) else {
            let pref = st.saved.as_ref().map_or(true, |s| s.advertise);
            return json!({ "state": "off", "port": st.saved.as_ref().map(|s| s.port).filter(|p| *p != 0), "advertise": pref });
        };
        let state = match (&st.live, &st.paused, &st.error) {
            (Some(_), _, _) => "listening",
            (None, Some(_), _) => "paused",
            (None, None, Some(_)) => "error",
            (None, None, None) => "starting",
        };
        let at = st.live.as_ref().map(|l| l.listener.at);
        let code = match (&at, &pin) {
            (Some(a), Some(p)) => netclass::addr_code(a.ip(), a.port(), p).ok(),
            _ => None,
        };
        let mode = match record.fingerprint {
            netwatch::Fingerprint::PointToPoint { .. } => "overlay",
            netwatch::Fingerprint::Lan { .. } => "lan",
        };
        json!({
            "state": state,
            "address": record.addr.to_string(),
            "port": saved.port,
            "iface": st.live.as_ref().map_or(record.iface.clone(), |l| l.binding.iface.clone()),
            "mode": mode,
            "at": at.map(|a| a.to_string()),
            "pin": pin,
            "address_code": code,
            "paused": st.paused.as_ref().map(describe),
            "question": st.question.as_ref().map(|q| json!({ "text": question_text(q), "alert": q.alerts(), "use_address": question_address(q) })),
            "error": st.error,
            "warning": st.warning,
            "advertise": saved.advertise,
            "advert": match &st.advert {
                Advertising::Up(_) if self.advertiser.trouble().is_some() => {
                    json!({ "state": "not-advertised", "why": self.advertiser.trouble() })
                }
                Advertising::Up(host) => json!({ "state": "registered", "host": host }),
                Advertising::Not(why) => json!({ "state": "not-advertised", "why": why }),
                Advertising::Off => json!({ "state": "off" }),
            },
        })
    }

    /// Where devices reach the listener, while it listens.
    #[cfg(test)]
    pub fn at(&self) -> Option<std::net::SocketAddr> {
        self.st.lock().unwrap().live.as_ref().map(|l| l.listener.at)
    }
}

/// The one usable interface that holds `address` (its zone, when it has one,
/// names it). None or several: the host names it.
fn holder(address: &str, snap: &Snapshot) -> Result<String, String> {
    if let Some((_, zone)) = address.split_once('%') {
        return Ok(zone.to_string());
    }
    let ip: IpAddr = address.parse().map_err(|_| format!("bad-address: {address:?} is not an address literal"))?;
    let holders: Vec<&str> = snap.ifaces.iter().filter(|i| i.usable() && i.holds(ip).is_some()).map(|i| i.name.as_str()).collect();
    match holders.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(format!("not-held: no interface of this host holds {ip}")),
        many => Err(format!("{ip} is on {}: name the interface", many.join(", "))),
    }
}

/// §4.1, §11.1 N1: a `100.64.0.0/10` address on an interface that is not a
/// tunnel may be a carrier's or a hotspot's shared space.
fn shared_space_warning(ip: IpAddr, iface: &str) -> String {
    format!(
        "{ip} is in 100.64.0.0/10 on {iface}, which is not a tunnel: on a phone hotspot or some carriers that range is shared with strangers. They cannot get past an enrolled key, but they can reach the port, and this host is not advertised there."
    )
}

/// The refusal with its code in front, once (`not-private: …`).
fn addr_err(e: netclass::AddrError) -> String {
    let m = e.to_string();
    if m.starts_with(e.code()) {
        m
    } else {
        format!("{}: {m}", e.code())
    }
}

/// One datagram from the bound address to its interface's own router, then
/// a moment for the answer (§4.1; netwatch's module doc). It fills the
/// router's neighbour entry, so the fingerprint records its link-layer
/// address rather than "not known yet" (on Linux the entry is missing until
/// the first packet to it); and it is the moment macOS's Local Network
/// permission (macOS 15 and later) asks, at the host's own act rather than at
/// a device's first connect — unmeasured (§10 B6). Best effort: a failure
/// changes nothing but what the record knows.
fn nudge_router(bound: &netclass::Bound, snap: &Snapshot) {
    if bound.point_to_point {
        return;
    }
    let Some(r) = snap.routers.iter().find(|r| r.iface == bound.iface && r.router.ip.is_ipv4() == bound.ip.is_ipv4()) else { return };
    let to = sock::sockaddr(r.router.ip, 9, bound.index);
    let sent = UdpSocket::bind(sock::sockaddr(bound.ip, 0, bound.index)).and_then(|s| s.send_to(&[], to));
    match sent {
        Ok(_) => std::thread::sleep(Duration::from_millis(250)),
        Err(e) => info(&format!("serve: remote: the enable-time datagram to {} did not go: {e}", r.router.ip)),
    }
}

/// A pause reason in words, for the banner and the record.
fn describe(w: &Why) -> String {
    match w {
        Why::AddressLeft { addr, iface } => format!("{addr} is no longer on {iface}"),
        Why::InterfaceReplaced { iface, .. } => format!("{iface} was replaced under the listener"),
        Why::NetworkChanged { addr, iface } => format!("{addr} on {iface} is on a different network now"),
        Why::Unconfirmed { addr, iface } => format!("the network {addr} on {iface} is on cannot be confirmed yet"),
    }
}

/// A question in the host's words (§4.1, §12.3 R2).
fn question_text(q: &Question) -> String {
    match q {
        Question::DifferentNetwork { addr, iface } => {
            format!("Remote access is paused. This host is now {addr} on {iface}, on a different network.")
        }
        Question::OtherInterface { addr, iface } => {
            format!("Remote access is paused. {addr} is on {iface} now, not on the interface remote access was turned on for.")
        }
        Question::GatewayChanged { iface, router, was, now } => format!(
            "The router's hardware address changed ({router} on {iface}: {was} → {now}). This can mean a different network, or someone on this one pretending to be the router."
        ),
        Question::TooManyMoves { from, to } => format!(
            "This host's address moved more than {} times in an hour (now {from} → {to}). Remote access is paused until you choose.",
            netwatch::MOVES_PER_HOUR
        ),
        Question::CannotConfirm { addr, iface } => {
            format!("This host is on {addr} on {iface}, but its network could not be confirmed when remote access was turned on.")
        }
    }
}

/// The address "Use this address" would enable (`remote_enable` again).
fn question_address(q: &Question) -> String {
    match q {
        Question::DifferentNetwork { addr, .. } | Question::OtherInterface { addr, .. } | Question::CannotConfirm { addr, .. } => addr.to_string(),
        Question::TooManyMoves { to, .. } => to.to_string(),
        Question::GatewayChanged { .. } => String::new(),
    }
}

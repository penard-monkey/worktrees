//! Notifications: serve calls APNs itself, with the team's key on the host and
//! no third party (the phone's `docs/design/notifications.md`; the wire is
//! `docs/push-contract.md`).
//!
//! [`rows`] are what a device does for ITSELF and the console's acts. [`store`]
//! holds each device's token, key and switches in their own file, read by
//! nothing else. [`seal`] and [`payload`] are the wire, [`plan`] the delivery
//! rules on a scripted clock, [`apns`] the one outbound connection, [`key`] the
//! team's `.p8`.
//!
//! **The sender runs on its own thread and reads what the poll left behind**
//! (`Clients`' last `sessions:busy`), so the poll never waits on anything here:
//! a slow Apple costs a task per device and nothing else.
//!
//! **What stops it.** Remote access off (`remote_disable` is the kill switch:
//! sends in the air are aborted too, "remote off" means nothing leaves for a
//! device); an unhealthy audit log, as §5.1 stops a device's acts; and serve
//! stopping, which an app-owned serve does after the app quits.
//!
//! **`scope::event` decides who hears what.** A push is cut with the same
//! projection `sessions:busy` gets on a socket, so a place outside the grant
//! never notifies, by name or by count. `mcp.rs` names none of this, and
//! `provenance`'s test holds it so.

pub mod apns;
pub mod key;
pub mod payload;
pub mod plan;
pub mod rows;
pub mod seal;
pub mod store;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::task::JoinHandle;
use worktrees_core::authz::{self, Caller, Capability, DeviceId, Registry};

use super::devices::{ConnectRefusal, DeviceEvent, DeviceRegistry, DisabledReason, Subject};
use super::scope;
use apns::{Request, Transport};
use payload::{Alert, Kind, Place};
use plan::{Delivery, Planned, Planner, View};
use store::{Standing, Store};

/// `err`: this device has no registration to push to.
pub const NOT_REGISTERED: &str = "not-registered";
/// `err`, as the phone shows it: the host has nothing to sign with. Bret's
/// `.p8` has to reach this host first.
pub const NOT_CONFIGURED: &str = "not-configured: this host has no APNs key yet.";
/// `err`, as the phone shows it: Apple could not be reached or refused.
pub const TEST_FAILED: &str = "unavailable: the host could not reach Apple. Try again later.";

const TICK: Duration = Duration::from_secs(1);
/// How often the sends' counts become an audit record.
const TALLY_SECS: i64 = 600;
/// `push_test` answers within this, whatever Apple does.
const TEST_WAIT: Duration = Duration::from_secs(30);

/// A waiting place's `(repo, slug, project)`.
type PlaceKey = (String, String, String);

/// What the sender reads from the rest of serve, as seams: a test scripts them.
pub struct Deps {
    /// The last `sessions:busy` payload.
    pub feed: Box<dyn Fn() -> Option<Value> + Send + Sync>,
    /// The registered roots, for `scope`.
    pub roots: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    /// A waiting path as a place: `(repo, slug, project)`.
    pub place: Box<dyn Fn(&str, &[String]) -> Option<PlaceKey> + Send + Sync>,
    /// Remote access is on.
    pub enabled: Box<dyn Fn() -> bool + Send + Sync>,
    /// The audit log can take a record.
    pub healthy: Box<dyn Fn() -> bool + Send + Sync>,
    /// One `push` audit record's `devices`.
    pub tally: Box<dyn Fn(Value) + Send + Sync>,
    /// A line for serve.log.
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
    pub now: Box<dyn Fn() -> i64 + Send + Sync>,
    pub host: String,
}

struct InAir {
    task: JoinHandle<()>,
    planned: Planned,
}

#[derive(Default)]
struct State {
    planner: Planner,
    /// Path → its place, while it waits. `None`: not a registered project's.
    places: HashMap<String, Option<PlaceKey>>,
    air: HashMap<DeviceId, InAir>,
    /// `push_test`s in the air, by a number of their own: a request's, so several
    /// can be up for one device. Stopped with everything else.
    tests: HashMap<u64, (DeviceId, JoinHandle<()>)>,
    next_test: u64,
    last_tally: i64,
    /// The last reason sends failed, so a run of them is one line.
    said: Option<&'static str>,
}

/// The host's half, one per serve. Reached by the rows through `Call`.
pub struct Push {
    config_dir: Option<PathBuf>,
    pub store: Store,
    devices: Arc<DeviceRegistry>,
    deps: Deps,
    key: Mutex<Option<Arc<key::ApnsKey>>>,
    /// Apple's connections live in the transport, so one is kept per key.
    apple: Mutex<Option<Arc<apns::H2>>>,
    /// Seal and log instead of calling APNs. Set at the console only, held in
    /// memory only: a restarted serve is a real one again.
    dry_run: AtomicBool,
    /// A test's transport, in place of both.
    #[cfg(test)]
    test_transport: Mutex<Option<Arc<dyn Transport>>>,
    rt: Option<tokio::runtime::Handle>,
    st: Mutex<State>,
    stop: AtomicBool,
}

impl Push {
    /// The service over `store`. Hears the registry at once, and sweeps what a
    /// crash left, unless the registry cannot be read: then it says nothing
    /// about who is live, and an unreadable file must not erase registrations.
    pub fn new(config_dir: Option<PathBuf>, store: Store, devices: Arc<DeviceRegistry>, deps: Deps, rt: Option<tokio::runtime::Handle>) -> Arc<Push> {
        let key = config_dir.as_deref().and_then(key::load).map(Arc::new);
        let me = Arc::new(Push {
            config_dir,
            store,
            devices: devices.clone(),
            deps,
            key: Mutex::new(key),
            apple: Mutex::new(None),
            dry_run: AtomicBool::new(false),
            #[cfg(test)]
            test_transport: Mutex::new(None),
            rt,
            st: Mutex::new(State::default()),
            stop: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&me);
        devices.subscribe(Arc::new(move |e| {
            if let Some(p) = weak.upgrade() {
                p.hear(e);
            }
        }));
        me.sweep();
        me
    }

    /// What the registry says about `id` now. While it cannot be read it says
    /// nothing, so what the person chose is kept and only the token goes.
    pub fn standing(&self, id: &DeviceId) -> Standing {
        self.devices.snapshot().map_or(Standing::Paused, |f| standing_in(&f, id))
    }

    /// What this registry says about each id, for [`Store::sweep`].
    fn sweep(&self) {
        let Some(file) = self.devices.snapshot() else {
            (self.deps.log)("serve: push: the device list can't be read, so nothing is swept");
            return;
        };
        let n = self.store.sweep(|id| standing_in(&file, id));
        if n > 0 {
            (self.deps.log)(&format!("serve: push: dropped what {n} device(s) no longer being served had left"));
        }
    }

    /// There is something to send through: a key, or the dry run.
    pub fn ready(&self) -> bool {
        self.transport().is_some()
    }

    fn transport(&self) -> Option<Arc<dyn Transport>> {
        #[cfg(test)]
        if let Some(t) = self.test_transport.lock().unwrap().clone() {
            return Some(t);
        }
        if self.dry_run.load(Ordering::SeqCst) {
            return Some(Arc::new(apns::DryRun { line: self.deps.log.clone() }));
        }
        let key = self.key.lock().unwrap().clone()?;
        let mut apple = self.apple.lock().unwrap();
        if apple.is_none() {
            match apns::Dial::apple() {
                Ok(d) => *apple = Some(apns::H2::new(key, d)),
                Err(e) => {
                    (self.deps.log)(&format!("serve: push: {e}"));
                    return None;
                }
            }
        }
        apple.clone().map(|a| Arc::new(a) as Arc<dyn Transport>)
    }

    #[cfg(test)]
    pub fn use_transport(&self, t: Arc<dyn Transport>) {
        *self.test_transport.lock().unwrap() = Some(t);
    }

    #[cfg(test)]
    pub fn in_air(&self) -> usize {
        let st = self.st.lock().unwrap();
        st.air.len() + st.tests.len()
    }

    pub fn set_dry_run(&self, on: bool) -> Value {
        self.dry_run.store(on, Ordering::SeqCst);
        (self.deps.log)(&format!("serve: push: dry run {}", if on { "on: pushes are sealed and logged, Apple is not called" } else { "off" }));
        self.status()
    }

    pub fn import_key(&self, p8: &str, key_id: &str, team_id: &str) -> Result<Value, String> {
        let dir = self.config_dir.as_deref().ok_or("this serve keeps no files")?;
        let k = key::import(dir, std::path::Path::new(p8), key_id, team_id)?;
        *self.key.lock().unwrap() = Some(Arc::new(k));
        *self.apple.lock().unwrap() = None;
        (self.deps.log)("serve: push: an APNs key was imported");
        Ok(self.status())
    }

    pub fn remove_key(&self) -> Result<Value, String> {
        let dir = self.config_dir.as_deref().ok_or("this serve keeps no files")?;
        key::remove(dir)?;
        *self.key.lock().unwrap() = None;
        *self.apple.lock().unwrap() = None;
        (self.deps.log)("serve: push: the APNs key was removed");
        Ok(self.status())
    }

    /// What the console may know: whether a key is there, never which; and per
    /// device whether it is registered and allowed, never its token or key.
    pub fn status(&self) -> Value {
        let devices: Vec<Value> = self
            .devices
            .snapshot()
            .map(|f| {
                f.devices
                    .iter()
                    .filter(|d| !d.is_revoked())
                    .map(|d| {
                        let r = self.store.record(&d.id).unwrap_or_default();
                        json!({ "id": d.id, "name": d.name, "registered": r.reg.is_some(), "env": r.reg.as_ref().map(|x| x.env), "security_allowed": r.security_allowed })
                    })
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "configured": self.key.lock().unwrap().is_some(),
            "dry_run": self.dry_run.load(Ordering::SeqCst),
            "store_readable": !self.store.is_broken(),
            "devices": devices,
        })
    }

    /// `push_allow_security`, the console's: owner principal only, and the
    /// act is a record in the audit log.
    pub fn allow_security(&self, id: &DeviceId, allowed: bool) -> Result<bool, String> {
        self.devices.readable()?;
        let f = self.devices.snapshot().ok_or("the device list can't be read")?;
        let d = f.device(id).ok_or_else(|| format!("no device {id}"))?;
        if d.is_revoked() {
            return Err(format!("device {id} is revoked"));
        }
        if !d.principal.is_owner() {
            return Err("security alerts go only to the owner's devices".into());
        }
        let moved = self.store.allow_security(id, allowed)?;
        if moved {
            self.devices.announce(DeviceEvent::PushSecurityChanged { subject: d.subject(), allowed });
        }
        Ok(moved)
    }

    /// The registry's events: a device that stops being served loses its token
    /// and key at once, and anything in the air for it; an alert is held for
    /// each of the owner's registered devices, and [`Push::tick`]'s view, which
    /// reads the console's word and the phone's switch as they stand when it
    /// would go, drops it for any that may not hear it.
    fn hear(&self, e: &DeviceEvent) {
        if let Some(id) = e.ends() {
            let standing = if matches!(e, DeviceEvent::Disabled { .. }) { Standing::Paused } else { Standing::Gone };
            self.store.drop_for(id, standing);
            let mut st = self.st.lock().unwrap();
            if let Some(a) = st.air.remove(id) {
                a.task.abort();
            }
            st.tests.retain(|_, (d, task)| {
                if d == id {
                    task.abort();
                }
                d != id
            });
            st.planner.forget(id);
        }
        if !e.alerts() {
            return;
        }
        let Some(alert) = self.alert_of(e) else { return };
        let targets = self.store.registered();
        let mut st = self.st.lock().unwrap();
        for (id, _) in targets {
            if self.devices.device_principal(&id).is_some_and(|p| p.is_owner()) {
                st.planner.queue_alert(&id, alert.clone());
            }
        }
    }

    fn named(&self, s: &Subject) -> Option<String> {
        self.devices.device_name(&s.principal, &s.device)
    }

    /// `DeviceEvent::alerts()`'s kinds, as the phone's copy rows say them.
    /// `reason` is the notification's body, so it is a sentence.
    fn alert_of(&self, e: &DeviceEvent) -> Option<Alert> {
        Some(match e {
            DeviceEvent::Enrolled { name, .. } => Alert { what: "enrolled", device: Some(name.clone()), reason: None },
            DeviceEvent::Disabled { subject, reason, .. } => Alert {
                what: "disabled",
                device: self.named(subject),
                reason: Some(
                    match reason {
                        DisabledReason::Idle => "It went unused for too long.",
                        DisabledReason::Until => "Its pairing reached its end date.",
                        DisabledReason::Unrecorded => "Its record said more than the host's audit log could back.",
                        DisabledReason::Console => "It was paused on the host.",
                    }
                    .to_string(),
                ),
            },
            DeviceEvent::ConnectRefused { subject, why, .. } => {
                let who = subject.as_ref().and_then(|s| self.named(s));
                let whom = who.clone().unwrap_or_else(|| "A device".to_string());
                let reason = match why {
                    ConnectRefusal::Revoked => format!("{whom} was revoked and tried to connect."),
                    ConnectRefusal::Disabled => format!("{whom} is disabled and tried to connect."),
                    ConnectRefusal::Expired => format!("{whom}'s pairing has lapsed and it tried to connect."),
                    ConnectRefusal::Unreadable => "The host can't read its device list.".to_string(),
                    ConnectRefusal::Unknown => return None,
                };
                Alert { what: "refused", device: who, reason: Some(reason) }
            }
            DeviceEvent::NewAddressClass { subject, .. } => {
                let who = self.named(subject);
                let reason = format!("{} connected from a new kind of network.", who.clone().unwrap_or_else(|| "A device".to_string()));
                Alert { what: "address", device: who, reason: Some(reason) }
            }
            DeviceEvent::IdentityRotated { .. } => Alert { what: "rotated", device: None, reason: Some("The host's identity was changed.".to_string()) },
            DeviceEvent::Unreadable { .. } => Alert { what: "unreadable", device: None, reason: Some("The host can't read its device list.".to_string()) },
            _ => return None,
        })
    }

    /// `remote_disable`: everything in the air stops now.
    pub fn kill(&self) {
        let mut st = self.st.lock().unwrap();
        let air: Vec<(DeviceId, InAir)> = st.air.drain().collect();
        for (id, a) in air {
            a.task.abort();
            st.planner.abandon(&id, &a.planned);
        }
        for (_, (_, task)) in st.tests.drain() {
            task.abort();
        }
        drop(st);
        // Nothing is left to carry: the next send dials afresh.
        *self.apple.lock().unwrap() = None;
    }

    /// serve is stopping: the counts land, and nothing more goes.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.kill();
        self.flush_tally(i64::MAX);
    }

    fn flush_tally(&self, now: i64) {
        let tallies = {
            let mut st = self.st.lock().unwrap();
            if now != i64::MAX && now - st.last_tally < TALLY_SECS {
                return;
            }
            st.last_tally = now;
            st.planner.take_tallies()
        };
        if tallies.is_empty() {
            return;
        }
        let list: Vec<Value> = tallies.into_iter().map(|(id, t)| json!({ "device": id, "sent": t.sent, "badge": t.badge, "failed": t.failed, "suppressed": t.suppressed })).collect();
        (self.deps.tally)(Value::Array(list));
    }

    /// The sender's thread: one tick a second until serve stops.
    pub fn spawn(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        let _ = std::thread::Builder::new().name("push-sender".into()).spawn(move || loop {
            std::thread::sleep(TICK);
            let Some(p) = me.upgrade() else { return };
            if p.stop.load(Ordering::SeqCst) {
                return;
            }
            let now = (p.deps.now)();
            p.tick(now);
            p.flush_tally(now);
        });
    }

    /// One look at the world: who waits, and for each registered device, the
    /// next push due. Nothing here waits on a send.
    pub fn tick(self: &Arc<Self>, now: i64) {
        let payload = (self.deps.feed)().unwrap_or(Value::Null);
        let waiting: Vec<String> = payload["waiting"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        self.resolve_places(&waiting);
        // A wait is a fact whether or not anything may be sent about it.
        self.st.lock().unwrap().planner.observe(now, &waiting);
        if !(self.deps.enabled)() {
            self.kill();
            return;
        }
        if !(self.deps.healthy)() {
            return;
        }
        let Some(transport) = self.transport() else { return };
        let roots = (self.deps.roots)();
        for (id, rec) in self.store.registered() {
            let Some(reg) = rec.reg.clone() else { continue };
            // The registry's word, now: a stale entry can never reach Apple.
            let Some(principal) = self.devices.device_principal(&id) else { continue };
            if self.devices.device_grant(&principal, &id).is_none() {
                continue;
            }
            let caller = Caller::paired(principal, id.clone());
            let grant = authz::effective(&*self.devices, &caller);
            let places = self.visible(&payload, &caller, &grant, &rec, &roots);
            let view = View { input: rec.input, reminder: rec.reminder, security: rec.security_allowed && rec.security, waiting: &places };
            let mut st = self.st.lock().unwrap();
            let Some(planned) = st.planner.plan(now, &id, &view) else { continue };
            let Some(request) = self.build(&planned.kind, planned.badge, &reg, now) else {
                st.planner.abandon(&id, &planned);
                continue;
            };
            let Some(rt) = &self.rt else {
                st.planner.abandon(&id, &planned);
                continue;
            };
            let sending = transport.send(request);
            let me = self.clone();
            let (dev, token) = (id.clone(), reg.token.clone());
            let task = rt.spawn(async move {
                let how = sending.await;
                me.finish(&dev, &token, how, now);
            });
            st.air.insert(id, InAir { task, planned });
        }
    }

    /// Names for what waits, looked up once per wait, outside the state lock:
    /// this is a git call.
    fn resolve_places(&self, waiting: &[String]) {
        let missing: Vec<String> = {
            let mut st = self.st.lock().unwrap();
            st.places.retain(|p, _| waiting.contains(p));
            waiting.iter().filter(|p| !st.places.contains_key(*p)).cloned().collect()
        };
        if missing.is_empty() {
            return;
        }
        let roots = (self.deps.roots)();
        let found: Vec<(String, Option<PlaceKey>)> = missing.into_iter().map(|p| {
            let v = (self.deps.place)(&p, &roots);
            (p, v)
        }).collect();
        self.st.lock().unwrap().places.extend(found);
    }

    /// What this device may hear of the waiting set: `scope`'s projection, its
    /// `read`, its mutes. Whatever does not resolve to a registered project's
    /// place says nothing.
    fn visible(&self, payload: &Value, caller: &Caller, grant: &authz::Grant, rec: &store::PushRecord, roots: &[String]) -> Vec<Place> {
        if !grant.allows(Capability::Read) {
            return Vec::new();
        }
        let Some(cut) = scope::event("sessions:busy", payload, caller, grant, &mut || roots.to_vec()) else { return Vec::new() };
        let st = self.st.lock().unwrap();
        cut["waiting"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter_map(|path| {
                        let (repo, slug, project) = st.places.get(path)?.clone()?;
                        if rec.muted.iter().any(|m| m.repo == repo && m.slug.as_ref().is_none_or(|s| s == &slug)) {
                            return None;
                        }
                        Some(Place { path: path.to_string(), place: slug.clone(), repo, slug, project, agent: agent_of(payload, path) })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The request for `kind`, sealed for this registration. `None` for a
    /// registration whose key does not decode, which the row never lets in.
    fn build(&self, kind: &Kind, badge: u32, reg: &store::PushReg, now: i64) -> Option<Request> {
        let key: [u8; seal::KEY_LEN] = super::stepup::b64_decode(&reg.key)?.try_into().ok()?;
        let key_id: [u8; seal::KEY_ID_LEN] = super::stepup::b64_decode(&reg.key_id)?.try_into().ok()?;
        let body = payload::body(kind, badge, &self.deps.host, &key, &key_id, seal::random_nonce());
        Some(Request {
            env: reg.env,
            token: reg.token.clone(),
            topic: reg.topic.clone(),
            collapse: kind.collapses().map(|p| seal::collapse_id(&key, &p.repo, &p.slug)),
            expiration: kind.expiration(now),
            priority: kind.priority(),
            body,
            kind: kind.word(),
            key_id: reg.key_id.clone(),
            now,
        })
    }

    /// A send came back. `now` is the tick that planned it.
    fn finish(&self, id: &DeviceId, token: &str, how: Delivery, now: i64) {
        self.note(&how);
        if let Delivery::Dead(why) = &how {
            self.drop_dead(id, token, why);
        }
        let mut st = self.st.lock().unwrap();
        if let Some(a) = st.air.remove(id) {
            st.planner.report(id, &a.planned, &how, now);
        }
    }

    fn drop_dead(&self, id: &DeviceId, token: &str, why: &'static str) {
        match self.store.drop_token(id, token) {
            Ok(true) => {
                if let Some(principal) = self.devices.device_principal(id) {
                    self.devices.announce(DeviceEvent::PushTokenDropped { subject: Subject { principal, device: id.clone() }, why });
                }
                (self.deps.log)(&format!("serve: push: APNs says a token is dead ({why}); dropped"));
            }
            Ok(false) => {}
            Err(e) => (self.deps.log)(&format!("serve: push: could not drop a dead token: {e}")),
        }
    }

    /// One serve.log line per run of one failure, a trust failure named as
    /// such. Never a token or a name: these words are constants.
    fn note(&self, how: &Delivery) {
        let why = match how {
            Delivery::Sent | Delivery::Dead(_) => None,
            Delivery::Retry(w) | Delivery::Auth(w) | Delivery::Rejected(w) => Some(*w),
        };
        let mut st = self.st.lock().unwrap();
        if st.said == why {
            return;
        }
        st.said = why;
        drop(st);
        match why {
            Some("trust") => (self.deps.log)("serve: push: APNs's certificate does not chain to the roots this build embeds: nothing is sent until a release updates them"),
            Some(w) => (self.deps.log)(&format!("serve: push: APNs: {w}")),
            None => (self.deps.log)("serve: push: APNs: delivering again"),
        }
    }

    /// `push_test`: one push, answered with how it ended. Runs on a request's
    /// blocking thread, so waiting here holds nothing else up.
    pub fn test(self: &Arc<Self>, id: &DeviceId) -> Result<Value, String> {
        // Remote off means nothing leaves, and that has no exception.
        if !(self.deps.enabled)() {
            return Err(TEST_FAILED.into());
        }
        let reg = self.store.record(id)?.reg.ok_or(NOT_REGISTERED)?;
        let transport = self.transport().ok_or(NOT_CONFIGURED)?;
        let rt = self.rt.as_ref().ok_or(NOT_CONFIGURED)?;
        let now = (self.deps.now)();
        if !self.st.lock().unwrap().planner.take_test(id, now) {
            return Err(super::budget::RATE.into());
        }
        let request = self.build(&Kind::Test, 0, &reg, now).ok_or_else(|| TEST_FAILED.to_string())?;
        let sending = transport.send(request);
        let (tx, rx) = std::sync::mpsc::channel();
        {
            // Held across the spawn, so the task cannot finish before its entry
            // exists. An aborted task drops `tx`, which answers the wait below.
            let mut st = self.st.lock().unwrap();
            st.next_test += 1;
            let n = st.next_test;
            let me = self.clone();
            let task = rt.spawn(async move {
                let how = sending.await;
                me.st.lock().unwrap().tests.remove(&n);
                let _ = tx.send(how);
            });
            st.tests.insert(n, (id.clone(), task));
        }
        let how = rx.recv_timeout(TEST_WAIT).map_err(|_| TEST_FAILED.to_string())?;
        self.note(&how);
        match how {
            Delivery::Sent => Ok(json!({ "sent": true })),
            Delivery::Dead(why) => {
                self.drop_dead(id, &reg.token, why);
                Err(NOT_REGISTERED.into())
            }
            _ => Err(TEST_FAILED.into()),
        }
    }
}

fn standing_in(file: &super::devices::DevicesFile, id: &DeviceId) -> Standing {
    match file.device(id) {
        None => Standing::Gone,
        Some(d) if d.is_revoked() => Standing::Gone,
        Some(d) if d.disabled.is_some() => Standing::Paused,
        Some(_) => Standing::Live,
    }
}

/// The provider's name for a waiting path: `waiting_by`'s first holder.
fn agent_of(payload: &Value, path: &str) -> Option<String> {
    let by = payload["waiting_by"].as_object()?;
    by.iter().find(|(_, v)| v.as_array().is_some_and(|a| a.iter().any(|p| p.as_str() == Some(path)))).and_then(|(id, _)| worktrees_core::provider::by_id(id)).map(|p| p.label.to_string())
}

/// The serve-wide seams, as a real serve fills them.
pub fn live_deps(clients: std::sync::Weak<super::conn::Clients>, audit: Arc<super::audit::Audit>) -> Deps {
    let (c1, c2, c3) = (clients.clone(), clients.clone(), clients);
    let (a1, a2) = (audit.clone(), audit);
    Deps {
        feed: Box::new(move || c1.upgrade()?.last_event("sessions:busy")),
        roots: Box::new(move || c2.upgrade().map(|c| c.event_roots()).unwrap_or_default()),
        place: Box::new(live_place),
        enabled: Box::new(move || c3.upgrade().and_then(|c| c.remote()).is_some_and(|r| r.is_on())),
        healthy: Box::new(move || a1.healthy()),
        tally: Box::new(move |v| a2.push_sends(v)),
        log: Arc::new(worktrees_api::log::info),
        now: Box::new(worktrees_core::sysclock::now_epoch),
        host: super::pairing::host_name(),
    }
}

/// `(registry key, slug, project name)`, or `None` for an unregistered repo.
fn live_place(path: &str, roots: &[String]) -> Option<PlaceKey> {
    let (root, slug) = worktrees_api::watch::place_key_for(path, roots)?;
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    let c = canon(&root);
    let key = roots.iter().find(|r| canon(r) == c).cloned().unwrap_or(root);
    let reg = worktrees_core::registry::read_lenient();
    let name = reg.by_root(&key).map(|e| e.name.clone()).unwrap_or_else(|| std::path::Path::new(&key).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Some((key, slug, name))
}

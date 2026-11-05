//! The rows, the store and the sender, against the contract's fixtures, on a
//! scripted clock. Nothing here reaches Apple: a recording transport, or
//! `apns::fixture`'s local server.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use serde_json::{json, Value};
use worktrees_core::authz::{Caller, Capability, Caps, Grant, PrincipalId, Projects};

use super::super::audit::{self, Audit, Fleet};
use super::super::devices::{ConnectRefusal, DeviceEvent, DeviceRecord, DeviceRegistry, DisabledReason, Enrollment, Expiry, Platform, Subject};
use super::super::dispatch::{self, Args, Call};
use super::super::resolve::fixture::{call_as, Fixture};
use super::apns::{self, Request, Sending, Transport};
use super::plan::Delivery;
use super::store::{PushEnv, PushReg, Standing, Store, FILE};
use super::{Deps, Push};

const NOW: i64 = 1_800_000_000;
const TOKEN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const KEY_ID: &str = "AQIDBAUGBwg=";

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wt-push-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn enroll_named(reg: &DeviceRegistry, name: &str, key: u8, caps: &[Capability], projects: Projects) -> DeviceRecord {
    let pin = |n: u8| super::super::devices::hex(&[n; 32]);
    reg.enroll(
        Enrollment {
            principal: PrincipalId::owner(),
            name: name.into(),
            platform: Platform::Ios,
            spki_sha256: pin(key),
            stepup_spki_sha256: Some(pin(key + 100)),
            grant: Grant { capabilities: Caps::of(caps), projects },
            expiry: Expiry::default(),
            addr: None,
        },
        NOW,
    )
    .unwrap()
}

fn enroll(reg: &DeviceRegistry, key: u8, caps: &[Capability], projects: Projects) -> DeviceRecord {
    enroll_named(reg, &format!("Phone {key}"), key, caps, projects)
}

fn caller(d: &DeviceRecord) -> Caller {
    Caller::paired(d.principal.clone(), d.id.clone())
}

fn args(v: Value) -> Args {
    Args(v.as_object().cloned().unwrap_or_default())
}

fn reg_args() -> Value {
    json!({ "token": TOKEN, "env": "development", "topic": "com.brethash.worktrees.ios", "key": KEY, "key_id": KEY_ID })
}

fn the_reg() -> PushReg {
    PushReg { token: TOKEN.into(), env: PushEnv::Development, topic: "com.brethash.worktrees.ios".into(), key: KEY.into(), key_id: KEY_ID.into() }
}

fn fill(v: &Value, f: &Value) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') => f[&s[1..]].clone(),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), fill(x, f))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(|x| fill(x, f)).collect()),
        other => other.clone(),
    }
}

// ── a scripted world ─────────────────────────────────────────────────────────

/// What a recording transport keeps of one push. Answers as scripted.
#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<Request>>,
    answer: Mutex<Option<Delivery>>,
    /// Never answer, for a slow Apple.
    hang: AtomicBool,
}

impl Transport for Arc<Recorder> {
    fn send(&self, r: Request) -> Sending {
        self.sent.lock().unwrap().push(r);
        let how = self.answer.lock().unwrap().clone().unwrap_or(Delivery::Sent);
        let hang = self.hang.load(Ordering::SeqCst);
        Box::pin(async move {
            if hang {
                std::future::pending::<()>().await;
            }
            how
        })
    }
}

struct World {
    reg: Arc<DeviceRegistry>,
    push: Arc<Push>,
    wire: Arc<Recorder>,
    feed: Arc<Mutex<Value>>,
    on: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    tallies: Arc<Mutex<Vec<Value>>>,
    lines: Arc<Mutex<Vec<String>>>,
    clock: Arc<Mutex<i64>>,
    rt: tokio::runtime::Runtime,
    dir: Option<PathBuf>,
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(d) = &self.dir {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

const ROOTS: [&str; 2] = ["/r/a", "/r/b"];

fn world_on(dir: Option<PathBuf>, reg: Arc<DeviceRegistry>) -> World {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let feed: Arc<Mutex<Value>> = Arc::new(Mutex::new(json!({ "busy": [], "waiting": [], "waiting_by": {} })));
    let (on, healthy) = (Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(true)));
    let tallies: Arc<Mutex<Vec<Value>>> = Default::default();
    let lines: Arc<Mutex<Vec<String>>> = Default::default();
    let clock = Arc::new(Mutex::new(NOW));
    let (f, o, h, t, l, c) = (feed.clone(), on.clone(), healthy.clone(), tallies.clone(), lines.clone(), clock.clone());
    let deps = Deps {
        feed: Box::new(move || Some(f.lock().unwrap().clone())),
        roots: Box::new(|| ROOTS.iter().map(|s| s.to_string()).collect()),
        // `/r/a/.worktrees/x` is `/r/a`'s place `x`.
        place: Box::new(|path, _| {
            let (repo, slug) = path.split_once("/.worktrees/")?;
            Some((repo.to_string(), slug.to_string(), repo.rsplit('/').next()?.to_string()))
        }),
        enabled: Box::new(move || o.load(Ordering::SeqCst)),
        healthy: Box::new(move || h.load(Ordering::SeqCst)),
        tally: Box::new(move |v| t.lock().unwrap().push(v)),
        log: Arc::new(move |s| l.lock().unwrap().push(s.to_string())),
        now: Box::new(move || *c.lock().unwrap()),
        host: "Studio Mac".into(),
    };
    let store = match &dir {
        Some(d) => Store::open(d),
        None => Store::in_memory(),
    };
    let push = Push::new(dir.clone(), store, reg.clone(), deps, Some(rt.handle().clone()));
    let wire = Arc::new(Recorder::default());
    push.use_transport(Arc::new(wire.clone()));
    World { reg, push, wire, feed, on, healthy, tallies, lines, clock, rt, dir }
}

fn world() -> World {
    world_on(None, Arc::new(DeviceRegistry::in_memory()))
}

fn path(repo: &str, slug: &str) -> String {
    format!("{repo}/.worktrees/{slug}")
}

impl World {
    fn phone(&self, name: &str, key: u8, projects: Projects) -> DeviceRecord {
        let d = enroll_named(&self.reg, name, key, &[Capability::Read], projects);
        self.push.store.register(&d.id, the_reg()).unwrap();
        d
    }
    fn wait(&self, paths: &[String]) {
        *self.feed.lock().unwrap() = json!({ "busy": [], "waiting": paths, "waiting_by": { "claude": paths } });
    }
    /// One tick at `t` seconds, and whatever it sent, come back.
    fn at(&self, t: i64) {
        let now = NOW + t;
        *self.clock.lock().unwrap() = now;
        self.push.tick(now);
        self.settle();
    }
    fn run(&self, from: i64, to: i64) {
        for t in from..=to {
            self.at(t);
        }
    }
    fn settle(&self) {
        if self.wire.hang.load(Ordering::SeqCst) {
            return;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !self.push.st.lock().unwrap().air.is_empty() {
            assert!(std::time::Instant::now() < deadline, "a send never came back");
            std::thread::yield_now();
        }
    }
    fn kinds(&self) -> Vec<&'static str> {
        self.wire.sent.lock().unwrap().iter().map(|r| r.kind).collect()
    }
    fn body(&self, i: usize) -> Value {
        serde_json::from_slice(&self.wire.sent.lock().unwrap()[i].body).unwrap()
    }
    /// The plaintext of push `i`, opened as the phone opens it.
    fn opened(&self, i: usize) -> Value {
        open(&self.body(i))
    }
    fn call(&self, c: Caller) -> Call {
        let world = Fixture { roots: ROOTS.iter().map(|s| s.to_string()).collect(), panes: Vec::new() };
        Call { devices: self.reg.clone(), world: Some(Arc::new(world)), push: Some(self.push.clone()), ..call_as(c, self.reg.clone()) }
    }
}

fn open(body: &Value) -> Value {
    let dec = |k: &str| crate::serve::stepup::b64_decode(body["wt"][k].as_str().unwrap()).unwrap();
    let key = crate::serve::stepup::b64_decode(KEY).unwrap();
    let k = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &key).unwrap());
    let nonce: [u8; 12] = dec("n").try_into().unwrap();
    let aad = [super::seal::LABEL, dec("k").as_slice()].concat();
    let mut c = dec("c");
    let plain = k.open_in_place(Nonce::assume_unique_for_key(nonce), Aad::from(aad), &mut c).expect("it opens");
    serde_json::from_slice(plain).unwrap()
}

// ── the rows ─────────────────────────────────────────────────────────────────

/// Every row's answer, in the order a phone meets them, from the file the
/// phone's lane reads. A device whose grant is Observe: `read` and one project.
#[test]
fn the_rows_answer_as_the_contract_says() {
    let fx: Value = serde_json::from_str(include_str!("../../../../../docs/push-fixtures/rows.json")).unwrap();
    let w = world();
    let d = enroll(&w.reg, 1, &[Capability::Read], Projects::only(["/r/a"]));
    let phone = w.call(caller(&d));
    let desk = w.call(Caller::console());
    let run = |cx: &Call, cmd: &str, a: Value| dispatch::call(cx, cmd, &args(a));
    for (i, s) in fx["steps"].as_array().unwrap().iter().enumerate() {
        let a = fill(&s["args"], &fx);
        if let Some(c) = s["console"].as_str() {
            let mut a = a;
            a["id"] = json!(d.id);
            run(&desk, c, a).unwrap_or_else(|e| panic!("step {i}: {e}"));
            continue;
        }
        let got = run(&phone, s["cmd"].as_str().unwrap(), a);
        let want = match s.get("ok") {
            Some(ok) => Ok(ok.clone()),
            None => Err(s["err"].clone()),
        };
        assert_eq!(got, want, "step {i}: {s}");
    }
    for (i, s) in fx["refused_args"].as_array().unwrap().iter().enumerate() {
        let got = run(&phone, s["cmd"].as_str().unwrap(), fill(&s["args"], &fx));
        assert_eq!(got, Err(s["err"].clone()), "refused {i}: {s}");
    }
    assert!(w.push.store.record(&d.id).unwrap().reg.is_none(), "no refused registration landed");
}

/// A mute outside the grant is not kept, not merely not answered: a read's own
/// filter must not be what hides it.
#[test]
fn a_mute_outside_the_grant_is_not_stored() {
    let w = world();
    let d = enroll(&w.reg, 1, &[Capability::Read], Projects::only(["/r/a"]));
    let phone = w.call(caller(&d));
    let a = args(json!({ "muted": [{ "repo": "/r/a" }, { "repo": "/r/b" }, { "repo": "/nowhere" }, { "repo": "/r/a", "slug": "../x" }] }));
    dispatch::call(&phone, "push_prefs", &a).unwrap();
    let held = w.push.store.record(&d.id).unwrap().muted;
    assert_eq!(serde_json::to_value(held).unwrap(), json!([{ "repo": "/r/a" }]));
}

/// The identity is the connection's. No argument names a device, and a frame
/// that tries acts on the caller's own record, never the named one.
#[test]
fn a_device_acts_only_on_its_own_record() {
    let w = world();
    let me = enroll(&w.reg, 1, &[Capability::Read], Projects::All);
    let other = enroll(&w.reg, 2, &[Capability::Read], Projects::All);
    let phone = w.call(caller(&me));
    let mut a = reg_args();
    for k in ["id", "device", "device_id", "subject"] {
        a[k] = json!(other.id);
    }
    dispatch::call(&phone, "push_register", &args(a)).unwrap();
    assert!(w.push.store.record(&me.id).unwrap().reg.is_some());
    assert!(w.push.store.record(&other.id).unwrap().reg.is_none(), "the other device's record is untouched");
    dispatch::call(&phone, "push_unregister", &args(json!({ "id": other.id }))).unwrap();
    assert!(w.push.store.record(&me.id).unwrap().reg.is_none());
}

/// The console has no record to act on, and a device never reaches the
/// console's rows, whatever its grant.
#[test]
fn the_rows_keep_to_their_side() {
    let w = world();
    let d = enroll(&w.reg, 1, &[Capability::Read, Capability::Write, Capability::Admin], Projects::All);
    let desk = w.call(Caller::console());
    for row in ["push_register", "push_unregister", "push_prefs", "push_test"] {
        let e = dispatch::call(&desk, row, &args(reg_args())).unwrap_err();
        assert!(e.as_str().unwrap().contains("console has no device record") || row == "push_test", "{row}: {e}");
    }
    let phone = w.call(caller(&d));
    for row in ["push_key_import", "push_key_remove", "push_status", "push_dry_run", "push_allow_security"] {
        let a = args(json!({ "id": d.id, "allowed": true, "on": true, "p8": "/x", "key_id": "ABC123DEFG", "team_id": "M882BTE3A3" }));
        assert_eq!(dispatch::call(&phone, row, &a), Err(json!("console-only")), "{row}");
    }
    assert!(!w.push.store.record(&d.id).unwrap().security_allowed);
}

/// A mute a narrowed grant no longer covers is not named back to the device.
#[test]
fn a_mute_outside_a_narrowed_grant_is_never_answered() {
    let w = world();
    let d = enroll(&w.reg, 1, &[Capability::Read], Projects::only(["/r/a", "/r/b"]));
    let phone = w.call(caller(&d));
    let got = dispatch::call(&phone, "push_prefs", &args(json!({ "muted": [{ "repo": "/r/a" }, { "repo": "/r/b", "slug": "x" }] }))).unwrap();
    assert_eq!(got["muted"], json!([{ "repo": "/r/a" }, { "repo": "/r/b", "slug": "x" }]));
    w.reg.set_scope(&d.id, Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::only(["/r/a"]) }).unwrap();
    let got = dispatch::call(&phone, "push_prefs", &args(json!({}))).unwrap();
    assert_eq!(got["muted"], json!([{ "repo": "/r/a" }]));
}

/// Only the owner's devices may be allowed security alerts (§2.8 in memory).
#[test]
fn only_the_owners_devices_may_hear_security_alerts() {
    use super::super::devices::{DevicesFile, PrincipalRecord};
    let plain = DeviceRegistry::in_memory();
    let d = enroll(&plain, 1, &[Capability::Read], Projects::All);
    let mut f: DevicesFile = plain.snapshot().unwrap();
    let bob = PrincipalId::new("p_bob");
    f.principals.push(PrincipalRecord { id: bob.clone(), name: "bob".into(), caps: Caps::ALL, projects: Projects::All, created_at: NOW });
    f.devices[0].principal = bob;
    let w = world_on(None, Arc::new(DeviceRegistry::from_file_for_test(f)));
    assert_eq!(w.push.allow_security(&d.id, true).unwrap_err(), "security alerts go only to the owner's devices");
    assert!(!w.push.store.record(&d.id).unwrap().security_allowed);
}

// ── the secrets ──────────────────────────────────────────────────────────────

fn assert_clean(what: &str, s: &str) {
    for secret in [TOKEN, KEY, KEY_ID] {
        assert!(!s.contains(secret), "{what} carries a secret: {s}");
    }
}

/// The token and the key are in the push file, `0600`, and nowhere else: not
/// `devices.json`, a row's answer, an event, an audit line, a checkpoint or a
/// `{:?}`. Driven for real through the disk registry and the real log.
#[test]
fn the_token_and_key_reach_only_their_file() {
    let dir = scratch("secrets");
    let reg = Arc::new(DeviceRegistry::open(&dir));
    let events: Arc<Mutex<Vec<DeviceEvent>>> = Default::default();
    let seen = events.clone();
    reg.subscribe(Arc::new(move |e| seen.lock().unwrap().push(e.clone())));
    let (log, _) = Audit::open(&dir, None, Fleet::of(&reg));
    log.hear(&reg);
    let w = world_on(Some(dir.clone()), reg.clone());
    let d = enroll(&reg, 1, &[Capability::Read], Projects::All);
    let phone = w.call(caller(&d));
    let desk = w.call(Caller::console());
    let con = audit::ConnAudit::new(log.clone(), 1, None, caller(&d));
    let through = |cmd: &str, a: Value| {
        let seen = con.request();
        let out = dispatch::call_seen(&phone, cmd, &args(a), &seen);
        match &out {
            Ok(v) => seen.finish(audit::Outcome::Ok(v)),
            Err(_) => seen.finish(audit::Outcome::Err),
        }
        out
    };
    through("push_register", reg_args()).unwrap();
    through("push_prefs", json!({ "input": false, "muted": [{ "repo": "/r/a" }] })).unwrap();
    through("push_prefs", json!({})).unwrap();

    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(FILE);
    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert!(on_disk.contains(TOKEN) && on_disk.contains(KEY), "the file holds them");
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert_clean("devices.json", &std::fs::read_to_string(dir.join(super::super::devices::FILE)).unwrap());
    let list = serde_json::to_string(&dispatch::call(&desk, "devices_list", &args(json!({}))).unwrap()).unwrap();
    assert_clean("devices_list", &list);
    let status = serde_json::to_string(&dispatch::call(&desk, "push_status", &args(json!({}))).unwrap()).unwrap();
    assert_clean("push_status", &status);
    assert!(status.contains("\"registered\":true") && !status.contains("com.brethash"), "{status}");
    assert_clean("the ok frame's device", &super::super::remote::listener::device_frame(&reg, &caller(&d)).unwrap().to_string());
    assert_clean("checkpoint", &serde_json::to_string(&reg.checkpoint().unwrap()).unwrap());
    let rec = w.push.store.record(&d.id).unwrap();
    assert_clean("a record's Debug", &format!("{rec:?}"));

    // A token Apple retired, the console's act, a revoke: each its event.
    w.push.allow_security(&d.id, true).unwrap();
    w.push.drop_dead(&d.id, TOKEN, "unregistered");
    reg.revoke(&Caller::console(), &Grant::everything(), &d.id, NOW).unwrap();
    for e in events.lock().unwrap().iter() {
        assert_clean("an event", &serde_json::to_string(e).unwrap());
        assert_clean("an event's Debug", &format!("{e:?}"));
    }
    log.stopped();
    let lines = std::fs::read_to_string(dir.join(audit::FILE)).unwrap();
    assert_clean("the audit log", &lines);
    assert!(!lines.contains("/r/a"), "a mute's name is not a record's: {lines}");
    let of = |cmd: &str| lines.lines().filter(|l| l.contains(&format!("\"cmd\":\"{cmd}\""))).collect::<Vec<_>>();
    assert_eq!(of("push_register").len(), 1, "{lines}");
    assert!(of("push_register")[0].contains("\"env\":\"development\""));
    assert_eq!(of("push_prefs").len(), 1, "a read has no record: {lines}");
    assert!(of("push_prefs")[0].contains("\"set\":[\"input\",\"muted\"]") && of("push_prefs")[0].contains("\"muted\":1"), "{lines}");
    assert!(lines.contains("push_token_dropped"), "{lines}");
    assert!(lines.contains("push_security_changed"), "the console's act is a record: {lines}");
    for l in w.lines.lock().unwrap().iter() {
        assert_clean("a serve.log line", l);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `DeviceRecord` did not change: a device file written with a registration on
/// record still reads in a build that refuses unknown fields. This is why the
/// store is its own file (a downgrade refusing every device).
#[test]
fn a_registration_leaves_the_device_file_as_it_was() {
    let dir = scratch("downgrade");
    let reg = Arc::new(DeviceRegistry::open(&dir));
    let w = world_on(Some(dir.clone()), reg.clone());
    let d = w.phone("Phone", 1, Projects::All);
    let f = dir.join(super::super::devices::FILE);
    let before = std::fs::read(&f).unwrap();
    w.push.allow_security(&d.id, true).unwrap();
    w.push.store.register(&d.id, the_reg()).unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), before);
    let v: Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(v["devices"][0].get("push"), None);
    assert_eq!(v["devices"][0].get("security_allowed"), None);
}

/// Revoke or disable drops them at once, whichever path did it, and the file
/// says so too. What the person chose stays through a disable, not a revoke.
#[test]
fn every_way_a_device_stops_being_served_drops_its_token_and_key() {
    let console = Caller::console();
    let all = Grant::everything();
    type Stop<'a> = Box<dyn Fn(&DeviceRegistry, &DeviceRecord) + 'a>;
    let cases: Vec<(&str, Stop, bool)> = vec![
        ("revoke", Box::new(|r, d| { r.revoke(&console, &all, &d.id, NOW).unwrap(); }), true),
        ("revoke its own", Box::new(|r, d| { r.revoke(&caller(d), &d.grant(), &d.id, NOW).unwrap(); }), true),
        ("every device but one", Box::new(|r, _| { r.revoke_all_others(None, NOW).unwrap(); }), true),
        ("console disable", Box::new(|r, d| { r.disable(&d.id, NOW).unwrap(); }), false),
        ("an expiry", Box::new(|r, _| { r.check_expiry(NOW + 40 * 86_400).unwrap(); }), false),
        ("the cross-check", Box::new(|r, _| { r.cross_check(&HashMap::new(), NOW).unwrap(); }), false),
    ];
    for (i, (what, stop, revoked)) in cases.iter().enumerate() {
        let dir = scratch(&format!("drops{i}"));
        let reg = Arc::new(DeviceRegistry::open(&dir));
        let w = world_on(Some(dir.clone()), reg.clone());
        let d = w.phone("Phone", 1, Projects::All);
        w.push.store.set_prefs(&d.id, super::store::PushPrefs { reminder: Some(false), ..Default::default() }).unwrap();
        assert!(std::fs::read_to_string(dir.join(FILE)).unwrap().contains(TOKEN));
        stop(&reg, &d);
        let on_disk = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(!on_disk.contains(TOKEN) && !on_disk.contains(KEY), "{what}: {on_disk}");
        assert!(w.push.store.registered().is_empty(), "{what}");
        if *revoked {
            assert!(!on_disk.contains(d.id.as_str()), "{what}: a tombstone keeps nothing: {on_disk}");
        } else {
            assert!(!w.push.store.record(&d.id).unwrap().reminder, "{what}: the person's choices stay");
        }
    }
}

/// A crash between a revoke and its drop leaves a token behind, and the next
/// boot sweeps it; a disabled device loses its token and keeps its choices; a
/// live one is untouched. No hook runs here: the file is planted as a crash
/// would leave it.
#[test]
fn the_boot_sweep_removes_what_a_crash_left() {
    let dir = scratch("sweep");
    let reg = Arc::new(DeviceRegistry::open(&dir));
    let live = enroll(&reg, 1, &[Capability::Read], Projects::All);
    let paused = enroll(&reg, 2, &[Capability::Read], Projects::All);
    let revoked = enroll(&reg, 3, &[Capability::Read], Projects::All);
    reg.disable(&paused.id, NOW).unwrap();
    reg.revoke(&Caller::console(), &Grant::everything(), &revoked.id, NOW).unwrap();
    let entry = json!({ "reg": { "token": TOKEN, "env": "development", "topic": "com.brethash.worktrees.ios", "key": KEY, "key_id": KEY_ID }, "input": true, "reminder": false, "security": false, "security_allowed": false, "muted": [] });
    let planted = json!({ "version": 1, "devices": { live.id.as_str(): entry, paused.id.as_str(): entry, revoked.id.as_str(): entry, "d_gone": entry } });
    std::fs::write(dir.join(FILE), planted.to_string()).unwrap();

    let w = world_on(Some(dir.clone()), reg.clone());
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(FILE)).unwrap()).unwrap();
    let held = on_disk["devices"].as_object().unwrap();
    assert_eq!(held.len(), 2, "the revoked and the forgotten are gone: {on_disk}");
    assert!(held[live.id.as_str()]["reg"].is_object(), "a live device is untouched");
    assert!(held[paused.id.as_str()].get("reg").is_none(), "a paused device loses its token");
    assert_eq!(held[paused.id.as_str()]["reminder"], false, "and keeps what it chose");
    assert!(w.lines.lock().unwrap().iter().any(|l| l.contains("dropped what 3 device(s)")), "{:?}", w.lines.lock().unwrap());
}

/// A device file nobody can read says nothing about who is live, so it erases
/// nothing here; and what it says at a tick is that nobody is, so nothing goes.
#[test]
fn an_unreadable_device_file_sweeps_nothing_and_sends_nothing() {
    let dir = scratch("unreadable-devices");
    std::fs::write(dir.join(super::super::devices::FILE), "{ not json").unwrap();
    let entry = json!({ "reg": { "token": TOKEN, "env": "development", "topic": "com.brethash.worktrees.ios", "key": KEY, "key_id": KEY_ID }, "input": true, "reminder": true, "security": false, "security_allowed": false, "muted": [] });
    let planted = json!({ "version": 1, "devices": { "d_x": entry } });
    std::fs::write(dir.join(FILE), planted.to_string()).unwrap();
    let reg = Arc::new(DeviceRegistry::open(&dir));
    let w = world_on(Some(dir.clone()), reg);
    assert!(std::fs::read_to_string(dir.join(FILE)).unwrap().contains(TOKEN), "not erased");
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 40);
    assert!(w.kinds().is_empty());
}

/// The push file unreadable: nothing is sent, a line says so once, a change is
/// refused rather than overwriting it, the device itself is served as before,
/// and repairing the file is enough.
#[test]
fn an_unreadable_push_file_stops_sends_and_nothing_else() {
    let dir = scratch("unreadable-push");
    let reg = Arc::new(DeviceRegistry::open(&dir));
    let d = enroll(&reg, 1, &[Capability::Read], Projects::All);
    std::fs::write(dir.join(FILE), "{ not json").unwrap();
    {
        let w = world_on(Some(dir.clone()), reg.clone());
        w.wait(&[path("/r/a", "ui")]);
        w.run(0, 40);
        assert!(w.kinds().is_empty(), "nothing is sent");
        let phone = w.call(caller(&d));
        assert_eq!(dispatch::call(&phone, "push_register", &args(reg_args())), Err(json!(super::store::UNAVAILABLE)));
        assert_eq!(dispatch::call(&phone, "push_prefs", &args(json!({}))), Err(json!(super::store::UNAVAILABLE)));
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), "{ not json", "never overwritten");
        assert!(dispatch::call(&phone, "get_changelog", &args(json!({}))).is_ok(), "the device is served");
        assert_eq!(reg.admit(&d.spki_sha256, NOW, None).ok(), Some(caller(&d)), "and may connect");
        let status = dispatch::call(&w.call(Caller::console()), "push_status", &args(json!({}))).unwrap();
        assert_eq!(status["store_readable"], false);
        std::fs::remove_file(dir.join(FILE)).unwrap();
        dispatch::call(&phone, "push_register", &args(reg_args())).unwrap();
        let said = w.lines.lock().unwrap().clone();
        assert!(said.iter().all(|l| !l.contains("not json")), "{said:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ── the sender ───────────────────────────────────────────────────────────────

/// Every kind, on the wire, is what `docs/push-fixtures/payloads.json` says:
/// the plaintext the phone opens, the clear part Apple sees, the headers.
#[test]
fn every_kind_is_what_the_contract_says() {
    let fx: Value = serde_json::from_str(include_str!("../../../../../docs/push-fixtures/payloads.json")).unwrap();
    let check = |w: &World, i: usize, name: &str, t: i64| {
        let want = &fx[name];
        let body = w.body(i);
        assert_eq!(body["aps"], want["aps"], "{name}: what Apple sees");
        if want["plaintext"].is_null() {
            assert_eq!(body.get("wt"), None, "{name}");
        } else {
            assert_eq!(w.opened(i), want["plaintext"], "{name}: what the phone opens");
        }
        let sent = w.wire.sent.lock().unwrap();
        let r = &sent[i];
        assert_eq!(r.priority.to_string(), want["headers"]["apns-priority"].as_str().unwrap(), "{name}");
        assert_eq!(r.expiration - (NOW + t), want["expires_in"].as_i64().unwrap(), "{name}");
        assert_eq!(r.collapse.is_some(), want["collapse"].as_bool().unwrap(), "{name}");
    };

    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 900);
    assert_eq!(w.kinds(), ["input", "reminder"]);
    check(&w, 0, "input", 30);
    check(&w, 1, "reminder", 900);
    let collapses: Vec<Option<String>> = w.wire.sent.lock().unwrap().iter().map(|r| r.collapse.clone()).collect();
    assert_eq!(collapses[0], collapses[1], "a reminder replaces its first notification");
    w.wait(&[]);
    w.run(901, 1000);
    assert_eq!(w.kinds(), ["input", "reminder", "badge"]);
    check(&w, 2, "badge", 901);

    let w = world();
    w.phone("Phone", 1, Projects::All);
    w.wait(&[path("/r/a", "p1"), path("/r/a", "p2")]);
    w.run(0, 30);
    check(&w, 0, "several", 30);
    let w = world();
    w.phone("Phone", 1, Projects::All);
    w.wait(&(1..=7).map(|i| path("/r/a", &format!("p{i}"))).collect::<Vec<_>>());
    w.run(0, 30);
    check(&w, 0, "several_with_count", 30);

    let w = world();
    let me = w.phone("Phone", 1, Projects::All);
    w.push.allow_security(&me.id, true).unwrap();
    let ipad = enroll_named(&w.reg, "iPad", 2, &[Capability::Read], Projects::All);
    w.at(0);
    check(&w, 0, "security_enrolled", 0);
    let s = Subject { principal: PrincipalId::owner(), device: ipad.id.clone() };
    w.reg.announce(DeviceEvent::Disabled { subject: s.clone(), reason: DisabledReason::Idle, by: super::super::devices::Actor::Serve });
    w.at(60);
    check(&w, 1, "security_disabled", 60);
    let ipad = enroll_named(&w.reg, "iPad", 3, &[Capability::Read], Projects::All);
    w.at(61);
    let s = Subject { principal: PrincipalId::owner(), device: ipad.id };
    w.at(120); // the second enrolled.
    w.reg.announce(DeviceEvent::ConnectRefused { subject: Some(s.clone()), spki_sha256: "e".repeat(64), why: ConnectRefusal::Revoked, peer: None });
    w.at(180);
    check(&w, 3, "security_refused", 180);
    w.reg.announce(DeviceEvent::NewAddressClass { subject: s, from: "tailnet".into(), to: "lan".into() });
    w.at(240);
    check(&w, 4, "security_address", 240);

    let w = world();
    let d2 = w.phone("Phone", 1, Projects::All);
    let phone = w.call(caller(&d2));
    assert_eq!(dispatch::call(&phone, "push_test", &args(json!({}))), Ok(json!({ "sent": true })));
    check(&w, 0, "test", 0);
    let _ = d;
}

/// A place outside the grant never notifies, by name or by count: not a push,
/// not a name in a `several`, not the badge. A narrowing applies at the next
/// tick. This is the projection `sessions:busy` gets, not a second one.
#[test]
fn a_place_outside_the_grant_never_notifies() {
    let w = world();
    w.phone("Phone", 1, Projects::only(["/r/a"]));
    let (ours, theirs) = (path("/r/a", "ui"), path("/r/b", "secret"));
    w.wait(&[theirs.clone()]);
    w.run(0, 100);
    assert!(w.kinds().is_empty(), "only theirs waits: {:?}", w.kinds());
    w.wait(&[theirs.clone(), ours]);
    w.run(101, 131);
    assert_eq!(w.kinds(), ["input"], "one of two is ours: not a `several`");
    assert_eq!(w.body(0)["aps"]["badge"], 1, "the count is ours alone");
    let p = w.opened(0);
    assert_eq!(p["repo"], "/r/a");
    assert!(!p.to_string().contains("secret"));
}

#[test]
fn a_narrowed_grant_and_a_device_without_read_hear_nothing_more() {
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    w.reg.set_scope(&d.id, Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::only(["/r/b"]) }).unwrap();
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 60);
    assert!(w.kinds().is_empty(), "narrowed away");
    w.reg.set_scope(&d.id, Grant { capabilities: Caps::NONE, projects: Projects::All }).unwrap();
    w.run(61, 120);
    assert!(w.kinds().is_empty(), "no read: not even in scope");
}

/// Muted is silent and uncounted, a project or one place, and an unmute is
/// heard. A push's `muted` matching is on the place's own project and slug.
#[test]
fn muted_is_silent_and_uncounted() {
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    let mute = |m: Value| {
        let phone = w.call(caller(&d));
        dispatch::call(&phone, "push_prefs", &args(json!({ "muted": m }))).unwrap();
    };
    mute(json!([{ "repo": "/r/a" }]));
    w.wait(&[path("/r/a", "ui"), path("/r/a", "other")]);
    w.run(0, 100);
    assert!(w.kinds().is_empty(), "a muted project");
    mute(json!([{ "repo": "/r/a", "slug": "ui" }]));
    w.run(101, 102);
    assert_eq!(w.kinds(), ["input"], "only the unmuted place");
    assert_eq!(w.opened(0)["slug"], "other");
    assert_eq!(w.body(0)["aps"]["badge"], 1, "muted is not in the badge");
}

#[test]
fn the_switches_are_the_devices() {
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    w.push.store.set_prefs(&d.id, super::store::PushPrefs { input: Some(false), ..Default::default() }).unwrap();
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 1000);
    assert!(w.kinds().is_empty());
}

/// A phone's `security` switch cannot enable alerts, and without the console's
/// word none is sent, whatever fires. `audit:alert` stays console-only: this is
/// the sender's own path, off the event's.
#[test]
fn security_alerts_need_the_console() {
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    let phone = w.call(caller(&d));
    assert_eq!(dispatch::call(&phone, "push_prefs", &args(json!({ "security": true }))), Err(json!("denied")));
    enroll_named(&w.reg, "iPad", 2, &[Capability::Read], Projects::All);
    w.run(0, 100);
    assert!(w.kinds().is_empty(), "not allowed: no alert");
    // Allowed, an event that is not an alert stays silent, and the console's
    // own pause is one of those (`DeviceEvent::alerts`).
    w.push.allow_security(&d.id, true).unwrap();
    let other = enroll_named(&w.reg, "Mac", 3, &[Capability::Read], Projects::All);
    w.run(101, 102);
    assert_eq!(w.kinds(), ["security"], "enrolled alerts");
    w.reg.disable(&other.id, NOW).unwrap();
    w.run(200, 300);
    assert_eq!(w.kinds(), ["security"], "a console pause does not");
    // The phone can turn it off, and the console can stop it.
    dispatch::call(&phone, "push_prefs", &args(json!({ "security": false }))).unwrap();
    enroll_named(&w.reg, "Mac2", 4, &[Capability::Read], Projects::All);
    w.run(400, 500);
    assert_eq!(w.kinds(), ["security"], "its own switch off");
}

/// Dead tokens, through the sender: dropped, recorded through the registry's
/// event, and the phone registers again. Only a dead one drops.
#[test]
fn a_dead_token_is_dropped_and_said() {
    for (how, gone) in [(Delivery::Dead("unregistered"), true), (Delivery::Dead("bad-device-token"), true), (Delivery::Dead("token-not-for-topic"), true), (Delivery::Retry("status-503"), false), (Delivery::Rejected("bad-header"), false)] {
        let w = world();
        let events: Arc<Mutex<Vec<DeviceEvent>>> = Default::default();
        let seen = events.clone();
        w.reg.subscribe(Arc::new(move |e| seen.lock().unwrap().push(e.clone())));
        let d = w.phone("Phone", 1, Projects::All);
        *w.wire.answer.lock().unwrap() = Some(how.clone());
        w.wait(&[path("/r/a", "ui")]);
        w.run(0, 30);
        assert_eq!(w.push.store.record(&d.id).unwrap().reg.is_none(), gone, "{how:?}");
        let said = events.lock().unwrap().iter().any(|e| matches!(e, DeviceEvent::PushTokenDropped { .. }));
        assert_eq!(said, gone, "{how:?}");
    }
}

/// A push in the air when the phone registers a new token: Apple's verdict on
/// the old one must not drop the new.
#[test]
fn a_verdict_on_an_old_token_leaves_a_new_one() {
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    assert!(!w.push.store.drop_token(&d.id, "feed").unwrap());
    assert!(w.push.store.record(&d.id).unwrap().reg.is_some());
    assert!(w.push.store.drop_token(&d.id, TOKEN).unwrap());
    assert!(!w.push.store.drop_token(&d.id, TOKEN).unwrap(), "once");
}

/// The kill switch: remote off, and nothing leaves for a device. Off when a
/// prompt comes due, nothing; back on, it is due; a send in the air is stopped.
#[test]
fn the_kill_switch_stops_pushes_and_what_is_in_the_air() {
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    w.on.store(false, Ordering::SeqCst);
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 100);
    assert!(w.kinds().is_empty(), "off: nothing leaves");
    let phone = w.call(caller(&d));
    let _ = phone;
    w.on.store(true, Ordering::SeqCst);
    w.at(101);
    assert_eq!(w.kinds(), ["input"], "on again: still due");

    let w = world();
    w.phone("Phone", 1, Projects::All);
    w.wire.hang.store(true, Ordering::SeqCst);
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 30);
    assert_eq!(w.push.st.lock().unwrap().air.len(), 1, "in the air");
    w.push.kill();
    assert_eq!(w.push.st.lock().unwrap().air.len(), 0, "stopped");
    // Not told after all: it goes when sends can, once more.
    w.wire.hang.store(false, Ordering::SeqCst);
    w.at(31);
    assert_eq!(w.kinds(), ["input", "input"]);
}

/// A revoke while a push is in the air stops it: no later send can carry the
/// dropped token, and its entry is gone.
#[test]
fn a_revoke_stops_a_send_in_the_air() {
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    w.wire.hang.store(true, Ordering::SeqCst);
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 30);
    assert_eq!(w.push.st.lock().unwrap().air.len(), 1);
    w.reg.revoke(&Caller::console(), &Grant::everything(), &d.id, NOW).unwrap();
    assert_eq!(w.push.st.lock().unwrap().air.len(), 0, "stopped at once");
    w.wire.hang.store(false, Ordering::SeqCst);
    w.run(31, 200);
    assert_eq!(w.kinds(), ["input"], "and nothing after");
}

/// §5.1: while the audit log cannot take its counts, nothing is sent. The wait
/// still runs, so a place keeps its place in line.
#[test]
fn an_unhealthy_audit_log_stops_sends() {
    let w = world();
    w.phone("Phone", 1, Projects::All);
    w.healthy.store(false, Ordering::SeqCst);
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 40);
    assert!(w.kinds().is_empty());
    w.healthy.store(true, Ordering::SeqCst);
    w.at(41);
    assert_eq!(w.kinds(), ["input"]);
    assert_eq!(w.opened(0)["waited_s"], 41, "its wait began when the prompt did");
}

/// A stale entry the registry no longer serves never reaches Apple, even with
/// the drop hook out of the picture.
#[test]
fn the_registry_is_asked_before_every_send() {
    let w = world();
    let d = enroll(&w.reg, 1, &[Capability::Read], Projects::All);
    w.reg.disable(&d.id, NOW).unwrap();
    // Registered after the disable's hook ran: as a race would leave it.
    w.push.store.register(&d.id, the_reg()).unwrap();
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 40);
    assert!(w.kinds().is_empty());
    // An alert is the case no grant cuts: only the registry's word stops it.
    w.push.allow_security(&d.id, true).unwrap();
    enroll_named(&w.reg, "iPad", 2, &[Capability::Read], Projects::All);
    w.run(41, 200);
    assert!(w.kinds().is_empty(), "{:?}", w.kinds());
}

/// A slow Apple stalls nothing: a tick with a push hanging in the air returns,
/// another device still gets its own, and that device's next waits its turn.
#[test]
fn a_hung_send_stalls_nothing_else() {
    let w = world();
    w.phone("One", 1, Projects::All);
    w.wire.hang.store(true, Ordering::SeqCst);
    w.wait(&[path("/r/a", "ui")]);
    let t0 = std::time::Instant::now();
    w.run(0, 200);
    assert_eq!(w.kinds(), ["input"], "one in the air per device, however long");
    assert!(t0.elapsed() < std::time::Duration::from_secs(5), "ticks returned at once: {:?}", t0.elapsed());
    w.phone("Two", 2, Projects::All);
    w.at(201);
    assert_eq!(w.kinds(), ["input", "input"], "another device is not held up");
}

/// Sends become counts in the audit, ten minutes apart and only when something
/// happened. A count has no names, tokens or keys.
#[test]
fn sends_are_counted_not_named() {
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    w.wait(&[path("/r/a", "secret-place")]);
    w.run(0, 30);
    w.push.flush_tally(NOW + 31);
    w.push.flush_tally(NOW + 31 + super::TALLY_SECS);
    let t = w.tallies.lock().unwrap().clone();
    assert_eq!(t.len(), 1, "{t:?}");
    assert_eq!(t[0], json!([{ "device": d.id, "sent": 1, "badge": 0, "failed": 0, "suppressed": 0 }]));
    w.push.flush_tally(NOW + 31 + 2 * super::TALLY_SECS);
    assert_eq!(w.tallies.lock().unwrap().len(), 1, "nothing new, no record");
}

/// `push_test`: its own answers, and it spends the hour.
#[test]
fn push_test_answers_how_it_ended() {
    let w = world();
    let d = enroll(&w.reg, 1, &[Capability::Read], Projects::All);
    let phone = w.call(caller(&d));
    let test = || dispatch::call(&phone, "push_test", &args(json!({})));
    assert_eq!(test(), Err(json!("not-registered")));
    w.push.store.register(&d.id, the_reg()).unwrap();
    assert_eq!(test(), Ok(json!({ "sent": true })));
    *w.wire.answer.lock().unwrap() = Some(Delivery::Retry("status-503"));
    assert_eq!(test(), Err(json!(super::TEST_FAILED)));
    *w.wire.answer.lock().unwrap() = Some(Delivery::Dead("unregistered"));
    assert_eq!(test(), Err(json!("not-registered")), "and it is dropped");
    assert!(w.push.store.record(&d.id).unwrap().reg.is_none());
    *w.wire.answer.lock().unwrap() = None;
    w.push.store.register(&d.id, the_reg()).unwrap();
    let n = (0..super::plan::HOURLY + 5).filter(|_| test().is_ok()).count();
    assert_eq!(n, super::plan::HOURLY - 3, "the flood reached the hour's cap, three spent above");
    assert_eq!(test(), Err(json!("rate")));
}

/// No key, no dry run: `not-configured`, and a tick sends nothing.
#[test]
fn with_no_key_nothing_is_sent() {
    let dir = scratch("nokey");
    let reg = Arc::new(DeviceRegistry::open(&dir));
    let w = world_on(Some(dir.clone()), reg);
    *w.push.test_transport.lock().unwrap() = None;
    let d = w.phone("Phone", 1, Projects::All);
    let phone = w.call(caller(&d));
    assert_eq!(dispatch::call(&phone, "push_test", &args(json!({}))), Err(json!(super::NOT_CONFIGURED)));
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 40);
    assert!(w.push.st.lock().unwrap().air.is_empty());
    assert!(!w.push.ready());
    // The console's dry run: the whole path, then a line, and the console alone
    // turns it on.
    let desk = w.call(Caller::console());
    assert_eq!(dispatch::call(&phone, "push_dry_run", &args(json!({ "on": true }))), Err(json!("console-only")));
    dispatch::call(&desk, "push_dry_run", &args(json!({ "on": true }))).unwrap();
    w.at(41);
    assert_eq!(dispatch::call(&phone, "push_test", &args(json!({}))), Ok(json!({ "sent": true })));
    let lines = w.lines.lock().unwrap().clone();
    let dry: Vec<&String> = lines.iter().filter(|l| l.contains("dry run: kind=")).collect();
    assert_eq!(dry.len(), 2, "{lines:?}");
    assert!(dry[0].contains("kind=input") && dry[0].contains("sealed=true") && dry[0].contains(KEY_ID), "{lines:?}");
    assert!(dry[1].contains("kind=test"), "{lines:?}");
    for l in &lines {
        assert_clean("a line", &l.replace(KEY_ID, "the-key-id"));
        assert!(!l.contains("/r/a") && !l.contains("Studio Mac") && !l.contains("Claude"), "{l}");
    }
}

/// The console's key rows, end to end: the import is usable and `push_status`
/// says only that one is there.
#[test]
fn the_key_rows_say_only_that_a_key_is_there() {
    let dir = scratch("keyrows");
    let reg = Arc::new(DeviceRegistry::open(&dir));
    let w = world_on(Some(dir.clone()), reg);
    *w.push.test_transport.lock().unwrap() = None;
    let desk = w.call(Caller::console());
    let p8 = dir.join("AuthKey_ABC123DEFG.p8");
    let (pem, _) = super::key::test_p8();
    std::fs::write(&p8, pem).unwrap();
    let got = dispatch::call(&desk, "push_key_import", &args(json!({ "p8": p8, "key_id": "ABC123DEFG", "team_id": "M882BTE3A3" }))).unwrap();
    assert_eq!(got["configured"], true);
    let s = dispatch::call(&desk, "push_status", &args(json!({}))).unwrap().to_string();
    assert!(!s.contains("ABC123DEFG") && !s.contains("M882BTE3A3") && !s.contains("PRIVATE"), "{s}");
    assert!(w.push.ready());
    let got = dispatch::call(&desk, "push_key_remove", &args(json!({}))).unwrap();
    assert_eq!(got["configured"], false);
    assert!(!w.push.ready());
}

/// The whole path through real HTTP/2 and TLS against the local fixture: the
/// sealed body, the collapse id, the sandbox host, and a 410 that drops the
/// token. Tests never reach Apple: this `Dial` names the fixture.
#[test]
fn the_whole_path_over_http2_to_the_fixture() {
    use apns::fixture::{start, Answer};
    let w = world();
    let d = w.phone("Phone", 1, Projects::All);
    let server = w.rt.block_on(start(vec![Answer::Status(200, ""), Answer::Status(410, "Unregistered")]));
    let dir = scratch("h2");
    let p8 = dir.join("k.p8");
    std::fs::write(&p8, super::key::test_p8().0).unwrap();
    let key = Arc::new(super::key::import(&dir, &p8, "ABC123DEFG", "M882BTE3A3").unwrap());
    let h2 = apns::H2::new(key, apns::Dial::fixture(server.at, Some(server.roots.clone())));
    w.push.use_transport(Arc::new(h2));
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 30);
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let s = &seen[0];
    assert_eq!(s.authority, apns::SANDBOX);
    assert_eq!(s.path, format!("/3/device/{TOKEN}"));
    assert_eq!(s.headers["apns-push-type"], "alert");
    assert_eq!(s.headers["apns-topic"], "com.brethash.worktrees.ios");
    assert_eq!(s.headers["apns-priority"], "10");
    let key = crate::serve::stepup::b64_decode(KEY).unwrap();
    let key: [u8; 32] = key.try_into().unwrap();
    assert_eq!(s.headers["apns-collapse-id"], super::seal::collapse_id(&key, "/r/a", "ui"));
    assert_eq!(s.headers["apns-expiration"], (NOW + 30 + 3600).to_string());
    let body: Value = serde_json::from_slice(&s.body).unwrap();
    assert_eq!(open(&body)["kind"], "input");
    let clear = String::from_utf8(s.body.clone()).unwrap();
    assert!(!clear.contains("/r/a") && !clear.contains("Studio Mac"));
    // The reminder comes back 410.
    w.run(31, 900);
    assert_eq!(server.seen.lock().unwrap().len(), 2);
    assert!(w.push.store.record(&d.id).unwrap().reg.is_none(), "a 410 drops the token");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A trust failure says so in the log, distinctly, once.
#[test]
fn a_trust_failure_is_named_in_the_log() {
    let w = world();
    w.phone("Phone", 1, Projects::All);
    *w.wire.answer.lock().unwrap() = Some(Delivery::Retry("trust"));
    w.wait(&[path("/r/a", "ui")]);
    w.run(0, 200);
    let lines = w.lines.lock().unwrap().clone();
    let hits: Vec<&String> = lines.iter().filter(|l| l.contains("does not chain to the roots")).collect();
    assert_eq!(hits.len(), 1, "{lines:?}");
    assert!(w.kinds().len() > 1, "it kept trying, at its pace");
}

#[test]
fn standing_sweep_leaves_a_broken_file_alone() {
    let dir = scratch("broken-sweep");
    std::fs::write(dir.join(FILE), "nope").unwrap();
    let s = Store::open(&dir);
    assert_eq!(s.sweep(|_| Standing::Gone), 0);
    assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), "nope");
    let _ = std::fs::remove_dir_all(&dir);
}

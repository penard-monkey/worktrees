//! The push rows' bodies.
//!
//! **A device acts on its own record only.** Its identity is the connection's
//! (`Call::caller`), set by the listener from the certificate that passed;
//! there is no device argument to forge. The rows are `Own`: any paired device,
//! whatever its grant, Observe included. Nothing here reads or changes anything
//! outside the caller's record, and what a push later says is cut to the
//! device's grant when it is sent, so holding a registration reaches nothing a
//! `sessions:busy` frame would not. They ask no step-up: they drive no agent,
//! and Turn On must not open Face ID.
//!
//! Errors are the phone's words: `bad-args…` it replaces with its own, `denied`
//! is `resolve::ERR_DENIED`, and neither ever quotes a value sent.

use serde_json::{json, Value};
use worktrees_core::authz::{self, Device, DeviceId};

use super::store::{Muted, PushEnv, PushPrefs, PushReg};
use super::super::dispatch::{Args, Call};
use super::super::resolve;
use super::seal::{KEY_ID_LEN, KEY_LEN};
use super::{NOT_CONFIGURED, NOT_REGISTERED};

/// An APNs token: 32 bytes today, and Apple says up to 100.
const TOKEN_HEX: std::ops::RangeInclusive<usize> = 64..=200;
/// The only topics this host sends to: the app and its preview build. The team's
/// key signs for every app on the team, and a paired device must not be able to
/// point this host at another of them.
pub const TOPICS: &[&str] = &["com.brethash.worktrees.ios", "com.brethash.worktrees.ios.preview"];
const MAX_MUTED: usize = 64;
const MAX_REPO: usize = 4096;

fn bad(what: &str) -> String {
    format!("bad-args: {what}")
}

fn me(cx: &Call) -> Result<&DeviceId, String> {
    match cx.caller.device() {
        Device::Paired(d) => Ok(d),
        Device::Console => Err("the console has no device record; these rows are a paired device's".into()),
    }
}

fn str_arg(a: &Args, key: &str) -> Result<String, String> {
    a.raw(key).as_str().map(str::to_string).ok_or_else(|| bad(key))
}

/// The arguments as a registration. Nothing is echoed: a token or a key in an
/// error would reach a log the moment anything logged it.
fn registration(a: &Args) -> Result<PushReg, String> {
    let token = str_arg(a, "token")?;
    if !TOKEN_HEX.contains(&token.len()) || token.len() % 2 != 0 || !token.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(bad("token"));
    }
    let env = PushEnv::parse(&str_arg(a, "env")?).ok_or_else(|| bad("env"))?;
    let topic = str_arg(a, "topic")?;
    if !TOPICS.contains(&topic.as_str()) {
        return Err(bad("topic"));
    }
    let key = str_arg(a, "key")?;
    if super::super::stepup::b64_decode(&key).is_none_or(|k| k.len() != KEY_LEN) {
        return Err(bad("key"));
    }
    let key_id = str_arg(a, "key_id")?;
    if super::super::stepup::b64_decode(&key_id).is_none_or(|k| k.len() != KEY_ID_LEN) {
        return Err(bad("key_id"));
    }
    Ok(PushReg { token, env, topic, key, key_id })
}

/// After a write to the store. A revoke or disable that landed while this request
/// was on its way has already swept the entry, and the write just put it back: put
/// the entry in the state that device's standing calls for, so nothing waits for
/// the next boot sweep. A disable keeps what the person chose.
fn still_served(cx: &Call, p: &super::Push, id: &DeviceId) -> Result<(), String> {
    if cx.registry.device_grant(cx.caller.principal(), id).is_some() {
        return Ok(());
    }
    p.store.drop_for(id, p.standing(id));
    Err(resolve::ERR_DENIED.into())
}

pub fn register(cx: &Call, a: &Args) -> Result<Value, String> {
    let id = me(cx)?;
    let reg = registration(a)?;
    let p = service(cx)?;
    p.store.register(id, reg)?;
    still_served(cx, p, id)?;
    Ok(json!({ "registered": true }))
}

pub fn unregister(cx: &Call) -> Result<Value, String> {
    service(cx)?.store.unregister(me(cx)?)?;
    Ok(json!({ "registered": false }))
}

/// The argument names a `push_prefs` frame sets: what makes it a change rather
/// than a read, for the body, the audit record and the budget alike. A `null`
/// sets nothing.
pub fn prefs_sets(a: &Args) -> Vec<&'static str> {
    ["input", "reminder", "security", "muted"].into_iter().filter(|k| !a.raw(k).is_null()).collect()
}

fn roots(cx: &Call) -> Vec<String> {
    match &cx.world {
        Some(w) => w.roots(),
        None => worktrees_core::registry::read_lenient().roots(),
    }
}

/// `a`'s `muted`, shape-checked, with whatever the grant does not cover
/// dropped: "a muted project the grant doesn't cover is dropped, not an error."
fn muted_arg(cx: &Call, a: &Args) -> Result<Option<Vec<Muted>>, String> {
    let raw = a.raw("muted");
    if raw.is_null() {
        return Ok(None);
    }
    let items = raw.as_array().ok_or_else(|| bad("muted"))?;
    if items.len() > MAX_MUTED {
        return Err(bad("muted"));
    }
    let grant = authz::effective(&*cx.registry, &cx.caller);
    let roots = roots(cx);
    let mut out: Vec<Muted> = Vec::new();
    for m in items {
        let o = m.as_object().ok_or_else(|| bad("muted"))?;
        if o.keys().any(|k| k != "repo" && k != "slug") {
            return Err(bad("muted"));
        }
        let repo = o.get("repo").and_then(Value::as_str).filter(|r| r.len() <= MAX_REPO).ok_or_else(|| bad("muted"))?;
        let slug = match o.get("slug") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.as_str()),
            _ => return Err(bad("muted")),
        };
        if !roots.iter().any(|r| r == repo) || !grant.projects.covers_root(repo) {
            continue;
        }
        if slug.is_some_and(|s| resolve::slug_rule(s).is_err()) {
            continue;
        }
        let m = Muted { repo: repo.to_string(), slug: slug.map(str::to_string) };
        if !out.contains(&m) {
            out.push(m);
        }
    }
    Ok(Some(out))
}

fn bool_arg(a: &Args, key: &str) -> Result<Option<bool>, String> {
    match a.raw(key) {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(b)),
        _ => Err(bad(key)),
    }
}

/// No arguments reads; each of `input`, `reminder`, `security`, `muted` given
/// is set. The answer is always the whole record. `security` is `null` until
/// the console has allowed it for this device, and setting it then is `denied`.
/// A mute the device's grant has since stopped covering is not named back.
pub fn prefs(cx: &Call, a: &Args) -> Result<Value, String> {
    let id = me(cx)?;
    let p = service(cx)?;
    if !prefs_sets(a).is_empty() {
        let change = PushPrefs { input: bool_arg(a, "input")?, reminder: bool_arg(a, "reminder")?, security: bool_arg(a, "security")?, muted: muted_arg(cx, a)? };
        p.store.set_prefs(id, change)?;
        still_served(cx, p, id)?;
    }
    let rec = p.store.record(id)?;
    let grant = authz::effective(&*cx.registry, &cx.caller);
    let roots = roots(cx);
    let muted: Vec<&Muted> = rec.muted.iter().filter(|m| roots.iter().any(|r| r == &m.repo) && grant.projects.covers_root(&m.repo)).collect();
    Ok(json!({
        "registered": rec.reg.is_some(),
        "input": rec.input,
        "reminder": rec.reminder,
        "security": if rec.security_allowed { json!(rec.security) } else { Value::Null },
        "muted": muted,
    }))
}

pub fn test(cx: &Call) -> Result<Value, String> {
    let id = me(cx)?;
    let Some(p) = cx.push.as_ref() else { return Err(NOT_CONFIGURED.into()) };
    if p.store.record(id)?.reg.is_none() {
        return Err(NOT_REGISTERED.into());
    }
    if !p.ready() {
        return Err(NOT_CONFIGURED.into());
    }
    p.test(id)
}

/// The console's: the service, which every real serve has.
pub fn service(cx: &Call) -> Result<&super::Push, String> {
    cx.push.as_deref().ok_or_else(|| "this serve has no push service".to_string())
}

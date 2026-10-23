//! The 170 commands, each with its exposure, capability and objects, in ONE
//! table.
//!
//! A row is `name: Exposure [capability, object, …] => body`, and the macro
//! makes two things of the list: `TABLE`, the exhaustive slice of [`Row`]s, and
//! the `match` the socket dispatches on. One list means the table cannot say
//! one thing while the dispatcher does another — a command missing here is a
//! request that fails with `unknown command`, never a silent default. The
//! precedent is `projcfg`'s user-only key list: a missing row is a hard error,
//! not a fallback. The desktop app is this table's client: its shim registers
//! only the `ClientLocal` rows (and two commands of its own), and
//! `app/scripts/serve-dispatch-check.mjs` fails if it registers any other row —
//! that would be a second engine for it. `engine-route-check.mjs` holds the
//! page's routing against the `ClientLocal` rows and every `invoke("…")` the
//! page makes against this table, so a command added to the frontend without
//! a row here fails CI rather than failing the app.
//!
//! **Per-object authorization (Phase 2, R1).** Every `Engine` and `Stream` row
//! names the capability it needs and the objects its arguments address
//! (`resolve::Obj`); a `HostConsole` or `Loopback` row may name objects but no
//! capability, and a `ClientLocal` row names neither. `Row::new` refuses any
//! other shape at COMPILE time. Before a body runs, `call` resolves those
//! objects against the grant of the caller's (principal, device) pair, writes
//! their canonical values back into the arguments, and hands the body a `Ctx`
//! narrowed to the same scope, so the api's own guard answers with the
//! caller's roots — two checks (`docs/proposals/server-edition-phase2.md`
//! §3.3, and §3.4 for the row-by-row mapping).
//!
//! Arguments are read by name from the request's `args` object exactly as
//! Tauri reads them: each parameter deserialized on its own, a missing key
//! read as JSON `null` (so an `Option` is `None` and a required `String` is
//! an error), and the key spelled the way the frontend spells it —
//! camelCase, which is Tauri 2's default — or in snake_case, both accepted.
//! Nested types (`UiEvent`, `PlaceBranch`, `ViewerPlace`, `ShellRef`) carry
//! their own serde attributes and are the same types Tauri deserializes from
//! the same JSON, so nothing is re-stated here.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::Arc;
use worktrees_api::{config, files, hostenv, meta, ops, prefs, profiles, project, status, term, viewer, workspace, Ctx};
use worktrees_core::authz::{self, Caller, Capability, Caps, DeviceId, Grant, Projects, Registry};
use worktrees_core::fetchpolicy::Fetcher;

use super::devices::{DeviceRegistry, Expiry};
use super::pairing::{Offer, Pairing};
use super::stepup;
use super::flight::{self, Flights, Join};
use super::resolve::{self, Absent, Obj, World};
use super::{provenance, scope};
use super::wire::{self, Frames, CLIENT_LOCAL};

/// What a command is, seen from a socket: which listener may reach it at all.
/// What it may then do is the row's capability and objects. `HostAdmin`, its
/// Phase 1 tag, is gone: its rows are `admin`-capability rows or console acts
/// now (§3.4), and one row cannot say two things about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposure {
    /// The point of the server: reads, and mutations scoped to a registered
    /// project or place. Served to any authenticated caller whose grant
    /// covers the row's capability and objects.
    Engine,
    /// Acts on the CLIENT's machine — its clipboard, its editor, its
    /// terminal emulator, its window, its app bundle. Refused over the socket
    /// with `err: "client-local"`, never run on the host: `copy_text` would
    /// write the SERVER's clipboard and `open_editor` would open a window
    /// nobody is sitting at. The shim keeps these, and the frontend routes
    /// exactly these names to it (`app/src/ipc.ts`, `engine-route-check.mjs`).
    /// The docs viewer is NOT one of them: its body registers a place with the
    /// host's documentation server and returns a URL, and the browser is opened
    /// by the frontend's own `openUrl` — so the registrations live here, where
    /// the host poll refreshes them.
    ClientLocal,
    /// The console only (`err: "console-only"` for anyone else): every act
    /// that can widen what this host's agents may do, administers the host
    /// itself, or replaces the binary serving the request (§3.4, decision 6).
    /// "A remote device can drive work; it cannot change what this machine's
    /// agents are allowed to do."
    HostConsole,
    /// The loopback listener only, but not a console ACT: per-machine data
    /// that never leaves the machine (`ui_events_append`, §III.6.2).
    Loopback,
    /// Owns or drives a byte stream: a tmux attach or a dock shell. Served,
    /// with its bytes going back as channel frames on the same connection, and
    /// laned per stream (`lane`).
    Stream,
    /// What any authenticated caller may always do for ITSELF, whatever its
    /// grant (Phase 2 §2.5): revoke its own device, prove a step-up on its own
    /// connection. No capability; the row's objects decide anything further
    /// (`revocable(id)` needs `admin` for a device other than the caller's).
    Own,
}

/// One command: its name, exposure, capability and objects.
#[derive(Debug)]
pub struct Row {
    pub name: &'static str,
    pub exposure: Exposure,
    /// What the caller's grant must hold. `None` exactly for the rows no grant
    /// reaches: `HostConsole`, `Loopback`, `ClientLocal`, and `Own`, which
    /// every caller may make.
    pub cap: Option<Capability>,
    /// What the arguments address, resolved in this order.
    pub objects: &'static [Obj],
}

impl Row {
    /// The shape rule, at compile time: a const panic in `TABLE` is a build
    /// error. Every row declares a capability and an object, or is
    /// `HostConsole`/`Loopback` (objects, no capability) or `ClientLocal`
    /// (neither) — "the macro will not compile otherwise" (§3.3).
    const fn new(name: &'static str, exposure: Exposure, cap: Option<Capability>, objects: &'static [Obj]) -> Row {
        match exposure {
            Exposure::Engine | Exposure::Stream => {
                if cap.is_none() || objects.is_empty() {
                    panic!("an Engine or Stream row declares [capability, object, …]");
                }
            }
            Exposure::HostConsole | Exposure::Loopback | Exposure::Own => {
                if cap.is_some() {
                    panic!("a HostConsole, Loopback or Own row declares no capability: write [console, object, …] or nothing");
                }
            }
            Exposure::ClientLocal => {
                if cap.is_some() || !objects.is_empty() {
                    panic!("a ClientLocal row declares no capability and no object");
                }
            }
        }
        Row { name, exposure, cap, objects }
    }
}

/// What a request is answered with: the engine's `Ctx`, the connection's
/// frame queue for anything the command streams back, and WHO is asking —
/// set by the listener that accepted the socket, never by the request (§3.3).
pub struct Call {
    pub ctx: Ctx,
    pub frames: Frames,
    pub caller: Caller,
    /// Where grants are read, on every request.
    pub registry: Arc<dyn Registry>,
    /// What objects resolve against: `None` is the live registry and tmux
    /// server (`resolve::Live`); serve's in-process tests hand in their own.
    pub world: Option<Arc<dyn World>>,
    /// The device registry the device rows change (`devices.json`). In serve
    /// it is also `registry`; a test may grant from a fixture instead.
    pub devices: Arc<DeviceRegistry>,
    /// The pairing windows the console opens (§2.2).
    pub pairing: Arc<Pairing>,
    /// The shared reads in the air, host-wide (`flight.rs`): every listener's
    /// calls join the same ones.
    pub flights: Arc<Flights>,
}

impl Call {
    /// This call on another `ctx` and `frames`: the same caller, registries,
    /// world and flights. A body's scoped `Ctx` (`call_in_seen`), and a
    /// request's frames (`conn`'s, carrying its attach slot).
    pub fn with(&self, ctx: Ctx, frames: Frames) -> Call {
        Call {
            ctx,
            frames,
            caller: self.caller.clone(),
            registry: self.registry.clone(),
            world: self.world.clone(),
            devices: self.devices.clone(),
            pairing: self.pairing.clone(),
            flights: self.flights.clone(),
        }
    }
}

/// The request's `args`, read one parameter at a time.
#[derive(Debug)]
pub struct Args(pub Map<String, Value>);

impl Args {
    /// Look `key` up as written, then as camelCase. Missing is JSON `null`,
    /// which is what Tauri hands a parameter whose key is absent.
    pub fn raw(&self, key: &str) -> Value {
        if let Some(v) = self.0.get(key) {
            return v.clone();
        }
        let camel = camel(key);
        self.0.get(&camel).cloned().unwrap_or(Value::Null)
    }

    /// One parameter. The error names the key the way Tauri's `invalid args`
    /// does, so a frontend bug reads the same over either transport.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T, String> {
        serde_json::from_value(self.raw(key)).map_err(|e| format!("invalid args `{key}`: {e}"))
    }

    /// Replace one parameter — the resolver's canonical value. Stored under
    /// the snake_case key, which `raw` reads first, and the camelCase spelling
    /// dropped so nothing reads the old value under its other name.
    pub fn set(&mut self, key: &str, v: Value) {
        self.0.remove(&camel(key));
        self.0.insert(key.to_string(), v);
    }

    /// A channel parameter, as the frontend's `Channel` serializes it.
    pub fn channel(&self, key: &str) -> Result<u64, String> {
        wire::channel_id(&self.raw(key)).map_err(|e| format!("invalid args `{key}`: {e}"))
    }
}

/// `show_ignored` → `showIgnored`. Only the first form is ever written in the
/// table, so the mapping needs to go one way.
pub fn camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut up = false;
    for c in snake.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// A command's `Result` as the wire's: both halves serialized.
fn reply<T: Serialize, E: Serialize>(r: Result<T, E>) -> Result<Value, Value> {
    match r {
        Ok(t) => serde_json::to_value(t).map_err(|e| Value::String(format!("serialize: {e}"))),
        Err(e) => Err(serde_json::to_value(e).unwrap_or_else(|e| Value::String(format!("serialize error: {e}")))),
    }
}

/// For bodies that return a plain value rather than a `Result`.
fn ok<T>(t: T) -> Result<T, String> {
    Ok(t)
}

/// The body of a `ClientLocal` row. Never evaluated — the exposure check runs
/// first — but a row needs a body to be a row.
fn refused() -> Result<(), String> {
    Err(CLIENT_LOCAL.into())
}

/// What `diagnostics` is told about the viewer bundle. A server has no bundle
/// dir of its own, so only the `WORKTREES_VIEWER_JS` override can name one —
/// which the desktop app sets when it starts serve, from its `resource_dir`.
fn viewer_path() -> String {
    match viewer::bundle_path(None) {
        Some(p) => p.to_string_lossy().into_owned(),
        None => "(no browser bundle — the Docs tab lists and reads without it)".to_string(),
    }
}

/// `open_docs_viewer`'s precondition, checked BEFORE the place is so much as
/// looked at: serve has no resource dir, so the browser bundle exists only if
/// `WORKTREES_VIEWER_JS` names one. The desktop app sets it when it starts
/// serve; a serve started from the CLI (or the dev stack) does not, and the
/// open must say so rather than fail later — `viewer::open` refuses too, but
/// in words about restarting an app, after the path guard has spoken first.
fn viewer_bundle() -> Result<(), String> {
    if viewer::bundle_path(None).is_some() {
        return Ok(());
    }
    Err("this worktrees serve has no documentation viewer bundle: WORKTREES_VIEWER_JS is not set \
         (the desktop app sets it when it starts serve; a serve started from the CLI needs it \
         exported). The Docs tab lists and reads without it; only the browser page cannot render."
        .to_string())
}

/// The registry's clock: wall-clock seconds (§2.5 compares `until` and
/// `last_seen` on the host's own clock).
fn now() -> i64 {
    worktrees_core::sysclock::now_epoch()
}

/// The `id` of a device row, already resolved within the caller's principal.
fn device_id(a: &Args) -> Result<DeviceId, String> {
    Ok(DeviceId::new(a.get::<String>("id")?))
}

/// A `{caps, projects}` scope from a device row: capability names (an unknown
/// one is an error, never dropped) and `"*"` or a list of project roots. With
/// `registered`, every root must be a registered project's registry key — the
/// console's widening names projects as `list_workspace` does; a narrowing
/// can only name roots the device already has.
fn scope_arg(cx: &Call, a: &Args, registered: bool) -> Result<Grant, String> {
    let names: Vec<String> = a.get("caps")?;
    let caps = names
        .iter()
        .map(|c| Capability::parse(c).ok_or_else(|| format!("unknown capability: {c}")))
        .collect::<Result<Vec<_>, _>>()?;
    let projects: Projects = a.get("projects")?;
    if let (true, Projects::Only(set)) = (registered, &projects) {
        let known = match &cx.world {
            Some(w) => w.roots(),
            None => worktrees_api::projects::read_projects(&cx.ctx),
        };
        if let Some(r) = set.iter().find(|r| !known.contains(r)) {
            return Err(format!("not a registered project: {r}"));
        }
    }
    Ok(Grant { capabilities: Caps::of(&caps), projects })
}

/// `read` → `Some(Capability::Read)`; `console` and `own` (a `HostConsole`
/// or `Own` row's placeholder before its objects) and nothing → `None`.
macro_rules! cap {
    () => { None };
    (console) => { None };
    (own) => { None };
    (read) => { Some(Capability::Read) };
    (write) => { Some(Capability::Write) };
    (answer) => { Some(Capability::Answer) };
    (instruct) => { Some(Capability::Instruct) };
    (stream) => { Some(Capability::Stream) };
    (admin) => { Some(Capability::Admin) };
}

/// One object spec → its `Obj`. The kinds are `resolve::Obj`'s, snake_case.
macro_rules! obj {
    (project($a:ident)) => { Obj::Project(stringify!($a)) };
    (opt_project($a:ident, $absent:ident)) => { Obj::OptProject(stringify!($a), Absent::$absent) };
    (place($r:ident, $s:ident)) => { Obj::Place(stringify!($r), stringify!($s)) };
    (slug($a:ident)) => { Obj::Slug(stringify!($a)) };
    (root($a:ident)) => { Obj::Root(stringify!($a)) };
    (file($a:ident)) => { Obj::File(stringify!($a)) };
    (probe($a:ident)) => { Obj::Probe(stringify!($a)) };
    (session($a:ident)) => { Obj::Session(stringify!($a)) };
    (session_in($a:ident, $b:ident)) => { Obj::SessionIn(stringify!($a), stringify!($b)) };
    (session_of($a:ident, $r:ident, $s:ident)) => { Obj::SessionOf(stringify!($a), stringify!($r), stringify!($s)) };
    (attach($a:ident)) => { Obj::Attach(stringify!($a)) };
    (close($a:ident)) => { Obj::Close(stringify!($a)) };
    (shell($r:ident, $s:ident, $i:ident)) => { Obj::Shell(stringify!($r), stringify!($s), stringify!($i)) };
    (shell_ref($a:ident)) => { Obj::ShellRef(stringify!($a)) };
    (job($a:ident)) => { Obj::Job(stringify!($a)) };
    (new_job($a:ident)) => { Obj::NewJob(stringify!($a)) };
    (device($a:ident)) => { Obj::Device(stringify!($a)) };
    (revocable($a:ident)) => { Obj::Revocable(stringify!($a)) };
    (new_path($a:ident)) => { Obj::NewPath(stringify!($a)) };
    (remote_url($a:ident)) => { Obj::RemoteUrl(stringify!($a)) };
    (places($a:ident)) => { Obj::Places(stringify!($a)) };
    (roots($a:ident)) => { Obj::Roots(stringify!($a)) };
    (candidates($a:ident)) => { Obj::Candidates(stringify!($a)) };
    (filtered($f:ident)) => { Obj::Filtered(scope::$f) };
    (opt($($inner:tt)+)) => { Obj::Opt(&obj!($($inner)+)) };
    (global) => { Obj::Global };
    (all) => { Obj::All };
    (not_remote) => { Obj::NotRemote };
}

macro_rules! commands {
    ($cx:ident, $a:ident; $( $name:ident : $exp:ident $( [ $cap:ident $(, $kind:ident $( ( $($arg:tt)* ) )? )* ] )? => $body:expr ),* $(,)?) => {
        /// Every command a client can send, with its exposure, capability and
        /// objects. Exhaustive by construction: the dispatcher below is
        /// generated from the same list.
        pub const TABLE: &[Row] = &[ $(
            Row::new(stringify!($name), Exposure::$exp, cap!($($cap)?), &[ $( $( obj!($kind $( ( $($arg)* ) )?) ),* )? ])
        ),* ];

        /// The body, once the resolver has passed the request. `None` = not a
        /// command. A `ClientLocal` row's body is never evaluated.
        fn run(name: &str, $cx: &Call, $a: &Args) -> Option<Result<Value, Value>> {
            match name {
                $( stringify!($name) => Some(match Exposure::$exp {
                    Exposure::ClientLocal => Err(Value::from(CLIENT_LOCAL)),
                    _ => (|| -> Result<Value, Value> { reply($body) })(),
                }), )*
                _ => None,
            }
        }
    };
}

/// A command only test builds have, whose body panics with its `why`: the
/// seam for the tests of what a panic is answered with (`conn::answer`).
/// Never in `TABLE`, so nothing built for use can reach it.
#[cfg(test)]
pub const TEST_PANIC: &str = "__test_panic";

/// A command only test builds have, that shares like §6's reads and starts
/// an agent like `new_place`: `{tag, ms}` sleeps `ms`, `{tag, joins}` waits
/// (at most 2 s) until that many calls have joined its `Flights`, and either
/// answers how many times a body with this `tag` has run. In test builds it
/// is a ROW ([`row`]), a `read` of nothing, so the precheck, the resolver,
/// the flight and a device launch's lane gate (`provenance::launches`) meet
/// it where they meet a real row. Never in `TABLE`.
#[cfg(test)]
pub const TEST_SHARED: &str = "__test_shared";

#[cfg(test)]
static TEST_SHARED_ROW: Row = Row::new(TEST_SHARED, Exposure::Engine, Some(Capability::Read), &[Obj::Global]);

#[cfg(test)]
fn test_shared(cx: &Call, args: &Args) -> Result<Value, Value> {
    use std::time::{Duration, Instant};
    static RUNS: std::sync::Mutex<Option<std::collections::HashMap<String, u64>>> = std::sync::Mutex::new(None);
    if let Some(n) = args.raw("joins").as_u64() {
        let t0 = Instant::now();
        while (cx.flights.joins() as u64) < n && t0.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    std::thread::sleep(Duration::from_millis(args.raw("ms").as_u64().unwrap_or(0)));
    let tag = args.raw("tag").as_str().unwrap_or("").to_string();
    let mut runs = RUNS.lock().unwrap();
    let n = runs.get_or_insert_with(Default::default).entry(tag).or_insert(0);
    *n += 1;
    Ok(Value::from(*n))
}

/// The answer to a command no row names.
fn unknown(cmd: &str) -> Value {
    Value::from(format!("unknown command: {cmd}"))
}

/// What a refused request is answered — THE one refusal path: both refusal
/// sites, [`precheck`] and [`call_in_seen`], come through here, so `seen`
/// records every refusal exactly once (§5.1), the stale close answered `ok`
/// included. Then the real reason, for the host's eyes; the console's
/// ordinary misses (a file deleted under the tree) are the body errors they
/// always were, and not worth a line each. A probe's refusal IS its answer
/// (`resolve::Obj::Probe`): a file this caller may not read is reported as
/// one that is not there.
fn refusal_answer(caller: &Caller, row: &'static Row, args: &Args, r: &resolve::Refusal, seen: &dyn Witness) -> Result<Value, Value> {
    seen.refused(row, args, r);
    // The idempotent close of an attach this connection does not own:
    // answered as closing a missing one is, and the body never runs.
    if r.kind == resolve::RefusalKind::NoopNotOwned {
        return Ok(Value::Null);
    }
    if !caller.is_console() {
        worktrees_api::log::info(&format!("serve: refused {} for {caller}: {:?}: {}", row.name, r.kind, r.detail));
    }
    if matches!(r.kind, resolve::RefusalKind::NotFound | resolve::RefusalKind::Denied) && row.objects.iter().any(Obj::answers_false) {
        return Ok(Value::Bool(false));
    }
    Err(r.wire_for(caller))
}

/// The checks that need no I/O, for serve's reader to run before a request
/// is admitted (Phase 2 §6): the command exists, and the caller's exposure
/// and capability allow it (`resolve::admissible`). A request refused here
/// never takes an in-flight slot, a lane or a thread: `Some` is its answer,
/// sent at once, and `seen` has recorded it. `pay` is asked first, so a
/// caller over its rate is answered with what `pay` says, and its refusal is
/// counted into that limit's episode (`budget.rs`) instead of costing a log
/// line and an audit record each.
///
/// `None` decides nothing: `call_in_seen` checks all of it again with the
/// grant read again, so a scope narrowed in between still applies to the
/// request, and then resolves the objects, which do need I/O.
pub fn precheck(
    caller: &Caller,
    registry: &dyn Registry,
    cmd: &str,
    args: &Args,
    seen: &dyn Witness,
    pay: impl FnOnce() -> Result<(), &'static str>,
) -> Option<Result<Value, Value>> {
    #[cfg(test)]
    if cmd == TEST_PANIC {
        return None;
    }
    let refusal = match row(cmd) {
        None => None,
        Some(row) => match resolve::admissible(row, caller, &authz::effective(registry, caller)) {
            Ok(()) => return None,
            Err(r) => Some((row, r)),
        },
    };
    if let Err(rate) = pay() {
        return Some(Err(Value::from(rate)));
    }
    Some(match refusal {
        None => {
            seen.unknown(cmd);
            Err(unknown(cmd))
        }
        Some((row, r)) => refusal_answer(caller, row, args, &r, seen),
    })
}

/// Dispatch one request, with nothing watching. `Err` is the wire's `err`
/// value. Test builds only: serve's one caller, `conn::answer`, goes through
/// [`call_seen`], so no request it serves can skip its audit record.
#[cfg(test)]
pub fn call(cx: &Call, cmd: &str, args: &Args) -> Result<Value, Value> {
    call_seen(cx, cmd, args, &())
}

/// What the audit sees of one request (§5.1): the resolver's verdict, before
/// any body runs. Handed to [`call_seen`] and [`precheck`] BESIDE the `Call`,
/// never inside it — a body is given the `Call`, so no body can reach the
/// audit. One of the three is called, once, for every request dispatch
/// decides, whichever of the two decides it; a request the budget answers
/// first (`busy`, `rate`) is counted into its limit's episode instead
/// (`budget.rs`). `()` sees nothing. `audit::Seen` is the one that writes.
pub trait Witness {
    /// No row has this name, and nothing ran.
    fn unknown(&self, _cmd: &str) {}
    /// The host refused `row` — the resolver, or a device launch's lane cap —
    /// and its body did not run: every refusal, the stale close a caller is
    /// answered `ok` for included.
    fn refused(&self, _row: &'static Row, _args: &Args, _r: &resolve::Refusal) {}
    /// The resolver admitted `row`, and its body is about to run on `args`:
    /// the canonical values it checked, and the projects they resolved to.
    fn admitted(&self, _row: &'static Row, _args: &Args, _resolved: &resolve::Resolved) {}
    /// The last word before the body: may `row` run? The audit refuses a
    /// device's drive-class act its budget cannot record in full
    /// (`RefusalKind::AuditBudget`). `()` lets everything run.
    fn allow(&self, _row: &'static Row, _args: &Args, _resolved: &resolve::Resolved) -> Result<(), resolve::Refusal> {
        Ok(())
    }
}

impl Witness for () {}

/// Dispatch one request, with its audit watching (`conn::answer`). `Err` is
/// the wire's `err` value.
pub fn call_seen(cx: &Call, cmd: &str, args: &Args, seen: &dyn Witness) -> Result<Value, Value> {
    with_world(cx, |w| call_in_seen(cx, cmd, args, w, seen))
}

/// `f` over the world `cx`'s objects resolve against: its own, or the live
/// registry and tmux server.
pub fn with_world<T>(cx: &Call, f: impl FnOnce(&dyn World) -> T) -> T {
    match &cx.world {
        Some(w) => f(&**w),
        None => f(&resolve::Live(&cx.ctx)),
    }
}

/// [`call`], resolving against an explicit world (the tests' fixtures).
#[cfg(test)]
pub fn call_in(cx: &Call, cmd: &str, args: &Args, world: &dyn World) -> Result<Value, Value> {
    call_in_seen(cx, cmd, args, world, &())
}

/// [`call_seen`], resolving against an explicit world.
///
/// In order, before any body: the row; the grant of the caller's (principal,
/// device) pair, read now and never cached (a narrowed scope applies to the
/// very next request); the resolver (exposure, capability, objects, canonical
/// values written back); step-up (§2.5). Then, for one of §6's poll-shaped
/// reads, the flight in the air for its key (`flight.rs`): a follower is
/// answered with its leader's outcome, through its own row filter, and goes
/// no further. The
/// leader, or a call that never shares, meets a device launch's live-lane cap
/// next, whose verdict `seen` is told, and then the body, on a `Ctx` scoped
/// to the grant and acting for the caller, a device's instruction write
/// stamped once it returns `Ok` (§7.3), and the result through the row's
/// filter, for this caller alone.
///
/// **The flight is joined BEFORE the lane gate, on purpose.** The gate holds
/// its lock across the body, so a follower that met it first would wait
/// there behind its own leader, find the flight landed, and run the body —
/// and take a lane — a second time. Joined first, a follower never touches
/// the gate. No row both shares and launches today; the test row does both
/// (`a_follower_never_reaches_the_lane_gate`). Step-up is the other way
/// round: it is the caller's own verdict, so it comes BEFORE the flight, or a
/// device whose step-up is due would be handed a leader's `diagnostics`
/// (`step_up_is_asked_before_a_flight_is_joined`).
pub fn call_in_seen(cx: &Call, cmd: &str, args: &Args, world: &dyn World, seen: &dyn Witness) -> Result<Value, Value> {
    #[cfg(test)]
    if cmd == TEST_PANIC {
        panic!("{}", args.raw("why").as_str().unwrap_or("a test body panicked"));
    }
    let Some(row) = row(cmd) else {
        seen.unknown(cmd);
        return Err(unknown(cmd));
    };
    let grant = authz::effective(&*cx.registry, &cx.caller);
    let mut args = Args(args.0.clone());
    // The resolver, then step-up, read once the objects resolved: a path
    // class or another device can RAISE the act to drive-class
    // (`stepup_gate`). Both are THIS caller's verdict, so both come before
    // the flight: a follower answered with its leader's outcome must have
    // passed them itself. Every refusal is the host's, never the body's, and
    // `seen` hears it so.
    let checked = resolve::authorize(row, cx, &grant, &mut args, world).and_then(|r| stepup_gate(cx, row, &args, &r).map(|()| r));
    let resolved = match checked {
        Ok(r) => r,
        Err(r) => return refusal_answer(&cx.caller, row, &args, &r, seen),
    };
    let scoped = cx.with(body_ctx(cx, &grant, &resolved), cx.frames.clone());
    let lead = match flight::key(row.name, &args, scoped.ctx.scope(), scoped.ctx.fetcher()).map(|k| cx.flights.join(k)) {
        Some(Join::Follow(f)) => {
            seen.admitted(row, &args, &resolved);
            return filtered(row, &grant, world, f.wait());
        }
        Some(Join::Lead(l)) => Some(l),
        None => None,
    };
    // A device launch's live-lane cap (§6, LLM10), whose lock is held across
    // the body so the next device launch counts this one. Its refusal is the
    // host's, never the body's, and `seen` hears it so. A leader's followers
    // get what the leader got, refusal included; a leader whose body panics
    // drops its `lead` unlanded, which answers them `internal error`.
    let out = match provenance::device_lane_gate(cx, row, &args, world) {
        Err(r) => {
            let out = refusal_answer(&cx.caller, row, &args, &r, seen);
            if let Some(l) = lead {
                l.land(&out);
            }
            return out;
        }
        // Last, the audit's word (§5.1): a device's drive-class act its
        // audit budget cannot record in full does not run. Its refusal is
        // recorded once, as every refusal is, and charged to the device's
        // refusals, not to the budget it was refused for.
        Ok(_launching) => match seen.allow(row, &args, &resolved) {
            Err(r) => refusal_answer(&cx.caller, row, &args, &r, seen),
            Ok(()) => {
                seen.admitted(row, &args, &resolved);
                let out = body(row.name, &scoped, &args);
                provenance::stamped(&cx.caller, resolved.instruction.as_ref(), out)
            }
        },
    };
    if let Some(l) = lead {
        l.land(&out);
    }
    filtered(row, &grant, world, out)
}

/// The row's body, by name.
fn body(name: &str, cx: &Call, args: &Args) -> Result<Value, Value> {
    #[cfg(test)]
    if name == TEST_SHARED {
        return test_shared(cx, args);
    }
    run(name, cx, args).unwrap_or_else(|| Err(unknown(name)))
}

/// A body's outcome through `row`'s filter, for this caller's grant alone.
fn filtered(row: &Row, grant: &authz::Grant, world: &dyn World, out: Result<Value, Value>) -> Result<Value, Value> {
    let out = out?;
    Ok(match row.objects.iter().find_map(Obj::filter) {
        Some(f) => {
            let roots = if grant.projects.is_all() { Vec::new() } else { world.roots() };
            f(&scope::View { projects: &grant.projects, roots: &roots }, out)
        }
        None => out,
    })
}

/// The `Ctx` a body runs on: scoped to the grant, holding its capabilities
/// (the bodies that list paths drop or mark what the caller may not read),
/// fetching as its caller — the console's git for the console, and for a
/// device the addresses the resolver checked, pinned (gap 17,
/// `worktrees_core::fetchpolicy`) — and acting for it, so what the body
/// writes and launches records who asked (§7.3, §7.5).
fn body_ctx(cx: &Call, grant: &authz::Grant, resolved: &resolve::Resolved) -> Ctx {
    let fetcher = if cx.caller.is_console() { Fetcher::Console } else { Fetcher::Device(resolved.pins.clone()) };
    cx.ctx
        .scoped(grant.projects.clone())
        .granted(grant.capabilities, cx.caller.is_console())
        .fetching(fetcher)
        .acting_for(provenance::acting(&cx.caller, grant))
}

/// The rows that OPEN a stream, which a step-up always gates (§2.5).
pub const OPENS_A_STREAM: &[&str] = &["term_open", "shell_open"];

/// Whether an act is drive-class for step-up (§2.5): it needs `instruct`,
/// `answer` or `admin` — the row's own capability or one its objects raised —
/// or it is a `stream` act that is not on a channel this connection already
/// HOLDS. Typing into, resizing or closing what this connection opened (an
/// open that was itself gated) is not re-asked; opening is, always; and so
/// is every act on a dock shell this connection never opened — dock shells
/// are the process's, so without this a device could type into the shell the
/// desktop opened and never be asked.
pub fn needs_stepup(row: &Row, needs: &[Capability], holds: bool) -> bool {
    needs.iter().any(|c| match c {
        Capability::Instruct | Capability::Answer | Capability::Admin => true,
        Capability::Stream => OPENS_A_STREAM.contains(&row.name) || !holds,
        Capability::Read | Capability::Write => false,
    })
}

/// The row acts on a channel this connection holds: its own attach (the
/// resolver's `attach`/`close` objects proved that), or a dock shell whose
/// LIVE attach is the one this connection made — the hold's generation is the
/// shell's current one (`term::shell_gen`; generations are unique per
/// process). A hold the desktop has since re-attached over, or one on a shell
/// since closed and spawned again under the same key, holds nothing.
fn holds_channel(cx: &Call, row: &Row, args: &Args) -> bool {
    holds_channel_with(cx, row, args, &|repo, slug, index| term::shell_gen(repo, slug, index, &cx.ctx.shells))
}

/// [`holds_channel`], with the live generation looked up by `live` — the
/// tests', since a real dock shell is a login shell on a pty.
pub fn holds_channel_with(cx: &Call, row: &Row, args: &Args, live: &dyn Fn(&str, &str, u32) -> Option<u64>) -> bool {
    row.objects.iter().any(|o| match *o {
        Obj::Attach(_) | Obj::Close(_) => true,
        Obj::Shell(r, s, i) => {
            let (Ok(repo), Ok(slug), Ok(index)) = (args.get::<String>(r), args.get::<String>(s), args.get::<u32>(i)) else { return false };
            let Some(gen) = live(&repo, &slug, index) else { return false };
            cx.frames.shells().iter().any(|h| (h.repo.as_str(), h.slug.as_str(), h.index, h.gen) == (repo.as_str(), slug.as_str(), index, gen))
        }
        _ => false,
    })
}

/// Step-up (§2.5, §11 Q8): a paired device whose record requires it makes a
/// drive-class act only with a proof that holds on this connection. Read
/// here, after the objects resolved, because a path class or another device
/// can RAISE the act to drive-class; never in the resolver, whose coverage
/// check is also what a reduction re-runs on open channels.
fn stepup_gate(cx: &Call, row: &Row, args: &Args, r: &resolve::Resolved) -> Result<(), resolve::Refusal> {
    let authz::Device::Paired(d) = cx.caller.device() else { return Ok(()) };
    if !needs_stepup(row, &r.needs, holds_channel(cx, row, args)) || !cx.registry.stepup_required(cx.caller.principal(), d) {
        return Ok(());
    }
    if cx.frames.stepup().fresh(std::time::Instant::now()) {
        return Ok(());
    }
    Err(resolve::Refusal::stepup(format!("{} is drive-class and {d}'s step-up is due", row.name)))
}

/// A command's row.
pub fn row(cmd: &str) -> Option<&'static Row> {
    #[cfg(test)]
    if cmd == TEST_SHARED {
        return Some(&TEST_SHARED_ROW);
    }
    TABLE.iter().find(|r| r.name == cmd)
}

pub fn exposure(cmd: &str) -> Option<Exposure> {
    row(cmd).map(|r| r.exposure)
}

/// The capability a command's row declares — `None` for a `HostConsole`,
/// `Loopback` or `ClientLocal` row, or an unknown command. The audit reads
/// it from the row it is handed (`Witness`), so only the tests ask by name.
#[cfg(test)]
pub fn cap_of(cmd: &str) -> Option<Capability> {
    row(cmd).and_then(|r| r.cap)
}

/// The rows allowed to name no project at all (`global`): data and settings
/// about the HOST — usage meters, versions, the changelog, harness-wide
/// configuration — never one project's (§3.3). Held against the table both
/// ways by a test, so a new `global` row is a second, deliberate edit here.
pub const GLOBAL_OK: &[&str] = &[
    // read
    "settings_info", "get_settings", "quota_settings", "slots_settings", "claude_usage", "codex_usage", "kimi_usage", "claude_status",
    "check_update", "list_releases", "get_ai_config", "get_codex_permissions", "get_fetch_interval",
    "get_term_history_opts", "agent_guidance_status", "agent_guidance_diff", "agent_models", "agent_user_skills",
    "kimi_version", "tmux_check", "log_info", "get_changelog", "new_profile_id", "skills_list", "ui_usage",
    // write
    "set_settings", "log_event",
    // admin
    "codex_mcp_status", "codex_mcp_uninstall", "codex_mcp_migration_plan", "pi_mcp_status", "pi_mcp_uninstall",
    "kimi_mcp_status", "delete_profile", "skill_remove", "skill_preview_git", "sync_hub_list", "diagnostics",
    "log_tail", "term_history_info", "profiles_info", "devices_list",
];

/// The lane a request runs on, or `None` for the concurrent pool.
///
/// Requests to ONE stream must reach it in the order they were sent — a
/// keystroke behind a keystroke, a resize behind the bytes typed before it —
/// and a dispatcher that gives every frame its own blocking task does not
/// keep that (measured: 26 in-order `shell_write`s reaching zsh as
/// `0124356789…`). So every `Stream` row that names an existing stream is
/// keyed by that stream: a tmux attach by its `id`, a dock shell by
/// `(repo, slug, index)`; the connection runs one lane per key, serially, and
/// everything else stays concurrent, so a long clone still cannot hold up a
/// keystroke. `term_open` has no id yet and needs no order: nothing can write
/// to an attach before the reply names it. The key is built from the RAW
/// args, so a malformed request lands on a lane and fails IN ORDER with its
/// proper error instead of racing past the requests around it; two keys
/// that happened to collide would only share a lane, never lose order.
pub fn lane(cmd: &str, args: &Args) -> Option<String> {
    // The panicking test body runs on the lane of the attach its `id` names,
    // when it names one, so a test can queue a request behind it.
    #[cfg(test)]
    if cmd == TEST_PANIC {
        let id = args.raw("id");
        return (!id.is_null()).then(|| format!("term:{id}"));
    }
    if exposure(cmd)? != Exposure::Stream {
        return None;
    }
    if let Some(_) = cmd.strip_prefix("term_") {
        if cmd == "term_open" {
            return None;
        }
        return Some(format!("term:{}", args.raw("id")));
    }
    if cmd.starts_with("shell_") {
        return Some(format!("shell:{}:{}:{}", args.raw("repo"), args.raw("slug"), args.raw("index")));
    }
    None
}

commands! { cx, a;
    // ── projects and their snapshots (workspace) ─────────────────────────
    list_places: Engine [read, project(repo)] => workspace::list_places(&cx.ctx, a.get("repo")?),
    list_workspace: Engine [read, filtered(workspace)] => workspace::list_workspace(&cx.ctx),
    add_project: Engine [admin, new_path(dir)] => workspace::add_project(&cx.ctx, a.get("dir")?),
    probe_dir: Engine [admin, new_path(dir)] => workspace::probe_dir(a.get("dir")?),
    create_project: Engine [admin, new_path(location)] => workspace::create_project(&cx.ctx, a.get("location")?, a.get("name")?),
    clone_project: Engine [admin, new_path(parent), new_job(id), remote_url(url)] => {
        let mut sink = cx.frames.typed::<worktrees_core::clone::CloneProgress>(a.channel("on_progress")?);
        workspace::clone_project(&cx.ctx, a.get("id")?, a.get("url")?, a.get("parent")?, a.get("name")?, &mut sink)
    },
    clone_cancel: Engine [admin, job(id)] => workspace::clone_cancel(a.get("id")?),
    init_repo: Engine [admin, new_path(dir)] => workspace::init_repo(&cx.ctx, a.get("dir")?),
    create_initial_commit: Engine [admin, project(repo)] => workspace::create_initial_commit(&cx.ctx, a.get("repo")?),
    remove_project: Engine [admin, project(root)] => workspace::remove_project(&cx.ctx, a.get("root")?),
    reorder_projects: Engine [write, all, roots(roots)] => workspace::reorder_projects(&cx.ctx, a.get("roots")?),

    // ── settings, planning, and the mutating verbs (ops) ─────────────────
    settings_info: Engine [read, global] => ops::settings_info(&cx.ctx),
    get_settings: Engine [read, global] => ops::get_settings(&cx.ctx),
    set_settings: Engine [write, global] => ops::set_settings(&cx.ctx, a.get("settings")?),
    set_lifecycle: Engine [write, place(repo, slug)] => ops::set_lifecycle(a.get("repo")?, a.get("slug")?, a.get("label")?),
    set_pin: Engine [write, place(repo, slug)] => ops::set_pin(a.get("repo")?, a.get("slug")?, a.get("on")?),
    set_note: Engine [write, place(repo, slug)] => ops::set_note(&cx.ctx, a.get("repo")?, a.get("slug")?, a.get("note")?),
    drop_reference: Engine [write, place(repo, slug), place(into_repo, into_slug), session_of(into_session, into_repo, into_slug)] => ops::drop_reference(
        a.get("repo")?, a.get("slug")?, a.get("into_repo")?, a.get("into_slug")?, a.get("into_session")?, a.get("provider")?,
    ),
    cross_project_status: Engine [read, filtered(by_project)] => ops::cross_project_status(),
    set_cross_project: HostConsole => ops::set_cross_project(a.get("level")?),
    planning_status: Engine [read, filtered(by_project)] => ops::planning_status(),
    set_planning_default: Engine [admin, all] => ops::set_planning_default(a.get("default")?),
    set_project_planning: Engine [admin, project(root)] => ops::set_project_planning(a.get("root")?, a.get("level")?, a.get("plan_path")?, a.get("scope")?),
    planning_suggested: Engine [read, project(root)] => ops::planning_suggested(a.get("root")?),
    quota_settings: Engine [read, global] => ops::quota_settings(),
    set_quota_settings: Engine [admin, all] => ops::set_quota_settings(a.get("gate")?, a.get("weekly_warn_pct")?),
    slots_settings: Engine [read, global] => ops::slots_settings(),
    set_slot_capacity: HostConsole => ops::set_slot_capacity(a.get("name")?, a.get("capacity")?),
    set_project_private: HostConsole [console, project(root)] => ops::set_project_private(a.get("root")?, a.get("private")?),
    copy_text: ClientLocal => refused(),
    set_title: Engine [write, place(repo, slug)] => ops::set_title(&cx.ctx, a.get("repo")?, a.get("slug")?, a.get("title")?),
    touch_place: Engine [write, place(repo, slug)] => ops::touch_place(a.get("repo")?, a.get("slug")?),
    mark_seen: Engine [write, place(repo, slug)] => ops::mark_seen(a.get("repo")?, a.get("slug")?, a.get("epoch")?),
    new_place: Engine [write, project(repo)] => ops::new_place(
        &cx.ctx, a.get("repo")?, a.get("branch")?, a.get("base")?, a.get("name")?, a.get("provider")?, a.get("model")?, a.get("force")?,
    ),
    switch_place: Engine [write, place(repo, slug)] => ops::switch_place(a.get("repo")?, a.get("slug")?, a.get("branch")?, a.get("base")?),
    list_branches: Engine [read, place(repo, slug)] => ops::list_branches(a.get("repo")?, a.get("slug")?),
    open_place: Engine [write, place(repo, slug)] => ops::open_place(
        &cx.ctx, a.get("repo")?, a.get("slug")?, a.get("fresh")?, a.get("provider")?, a.get("model")?, a.get("force")?,
    ),
    // Host policy (`worktrees_api::hostpolicy`): `admin`, not console-only like Codex's, as it widens no agent (decision 6).
    set_fetch_interval: Engine [admin, all] => ops::set_fetch_interval(&cx.ctx, a.get("mins")?),
    get_fetch_interval: Engine [read, global] => ops::get_fetch_interval(),
    fetch_origin: Engine [write, project(root)] => ops::fetch_origin(a.get("root")?),
    list_drafts: Engine [read, filtered(drafts)] => ops::list_drafts(),
    close_place: Engine [write, place(repo, slug), opt(session_of(session, repo, slug))] => ops::close_place(
        a.get("repo")?, a.get("slug")?, a.get("yes")?, a.get("session")?, a.get("provider")?, &cx.ctx.shells,
    ),
    remove_place: Engine [write, place(repo, slug)] => prefs::remove_place(
        &cx.ctx, a.get("repo")?, a.get("slug")?, a.get("del_branch")?, a.get("force")?, &cx.ctx.shells,
    ),

    // ── usage, service status, releases (status) ─────────────────────────
    claude_usage: Engine [read, global] => status::claude_usage(),
    codex_usage: Engine [read, global] => status::codex_usage(),
    // Spawns `kimi web`, asks once, ends it; every row already runs on the
    // blocking pool, which is what the shim's spawn_blocking buys it.
    kimi_usage: Engine [read, global] => status::kimi_usage(),
    claude_status: Engine [read, global] => status::claude_status(),
    check_update: Engine [read, global] => status::check_update(),
    update_cli: HostConsole => status::update_cli(a.get("tag")?),
    list_releases: Engine [read, global] => status::list_releases(),
    install_app_version: ClientLocal => refused(),

    // ── config surfaces, MCP setup, Ask Claude (config) ──────────────────
    get_ai_config: Engine [read, global] => config::get_ai_config(),
    mcp_status: Engine [admin, opt_project(repo, Free)] => config::mcp_status(a.get("repo")?),
    mcp_install: HostConsole [console, opt_project(repo, Free)] => config::mcp_install(a.get("repo")?, a.get("mutations")?),
    mcp_uninstall: Engine [admin, opt_project(repo, AllScope)] => config::mcp_uninstall(a.get("repo")?),
    codex_mcp_status: Engine [admin, global] => config::codex_mcp_status(),
    codex_mcp_install: HostConsole => config::codex_mcp_install(a.get("mutations")?),
    codex_mcp_uninstall: Engine [admin, global] => config::codex_mcp_uninstall(),
    codex_mcp_migration_plan: Engine [admin, global] => config::codex_mcp_migration_plan(),
    pi_mcp_status: Engine [admin, global] => config::pi_mcp_status(),
    pi_mcp_install: HostConsole => config::pi_mcp_install(a.get("mutations")?),
    pi_mcp_uninstall: Engine [admin, global] => config::pi_mcp_uninstall(),
    kimi_mcp_status: Engine [admin, global] => config::kimi_mcp_status(),
    kimi_mcp_set: HostConsole => config::kimi_mcp_set(a.get("mode")?),
    codex_mcp_migrate: HostConsole => config::codex_mcp_migrate(a.get("names")?),
    project_config_read: Engine [read, project(repo)] => config::project_config_read(a.get("repo")?),
    doctor: Engine [read, project(repo), opt(slug(slug))] => config::doctor(a.get("repo")?, a.get("slug")?),
    place_health: Engine [read, place(repo, slug)] => config::place_health(a.get("repo")?, a.get("slug")?),
    ai_status_report: Engine [read, place(repo, slug)] => config::ai_status_report(a.get("repo")?, a.get("slug")?),
    relink: Engine [write, project(repo), opt(slug(slug))] => config::relink(a.get("repo")?, a.get("slug")?, a.get("force")?),
    provision: Engine [write, project(repo), opt(slug(slug))] => config::provision(a.get("repo")?, a.get("slug")?),

    // ── sync, agent setup, diagnostics, pull requests (project) ──────────
    sync_status: Engine [read, project(repo)] => project::sync_status(a.get("repo")?),
    sync_hub_list: Engine [admin, global] => project::sync_hub_list(),
    sync_preview: Engine [read, opt_project(repo, AdminAll)] => project::sync_preview(a.get("repo")?, a.get("name")?, a.get("direction")?, a.get("with_sessions")?),
    sync_apply: Engine [admin, opt_project(repo, Console)] => {
        let mut sink = cx.frames.typed::<worktrees_core::sync::SyncProgress>(a.channel("on_progress")?);
        project::sync_apply(
            &cx.ctx, a.get("repo")?, a.get("name")?, a.get("direction")?, a.get("with_sessions")?,
            a.get("confirmed")?, a.get("install")?, &mut sink,
        )
    },
    init_suggest: Engine [read, project(repo)] => project::init_suggest(a.get("repo")?),
    init_write: Engine [instruct, project(repo)] => project::init_write(a.get("repo")?),
    agent_setup_status: Engine [read, project(repo)] => project::agent_setup_status(a.get("repo")?),
    agent_setup_fix: Engine [instruct, project(repo)] => project::agent_setup_fix(&cx.ctx, a.get("repo")?, a.get("push")?),
    rules_apply: Engine [instruct, project(repo)] => project::rules_apply(&cx.ctx, a.get("repo")?, a.get("push")?, a.get("force")?),
    rules_review: Engine [write, project(repo)] => project::rules_review(&cx.ctx, a.get("repo")?, a.get("provider")?, a.get("model")?),
    rules_lane_remove: Engine [write, project(repo)] => project::rules_lane_remove(&cx.ctx, a.get("repo")?),
    agent_link_skills: HostConsole => project::agent_link_skills(),
    agent_user_skills: Engine [read, global] => project::agent_user_skills(),
    // Host policy (`worktrees_api::hostpolicy`): written only by this explicit
    // act, read by the engine; the getter is a read like any other.
    set_codex_permissions: HostConsole => project::set_codex_permissions(&cx.ctx, a.get("mode")?),
    get_codex_permissions: Engine [read, global] => project::get_codex_permissions(),
    agent_guidance_status: Engine [read, global] => project::agent_guidance_status(),
    set_agent_guidance: HostConsole => project::set_agent_guidance(a.get("enabled")?, a.get("guard")?),
    set_agent_guidance_skill: HostConsole => project::set_agent_guidance_skill(a.get("text")?),
    agent_guidance_diff: Engine [read, global] => project::agent_guidance_diff(a.get("old")?, a.get("new")?),
    agent_guidance_merge: HostConsole => project::agent_guidance_merge(),
    agent_models: Engine [read, global] => project::agent_models(a.get("harness")?),
    pi_status: Engine [read, opt_project(repo, Free)] => project::pi_status(a.get("repo")?),
    kimi_status: Engine [read, opt_project(repo, Free)] => project::kimi_status(a.get("repo")?),
    // Starts a kimi process for its --version; a read.
    kimi_version: Engine [read, global] => project::kimi_version(),
    set_pi_trust: HostConsole => project::set_pi_trust(&cx.ctx, a.get("mode")?),
    set_pi_allowed: HostConsole [console, project(repo)] => project::set_pi_allowed(a.get("repo")?, a.get("allow")?),
    set_kimi_allowed: HostConsole [console, project(repo)] => project::set_kimi_allowed(a.get("repo")?, a.get("allow")?),
    remote_url: Engine [read, project(repo)] => project::remote_url(a.get("repo")?),
    project_prs: Engine [read, project(repo), places(places)] => project::project_prs(a.get("repo")?, a.get("max_age_secs")?, a.get("places")?),
    open_editor: ClientLocal => refused(),
    detect_editors: ClientLocal => refused(),
    open_terminal: ClientLocal => refused(),
    diagnostics: Engine [admin, global] => project::diagnostics(&cx.ctx, a.get("gh")?, viewer_path()),
    tmux_check: Engine [read, global] => hostenv::tmux_check(a.get("refresh")?),
    set_zoom: ClientLocal => refused(),

    // ── the app's own metadata (meta) ────────────────────────────────────
    log_info: Engine [read, global] => ok(meta::log_info()),
    log_event: Engine [write, global] => { meta::log_event(&a.get::<String>("level")?, a.get("msg")?); ok(()) },
    log_tail: Engine [admin, global, all] => ok(meta::log_tail(a.get("lines")?)),
    get_changelog: Engine [read, global] => ok(meta::get_changelog()),

    // ── the Files tab: tree, diffs, reads and writes (files) ─────────────
    list_dir: Engine [read, file(path)] => files::list_dir(&cx.ctx, a.get("path")?, a.get("show_ignored")?),
    changed_files: Engine [read, root(root)] => files::changed_files(&cx.ctx, a.get("root")?),
    file_diff: Engine [read, file(path)] => files::file_diff(&cx.ctx, a.get("path")?, a.get("against")?),
    read_file: Engine [read, file(path)] => files::read_file(&cx.ctx, a.get("path")?, a.get("max_bytes")?),
    file_readable: Engine [read, probe(path)] => files::file_readable(&cx.ctx, a.get("path")?),
    resolve_term_paths: Engine [read, root(root), opt(session_in(session, root)), shell_ref(shell), candidates(paths)] => files::resolve_term_paths(
        &cx.ctx, &cx.ctx.shells, a.get("root")?, a.get("shell")?, a.get("session")?, a.get("paths")?,
    ),
    resolve_doc_paths: Engine [read, root(root), opt(file(doc)), candidates(paths)] => files::resolve_doc_paths(
        &cx.ctx, &cx.ctx.docnames, a.get("root")?, a.get("doc")?, a.get("generation")?, a.get("paths")?,
    ),
    list_docs: Engine [read, project(repo), root(root)] => files::list_docs(&cx.ctx, a.get("repo")?, a.get("root")?),
    place_plan: Engine [read, root(root)] => files::place_plan(&cx.ctx, a.get("root")?),
    plan_prompt: Engine [write, session(session)] => files::plan_prompt(a.get("session")?, a.get("provider")?),
    plan_ask_main: Engine [write, root(root), session_in(session, root)] => files::plan_ask_main(&cx.ctx, a.get("root")?, a.get("session")?, a.get("provider")?),
    // Engine, not client-local: the registrations and the docs server are the
    // host's, refreshed by its poll; the frontend opens the returned URL itself.
    // The URL is loopback on the HOST, so it means something only to a client
    // on the same machine (the proposal's Queued list has the remote answer).
    open_docs_viewer: Engine [read, not_remote, place(repo, slug), root(root), opt(file(path))] => {
        viewer_bundle()?;
        files::open_docs_viewer(
            &cx.ctx, None, &cx.ctx.viewer, a.get("repo")?, a.get("root")?, a.get("slug")?, a.get("path")?, a.get("place")?,
        )
    },
    read_file_base64: Engine [read, file(path)] => files::read_file_base64(&cx.ctx, a.get("path")?, a.get("max_bytes")?),
    write_file: Engine [write, file(path)] => files::write_file(&cx.ctx, a.get("path")?, a.get("content")?, a.get("expected_mtime")?),

    // ── the dock: shells, scrollback, and the tmux attach (term) ─────────
    list_shell_sessions: Engine [read, place(repo, slug)] => term::list_shell_sessions(a.get("repo")?, a.get("slug")?, &cx.ctx.shells),
    close_shell_session: Engine [stream, shell(repo, slug, index)] => {
        let (repo, slug, index): (String, String, u32) = (a.get("repo")?, a.get("slug")?, a.get("index")?);
        cx.frames.forget_shell(&repo, &slug, index);
        term::close_shell_session(&cx.ctx, repo, slug, index, a.get("keep_cwd")?, &cx.ctx.shells)
    },
    shell_open: Stream [stream, shell(repo, slug, index)] => {
        let (repo, slug, index): (String, String, u32) = (a.get("repo")?, a.get("slug")?, a.get("index")?);
        let at = term::shell_open(
            &cx.ctx, repo.clone(), slug.clone(), index, a.get("cols")?, a.get("rows")?,
            cx.frames.bytes(a.channel("on_bytes")?), &cx.ctx.shells,
        );
        // Which sink is this connection's, so a reduction can let go of it.
        if let Ok(at) = &at {
            cx.frames.note_shell(super::wire::ShellHold { repo, slug, index, gen: at.gen });
        }
        at
    },
    shell_write: Stream [stream, shell(repo, slug, index)] => term::shell_write(a.get("repo")?, a.get("slug")?, a.get("index")?, a.get("data")?, &cx.ctx.shells),
    shell_resize: Stream [stream, shell(repo, slug, index)] => term::shell_resize(
        a.get("repo")?, a.get("slug")?, a.get("index")?, a.get("cols")?, a.get("rows")?, &cx.ctx.shells,
    ),
    shell_detach: Stream [stream, shell(repo, slug, index)] => {
        let (repo, slug, index, gen): (String, String, u32, u64) = (a.get("repo")?, a.get("slug")?, a.get("index")?, a.get("gen")?);
        cx.frames.take_shell_gen(&repo, &slug, index, gen);
        term::shell_detach(repo, slug, index, gen, &cx.ctx.shells)
    },
    term_open: Stream [stream, session(session)] => {
        let session: String = a.get("session")?;
        let id = term::term_open(
            session.clone(), a.get("cols")?, a.get("rows")?, cx.frames.bytes(a.channel("on_bytes")?), &cx.ctx.terminals,
        );
        // Remembered per connection, so a client that vanishes takes its
        // attaches with it instead of leaving a tmux client on every session
        // it ever looked at — and with its session, so a narrowed grant ends
        // the attaches it no longer covers (§3.3).
        if let Ok(id) = &id {
            cx.frames.note_term(*id, session);
        }
        id
    },
    term_write: Stream [stream, attach(id)] => term::term_write(a.get("id")?, a.get("data")?, &cx.ctx.terminals),
    term_wheel: Stream [stream, attach(id)] => term::term_wheel(a.get("id")?, a.get("lines")?, &cx.ctx.terminals),
    term_resize: Stream [stream, attach(id)] => term::term_resize(a.get("id")?, a.get("cols")?, a.get("rows")?, &cx.ctx.terminals),
    term_close: Stream [stream, close(id)] => {
        let id: u32 = a.get("id")?;
        cx.frames.forget_term(id);
        term::term_close(id, &cx.ctx.terminals)
    },

    // ── backend preferences, the Data pane, local usage metrics (prefs) ──
    // Host policy (`worktrees_api::hostpolicy`): `admin`, not console-only, as it widens no agent (decision 6).
    set_term_history_opts: Engine [admin, all] => prefs::set_term_history_opts(&cx.ctx, a.get("persist_scrollback")?, a.get("per_tab_history")?),
    get_term_history_opts: Engine [read, global] => prefs::get_term_history_opts(),
    term_history_info: Engine [admin, global] => prefs::term_history_info(&cx.ctx),
    term_history_clear: Engine [admin, all] => prefs::term_history_clear(&cx.ctx, &cx.ctx.shells),
    ui_events_append: Loopback => prefs::ui_events_append(&cx.ctx, a.get("events")?),
    ui_usage: Engine [read, global] => prefs::ui_usage(&cx.ctx, a.get("days")?, a.get("tz_offset_min")?),
    ui_events_clear: Engine [admin, all] => prefs::ui_events_clear(&cx.ctx),

    // ── AI profiles + the skill store (profiles) ─────────────────────────
    profiles_info: Engine [admin, global, opt_project(repo, Free)] => profiles::profiles_info(a.get("repo")?),
    save_profile: HostConsole => profiles::save_profile(a.get("profile")?),
    new_profile_id: Engine [read, global] => profiles::new_profile_id(a.get("name")?),
    delete_profile: Engine [admin, global] => profiles::delete_profile(a.get("id")?),
    set_project_profile: HostConsole [console, project(repo)] => profiles::set_project_profile(a.get("repo")?, a.get("id")?),
    set_default_profile: HostConsole => profiles::set_default_profile(a.get("id")?),
    skills_list: Engine [read, global] => profiles::skills_list(),
    skill_inspect: Engine [admin, new_path(path)] => profiles::skill_inspect(a.get("path")?),
    skill_install_local: HostConsole => profiles::skill_install_local(a.get("path")?),
    skill_preview_git: Engine [admin, global, all, remote_url(url)] => profiles::skill_preview_git(&cx.ctx, a.get("url")?, a.get("rev")?),
    skill_install_git: HostConsole => profiles::skill_install_git(a.get("url")?, a.get("rev")?, a.get("sha")?, a.get("name")?),
    skill_remove: Engine [admin, global] => profiles::skill_remove(a.get("name")?),

    // ── paired devices (devices.json; Phase 2 §2.5, §2.8, §3.2) ──────────
    // A listing is the caller's principal's devices (§3.3), tombstones too.
    devices_list: Engine [admin, global] => cx.devices.list(cx.caller.principal()),
    // A device may always revoke ITSELF; another device of its principal
    // needs `admin` (the object checks; the registry checks again).
    device_revoke: Own [own, revocable(id)] => cx.devices.revoke(&cx.caller, &authz::effective(&*cx.registry, &cx.caller), &device_id(a)?, now()),
    // Take capabilities or projects away from a PEER; never add, never self.
    device_narrow: Engine [admin, device(id)] => cx.devices.narrow(
        &cx.caller, &authz::effective(&*cx.registry, &cx.caller), &device_id(a)?, scope_arg(cx, a, false)?,
    ),
    // The host's alone (§2.5, §2.6, §3.2): widening, pausing, re-enabling,
    // expiry and step-up, and "every device but this one".
    device_set_scope: HostConsole [console, device(id)] => cx.devices.set_scope(&device_id(a)?, scope_arg(cx, a, true)?),
    device_disable: HostConsole [console, device(id)] => cx.devices.disable(&device_id(a)?, now()),
    device_enable: HostConsole [console, device(id)] => cx.devices.enable(&device_id(a)?, now()),
    device_set_expiry: HostConsole [console, device(id)] => cx.devices.set_expiry(&device_id(a)?, Expiry::from_value(&a.raw("expiry"))?),
    device_set_stepup: HostConsole [console, device(id)] => cx.devices.set_stepup(&device_id(a)?, a.get("required")?),
    devices_revoke_all_others: HostConsole [console, opt(device(keep))] => {
        let keep: Option<String> = a.get("keep")?;
        cx.devices.revoke_all_others(keep.filter(|k| !k.is_empty()).map(DeviceId::new).as_ref(), now())
    },
    // A new server key (§2.5): every device's pin is now wrong, so every
    // device is revoked and pairs again. Meant for a suspected compromise,
    // where a handover signed by the old key would be signed by the thief.
    // ── pairing (§2.2–§2.4): the host opens the window, the host decides ──
    // The scope and the expiry are chosen BEFORE the window exists (§2.2
    // step 1); a missing expiry is the default, 30 days unused.
    pair_begin: HostConsole => {
        let expiry = match a.raw("expiry") {
            Value::Null => Expiry::default(),
            v => Expiry::from_value(&v)?,
        };
        cx.pairing.begin(Offer { grant: scope_arg(cx, a, true)?, expiry }, now())
    },
    pair_decide: HostConsole => cx.pairing.decide(a.get("allow")?, now()),
    pair_cancel: HostConsole => ok(cx.pairing.cancel()),
    pair_status: HostConsole => ok(cx.pairing.status()),
    // ── step-up (§2.5): what a device does for itself, on its own connection ──
    stepup_challenge: Own => cx.frames.stepup().challenge(std::time::Instant::now()).map(|c| serde_json::json!({
        "challenge": worktrees_api::files::b64_encode(&c), "valid_secs": stepup::CHALLENGE_FOR.as_secs(),
    })),
    stepup_prove: Own => {
        let authz::Device::Paired(d) = cx.caller.device() else { return Err(Value::from("the console needs no step-up")) };
        let pin = cx.devices.get(cx.caller.principal(), d).and_then(|r| r.stepup_spki_sha256);
        let spki = stepup::b64_decode(&a.get::<String>("spki")?).ok_or("invalid args `spki`: not base64")?;
        let sig = stepup::b64_decode(&a.get::<String>("signature")?).ok_or("invalid args `signature`: not base64")?;
        cx.frames.stepup().prove(pin.as_deref(), &spki, &sig, std::time::Instant::now()).map(|_| serde_json::json!({
            "valid_secs": stepup::VALID_FOR.as_secs(),
        }))
    },
    identity_rotate: HostConsole => {
        let id = super::identity::Identity::rotate(&cx.ctx.config_dir)?;
        cx.devices.announce(super::devices::DeviceEvent::IdentityRotated { pin: id.pin().to_string() });
        let revoked = cx.devices.revoke_all_others(None, now())?;
        ok(serde_json::json!({ "pin": id.pin(), "revoked": revoked }))
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::{BTreeMap, HashSet};
    use std::sync::Arc;
    use worktrees_core::authz::NoDevices;

    use super::super::resolve::fixture::{git_repo, scratch, Fixture};

    /// A `Call` from the console over a temp config dir with a queue nobody
    /// drains — enough to dispatch the commands that touch only the config dir.
    fn call(tag: &str) -> (Call, std::path::PathBuf, tokio::sync::mpsc::UnboundedReceiver<tokio_tungstenite::tungstenite::Message>) {
        let dir = std::env::temp_dir().join(format!("wt-dispatch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        struct Quiet;
        impl worktrees_api::Host for Quiet {
            fn emit(&self, _: &str, _: Value) {}
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(dir.clone(), Arc::new(Quiet));
        let call = Call {
            ctx,
            frames: Frames::new(wire::Outbox::unbounded(tx)),
            caller: Caller::console(),
            registry: Arc::new(NoDevices),
            world: None,
            devices: Arc::new(DeviceRegistry::in_memory()),
            pairing: Arc::new(Pairing::new(None, Arc::new(DeviceRegistry::in_memory()))),
            flights: Default::default(),
        };
        (call, dir, rx)
    }

    fn args(v: Value) -> Args {
        Args(v.as_object().cloned().unwrap_or_default())
    }

    /// 170 rows, no duplicates: the command layer's (143 at the extraction;
    /// host policy's three getters, Kimi's six and the slots pair since) and
    /// Phase 2 slice 5's sixteen — the device rows, pairing and step-up; the
    /// check scripts hold the page's commands against them.
    #[test]
    fn the_table_has_every_command_once() {
        let names: HashSet<&str> = TABLE.iter().map(|r| r.name).collect();
        assert_eq!(names.len(), TABLE.len(), "a command is listed twice");
        assert_eq!(TABLE.len(), 170, "the table has 170 commands");
    }

    /// The client-local set, spelled out: these must never run on the host.
    #[test]
    fn client_local_commands_are_refused_without_running() {
        let (c, dir, _rx) = call("local");
        let want: HashSet<&str> = ["copy_text", "open_editor", "open_terminal", "detect_editors", "set_zoom", "install_app_version"].into();
        let tagged: HashSet<&str> = TABLE.iter().filter(|r| r.exposure == Exposure::ClientLocal).map(|r| r.name).collect();
        assert_eq!(tagged, want);
        for n in &want {
            // Args that would make the body misbehave if it ran: copy_text's
            // text would land on this machine's clipboard.
            let r = super::call(&c, n, &args(json!({"text": "NEVER", "path": "/", "cmd": "x", "session": "s", "factor": 1.0, "tag": "v0"})));
            assert_eq!(r, Err(Value::from(CLIENT_LOCAL)), "{n}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The docs viewer runs HERE now (its registrations are the host poll's to
    /// refresh), and a serve nobody handed a browser bundle — one started from
    /// the CLI or the dev stack — must refuse the open in words that say why,
    /// before the place is looked at by its body, and without panicking.
    #[test]
    fn open_docs_viewer_without_a_bundle_is_a_clear_refusal() {
        assert!(
            std::env::var("WORKTREES_VIEWER_JS").is_err(),
            "unset WORKTREES_VIEWER_JS to run this test — it decides the question under test",
        );
        assert_eq!(exposure("open_docs_viewer"), Some(Exposure::Engine));
        let (c, dir, _rx) = call("viewer");
        let root = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
        // The place must pass the resolver first: a registered project.
        let world = Fixture { roots: vec![root.clone()], panes: vec![] };
        let place = json!({"branch": "main", "behind": 0, "dirty": false, "dirty_files": null, "last_commit_subject": null, "last_commit_epoch": null});
        let r = super::call_in(&c, "open_docs_viewer", &args(json!({
            "repo": root, "root": root, "slug": "(main)", "path": null, "place": place,
        })), &world);
        let e = r.unwrap_err();
        let e = e.as_str().unwrap_or_default();
        assert!(e.contains("no documentation viewer bundle") && e.contains("WORKTREES_VIEWER_JS"), "{e}");
        // Nothing registered and no port bound on the way to saying so is
        // `viewer::open`'s own test (the precondition here runs before it).
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_command_is_an_error_not_a_default() {
        let (c, dir, _rx) = call("unknown");
        assert_eq!(super::call(&c, "not_a_command", &args(json!({}))), Err(Value::from("unknown command: not_a_command")));
        assert!(exposure("not_a_command").is_none());
        assert_eq!(exposure("term_open"), Some(Exposure::Stream));
        assert_eq!(cap_of("term_open"), Some(Capability::Stream));
        assert_eq!(cap_of("update_cli"), None, "a console act has no capability: the console holds them all");
        assert_eq!(cap_of("not_a_command"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Args by name, both spellings, missing = null: a settings round trip
    /// through the dispatcher with the keys the frontend actually sends.
    #[test]
    fn args_are_read_by_key_in_either_spelling_and_missing_is_null() {
        let (c, dir, _rx) = call("args");
        assert_eq!(super::call(&c, "get_settings", &args(json!({}))).unwrap(), Value::Null, "nothing stored yet → None → null");
        super::call(&c, "set_settings", &args(json!({"settings": {"theme": "nord"}}))).unwrap();
        assert_eq!(super::call(&c, "get_settings", &args(json!({}))).unwrap()["theme"], json!("nord"));
        // camelCase as the frontend spells it, snake_case as the table does.
        let a1 = args(json!({"days": 7, "tzOffsetMin": 0}));
        let a2 = args(json!({"days": 7, "tz_offset_min": 0}));
        assert!(super::call(&c, "ui_usage", &a1).is_ok());
        assert!(super::call(&c, "ui_usage", &a2).is_ok());
        // A required key missing is Tauri's `invalid args`, naming the key.
        let e = super::call(&c, "ui_usage", &args(json!({"days": 7}))).unwrap_err();
        assert!(e.as_str().unwrap().starts_with("invalid args `tz_offset_min`"), "{e}");
        assert_eq!(camel("tz_offset_min"), "tzOffsetMin");
        assert_eq!(camel("repo"), "repo");
        // A value the resolver rewrites replaces BOTH spellings.
        let mut a = args(json!({"intoRepo": "/x"}));
        a.set("into_repo", json!("/y"));
        assert_eq!(a.raw("into_repo"), json!("/y"));
        assert!(a.0.get("intoRepo").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `clone_project`'s error is the struct its dialog branches on, not a
    /// string — `err` carries what the Tauri `Err` serializes to.
    #[test]
    fn a_structured_error_crosses_the_wire_as_its_object() {
        let (c, dir, _rx) = call("cloneerr");
        let e = super::call(&c, "clone_project", &args(json!({
            "id": "j1", "url": "not a url at all", "parent": dir.to_string_lossy(), "name": null, "onProgress": "__CHANNEL__:1"
        })))
        .unwrap_err();
        assert!(e.is_object(), "{e}");
        assert!(e.get("kind").is_some(), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream command needs its channel; without one it is an args error
    /// naming the key, before anything is opened. (The session must be a
    /// place's to get that far: the fixture's pane puts it in a registered
    /// project, and no tmux server is asked.)
    #[test]
    fn a_stream_command_without_a_channel_is_an_args_error() {
        let (c, dir, _rx) = call("chan");
        let root = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
        let world = Fixture { roots: vec![root.clone()], panes: vec![("s".into(), root, "zsh".into())] };
        let e = super::call_in(&c, "term_open", &args(json!({"session": "s", "cols": 80, "rows": 24})), &world).unwrap_err();
        assert!(e.as_str().unwrap().starts_with("invalid args `on_bytes`"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Which requests share a lane: the writes, resizes, wheel, close and
    /// detach of one stream, by that stream; opens of attaches and everything
    /// that is not a stream run concurrently.
    #[test]
    fn stream_requests_are_keyed_by_their_stream_and_nothing_else_is() {
        let t = args(json!({ "id": 4, "data": [97] }));
        assert_eq!(lane("term_write", &t).as_deref(), Some("term:4"));
        assert_eq!(lane("term_resize", &args(json!({ "id": 4, "cols": 1, "rows": 1 }))).as_deref(), Some("term:4"));
        assert_eq!(lane("term_wheel", &t), lane("term_close", &t));
        assert_ne!(lane("term_write", &t), lane("term_write", &args(json!({ "id": 5 }))));
        assert_eq!(lane("term_open", &args(json!({ "session": "s" }))), None);
        let sh = args(json!({ "repo": "/r", "slug": "s", "index": 2, "data": [97] }));
        assert_eq!(lane("shell_write", &sh).as_deref(), Some("shell:\"/r\":\"s\":2"));
        assert_eq!(lane("shell_open", &sh), lane("shell_detach", &sh));
        assert_eq!(lane("shell_resize", &sh), lane("shell_write", &sh));
        assert_ne!(lane("shell_write", &sh), lane("shell_write", &args(json!({ "repo": "/r", "slug": "s", "index": 3 }))));
        assert_eq!(lane("list_workspace", &args(json!({}))), None);
        assert_eq!(lane("clone_project", &args(json!({}))), None);
        assert_eq!(lane("no_such", &args(json!({}))), None);
    }

    /// The console sees what it always saw: a filtered row's result passes
    /// through untouched for scope `"*"`, and a scoped device's is cut. Driven
    /// through `call_in` with a real registered project.
    #[test]
    fn a_filtered_result_is_cut_to_a_devices_scope_and_whole_for_the_console() {
        use worktrees_core::authz::{Caps, DeviceId, Grant, PrincipalId, Projects};
        let dir = scratch("filtered");
        git_repo(&dir.join("a"));
        let a = dir.join("a").to_string_lossy().into_owned();
        let world = Fixture { roots: vec![a.clone()], panes: vec![] };
        let (con, cfg, _rx) = call("filtered");
        let whole = super::call_in(&con, "planning_status", &args(json!({})), &world).unwrap();
        let mut reg = super::super::resolve::fixture::Devices::default();
        reg.devices.insert(
            DeviceId::new("phone"),
            (PrincipalId::owner(), Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::only([a.clone()]) }),
        );
        let phone = Call { caller: Caller::paired(PrincipalId::owner(), DeviceId::new("phone")), registry: Arc::new(reg), ..con };
        let cut = super::call_in(&phone, "planning_status", &args(json!({})), &world).unwrap();
        let roots = |v: &Value| v["projects"].as_array().map(|p| p.iter().map(|x| x["root"].clone()).collect::<Vec<_>>()).unwrap_or_default();
        assert!(roots(&cut).iter().all(|r| r == &json!(a)), "{cut}");
        assert_eq!(whole["default"], cut["default"], "host-wide fields are unchanged");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&cfg);
    }

    // ── what keeps the table honest (§3.3) ───────────────────────────────

    /// The shipped source of the `commands!` block, `mod tests` sliced off at
    /// the `#[cfg(test)]` that HEADS it (AGENTS.md: a cut at the first
    /// `#[cfg(test)]` reads an empty table). One entry per row: its name and
    /// its text up to the next row.
    fn row_sources() -> BTreeMap<String, String> {
        let src = include_str!("dispatch.rs");
        let shipped = src.split("\n#[cfg(test)]\nmod tests {").next().unwrap();
        let open = shipped.find("commands! { cx, a;").expect("the commands! block");
        let block = &shipped[open..];
        let mut out = BTreeMap::new();
        let mut cur: Option<String> = None;
        for line in block.lines() {
            let head = line.strip_prefix("    ").filter(|l| !l.starts_with(' ') && !l.starts_with("//"));
            let name = head.and_then(|l| l.split_once(": ")).map(|(n, _)| n).filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
            if let Some(n) = name {
                cur = Some(n.to_string());
            }
            if let Some(n) = &cur {
                let e: &mut String = out.entry(n.clone()).or_default();
                e.push_str(line);
                e.push('\n');
            }
        }
        out
    }

    /// The argument names that name an OBJECT (§3.3), and the arrays and
    /// nested structs that address one.
    fn names_an_object(key: &str) -> bool {
        const OBJECTS: &[&str] = &[
            "repo", "root", "path", "slug", "session", "id", "dir", "location", "parent", "doc", "index", "shell", "place",
            "places", "roots", "paths", "keep",
        ];
        OBJECTS.contains(&key) || key.starts_with("into_")
    }

    /// Arguments that LOOK like objects and are not, each with its reason.
    /// A new entry is a claim; write the reason down.
    const NOT_OBJECTS: &[(&str, &str, &str)] = &[
        ("delete_profile", "id", "a profile id: profiles are the host's, not a project's"),
        ("set_default_profile", "id", "a profile id (a console act)"),
        ("set_project_profile", "id", "a profile id; the project is `repo`, declared"),
        ("skill_install_local", "path", "any directory the user picks to install a skill FROM (a console act)"),
        ("open_docs_viewer", "place", "the place's branch and dirty state for the page header; addresses nothing"),
    ];

    /// Every `a.get("…")` a body reads that names an object is covered by the
    /// row's object spec, or listed in `NOT_OBJECTS` with a reason. A body
    /// that grows a `repo` argument its row does not resolve fails HERE, not
    /// in a review that has to notice it.
    #[test]
    fn every_object_argument_a_body_reads_is_declared() {
        let rows = row_sources();
        assert_eq!(rows.len(), TABLE.len(), "every row was found in the source: {:?}", rows.keys().collect::<Vec<_>>());
        let mut missing = Vec::new();
        for r in TABLE.iter().filter(|r| r.exposure != Exposure::ClientLocal) {
            let src = &rows[r.name];
            let covered: HashSet<&str> = r.objects.iter().flat_map(|o| o.args()).collect();
            for part in src.split("a.get").skip(1) {
                let Some(key) = part.split('"').nth(1) else { continue };
                if !names_an_object(key) || covered.contains(key) {
                    continue;
                }
                if NOT_OBJECTS.iter().any(|(n, k, _)| *n == r.name && *k == key) {
                    continue;
                }
                missing.push(format!("{}.{key}", r.name));
            }
        }
        assert!(missing.is_empty(), "arguments that name an object with no object spec: {missing:?}");
        // And the allowlist is not stale: each entry is still a read argument.
        for (n, k, _) in NOT_OBJECTS {
            assert!(rows[*n].contains(&format!("a.get(\"{k}\")")), "NOT_OBJECTS lists {n}.{k}, which the row no longer reads");
        }
    }

    /// `GLOBAL_OK` is the `global` rows, both ways: a row cannot become
    /// project-free by a one-word edit to its spec.
    #[test]
    fn the_global_rows_are_exactly_global_ok() {
        let global: HashSet<&str> = TABLE.iter().filter(|r| r.objects.iter().any(Obj::is_global)).map(|r| r.name).collect();
        let ok: HashSet<&str> = GLOBAL_OK.iter().copied().collect();
        assert_eq!(ok.len(), GLOBAL_OK.len(), "GLOBAL_OK lists a row twice");
        let outside: Vec<_> = global.difference(&ok).collect();
        let stale: Vec<_> = ok.difference(&global).collect();
        assert!(outside.is_empty(), "global rows not on GLOBAL_OK: {outside:?}");
        assert!(stale.is_empty(), "GLOBAL_OK names rows that are not global: {stale:?}");
    }

    /// Every row has exactly ONE base group — its capability, or its class
    /// when it has none — and the counts are §3.4's, plus the two host-policy
    /// getters #532 added after it (`read`), the slots pair (`slots_settings`
    /// `read`, `set_slot_capacity` console) and slice 5's rows: two `admin`
    /// (`devices_list`, `device_narrow`); eleven console (six device acts,
    /// `identity_rotate` and four pairing rows); and three a device makes for
    /// itself (`own`: `device_revoke`, `stepup_challenge`, `stepup_prove`). A
    /// row that moves group fails here until the count (and the proposal's
    /// table) move with it.
    #[test]
    fn every_row_has_one_base_group_and_the_counts_are_section_3_4s() {
        let mut groups: BTreeMap<&str, usize> = BTreeMap::new();
        for r in TABLE {
            let g = match (r.exposure, r.cap) {
                (Exposure::ClientLocal, None) => "client-local",
                (Exposure::HostConsole, None) => "console",
                (Exposure::Loopback, None) => "loopback-only",
                (Exposure::Own, None) => "own",
                (Exposure::Engine | Exposure::Stream, Some(c)) => c.name(),
                other => panic!("{} has no base group: {other:?}", r.name),
            };
            *groups.entry(g).or_default() += 1;
        }
        let want: BTreeMap<&str, usize> = [
            ("read", 56), ("write", 23), ("admin", 35), ("console", 33), ("stream", 10), ("instruct", 3),
            ("client-local", 6), ("loopback-only", 1), ("own", 3),
        ]
        .into();
        assert_eq!(groups, want);
        // The answer capability is defined and reserved: no row needs it until
        // Phase 3's `agent_answer`.
        assert!(TABLE.iter().all(|r| r.cap != Some(Capability::Answer)));
    }

    /// Every event the api emits has a projection (`scope::EVENTS`); one
    /// without would reach the console and nobody else, silently. Read from
    /// the api crate's source, every `emit("…")` outside its tests.
    #[test]
    fn every_emitted_event_has_a_projection() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../worktrees-api/src");
        let mut emitted = HashSet::new();
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            let shipped = text.split("\n#[cfg(test)]\nmod tests {").next().unwrap();
            for part in shipped.split("emit(").skip(1) {
                let t = part.trim_start();
                if let Some(rest) = t.strip_prefix('"') {
                    emitted.insert(rest.split('"').next().unwrap().to_string());
                }
            }
        }
        assert!(emitted.len() >= 5, "found the emitters: {emitted:?}");
        let projected: HashSet<String> = scope::EVENTS.iter().map(|(n, _)| n.to_string()).collect();
        let unprojected: Vec<_> = emitted.difference(&projected).collect();
        assert!(unprojected.is_empty(), "events with no projection: {unprojected:?}");
    }

    /// gap 17: a body's `Ctx` fetches as its caller. A device's is pinned to
    /// the resolver's ONE lookup, and any URL it was not pinned for is
    /// checked again; the console's is today's git, whatever it is handed.
    #[test]
    fn a_body_fetches_as_its_caller() {
        use worktrees_core::authz::{DeviceId, Grant, PrincipalId};
        use worktrees_core::fetchpolicy::Policy;
        let (con, _, _rx) = call("fetch-as");
        let pin = resolve::remote_only("https://140.82.112.3/o/r.git").unwrap();
        let resolved = || resolve::Resolved { pins: vec![pin.clone()], ..Default::default() };
        let ctx = body_ctx(&con, &Grant::everything(), &resolved());
        assert_eq!(*ctx.fetcher(), Fetcher::Console, "the console's git, even handed a pin");
        let dev = Call { caller: Caller::paired(PrincipalId::owner(), DeviceId::new("phone")), ..con };
        let ctx = body_ctx(&dev, &Grant::everything(), &resolved());
        assert_eq!(*ctx.fetcher(), Fetcher::Device(vec![pin.clone()]));
        assert_eq!(ctx.fetch_policy("https://140.82.112.3/o/r.git").unwrap(), Policy::Pinned(pin));
        assert!(ctx.fetch_policy("file:///etc").is_err(), "a URL the resolver did not pin is checked again");
        assert!(!ctx.fetching(Fetcher::Console).fetcher().is_console(), "a device's Ctx cannot be made the console's");
    }

    /// A refused call never joins a flight (§6): the resolver runs first, for
    /// each caller, and only a call it lets through shares a body. A console
    /// naming a project that is not registered and a device the registry
    /// grants nothing both ask for a shared read; neither joins. The
    /// console's `doctor` on a registered project joins exactly once.
    #[test]
    fn a_refused_call_never_joins_a_flight() {
        use worktrees_core::authz::{DeviceId, PrincipalId};
        let (con, dir, _rx) = call("flightrefuse");
        let root = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
        let world = Fixture { roots: vec![root.clone()], panes: vec![] };
        let flights = con.flights.clone();
        let gone = format!("{root}-not-registered");
        assert!(super::call_in(&con, "doctor", &args(json!({ "repo": gone })), &world).is_err());
        assert_eq!(flights.joins(), 0, "a project the resolver did not find");
        let _ = super::call_in(&con, "doctor", &args(json!({ "repo": root })), &world);
        assert_eq!(flights.joins(), 1, "the call the resolver let through");
        let phone = Call { caller: Caller::paired(PrincipalId::owner(), DeviceId::new("phone")), ..con };
        let denied = super::call_in(&phone, "doctor", &args(json!({ "repo": root })), &world);
        assert_eq!(denied, Err(json!(super::super::resolve::ERR_DENIED)));
        assert_eq!(flights.joins(), 1, "a device the registry grants nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reader's precheck answers what needs no I/O and passes the rest
    /// on. A device the registry grants nothing is told `denied` for a read,
    /// a refused probe is `false` exactly as `call_in` answers it, nonsense
    /// is `unknown command`, and a caller over its rate is told `rate`
    /// instead. The console's read passes, decided later by `call_in`.
    #[test]
    fn the_precheck_answers_what_needs_no_io_and_passes_the_rest() {
        use super::super::resolve::ERR_DENIED;
        use worktrees_core::authz::{DeviceId, PrincipalId};
        let phone = Caller::paired(PrincipalId::owner(), DeviceId::new("phone"));
        let free = || Ok(());
        let pre = |c: &Caller, cmd: &str, pay: &dyn Fn() -> Result<(), &'static str>| precheck(c, &NoDevices, cmd, &args(json!({})), &(), pay);
        assert_eq!(pre(&phone, "get_settings", &free), Some(Err(json!(ERR_DENIED))));
        assert_eq!(pre(&phone, "file_readable", &free), Some(Ok(json!(false))), "a probe's refusal is its answer");
        assert_eq!(pre(&phone, "not_a_command", &free), Some(Err(json!("unknown command: not_a_command"))));
        assert_eq!(pre(&phone, "get_settings", &|| Err("rate")), Some(Err(json!("rate"))), "over its rate");
        assert_eq!(pre(&Caller::console(), "get_settings", &|| panic!("a pass pays nothing")), None);
        assert_eq!(pre(&Caller::console(), "read_file", &free), None, "objects are call_in's");
    }

    /// What a `Witness` heard, in order.
    #[derive(Default)]
    struct Heard(std::sync::Mutex<Vec<String>>);

    impl Witness for Heard {
        fn unknown(&self, cmd: &str) {
            self.0.lock().unwrap().push(format!("unknown {cmd}"));
        }
        fn refused(&self, row: &'static Row, _: &Args, r: &resolve::Refusal) {
            self.0.lock().unwrap().push(format!("refused {} {:?}", row.name, r.kind));
        }
        fn admitted(&self, row: &'static Row, _: &Args, _: &resolve::Resolved) {
            self.0.lock().unwrap().push(format!("admitted {}", row.name));
        }
    }

    fn heard(f: impl FnOnce(&Heard)) -> Vec<String> {
        let h = Heard::default();
        f(&h);
        h.0.into_inner().unwrap()
    }

    /// §5.1: every refusal is witnessed exactly once, whichever of the two
    /// sites makes it — the reader's precheck (exposure, capability, an
    /// unknown name) or `call_in_seen` (an object, the stale close answered
    /// `ok`) — and a request refused for its RATE is not witnessed at all:
    /// it is counted into that limit's episode (`budget.rs`), so a flood
    /// costs the log one record, not one a request.
    #[test]
    fn every_refusal_is_witnessed_exactly_once_wherever_it_is_made() {
        use worktrees_core::authz::{DeviceId, PrincipalId};
        let (con, dir, _rx) = call("witness");
        let phone = Caller::paired(PrincipalId::owner(), DeviceId::new("phone"));
        let none = args(json!({}));
        let pre = |c: &Caller, cmd: &str, h: &Heard| precheck(c, &NoDevices, cmd, &none, h, || Ok(()));
        assert_eq!(heard(|h| drop(pre(&phone, "get_settings", h))), ["refused get_settings Denied"]);
        assert_eq!(heard(|h| drop(pre(&phone, "copy_text", h))), ["refused copy_text ClientLocal"]);
        assert_eq!(heard(|h| drop(pre(&phone, "nope", h))), ["unknown nope"]);
        assert_eq!(heard(|h| drop(precheck(&phone, &NoDevices, "get_settings", &none, h, || Err("rate")))), Vec::<String>::new());
        let world = Fixture { roots: vec![], panes: vec![] };
        let gone = args(json!({ "repo": dir.join("not-registered").to_string_lossy() }));
        assert_eq!(heard(|h| drop(call_in_seen(&con, "doctor", &gone, &world, h))), ["refused doctor NotFound"]);
        let stale = args(json!({ "id": 987_654 }));
        let mut out = None;
        assert_eq!(heard(|h| out = Some(call_in_seen(&con, "term_close", &stale, &world, h))), ["refused term_close NoopNotOwned"]);
        assert_eq!(out, Some(Ok(Value::Null)), "the stale close is answered ok, and recorded once");
        assert_eq!(heard(|h| drop(call_in_seen(&con, "nope", &none, &world, h))), ["unknown nope"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step-up (§2.5) is the caller's own verdict, so it is asked BEFORE a
    /// flight is joined. A device whose step-up is due, asking for
    /// `diagnostics` — `admin`, so drive-class, and one of §6's shared reads
    /// — is refused `step-up` and joins nothing. Joined first, it could be
    /// handed the answer a console leader computed.
    #[test]
    fn step_up_is_asked_before_a_flight_is_joined() {
        use super::super::resolve::fixture::call_as;
        use worktrees_core::authz::{DeviceId, Grant, PrincipalId};
        struct Due;
        impl Registry for Due {
            fn principal_grant(&self, _: &PrincipalId) -> Option<Grant> {
                Some(Grant::everything())
            }
            fn device_grant(&self, _: &PrincipalId, _: &DeviceId) -> Option<Grant> {
                Some(Grant::everything())
            }
            fn device_principal(&self, _: &DeviceId) -> Option<PrincipalId> {
                Some(PrincipalId::owner())
            }
            fn stepup_required(&self, _: &PrincipalId, _: &DeviceId) -> bool {
                true
            }
            fn generation(&self) -> u64 {
                0
            }
        }
        let cx = call_as(Caller::paired(PrincipalId::owner(), DeviceId::new("phone")), Arc::new(Due));
        let world = Fixture { roots: vec![], panes: vec![] };
        let mut out = None;
        let h = heard(|h| out = Some(call_in_seen(&cx, "diagnostics", &args(json!({})), &world, h)));
        assert_eq!(out, Some(Err(json!(super::super::stepup::ERR_STEPUP))));
        assert_eq!(h, ["refused diagnostics StepUp"]);
        assert_eq!(cx.flights.joins(), 0, "a call step-up refused joins no flight");
    }

    /// A flight's follower never reaches a device launch's lane gate, whose
    /// lock is held across the leader's body (§6, LLM10). Four calls from a
    /// device of a row that both shares and is gated (the test row, which
    /// no real row is) share ONE run of the body, and each is witnessed as
    /// admitted. Join the flight after the gate instead and the leader holds
    /// the lock while the others wait on it outside the flight: each then
    /// runs the body itself, one after another, and answers 1, 2, 3, 4.
    #[test]
    fn a_follower_never_reaches_the_lane_gate() {
        use super::super::resolve::fixture::{call_as, Devices};
        use worktrees_core::authz::{Caps, DeviceId, Grant, PrincipalId, Projects};
        let mut reg = Devices::default();
        let read = Grant { capabilities: Caps::of(&[Capability::Read]), projects: Projects::All };
        reg.devices.insert(DeviceId::new("phone"), (PrincipalId::owner(), read));
        let reg = Arc::new(reg);
        let phone = Caller::paired(PrincipalId::owner(), DeviceId::new("phone"));
        let flights: Arc<Flights> = Default::default();
        let world = Arc::new(Fixture { roots: vec![], panes: vec![] });
        let tag = format!("gate-follow-{}", std::process::id());
        let calls: Vec<_> = (0..4)
            .map(|_| {
                let cx = Call { flights: flights.clone(), ..call_as(phone.clone(), reg.clone()) };
                let (a, world) = (args(json!({ "tag": tag, "joins": 4 })), world.clone());
                std::thread::spawn(move || {
                    let h = Heard::default();
                    let out = call_in_seen(&cx, TEST_SHARED, &a, &*world, &h);
                    (out, h.0.into_inner().unwrap())
                })
            })
            .collect();
        for c in calls {
            let (out, heard) = c.join().unwrap();
            assert_eq!(out, Ok(json!(1)), "every caller got the one run's answer");
            assert_eq!(heard, [format!("admitted {TEST_SHARED}")]);
        }
        assert_eq!(flights.joins(), 4);
    }
}

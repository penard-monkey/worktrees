//! Keep awake: hold off this host's idle sleep while the engine runs.
//!
//! The agents and their tmux server are on the HOST, so the hold is the
//! engine's, not a window's: a window driving a paired host turns the host's
//! on and off. It is one switch, off by default, kept in `host-policy.json`
//! (`keep_awake`) so a restarted serve holds it again. It is held whether or
//! not an agent is busy at this moment, and it ends with serve.
//!
//! **What is held.** Idle sleep only. The display may still dim and sleep, and
//! closing the lid, the Sleep command and a low battery still put the computer
//! to sleep, on both systems.
//!
//! **macOS: an IOKit power assertion, in this process**
//! (`IOPMAssertionCreateWithName`, `PreventUserIdleSystemSleep`), which is what
//! `caffeinate -i` makes. Measured on 27.2.1 with `pmset -g assertions`: both
//! end with their process, SIGKILL included (`caffeinate` needs `-w <pid>` for
//! that), so nothing needs reaping either way. The difference is the name:
//! ours reads `pid N(worktrees) … named: "worktrees: Keep awake is on"`, and
//! every `caffeinate` reads `caffeinate command-line tool`, Claude Code's own
//! among them. No crate and no child. `caffeinate -s` (`PreventSystemSleep`)
//! is deliberately not matched: IOPMLib.h deprecates that type ("not supported
//! in any OS X releases").
//!
//! **Linux: `systemd-inhibit`, a child.** logind's lock lives as long as its
//! file descriptor, which takes D-Bus to get, and a D-Bus crate is thirty
//! crates (the mDNS decision). The command it runs is a `cat` on a pipe this
//! process holds: when serve goes, however it goes, the pipe closes, `cat`
//! reads its end and the lock is dropped. It prints one line first, and
//! systemd-inhibit starts it only once it has the lock, so that line is the
//! proof. Available where systemd is the init and the command is on `PATH`;
//! anywhere else [`State::supported`] is false and no client draws a switch.
//!
//! A change is announced as `keep_awake:changed`, whose payload is the whole
//! [`State`].

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::{hostpolicy, log, Ctx};

/// How the assertion names itself to the system (`pmset -g assertions`,
/// `systemd-inhibit --list`).
const NAME: &str = "worktrees: Keep awake is on";

/// How long a hold that died on its own (somebody ended the child) waits
/// before the poll tries again.
const RETRY: Duration = Duration::from_secs(60);

/// A hold in effect. Dropping it lets the computer sleep again.
pub trait Held: Send {
    /// Still in effect. Only a child can go by itself.
    fn alive(&mut self) -> bool {
        true
    }
}

/// A way to take one.
pub trait Mechanism: Send + Sync {
    /// Named in the log and in Settings.
    fn id(&self) -> &'static str;
    fn hold(&self) -> Result<Box<dyn Held>, String>;
}

/// What a client is told.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct State {
    /// This host has a mechanism. False: no switch is drawn.
    pub supported: bool,
    /// The host's setting.
    pub on: bool,
    /// The hold is in effect now. `on` without it means `error` says why.
    pub held: bool,
    /// `iokit` or `systemd-inhibit`.
    pub mechanism: Option<&'static str>,
    /// `macos` or `linux`: a client names the HOST's computer, not its own.
    pub os: &'static str,
    pub error: Option<String>,
}

#[derive(Default)]
struct Inner {
    on: bool,
    held: Option<Box<dyn Held>>,
    error: Option<String>,
    /// When a hold that should be there was last missed.
    tried: Option<Instant>,
}

/// The engine's one hold. A registry of the process, shared by every clone of
/// its `Ctx`.
pub struct KeepAwake {
    mech: Option<Arc<dyn Mechanism>>,
    inner: Mutex<Inner>,
}

impl Default for KeepAwake {
    fn default() -> Self {
        KeepAwake::with(platform())
    }
}

impl KeepAwake {
    /// With a given mechanism, or none: the tests' seam, so that none of them
    /// keeps a machine awake.
    pub fn with(mech: Option<Arc<dyn Mechanism>>) -> Self {
        KeepAwake { mech, inner: Mutex::new(Inner::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn state_of(&self, i: &Inner) -> State {
        State {
            supported: self.mech.is_some(),
            on: i.on,
            held: i.held.is_some(),
            mechanism: self.mech.as_ref().map(|m| m.id()),
            os: std::env::consts::OS,
            error: i.error.clone(),
        }
    }

    pub fn state(&self) -> State {
        self.state_of(&self.lock())
    }

    /// The act: hold or let go, and persist it. All or nothing: a hold that
    /// cannot be taken is not written down, and a write that fails changes
    /// nothing. The lock is held across both, so two clients' acts land in
    /// one order in memory and in the file.
    fn set(&self, on: bool, persist: impl FnOnce(bool) -> Result<(), String>) -> Result<State, String> {
        let mut i = self.lock();
        if !on {
            persist(false)?;
            i.held = None;
            i.error = None;
            i.on = false;
            return Ok(self.state_of(&i));
        }
        let mech = self.mech.as_ref().ok_or_else(unsupported)?;
        if !i.held.as_mut().is_some_and(|h| h.alive()) {
            let h = mech.hold()?;
            persist(true)?;
            i.held = Some(h);
        } else {
            persist(true)?;
        }
        i.error = None;
        i.tried = None;
        i.on = true;
        Ok(self.state_of(&i))
    }

    /// What the file says, at boot. A hold that fails leaves the setting on
    /// and the reason in the state, and the poll tries again. The file is read
    /// under the lock, so a client's act just before this is not undone by an
    /// older reading of it.
    fn restore(&self, read: impl FnOnce() -> bool, now: Instant) -> State {
        let mut i = self.lock();
        i.on = read() && self.mech.is_some();
        if i.on && i.held.is_none() {
            self.take(&mut i, now);
        }
        self.state_of(&i)
    }

    fn take(&self, i: &mut Inner, now: Instant) {
        let Some(mech) = self.mech.as_ref() else { return };
        i.tried = Some(now);
        match mech.hold() {
            Ok(h) => {
                i.held = Some(h);
                i.error = None;
                log::info(&format!("keep-awake: held ({})", mech.id()));
            }
            Err(e) => {
                log::warn(&format!("keep-awake: {e}"));
                i.error = Some(e);
            }
        }
    }

    /// The poll's beat: a hold that ended on its own is noticed and, after
    /// [`RETRY`], taken again. `Some` when what a client sees has moved.
    fn check(&self, now: Instant) -> Option<State> {
        let mut i = self.lock();
        if !i.on {
            return None;
        }
        let before = self.state_of(&i);
        if i.held.as_mut().is_some_and(|h| !h.alive()) {
            i.held = None;
            i.tried = Some(now);
            i.error = Some("the hold ended on its own; it is tried again within a minute".into());
            log::warn("keep-awake: the hold ended on its own");
        }
        if i.held.is_none() && i.tried.is_none_or(|t| now.duration_since(t) >= RETRY) {
            self.take(&mut i, now);
        }
        let after = self.state_of(&i);
        (after != before).then_some(after)
    }

    /// Let go, the setting untouched: the engine is stopping.
    pub fn release(&self) {
        let mut i = self.lock();
        // Off in memory too, so a poll tick still running takes nothing back.
        i.on = false;
        if i.held.take().is_some() {
            log::info("keep-awake: released (the engine is stopping)");
        }
    }
}

fn unsupported() -> String {
    "this computer has nothing Worktrees can hold sleep off with (Linux needs systemd and systemd-inhibit)".into()
}

/// The rail button and Settings → General → Keep awake: HOST policy,
/// persisted in host-policy.json and held by the engine.
pub fn set_keep_awake(ctx: &Ctx, on: bool) -> Result<State, String> {
    let s = ctx
        .keep_awake
        .set(on, |on| hostpolicy::set(&ctx.config_dir, hostpolicy::KEEP_AWAKE, on))
        .inspect_err(|e| log::error(&format!("set_keep_awake: {e}")))?;
    log::info(&format!("host-policy: keep_awake set to {on} ({})", s.mechanism.unwrap_or("no mechanism")));
    announce(ctx, &s);
    Ok(s)
}

pub fn get_keep_awake(ctx: &Ctx) -> Result<State, String> {
    Ok(ctx.keep_awake.state())
}

/// What the poll's thread does first: a hold can take seconds to be granted
/// or refused (`systemd-inhibit`), which engine start must not wait for.
pub fn start(ctx: &Ctx) {
    let s = ctx.keep_awake.restore(|| hostpolicy::keep_awake(&ctx.config_dir), Instant::now());
    if s.on {
        announce(ctx, &s);
    }
}

/// The poll's beat.
pub fn tick(ctx: &Ctx) {
    if let Some(s) = ctx.keep_awake.check(Instant::now()) {
        announce(ctx, &s);
    }
}

fn announce(ctx: &Ctx, s: &State) {
    ctx.emit("keep_awake:changed", serde_json::to_value(s).unwrap_or(serde_json::Value::Null));
}

// ── the mechanisms ───────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn platform() -> Option<Arc<dyn Mechanism>> {
    Some(Arc::new(iokit::Iokit))
}

#[cfg(target_os = "linux")]
fn platform() -> Option<Arc<dyn Mechanism>> {
    // `sd_booted(3)`'s own test, then the command.
    if !std::path::Path::new("/run/systemd/system").is_dir() {
        return None;
    }
    worktrees_core::profile::bin_on_path("systemd-inhibit")?;
    Some(Arc::new(inhibit::Inhibit::new(inhibit::ATTEMPTS.map(inhibit::argv).to_vec())))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform() -> Option<Arc<dyn Mechanism>> {
    None
}

#[cfg(target_os = "macos")]
mod iokit {
    use super::{Held, Mechanism, NAME};
    use std::ffi::{c_char, c_void, CString};

    type CFStringRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(kind: CFStringRef, level: u32, name: CFStringRef, id: *mut u32) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    const UTF8: u32 = 0x0800_0100;
    const LEVEL_ON: u32 = 255;
    /// `kIOPMAssertPreventUserIdleSystemSleep`.
    const KIND: &str = "PreventUserIdleSystemSleep";

    pub struct Iokit;

    struct Assertion(u32);

    impl Held for Assertion {}

    impl Drop for Assertion {
        fn drop(&mut self) {
            // SAFETY: an id this process was given and releases once.
            unsafe { IOPMAssertionRelease(self.0) };
        }
    }

    /// A CFString, released with its owner.
    struct Cf(CFStringRef);

    impl Cf {
        fn new(s: &str) -> Result<Cf, String> {
            let c = CString::new(s).map_err(|e| e.to_string())?;
            // SAFETY: a NUL-terminated UTF-8 string, copied by the call.
            let r = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) };
            if r.is_null() {
                return Err("CFStringCreateWithCString failed".into());
            }
            Ok(Cf(r))
        }
    }

    impl Drop for Cf {
        fn drop(&mut self) {
            // SAFETY: created above with a +1 reference, released once.
            unsafe { CFRelease(self.0) };
        }
    }

    impl Mechanism for Iokit {
        fn id(&self) -> &'static str {
            "iokit"
        }

        fn hold(&self) -> Result<Box<dyn Held>, String> {
            let (kind, name) = (Cf::new(KIND)?, Cf::new(NAME)?);
            let mut id = 0u32;
            // SAFETY: both strings outlive the call, which retains what it
            // keeps; `id` is a plain out-parameter.
            let r = unsafe { IOPMAssertionCreateWithName(kind.0, LEVEL_ON, name.0, &mut id) };
            if r != 0 {
                return Err(format!("macOS refused the power assertion (IOReturn {r:#x})"));
            }
            Ok(Box::new(Assertion(id)))
        }
    }
}

/// Compiled on every unix so that its tests run on the Mac too; only Linux
/// builds one for real.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod inhibit {
    use super::{Held, Mechanism, NAME};
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Child, ChildStdin, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// What is held, best first. `sleep` as a block lock is what stops a
    /// desktop's own auto-suspend, which asks logind to suspend; polkit
    /// refuses it to a session that is not a local one (`allow_any: no`,
    /// systemd 255), where `idle` alone is still granted.
    pub const ATTEMPTS: [&str; 2] = ["idle:sleep", "idle"];

    /// Runs only once systemd-inhibit holds the lock. `cat` ends when the
    /// pipe serve holds does.
    pub const HOLDER: &str = "echo held; exec cat >/dev/null";

    /// How long systemd-inhibit gets to take the lock or refuse.
    const TAKE: Duration = Duration::from_secs(5);
    /// And to go, once its pipe is closed, before it is killed.
    const LET_GO: Duration = Duration::from_secs(2);

    pub fn argv(what: &str) -> Vec<String> {
        ["systemd-inhibit", &format!("--what={what}"), "--who=worktrees", &format!("--why={NAME}"), "--mode=block", "sh", "-c", HOLDER]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    pub struct Inhibit {
        attempts: Vec<Vec<String>>,
        take: Duration,
    }

    impl Inhibit {
        pub fn new(attempts: Vec<Vec<String>>) -> Self {
            Inhibit { attempts, take: TAKE }
        }

        #[cfg(test)]
        pub fn within(attempts: Vec<Vec<String>>, take: Duration) -> Self {
            Inhibit { attempts, take }
        }
    }

    pub struct Holder {
        child: Child,
        pipe: Option<ChildStdin>,
    }

    #[cfg(test)]
    impl Holder {
        pub fn pid(&self) -> u32 {
            self.child.id()
        }
    }

    impl Held for Holder {
        fn alive(&mut self) -> bool {
            matches!(self.child.try_wait(), Ok(None))
        }
    }

    impl Drop for Holder {
        fn drop(&mut self) {
            drop(self.pipe.take());
            let deadline = Instant::now() + LET_GO;
            while Instant::now() < deadline {
                if !matches!(self.child.try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    impl Mechanism for Inhibit {
        fn id(&self) -> &'static str {
            "systemd-inhibit"
        }

        fn hold(&self) -> Result<Box<dyn Held>, String> {
            self.take().map(|h| Box::new(h) as Box<dyn Held>)
        }
    }

    impl Inhibit {
        pub fn take(&self) -> Result<Holder, String> {
            let mut last = String::from("no way to hold sleep off");
            for argv in &self.attempts {
                match attempt(argv, self.take) {
                    Ok(h) => return Ok(h),
                    Err(e) => last = e,
                }
            }
            Err(last)
        }
    }

    fn attempt(argv: &[String], within: Duration) -> Result<Holder, String> {
        let mut child = worktrees_core::childenv::command(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", argv[0]))?;
        let pipe = child.stdin.take();
        let out = child.stdout.take().expect("piped");
        let err = child.stderr.take().expect("piped");
        let mut holder = Holder { child, pipe };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(out.take(64)).read_line(&mut line);
            let _ = tx.send(line);
        });
        match rx.recv_timeout(within) {
            Ok(l) if l.trim() == "held" => Ok(holder),
            Ok(_) => {
                // Its stdout closed without the line: it refused and is
                // going. Not waited on past the bound (`Drop`).
                drop(holder.pipe.take());
                let deadline = Instant::now() + LET_GO;
                while holder.alive() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let mut why = String::new();
                if !holder.alive() {
                    let _ = err.take(400).read_to_string(&mut why);
                }
                let why = why.trim();
                Err(format!("{}: {}", argv[0], if why.is_empty() { "it ended without holding anything" } else { why }))
            }
            Err(_) => {
                let _ = holder.child.kill();
                Err(format!("{}: no answer within {}s", argv[0], within.as_secs().max(1)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Host;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// A mechanism that keeps nothing awake and counts what it is asked.
    #[derive(Default)]
    struct Fake {
        taken: AtomicUsize,
        live: Arc<AtomicUsize>,
        refuse: AtomicBool,
        /// Set: every hold given out reads as ended on its own.
        dead: Arc<AtomicBool>,
    }

    struct FakeHeld(Arc<AtomicUsize>, Arc<AtomicBool>);

    impl Held for FakeHeld {
        fn alive(&mut self) -> bool {
            !self.1.load(Ordering::SeqCst)
        }
    }

    impl Drop for FakeHeld {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl Mechanism for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn hold(&self) -> Result<Box<dyn Held>, String> {
            if self.refuse.load(Ordering::SeqCst) {
                return Err("refused".into());
            }
            self.taken.fetch_add(1, Ordering::SeqCst);
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeHeld(self.live.clone(), self.dead.clone())))
        }
    }

    impl Fake {
        fn live(&self) -> usize {
            self.live.load(Ordering::SeqCst)
        }
    }

    #[derive(Default)]
    struct Rec(Mutex<Vec<(String, serde_json::Value)>>);

    impl Host for Rec {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().unwrap().push((event.into(), payload));
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-keepawake-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn engine(d: &Path, mech: Option<Arc<Fake>>) -> (Ctx, Arc<Rec>) {
        let rec = Arc::new(Rec::default());
        let mech = mech.map(|m| m as Arc<dyn Mechanism>);
        (Ctx::new(d.to_path_buf(), rec.clone()).with_keep_awake(KeepAwake::with(mech)), rec)
    }

    fn on_disk(d: &Path) -> Option<serde_json::Value> {
        hostpolicy::read(d).get(hostpolicy::KEEP_AWAKE).cloned()
    }

    #[test]
    fn it_is_off_until_turned_on_and_a_set_holds_persists_and_announces() {
        let d = dir("set");
        let f = Arc::new(Fake::default());
        let (ctx, rec) = engine(&d, Some(f.clone()));
        start(&ctx);
        assert_eq!((f.live(), get_keep_awake(&ctx).unwrap().on, on_disk(&d)), (0, false, None));

        let s = set_keep_awake(&ctx, true).unwrap();
        assert!(s.supported && s.on && s.held, "{s:?}");
        assert_eq!((f.live(), on_disk(&d)), (1, Some(true.into())));
        // Asking twice holds once.
        set_keep_awake(&ctx, true).unwrap();
        assert_eq!((f.live(), f.taken.load(Ordering::SeqCst)), (1, 1));

        let s = set_keep_awake(&ctx, false).unwrap();
        assert!(!s.on && !s.held, "{s:?}");
        assert_eq!((f.live(), on_disk(&d)), (0, Some(false.into())));
        let got: Vec<(String, bool)> = rec.0.lock().unwrap().iter().map(|(e, p)| (e.clone(), p["held"] == true)).collect();
        assert_eq!(got, [("keep_awake:changed".into(), true), ("keep_awake:changed".into(), true), ("keep_awake:changed".into(), false)]);
    }

    #[test]
    fn a_restarted_engine_holds_again_and_a_stopping_one_lets_go_and_keeps_the_setting() {
        let d = dir("restart");
        let f = Arc::new(Fake::default());
        let (ctx, _) = engine(&d, Some(f.clone()));
        set_keep_awake(&ctx, true).unwrap();
        ctx.keep_awake.release();
        assert_eq!((f.live(), on_disk(&d)), (0, Some(true.into())), "a stop is not a set");
        tick(&ctx);
        assert_eq!(f.live(), 0, "a poll tick after the stop took it back");

        let again = Arc::new(Fake::default());
        let (ctx, _) = engine(&d, Some(again.clone()));
        start(&ctx);
        assert_eq!(again.live(), 1);
        assert!(get_keep_awake(&ctx).unwrap().held);
    }

    #[test]
    fn a_refused_hold_or_a_refused_write_changes_nothing() {
        let d = dir("refused");
        let f = Arc::new(Fake::default());
        let (ctx, rec) = engine(&d, Some(f.clone()));
        f.refuse.store(true, Ordering::SeqCst);
        assert_eq!(set_keep_awake(&ctx, true).unwrap_err(), "refused");
        assert_eq!((on_disk(&d), get_keep_awake(&ctx).unwrap().on), (None, false));

        // A file the host cannot write over: the hold just taken is let go.
        f.refuse.store(false, Ordering::SeqCst);
        std::fs::write(hostpolicy::path(&d), "{ not json").unwrap();
        assert!(set_keep_awake(&ctx, true).unwrap_err().contains("not valid JSON"));
        assert_eq!((f.live(), get_keep_awake(&ctx).unwrap().on), (0, false));
        assert!(rec.0.lock().unwrap().is_empty(), "a refused act was announced");

        // And a hold in effect survives a refused OFF.
        std::fs::remove_file(hostpolicy::path(&d)).unwrap();
        set_keep_awake(&ctx, true).unwrap();
        std::fs::write(hostpolicy::path(&d), "{ not json").unwrap();
        assert!(set_keep_awake(&ctx, false).is_err());
        assert_eq!((f.live(), get_keep_awake(&ctx).unwrap().held), (1, true));
    }

    #[test]
    fn a_host_with_no_mechanism_says_so_and_refuses_on() {
        let d = dir("none");
        let (ctx, _) = engine(&d, None);
        // Another host's file (a synced config dir) asks for it anyway.
        hostpolicy::set(&d, hostpolicy::KEEP_AWAKE, true).unwrap();
        start(&ctx);
        let s = get_keep_awake(&ctx).unwrap();
        assert!(!s.supported && !s.on && !s.held, "{s:?}");
        assert!(set_keep_awake(&ctx, true).unwrap_err().contains("systemd-inhibit"));
    }

    #[test]
    fn a_hold_that_ends_on_its_own_is_said_and_taken_again_after_the_wait() {
        let f = Arc::new(Fake::default());
        let k = KeepAwake::with(Some(f.clone()));
        let t0 = Instant::now();
        k.restore(|| true, t0);
        assert_eq!(k.check(t0), None, "nothing moved");
        f.dead.store(true, Ordering::SeqCst);
        let s = k.check(t0 + Duration::from_secs(3)).expect("the loss is a change");
        assert!(s.on && !s.held && s.error.is_some(), "{s:?}");
        f.dead.store(false, Ordering::SeqCst);
        assert_eq!(k.check(t0 + Duration::from_secs(6)), None, "not retried inside the wait");
        let s = k.check(t0 + Duration::from_secs(3) + RETRY).expect("taken again");
        assert!(s.held && s.error.is_none(), "{s:?}");
        assert_eq!(f.live(), 1);
    }

    #[test]
    fn a_value_this_build_does_not_know_is_off() {
        let d = dir("odd");
        std::fs::write(hostpolicy::path(&d), r#"{"keep_awake": "yes", "other": 1}"#).unwrap();
        assert!(!hostpolicy::keep_awake(&d));
        // A key another build wrote survives this one's write.
        hostpolicy::set(&d, hostpolicy::KEEP_AWAKE, true).unwrap();
        assert!(hostpolicy::keep_awake(&d));
        assert_eq!(hostpolicy::read(&d).get("other"), Some(&1.into()));
    }

    // ── the Linux child, on any unix ────────────────────────────────────────

    fn sh(script: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), script.into()]
    }

    /// Gone, or a zombie nobody has reaped.
    fn gone(pid: u32) -> bool {
        let out = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        s.is_empty() || s.starts_with('Z')
    }

    #[test]
    fn the_child_is_held_until_dropped_and_goes_with_its_pipe() {
        let m = inhibit::Inhibit::new(vec![sh(inhibit::HOLDER)]);
        let mut h = m.take().unwrap();
        let pid = h.pid();
        assert!(h.alive() && !gone(pid));
        let t = Instant::now();
        drop(h);
        assert!(gone(pid));
        assert!(t.elapsed() < Duration::from_secs(2), "it was killed, not let go: {:?}", t.elapsed());
    }

    #[test]
    fn a_refused_attempt_falls_to_the_next_and_the_last_ones_reason_is_said() {
        let no = sh("echo 'Failed to inhibit: Access denied' >&2; exit 1");
        assert!(inhibit::Inhibit::new(vec![no.clone(), sh(inhibit::HOLDER)]).take().is_ok());
        let e = inhibit::Inhibit::new(vec![no]).take().err().unwrap();
        assert!(e.contains("Access denied"), "{e}");
        assert!(inhibit::Inhibit::new(vec![vec!["/nonexistent/inhibit".into()]]).take().is_err());
    }

    #[test]
    fn a_child_that_never_answers_is_ended() {
        let t = Instant::now();
        let m = inhibit::Inhibit::within(vec![sh("exec sleep 60")], Duration::from_millis(300));
        let e = m.take().err().unwrap();
        assert!(e.contains("no answer"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    }

    #[test]
    fn the_inhibit_command_names_worktrees_and_blocks() {
        let a = inhibit::argv("idle:sleep");
        assert_eq!(a[0], "systemd-inhibit");
        for want in ["--what=idle:sleep", "--who=worktrees", "--mode=block"] {
            assert!(a.iter().any(|x| x == want), "{a:?}");
        }
    }
}

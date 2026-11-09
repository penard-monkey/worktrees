//! The windows' frames across launches — Settings → Behavior → "Restore
//! window". Quit fullscreen, relaunch (by hand or through an in-app update) and
//! the window comes back fullscreen; quit it small in a corner, it comes back
//! small in that corner.
//!
//! Why not `tauri-plugin-window-state`: its `Resized` handler records the size
//! on every resize that is not a MAXIMIZE, fullscreen included. Quit while
//! fullscreen and the "normal" size it saved is the whole screen, so the window
//! comes back fullscreen and then, on leaving fullscreen, stays screen-sized —
//! the one case this setting exists for, restored wrong. Here the frame is
//! recorded only while the window is in none of the special states, and the
//! states are kept beside it, so leaving fullscreen lands on the frame you had
//! before entering it.
//!
//! Three rules the rest of this file follows:
//! - `ui-state.json` is READ, never written: the frontend writes it whole-blob
//!   and would erase anything the backend put there. The frame lives in its own
//!   backend-owned `window-state.json`, like `shell-cwds.json`.
//! - It is ALWAYS recorded and only restored when the setting is on, so turning
//!   the setting on picks up where the last session actually was.
//! - Saved WHILE the app runs, about a second after the window settles, and
//!   never at exit. The first version saved on `RunEvent::Exit` and a real
//!   fullscreen → ⌘Q → relaunch came back windowed, with the file saying
//!   `fullscreen: false` over the pre-fullscreen frame: whatever the window
//!   reports while AppKit tears it down is not the state the user left it in.
//!   Freezing at exit and writing only settled states makes the quit path —
//!   ⌘Q, closing the window, the updater's `relaunch()`, even a crash —
//!   irrelevant. The price is a move made in the last second before quitting.
//!
//! Two windows (Ideas phase 0). `main` is declared in tauri.conf.json; the
//! Ideas `companion` is made on demand (companion.rs). Each has a SLOT here,
//! under the rules above, and the one file holds both:
//! - `main`'s state stays at the TOP level, exactly where every earlier build
//!   reads it; the others sit under `windows`, which an older build ignores
//!   (serde skips unknown keys). A downgrade keeps the main window's frame and
//!   loses only the companion's, the next time it writes.
//! - A slot is seeded from the file the first time any window attaches, and the
//!   whole file is written from all of them, so a settled main window never
//!   erases the frame of a companion that was not opened this session.
//! - The companion also keeps `open`: true when it is made, false only when the
//!   person CLOSES it. ⌘Q sends no `CloseRequested` (tao 0.35 has no
//!   `applicationShouldTerminate`, so AppKit terminates and the loop ends), and
//!   closing `main` destroys the companion without one, so either way a
//!   companion open at quit is reopened at launch — when the setting is on.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewWindow};

pub const FILE: &str = "window-state.json";
/// The `ui-state.json` key the frontend's Settings writes (`settings.ts`).
pub const SETTING: &str = "restore_window";
/// The window tauri.conf.json declares (no label there: tauri's default).
pub const MAIN: &str = "main";
/// tauri.conf.json's `minWidth`/`minHeight` — a hand-edited file must not hand
/// back a window smaller than the app will ever lay out.
pub const MAIN_MIN: (f64, f64) = (900.0, 560.0);
/// How much of the frame's top edge must sit on a monitor for the saved
/// position to be used: enough title bar to grab. A frame left on a monitor
/// that is no longer attached gets the OS's placement instead.
const GRAB_W: f64 = 100.0;
const GRAB_H: f64 = 30.0;
/// A change is written once the window has been still this long — past the
/// end of a drag, and past the ~0.7s fullscreen animation.
const SETTLE: Duration = Duration::from_millis(1000);
const TICK: Duration = Duration::from_millis(250);
/// How long a fullscreen request gets before it is checked (and retried once).
const FULLSCREEN_CHECK: Duration = Duration::from_millis(1500);

/// A normal (not fullscreen, not maximized) window's frame, in logical points:
/// outer top-left, inner size — the pair `set_position`/`set_size` take.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct WinState {
    #[serde(default)]
    pub frame: Option<Frame>,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
}

/// A window other than `main`, as the file keeps it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Other {
    #[serde(flatten)]
    pub state: WinState,
    /// Open when the app last ran (see the module doc for when it turns false).
    #[serde(default)]
    pub open: bool,
}

/// `window-state.json`: `main` at the top level, as it always was, and every
/// other window under `windows`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct StateFile {
    #[serde(flatten)]
    pub main: WinState,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub windows: BTreeMap<String, Other>,
}

impl StateFile {
    /// The slot for `label`, as this file has it.
    pub fn slot(&self, label: &str) -> Other {
        if label == MAIN {
            Other { state: self.main, open: true }
        } else {
            self.windows.get(label).copied().unwrap_or_default()
        }
    }

    fn set(&mut self, label: &str, o: Other) {
        if label == MAIN {
            self.main = o.state;
        } else {
            self.windows.insert(label.to_string(), o);
        }
    }

    /// Whether `label` was open when the app last ran, and so should come back
    /// at launch. `main` always does.
    pub fn was_open(&self, label: &str) -> bool {
        label == MAIN || self.windows.get(label).is_some_and(|o| o.open)
    }
}

/// One window's live slot.
#[derive(Default)]
struct Slot {
    state: Other,
    /// Fullscreen is applied once the event loop is running (`RunEvent::Ready`
    /// for a window that exists at launch, at once for one made later):
    /// asking AppKit to toggle fullscreen on a window the event loop has not
    /// shown yet is a request it is free to drop.
    pending_fullscreen: bool,
}

/// What each window is right now, tracked from its events so the exit save does
/// not depend on a window still answering questions at `RunEvent::Exit`.
#[derive(Default)]
pub struct Tracker {
    /// The file as read at the first attach; every slot starts from it.
    seed: OnceLock<StateFile>,
    slots: Mutex<BTreeMap<String, Slot>>,
    /// When some slot last CHANGED and has not been written since.
    dirty: Mutex<Option<Instant>>,
    /// Set at exit: no more samples, no more writes.
    stopped: AtomicBool,
    /// `RunEvent::Ready` has passed: a window attached from now on applies its
    /// fullscreen at once.
    ready: AtomicBool,
    /// The one writer thread, started by the first attach.
    writer: OnceLock<()>,
}

impl Tracker {
    /// The file to write: the seed with every live slot laid over it.
    fn snapshot(&self) -> StateFile {
        let mut f = self.seed.get().cloned().unwrap_or_default();
        for (label, slot) in self.slots.lock().unwrap().iter() {
            f.set(label, slot.state);
        }
        f
    }

    fn touch(&self) {
        *self.dirty.lock().unwrap() = Some(Instant::now());
    }
}

/// Whether a pending change has been still long enough to write.
pub fn settled(dirty: Option<Instant>, now: Instant) -> bool {
    dirty.is_some_and(|t| now.saturating_duration_since(t) >= SETTLE)
}

fn describe(st: &WinState) -> String {
    let frame = st
        .frame
        .map(|f| format!("{:.0}x{:.0} at {:.0},{:.0}", f.width, f.height, f.x, f.y))
        .unwrap_or_else(|| "none".into());
    format!("fullscreen={} maximized={} frame={frame}", st.fullscreen, st.maximized)
}

fn describe_file(f: &StateFile) -> String {
    let mut s = format!("main: {}", describe(&f.main));
    for (label, o) in &f.windows {
        s.push_str(&format!("; {label}: {} open={}", describe(&o.state), o.open));
    }
    s
}

/// Missing key ⇒ ON, matching `DEFAULTS.restore_window` in settings.ts. A
/// fresh install has no ui-state.json at all and must behave like the default.
pub fn enabled(ui_state: Option<&serde_json::Value>) -> bool {
    ui_state.and_then(|v| v.get(SETTING)).and_then(|v| v.as_bool()).unwrap_or(true)
}

/// The setting as `ui-state.json` in `dir` says it now. Read afresh each time,
/// never cached: a companion opened an hour into the session follows the
/// setting as it is then.
pub fn enabled_in(dir: &Path) -> bool {
    let ui = std::fs::read(dir.join("ui-state.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    enabled(ui.as_ref())
}

/// A corrupt or absent file is "nothing saved", never an error on launch.
pub fn load(dir: &Path) -> Option<StateFile> {
    std::fs::read(dir.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// The next state after one observation. Pure, so the rule is testable: the
/// FRAME moves only while the window is plain; the flags move unless the window
/// is minimized (a minimized window reports nothing worth keeping — quitting
/// from the Dock should bring back what was minimized, not a minimized window).
pub fn observe(prev: WinState, frame: Frame, fullscreen: bool, maximized: bool, minimized: bool) -> WinState {
    if minimized {
        return prev;
    }
    WinState {
        frame: if fullscreen || maximized { prev.frame } else { Some(frame) },
        maximized,
        fullscreen,
    }
}

/// Monitors as logical `(x, y, width, height)`.
pub fn reachable(f: &Frame, monitors: &[(f64, f64, f64, f64)]) -> bool {
    let grab_w = f.width.min(GRAB_W);
    monitors.iter().any(|&(mx, my, mw, mh)| {
        // Some grab-sized piece of the top edge [x, x+width] × [y, y+GRAB_H]
        // overlaps this monitor.
        let left = f.x.max(mx);
        let right = (f.x + f.width).min(mx + mw);
        let top = f.y.max(my);
        let bottom = (f.y + GRAB_H).min(my + mh);
        right - left >= grab_w && bottom - top >= GRAB_H
    })
}

/// Clamp a saved size up to the window's minimum.
pub fn clamp_size(f: &Frame, min: (f64, f64)) -> (f64, f64) {
    (f.width.max(min.0), f.height.max(min.1))
}

fn current<R: Runtime>(w: &WebviewWindow<R>) -> Option<(Frame, bool, bool, bool)> {
    let scale = w.scale_factor().ok()?;
    let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
    let size = w.inner_size().ok()?.to_logical::<f64>(scale);
    let frame = Frame { x: pos.x, y: pos.y, width: size.width, height: size.height };
    Some((
        frame,
        w.is_fullscreen().unwrap_or(false),
        w.is_maximized().unwrap_or(false),
        w.is_minimized().unwrap_or(false),
    ))
}

fn sample<R: Runtime>(w: &WebviewWindow<R>) {
    let tracker = w.state::<Tracker>();
    if tracker.stopped.load(Ordering::SeqCst) {
        return;
    }
    let Some((frame, fs, max, min)) = current(w) else { return };
    let mut slots = tracker.slots.lock().unwrap();
    let slot = slots.entry(w.label().to_string()).or_default();
    let next = observe(slot.state.state, frame, fs, max, min);
    if next != slot.state.state {
        slot.state.state = next;
        drop(slots);
        tracker.touch();
    }
}

/// Record that `label` is open or closed — a fact about the session, not the
/// frame, written like any other change.
fn mark_open<R: Runtime>(app: &AppHandle<R>, label: &str, open: bool) {
    let tracker = app.state::<Tracker>();
    if tracker.stopped.load(Ordering::SeqCst) || label == MAIN {
        return;
    }
    let mut slots = tracker.slots.lock().unwrap();
    let slot = slots.entry(label.to_string()).or_default();
    if slot.state.open != open {
        slot.state.open = open;
        drop(slots);
        tracker.touch();
    }
}

fn write(dir: &Path, f: &StateFile) {
    let empty = |s: &WinState| s.frame.is_none() && !s.fullscreen && !s.maximized;
    if empty(&f.main) && f.windows.values().all(|o| empty(&o.state) && !o.open) {
        return; // nothing observed — leave whatever is on disk alone
    }
    // The dir first: on a fresh install it is serve that makes it, and the
    // window can settle before serve has started. Replaced with
    // `fsx::write_atomic` (a random temp name, O_EXCL|O_NOFOLLOW), the one way
    // this repo replaces a file (AGENTS.md).
    let res = serde_json::to_vec_pretty(f)
        .map_err(|e| e.to_string())
        .and_then(|b| std::fs::create_dir_all(dir).map(|()| b).map_err(|e| e.to_string()))
        .and_then(|b| worktrees_core::fsx::write_atomic(&dir.join(FILE), &b, None).map_err(|e| e.to_string()));
    match res {
        Ok(()) => crate::applog("info", &format!("window state saved: {}", describe_file(f))),
        Err(e) => crate::applog("warn", &format!("window state: could not save: {e}")),
    }
}

/// Writes each settled change until `stop`. One writer for every window.
fn spawn_writer<R: Runtime>(app: AppHandle<R>, dir: PathBuf) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        let tracker = app.state::<Tracker>();
        if tracker.stopped.load(Ordering::SeqCst) {
            return;
        }
        let due = {
            let mut d = tracker.dirty.lock().unwrap();
            let due = settled(*d, Instant::now());
            if due {
                *d = None;
            }
            due
        };
        if due {
            write(&dir, &tracker.snapshot());
        }
    });
}

fn monitors<R: Runtime>(w: &WebviewWindow<R>) -> Vec<(f64, f64, f64, f64)> {
    w.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let s = m.scale_factor();
            let p = m.position().to_logical::<f64>(s);
            let z = m.size().to_logical::<f64>(s);
            (p.x, p.y, z.width, z.height)
        })
        .collect()
}

/// The file as this session first read it (read once, at the first attach).
pub fn seed<R: Runtime>(app: &AppHandle<R>, dir: &Path) -> StateFile {
    app.state::<Tracker>().seed.get_or_init(|| load(dir).unwrap_or_default()).clone()
}

/// What a window attaching now restores from: its LIVE slot when it already
/// had one this session (closed and reopened — the frame it was just left at),
/// else the file as the session first read it. The seed is a launch-time
/// snapshot and goes stale the moment a window moves.
pub fn restore_from(live: Option<Other>, seed: &StateFile, label: &str) -> Other {
    live.unwrap_or_else(|| seed.slot(label))
}

/// A new window: seed its slot, restore when `restore` says so, and start
/// tracking. `min` is the window's own minimum size.
pub fn attach<R: Runtime>(w: &WebviewWindow<R>, dir: &Path, restore: bool, min: (f64, f64)) {
    let label = w.label().to_string();
    let file = seed(w.app_handle(), dir);
    let tracker = w.state::<Tracker>();
    let live = tracker.slots.lock().unwrap().get(&label).map(|s| s.state);
    let saved = restore_from(live, &file, &label);
    let have = saved.state != WinState::default();
    crate::applog(
        "info",
        &format!(
            "window restore ({label}): {}, saved: {}",
            if restore { "on" } else { "off" },
            if have { describe(&saved.state) } else { "nothing".into() }
        ),
    );
    let mut pending = false;
    if restore && have {
        let st = saved.state;
        if let Some(f) = st.frame {
            if reachable(&f, &monitors(w)) {
                let _ = w.set_position(LogicalPosition::new(f.x, f.y));
            }
            let (width, height) = clamp_size(&f, min);
            let _ = w.set_size(LogicalSize::new(width, height));
        }
        if st.maximized && !st.fullscreen {
            let _ = w.maximize();
        }
        pending = st.fullscreen;
        // Seed with the file as-is, so the saved frame survives even if no
        // event reports a plain window this session (e.g. it stays
        // fullscreen until quit).
        tracker.slots.lock().unwrap().insert(label.clone(), Slot { state: saved, pending_fullscreen: pending });
    } else {
        // Not restoring (or nothing saved): the window is its plain default
        // frame, so that is what a quit right now should record. The `open`
        // flag is the file's until `mark_open` says otherwise.
        tracker.slots.lock().unwrap().insert(label.clone(), Slot { state: Other { state: WinState::default(), open: saved.open }, pending_fullscreen: false });
        sample(w);
    }
    mark_open(w.app_handle(), &label, true);
    let win = w.clone();
    w.on_window_event(move |e| match e {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => sample(&win),
        // The person closed it. Never sent by ⌘Q (module doc), so this is the
        // one way `open` turns false.
        tauri::WindowEvent::CloseRequested { .. } => mark_open(win.app_handle(), win.label(), false),
        _ => {}
    });
    let (app, d) = (w.app_handle().clone(), dir.to_path_buf());
    tracker.writer.get_or_init(move || spawn_writer(app, d));
    if pending && tracker.ready.load(Ordering::SeqCst) {
        enter_fullscreen(w.app_handle(), &label);
    }
}

/// The deferred half of a fullscreen restore. Checked after a moment and
/// retried once, because `set_fullscreen` only QUEUES the request — an `Ok`
/// here says nothing about whether AppKit acted on it.
fn enter_fullscreen<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let tracker = app.state::<Tracker>();
    let pending = tracker
        .slots
        .lock()
        .unwrap()
        .get_mut(label)
        .map(|s| std::mem::take(&mut s.pending_fullscreen))
        .unwrap_or(false);
    if !pending {
        return;
    }
    let Some(w) = app.get_webview_window(label) else { return };
    let label = label.to_string();
    crate::applog("info", &format!("window restore ({label}): entering fullscreen"));
    if let Err(e) = w.set_fullscreen(true) {
        crate::applog("warn", &format!("window restore ({label}): fullscreen: {e}"));
    }
    std::thread::spawn(move || {
        for attempt in 1..=2 {
            std::thread::sleep(FULLSCREEN_CHECK);
            if w.is_fullscreen().unwrap_or(false) {
                crate::applog("info", &format!("window restore ({label}): fullscreen confirmed (check {attempt})"));
                return;
            }
            crate::applog("warn", &format!("window restore ({label}): not fullscreen after check {attempt}"));
            if attempt == 1 {
                let _ = w.set_fullscreen(true);
            }
        }
    });
}

/// `RunEvent::Ready`: every window attached before the loop ran applies its
/// fullscreen now; one attached later does it itself.
pub fn ready<R: Runtime>(app: &AppHandle<R>) {
    let tracker = app.state::<Tracker>();
    tracker.ready.store(true, Ordering::SeqCst);
    let labels: Vec<String> = tracker.slots.lock().unwrap().iter().filter(|(_, s)| s.pending_fullscreen).map(|(l, _)| l.clone()).collect();
    for label in labels {
        enter_fullscreen(app, &label);
    }
}

/// `RunEvent::Exit`: stop sampling and writing. Deliberately NOT a save — see
/// the module doc for why the exit path is the one moment not to trust.
pub fn stop<R: Runtime>(app: &AppHandle<R>) {
    app.state::<Tracker>().stopped.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const F: Frame = Frame { x: 100.0, y: 80.0, width: 1200.0, height: 800.0 };
    const SCREEN: Frame = Frame { x: 0.0, y: 0.0, width: 1728.0, height: 1117.0 };

    #[test]
    fn the_setting_defaults_on_and_reads_only_a_bool() {
        assert!(enabled(None), "fresh install: no ui-state.json");
        assert!(enabled(Some(&json!({}))), "older ui-state.json without the key");
        assert!(enabled(Some(&json!({ "restore_window": true }))));
        assert!(!enabled(Some(&json!({ "restore_window": false }))));
        assert!(enabled(Some(&json!({ "restore_window": "no" }))), "garbage is not a no");
    }

    /// The reason this module exists instead of the plugin: resizes while
    /// fullscreen must not become the frame you get back on leaving it.
    #[test]
    fn a_fullscreen_or_maximized_resize_keeps_the_normal_frame() {
        let st = observe(WinState::default(), F, false, false, false);
        let st = observe(st, SCREEN, true, false, false);
        assert_eq!(st, WinState { frame: Some(F), maximized: false, fullscreen: true });
        let st = observe(st, SCREEN, false, true, false);
        assert_eq!(st, WinState { frame: Some(F), maximized: true, fullscreen: false });
        let st = observe(st, F, false, false, false);
        assert_eq!(st, WinState { frame: Some(F), maximized: false, fullscreen: false });
    }

    #[test]
    fn a_change_is_written_only_once_the_window_is_still() {
        let t0 = Instant::now();
        assert!(!settled(None, t0 + SETTLE * 5), "nothing changed, nothing to write");
        assert!(!settled(Some(t0), t0 + SETTLE / 2), "mid-drag / mid-animation");
        assert!(settled(Some(t0), t0 + SETTLE));
    }

    #[test]
    fn a_minimized_window_changes_nothing() {
        let before = WinState { frame: Some(F), maximized: false, fullscreen: true };
        assert_eq!(observe(before, SCREEN, false, false, true), before);
    }

    #[test]
    fn a_frame_needs_its_title_bar_on_some_monitor() {
        let laptop = [(0.0, 0.0, 1728.0, 1117.0)];
        assert!(reachable(&F, &laptop));
        // Left on an external display to the right that is gone now.
        let gone = Frame { x: 2000.0, ..F };
        assert!(!reachable(&gone, &laptop));
        assert!(reachable(&gone, &[(0.0, 0.0, 1728.0, 1117.0), (1728.0, 0.0, 2560.0, 1440.0)]));
        // Only a sliver of the title bar still on screen is not grabbable.
        let sliver = Frame { x: 1700.0, ..F };
        assert!(!reachable(&sliver, &laptop));
        // Title bar above the top of the screen.
        let above = Frame { y: -200.0, ..F };
        assert!(!reachable(&above, &laptop));
    }

    #[test]
    fn a_saved_size_never_goes_under_the_window_minimum() {
        assert_eq!(clamp_size(&Frame { width: 300.0, height: 200.0, ..F }, MAIN_MIN), (900.0, 560.0));
        assert_eq!(clamp_size(&F, MAIN_MIN), (1200.0, 800.0));
        // Each window has its own: the companion's is smaller than main's.
        assert_eq!(clamp_size(&Frame { width: 300.0, height: 200.0, ..F }, (480.0, 360.0)), (480.0, 360.0));
    }

    /// On a fresh install the config dir does not exist yet when the window
    /// first settles: serve creates it, and since the app became serve's
    /// client nothing else in the shim does. The first save must not be a
    /// "could not save: No such file or directory".
    #[test]
    fn the_first_save_makes_the_config_dir_it_lands_in() {
        let base = std::env::temp_dir().join(format!("winstate-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("net.example.fresh");
        let st = StateFile { main: WinState { frame: Some(F), maximized: false, fullscreen: false }, windows: BTreeMap::new() };
        write(&dir, &st);
        assert_eq!(load(&dir), Some(st), "the save did not land in a config dir that was not there yet");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_partial_or_corrupt_file_is_tolerated() {
        let dir = std::env::temp_dir().join(format!("winstate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), b"{\"fullscreen\":true}").unwrap();
        assert_eq!(
            load(&dir),
            Some(StateFile { main: WinState { frame: None, maximized: false, fullscreen: true }, windows: BTreeMap::new() })
        );
        std::fs::write(dir.join(FILE), b"not json").unwrap();
        assert_eq!(load(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ── two windows in one file ─────────────────────────────────────────────

    const C: Frame = Frame { x: 300.0, y: 120.0, width: 900.0, height: 640.0 };

    fn both() -> StateFile {
        let mut windows = BTreeMap::new();
        windows.insert("companion".to_string(), Other { state: WinState { frame: Some(C), maximized: false, fullscreen: true }, open: true });
        StateFile { main: WinState { frame: Some(F), maximized: true, fullscreen: false }, windows }
    }

    /// The file a build from before the companion wrote is read unchanged: main
    /// at the top level, no companion, which therefore does not reopen.
    #[test]
    fn a_file_from_before_the_companion_still_restores_main() {
        let old = json!({ "frame": { "x": 100.0, "y": 80.0, "width": 1200.0, "height": 800.0 }, "maximized": true, "fullscreen": false });
        let f: StateFile = serde_json::from_value(old).unwrap();
        assert_eq!(f.slot(MAIN).state, WinState { frame: Some(F), maximized: true, fullscreen: false });
        assert_eq!(f.slot("companion"), Other::default());
        assert!(f.was_open(MAIN) && !f.was_open("companion"));
    }

    /// …and the other way round: what this build writes, an older build (whose
    /// whole file type was `WinState`) reads as main's frame — a downgrade keeps
    /// the main window where it was.
    #[test]
    fn an_older_build_reads_main_from_this_builds_file() {
        let text = serde_json::to_string(&both()).unwrap();
        let old: WinState = serde_json::from_str(&text).unwrap();
        assert_eq!(old, WinState { frame: Some(F), maximized: true, fullscreen: false });
        let back: StateFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back, both(), "round trip: {text}");
    }

    /// A file with only main in it is written exactly as before: no `windows`
    /// key appears for a session that never opened the companion.
    #[test]
    fn main_alone_writes_the_old_shape() {
        let f = StateFile { main: WinState { frame: Some(F), maximized: false, fullscreen: false }, windows: BTreeMap::new() };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v, json!({ "frame": { "x": 100.0, "y": 80.0, "width": 1200.0, "height": 800.0 }, "maximized": false, "fullscreen": false }));
    }

    /// A settled main window writes the whole file, and the companion's saved
    /// slot — seeded, never opened this session — survives it.
    #[test]
    fn a_main_save_keeps_a_companion_that_was_not_opened() {
        let t = Tracker::default();
        let _ = t.seed.set(both());
        t.slots.lock().unwrap().insert(MAIN.into(), Slot { state: Other { state: WinState { frame: Some(SCREEN), maximized: false, fullscreen: false }, open: true }, pending_fullscreen: false });
        let snap = t.snapshot();
        assert_eq!(snap.main.frame, Some(SCREEN));
        assert_eq!(snap.windows.get("companion"), both().windows.get("companion"));
    }

    /// Closed and reopened in the same session: it comes back where it was
    /// LEFT, not where the app launched with it — whatever the seed says, and
    /// including a companion closed while fullscreen or maximised (its normal
    /// frame kept beside the flag, as `observe` keeps it).
    #[test]
    fn a_reopen_in_the_same_session_restores_the_frame_it_was_closed_at() {
        let seed = both();
        let moved = Frame { x: 40.0, y: 60.0, width: 700.0, height: 500.0 };
        let left = Other { state: WinState { frame: Some(moved), maximized: false, fullscreen: false }, open: false };
        assert_eq!(restore_from(Some(left), &seed, "companion"), left, "the live slot, not the launch-time file");
        let fs = Other { state: WinState { frame: Some(moved), maximized: false, fullscreen: true }, open: false };
        assert_eq!(restore_from(Some(fs), &seed, "companion"), fs, "closed fullscreen: fullscreen again, over its normal frame");
        let max = Other { state: WinState { frame: Some(moved), maximized: true, fullscreen: false }, open: false };
        assert_eq!(restore_from(Some(max), &seed, "companion"), max, "closed maximised: maximised again");
        // A first run: nothing in the file, but the window was opened and moved.
        assert_eq!(restore_from(Some(left), &StateFile::default(), "companion"), left);
        // No live slot yet this session: the file decides.
        assert_eq!(restore_from(None, &seed, "companion"), seed.slot("companion"));
        assert_eq!(restore_from(None, &StateFile::default(), "companion"), Other::default());
    }

    /// Closed by the person: still remembered where it was, but not reopened.
    #[test]
    fn a_closed_companion_keeps_its_frame_and_does_not_reopen() {
        let mut f = both();
        let mut c = f.slot("companion");
        c.open = false;
        f.set("companion", c);
        assert!(!f.was_open("companion"));
        assert_eq!(f.slot("companion").state.frame, Some(C));
    }
}

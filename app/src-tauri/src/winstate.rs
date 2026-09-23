//! The main window's frame across launches — Settings → Behavior → "Restore
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

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewWindow};

pub const FILE: &str = "window-state.json";
/// The `ui-state.json` key the frontend's Settings writes (`settings.ts`).
pub const SETTING: &str = "restore_window";
/// tauri.conf.json's `minWidth`/`minHeight` — a hand-edited file must not hand
/// back a window smaller than the app will ever lay out.
const MIN_W: f64 = 900.0;
const MIN_H: f64 = 560.0;
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

/// What the window is right now, tracked from its events so the exit save does
/// not depend on the window still answering questions at `RunEvent::Exit`.
#[derive(Default)]
pub struct Tracker {
    state: Mutex<WinState>,
    /// Fullscreen is applied on `RunEvent::Ready`, not in setup: asking AppKit
    /// to toggle fullscreen on a window the event loop has not shown yet is a
    /// request it is free to drop.
    pending_fullscreen: Mutex<bool>,
    /// When the state last CHANGED and has not been written since.
    dirty: Mutex<Option<Instant>>,
    /// Set at exit: no more samples, no more writes.
    stopped: AtomicBool,
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

/// Missing key ⇒ ON, matching `DEFAULTS.restore_window` in settings.ts. A
/// fresh install has no ui-state.json at all and must behave like the default.
pub fn enabled(ui_state: Option<&serde_json::Value>) -> bool {
    ui_state.and_then(|v| v.get(SETTING)).and_then(|v| v.as_bool()).unwrap_or(true)
}

/// A corrupt or absent file is "nothing saved", never an error on launch.
pub fn load(dir: &Path) -> Option<WinState> {
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
pub fn clamp_size(f: &Frame) -> (f64, f64) {
    (f.width.max(MIN_W), f.height.max(MIN_H))
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
    let mut st = tracker.state.lock().unwrap();
    let next = observe(*st, frame, fs, max, min);
    if next != *st {
        *st = next;
        *tracker.dirty.lock().unwrap() = Some(Instant::now());
    }
}

fn write(dir: &Path, st: &WinState) {
    if st.frame.is_none() && !st.fullscreen && !st.maximized {
        return; // nothing observed — leave whatever is on disk alone
    }
    let res = serde_json::to_vec_pretty(st)
        .map_err(|e| e.to_string())
        .and_then(|b| std::fs::write(dir.join(FILE), b).map_err(|e| e.to_string()));
    match res {
        Ok(()) => crate::applog("info", &format!("window state saved: {}", describe(st))),
        Err(e) => crate::applog("warn", &format!("window state: could not save: {e}")),
    }
}

/// Writes each settled change until `stop`.
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
            let st = *tracker.state.lock().unwrap();
            write(&dir, &st);
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

/// Setup: seed the tracker, restore when `restore` says so, and start tracking.
pub fn attach<R: Runtime>(w: &WebviewWindow<R>, dir: &Path, restore: bool) {
    let saved = load(dir);
    let tracker = w.state::<Tracker>();
    crate::applog(
        "info",
        &format!(
            "window restore: {}, saved: {}",
            if restore { "on" } else { "off" },
            saved.as_ref().map(describe).unwrap_or_else(|| "nothing".into())
        ),
    );
    match saved {
        Some(st) if restore => {
            if let Some(f) = st.frame {
                if reachable(&f, &monitors(w)) {
                    let _ = w.set_position(LogicalPosition::new(f.x, f.y));
                }
                let (width, height) = clamp_size(&f);
                let _ = w.set_size(LogicalSize::new(width, height));
            }
            if st.maximized && !st.fullscreen {
                let _ = w.maximize();
            }
            *tracker.pending_fullscreen.lock().unwrap() = st.fullscreen;
            // Seed with the file as-is, so the saved frame survives even if no
            // event reports a plain window this session (e.g. it stays
            // fullscreen until quit).
            *tracker.state.lock().unwrap() = st;
        }
        _ => {
            // Not restoring (or nothing saved): the window is the config's plain
            // frame, so that is what a quit right now should record.
            sample(w);
        }
    }
    let win = w.clone();
    w.on_window_event(move |e| {
        if matches!(e, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) {
            sample(&win);
        }
    });
    spawn_writer(w.app_handle().clone(), dir.to_path_buf());
}

/// `RunEvent::Ready`: the deferred half of a fullscreen restore. Checked after
/// a moment and retried once, because `set_fullscreen` only QUEUES the request
/// — an `Ok` here says nothing about whether AppKit acted on it.
pub fn ready<R: Runtime>(app: &AppHandle<R>) {
    let tracker = app.state::<Tracker>();
    let pending = std::mem::take(&mut *tracker.pending_fullscreen.lock().unwrap());
    if !pending {
        return;
    }
    let Some(w) = app.get_webview_window("main") else { return };
    crate::applog("info", "window restore: entering fullscreen");
    if let Err(e) = w.set_fullscreen(true) {
        crate::applog("warn", &format!("window restore: fullscreen: {e}"));
    }
    std::thread::spawn(move || {
        for attempt in 1..=2 {
            std::thread::sleep(FULLSCREEN_CHECK);
            if w.is_fullscreen().unwrap_or(false) {
                crate::applog("info", &format!("window restore: fullscreen confirmed (check {attempt})"));
                return;
            }
            crate::applog("warn", &format!("window restore: not fullscreen after check {attempt}"));
            if attempt == 1 {
                let _ = w.set_fullscreen(true);
            }
        }
    });
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
        assert_eq!(clamp_size(&Frame { width: 300.0, height: 200.0, ..F }), (900.0, 560.0));
        assert_eq!(clamp_size(&F), (1200.0, 800.0));
    }

    #[test]
    fn a_partial_or_corrupt_file_is_tolerated() {
        let dir = std::env::temp_dir().join(format!("winstate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), b"{\"fullscreen\":true}").unwrap();
        assert_eq!(load(&dir), Some(WinState { frame: None, maximized: false, fullscreen: true }));
        std::fs::write(dir.join(FILE), b"not json").unwrap();
        assert_eq!(load(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

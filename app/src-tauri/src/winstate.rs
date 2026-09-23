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
//! - Saved on `RunEvent::Exit`, which ⌘Q, closing the window and the updater's
//!   `relaunch()` (plugin-process → `request_restart` → exit) all reach. A crash
//!   does not, and then the frame from the last clean exit is used.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
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
    let Some((frame, fs, max, min)) = current(w) else { return };
    let tracker = w.state::<Tracker>();
    let mut st = tracker.state.lock().unwrap();
    *st = observe(*st, frame, fs, max, min);
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
}

/// `RunEvent::Ready`: the deferred half of a fullscreen restore.
pub fn ready<R: Runtime>(app: &AppHandle<R>) {
    let tracker = app.state::<Tracker>();
    let pending = std::mem::take(&mut *tracker.pending_fullscreen.lock().unwrap());
    if pending {
        if let Some(w) = app.get_webview_window("main") {
            if let Err(e) = w.set_fullscreen(true) {
                crate::applog("warn", &format!("window restore: fullscreen: {e}"));
            }
        }
    }
}

/// `RunEvent::Exit`: one last sample if the window still answers, then write.
pub fn save<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        sample(&w);
    }
    let st = *app.state::<Tracker>().state.lock().unwrap();
    if st.frame.is_none() && !st.fullscreen && !st.maximized {
        return; // nothing observed — leave whatever is on disk alone
    }
    let Ok(dir) = app.path().app_config_dir() else { return };
    let res = serde_json::to_vec_pretty(&st)
        .map_err(|e| e.to_string())
        .and_then(|b| std::fs::write(dir.join(FILE), b).map_err(|e| e.to_string()));
    if let Err(e) = res {
        crate::applog("warn", &format!("window state: could not save: {e}"));
    }
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

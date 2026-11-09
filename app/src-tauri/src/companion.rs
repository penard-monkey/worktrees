//! The Ideas companion window (Ideas phase 0, docs/proposals/idea-board-phases.md
//! §5.1). A second window beside `main`, made on demand — tauri.conf.json
//! declares only `main` — which can be maximised or full screen on its own.
//!
//! Its page is `app/companion/index.html`, a build of its own
//! (`vite.companion.config.ts`) that lands in `app/dist/companion/`, so the
//! same `WebviewUrl::App` path is the dev server's page under `tauri dev` and
//! the bundled one in a release. It is one more loopback client of serve
//! through the same door as `main` (`app/src/ipc.ts`), and it never starts
//! serve: its retries are reconnect-only (`reconnect.ts`'s `neverStart`), so
//! `main` alone restarts a lost engine.
//!
//! Its rights are `capabilities/companion.json`, not main's.

use std::path::Path;

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

use crate::winstate;

/// The window's label: its capability file and its winstate slot name it.
pub const LABEL: &str = "companion";
/// The page, relative to the frontend's root (`devUrl` in dev, `frontendDist`
/// in a build).
const PAGE: &str = "companion/index.html";
/// The default frame and the floor a saved one is clamped to.
const SIZE: (f64, f64) = (1040.0, 720.0);
pub const MIN: (f64, f64) = (480.0, 360.0);

/// Bring the companion forward, making it first if it is not open. `dir` is
/// the app's config dir, where its frame is kept.
pub fn open<R: Runtime>(app: &AppHandle<R>, dir: &Path, why: &str) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let restore = winstate::enabled_in(dir);
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(PAGE.into()))
        .title("Ideas")
        .inner_size(SIZE.0, SIZE.1)
        .min_inner_size(MIN.0, MIN.1)
        // As tauri.conf.json sets for `main`: the page owns drag and drop, and
        // the cross-window probe needs the webview's own HTML5 events.
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| {
            crate::applog("error", &format!("companion: could not open: {e}"));
            e.to_string()
        })?;
    crate::applog("info", &format!("companion: opened ({why})"));
    winstate::attach(&w, dir, restore, MIN);
    Ok(())
}

/// The `ui-state.json` key of Settings → General → Beta features → Ideas and
/// workspaces (`beta_ideas` in settings.ts, asked through `beta.ts`). Read,
/// never written, like `winstate::SETTING`.
pub const BETA: &str = "beta_ideas";

/// Missing or not a bool ⇒ OFF, matching `DEFAULTS.beta_ideas`: a fresh
/// install, and every install from before the key, has no Ideas.
pub fn beta(ui_state: Option<&serde_json::Value>) -> bool {
    ui_state.and_then(|v| v.get(BETA)).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Does a companion that was open at the last quit come back at launch? Only
/// with Restore window on (winstate.rs) AND the beta on, both as
/// `ui-state.json` in `dir` says now. `WORKTREES_OPEN_COMPANION` is not this
/// rule's: lib.rs honours it in a debug build whatever the file says.
pub fn reopens_at_launch(dir: &Path, was_open: bool) -> bool {
    let ui = winstate::ui_state_in(dir);
    was_open && winstate::enabled(ui.as_ref()) && beta(ui.as_ref())
}

/// Close the companion if it is open: the beta was turned off. `close`, not
/// `destroy`: it goes through `CloseRequested` like its own close button, so
/// winstate marks it closed and keeps its frame for when it is next opened.
pub fn close<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    match app.get_webview_window(LABEL) {
        Some(w) => {
            crate::applog("info", "companion: closed (the Ideas beta was turned off)");
            w.close().map_err(|e| e.to_string())
        }
        None => Ok(()),
    }
}

/// `main` is gone: take the companion with it, so the app quits as it always
/// has when its window closes. `destroy`, not `close`: no `CloseRequested`, so
/// winstate keeps it marked open and it comes back at the next launch.
pub fn main_closed<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(LABEL) {
        crate::applog("info", "companion: main window closed, closing the companion with it");
        let _ = w.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_beta_defaults_off_and_reads_only_a_bool() {
        assert!(!beta(None), "fresh install: no ui-state.json");
        assert!(!beta(Some(&json!({}))), "an install from before the key");
        assert!(beta(Some(&json!({ "beta_ideas": true }))));
        assert!(!beta(Some(&json!({ "beta_ideas": false }))));
        assert!(!beta(Some(&json!({ "beta_ideas": "yes" }))), "garbage is not a yes");
    }

    /// The launch rule, read from a real `ui-state.json` in a temp dir: the
    /// companion comes back only when it was open, Restore window is on and
    /// the beta is on.
    #[test]
    fn the_companion_reopens_at_launch_only_with_the_beta_on() {
        let d = std::env::temp_dir().join(format!("wt-companion-beta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ui = |v: serde_json::Value| std::fs::write(d.join("ui-state.json"), v.to_string()).unwrap();

        assert!(!reopens_at_launch(&d, true), "no ui-state.json: the beta is off");
        ui(json!({ "restore_window": true }));
        assert!(!reopens_at_launch(&d, true), "a file from before the key: off");
        ui(json!({ "restore_window": true, "beta_ideas": false }));
        assert!(!reopens_at_launch(&d, true), "beta off");
        ui(json!({ "restore_window": true, "beta_ideas": true }));
        assert!(reopens_at_launch(&d, true), "beta on, restore on, was open");
        assert!(!reopens_at_launch(&d, false), "it was not open at quit");
        ui(json!({ "restore_window": false, "beta_ideas": true }));
        assert!(!reopens_at_launch(&d, true), "Restore window off");
        ui(json!({ "beta_ideas": true }));
        assert!(reopens_at_launch(&d, true), "Restore window defaults on");
        std::fs::write(d.join("ui-state.json"), b"{not json").unwrap();
        assert!(!reopens_at_launch(&d, true), "an unreadable file is the defaults: off");

        let _ = std::fs::remove_dir_all(&d);
    }
}

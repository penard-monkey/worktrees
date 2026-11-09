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

/// `main` is gone: take the companion with it, so the app quits as it always
/// has when its window closes. `destroy`, not `close`: no `CloseRequested`, so
/// winstate keeps it marked open and it comes back at the next launch.
pub fn main_closed<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(LABEL) {
        crate::applog("info", "companion: main window closed, closing the companion with it");
        let _ = w.destroy();
    }
}

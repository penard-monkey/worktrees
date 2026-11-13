//! The app menu's own items. Today that is one: Window → Cycle Through
//! Windows (⌘`), which moves between `main` and the Ideas companion.
//!
//! macOS has that shortcut built in, and tauri's default menu registers its
//! Window submenu with AppKit, yet in this app the key did nothing from either
//! window (the owner's report, 2026-11-12, on a U.S. layout with the system
//! shortcut at its default). So the item does the cycling itself rather than
//! depending on AppKit's.
//!
//! macOS only: the menu bar there is the app's, where on Linux a menu set on
//! the builder becomes a bar inside every window, which this app does not have.

use tauri::{AppHandle, Manager, Runtime};

/// The item's id, as `on_menu_event` hears it.
const CYCLE: &str = "cycle-windows";

/// The label to focus after `focused`, among `labels` (the windows that can
/// take focus, in a stable order). The first when none of them has it; `None`
/// when there is nowhere else to go.
fn next_label<'a>(labels: &'a [String], focused: Option<&str>) -> Option<&'a str> {
    let at = focused.and_then(|f| labels.iter().position(|l| l == f));
    let next = match at {
        Some(i) => labels.get((i + 1) % labels.len())?,
        None => labels.first()?,
    };
    (Some(next.as_str()) != focused).then_some(next.as_str())
}

/// Focus the next window. A minimised or hidden one is skipped, as the
/// system's own cycling skips it.
fn cycle<R: Runtime>(app: &AppHandle<R>) {
    let windows = app.webview_windows();
    let mut labels: Vec<String> = windows
        .iter()
        .filter(|(_, w)| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
        .map(|(l, _)| l.clone())
        .collect();
    // `main` first, then the rest by name: the map's own order is not stable.
    labels.sort_by_key(|l| (l != crate::winstate::MAIN, l.clone()));
    let focused = windows.iter().find(|(_, w)| w.is_focused().unwrap_or(false)).map(|(l, _)| l.as_str());
    let Some(next) = next_label(&labels, focused) else { return };
    if let Some(w) = windows.get(next) {
        if let Err(e) = w.set_focus() {
            crate::applog("warn", &format!("window menu: could not focus {next}: {e}"));
        }
    }
}

/// Add the menu to the builder: tauri's default one plus our item.
#[cfg(target_os = "macos")]
pub fn install<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, WINDOW_SUBMENU_ID};
    builder
        .menu(|app| {
            let menu = Menu::default(app)?;
            if let Some(window) = menu.get(WINDOW_SUBMENU_ID).as_ref().and_then(|k| k.as_submenu()) {
                window.append(&PredefinedMenuItem::separator(app)?)?;
                window.append(&MenuItem::with_id(app, CYCLE, "Cycle Through Windows", true, Some("Cmd+`"))?)?;
            }
            Ok(menu)
        })
        .on_menu_event(|app, event| {
            if event.id() == CYCLE {
                cycle(app);
            }
        })
}

#[cfg(not(target_os = "macos"))]
pub fn install<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    // Keep the cycle reachable to the compiler on every OS.
    let _ = cycle::<R>;
    let _ = CYCLE;
    builder
}

#[cfg(test)]
mod tests {
    use super::next_label;

    fn l(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_cycle_goes_round_and_stops_when_there_is_nowhere_to_go() {
        let both = l(&["main", "companion"]);
        assert_eq!(next_label(&both, Some("main")), Some("companion"));
        assert_eq!(next_label(&both, Some("companion")), Some("main"));
        // Nothing of ours has focus (another app is in front): the first.
        assert_eq!(next_label(&both, None), Some("main"));
        // The focused window is not in the list (it is hidden): the first.
        assert_eq!(next_label(&l(&["main"]), Some("companion")), Some("main"));
        // One window, and it has focus: nothing to do.
        assert_eq!(next_label(&l(&["main"]), Some("main")), None);
        assert_eq!(next_label(&[], None), None);
    }
}

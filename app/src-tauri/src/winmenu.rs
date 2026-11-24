//! The app menu's own items: what the app can do, as the macOS menu bar.
//!
//! **An item never does the thing itself.** The page owns every chord (App.tsx's
//! keydown handler, the companion's, a focused editor's), and an item hands the
//! focused window's page the same chord, or for an action with no chord its
//! name (`app/src/menu.ts`). `ITEMS` is the one table; menu-check.mjs drives
//! every chord in it through the page's real handler.
//!
//! **Why one press is one act.** macOS gives a ⌘ key to the focused WKWebView
//! FIRST, and only a press its page does not `preventDefault` is sent on to the
//! menu (measured with real keys on macOS 27.0.1, 2026-11-23). So a chord the
//! page takes never reaches the menu. One it declined (a held ⌘B's repeats, a
//! chord behind a dialog) does, and the page then ignores the menu's message,
//! because it saw that press itself. What is left for the item is a click, and
//! a press made while something other than the webview has the keyboard.
//!
//! Window → Cycle Through Windows (⌘`) is the exception, done here: it moves
//! between `main` and the Ideas companion.
//!
//! macOS has that shortcut built in, and tauri's default menu registers its
//! Window submenu with AppKit, yet in this app the key did nothing from either
//! window (the owner's report, 2026-11-12, on a U.S. layout with the system
//! shortcut at its default). So the item does the cycling itself rather than
//! depending on AppKit's.
//!
//! macOS only: the menu bar there is the app's, where on Linux a menu set on
//! the builder becomes a bar inside every window, which this app does not have.

use serde::Serialize;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

/// The event the focused window's page hears (`app/src/menu.ts`).
const EVENT: &str = "menu:item";

/// Which window an item belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Scope {
    Main,
    Companion,
    /// Whichever has focus.
    Focused,
}

/// A chord as the page's `keydown` would carry it; ⌘ is always held.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct Chord {
    key: &'static str,
    code: &'static str,
    shift: bool,
    alt: bool,
    /// For the focused ELEMENT (an editor's Save), not the window: a Toggle
    /// Places clicked while a document editor has the caret must not be that
    /// editor's ⌘B.
    focus: bool,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct Item {
    id: &'static str,
    label: &'static str,
    accel: Option<&'static str>,
    scope: Scope,
    /// `None`: no chord does it, and the page acts on the id.
    chord: Option<Chord>,
}

const fn chord(id: &'static str, label: &'static str, accel: &'static str, key: &'static str, code: &'static str) -> Item {
    Item { id, label, accel: Some(accel), scope: Scope::Main, chord: Some(Chord { key, code, shift: false, alt: false, focus: false }) }
}

const fn act(id: &'static str, label: &'static str) -> Item {
    Item { id, label, accel: None, scope: Scope::Main, chord: None }
}

impl Item {
    const fn with(self, shift: bool, alt: bool) -> Item {
        match self.chord {
            Some(c) => Item { chord: Some(Chord { shift, alt, ..c }), ..self },
            None => self,
        }
    }
    const fn at_focus(self) -> Item {
        match self.chord {
            Some(c) => Item { chord: Some(Chord { focus: true, ..c }), ..self },
            None => self,
        }
    }
    const fn scope(self, scope: Scope) -> Item {
        Item { scope, ..self }
    }
}

/// Every item of ours but Cycle. `NumpadAdd` is how muda spells the key
/// equivalent "+", which is what the menu shows; ⌘= and ⌘⇧= are the page's.
const ITEMS: &[Item] = &[
    chord("settings", "Settings…", "Cmd+,", ",", "Comma"),
    act("updates", "Check for Updates…"),
    chord("new-worktree", "New Worktree…", "Cmd+N", "n", "KeyN"),
    chord("new-terminal", "New Terminal", "Cmd+T", "t", "KeyT"),
    act("add-project", "Add Project…"),
    act("new-idea", "New Idea").scope(Scope::Companion),
    chord("open-editor", "Open in Editor", "Cmd+E", "e", "KeyE"),
    chord("save", "Save", "Cmd+S", "s", "KeyS").at_focus(),
    chord("find", "Find…", "Cmd+F", "f", "KeyF"),
    chord("find-next", "Find Next", "Cmd+G", "g", "KeyG").at_focus(),
    chord("find-prev", "Find Previous", "Cmd+Shift+G", "g", "KeyG").with(true, false).at_focus(),
    chord("zoom-in", "Zoom In", "Cmd+NumpadAdd", "+", "Equal").scope(Scope::Focused),
    chord("zoom-out", "Zoom Out", "Cmd+-", "-", "Minus").scope(Scope::Focused),
    chord("zoom-reset", "Actual Size", "Cmd+0", "0", "Digit0").scope(Scope::Focused),
    chord("doc-zoom-in", "Make Document Text Bigger", "Cmd+Alt+NumpadAdd", "+", "Equal").with(false, true),
    chord("doc-zoom-out", "Make Document Text Smaller", "Cmd+Alt+-", "-", "Minus").with(false, true),
    chord("doc-zoom-reset", "Reset Document Text Size", "Cmd+Alt+0", "0", "Digit0").with(false, true),
    chord("switcher", "Quick Switcher…", "Cmd+K", "k", "KeyK"),
    chord("home", "Home", "Cmd+1", "1", "Digit1"),
    chord("places", "Places", "Cmd+2", "2", "Digit2"),
    chord("back", "Back", "Cmd+[", "[", "BracketLeft"),
    chord("forward", "Forward", "Cmd+]", "]", "BracketRight"),
    chord("toggle-places", "Show or Hide Places", "Cmd+B", "b", "KeyB"),
    chord("toggle-dock", "Show or Hide Reference Panels", "Cmd+J", "j", "KeyJ"),
    chord("reading", "Reading Mode", "Cmd+Shift+E", "e", "KeyE").with(true, false),
    chord("ideas", "Ideas", "Cmd+Shift+I", "i", "KeyI").with(true, false),
    act("documentation", "Documentation"),
    act("release-notes", "Release Notes"),
];

/// Each submenu's items in order, `-` a separator, by tauri's own title for it.
/// They go where `At` says among the predefined ones.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum At {
    /// Before everything.
    Top,
    /// After the first item (the app menu's About).
    AfterFirst,
    /// After everything.
    Bottom,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const LAYOUT: &[(&str, At, &[&str])] = &[
    ("", At::AfterFirst, &["updates", "-", "settings"]),
    ("File", At::Top, &["new-worktree", "new-terminal", "add-project", "-", "open-editor", "save", "-"]),
    ("Edit", At::Bottom, &["-", "find", "find-next", "find-prev"]),
    (
        "View",
        At::Top,
        &[
            "zoom-in", "zoom-out", "zoom-reset", "-", "doc-zoom-in", "doc-zoom-out", "doc-zoom-reset", "-", "switcher", "home", "places", "back", "forward", "-",
            "toggle-places", "toggle-dock", "reading", "-",
        ],
    ),
    ("Window", At::Bottom, &["-", CYCLE]),
    ("Help", At::Bottom, &["documentation", "release-notes"]),
];

/// Where a beta item goes while its beta is on: right before this one.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const IDEAS_AT: &[(&str, &str, &str)] = &[("new-idea", "File", "new-terminal"), ("ideas", "Window", CYCLE)];

/// Is an item of `scope` live while `focused` is the window in front? None of
/// ours in front: only main's (and it is main that then hears it).
fn live(scope: Scope, focused: Option<&str>) -> bool {
    match scope {
        Scope::Focused => focused.is_some(),
        Scope::Main => focused.is_none() || focused == Some(crate::winstate::MAIN),
        Scope::Companion => focused == Some(crate::companion::LABEL),
    }
}

/// The window a chosen item speaks to, or none.
fn route(scope: Scope, focused: Option<&str>) -> Option<String> {
    if !live(scope, focused) {
        return None;
    }
    Some(focused.unwrap_or(crate::winstate::MAIN).to_string())
}

#[derive(Clone, Serialize)]
struct Chosen {
    id: &'static str,
    chord: Option<Chord>,
}

fn in_front<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    app.webview_windows().into_iter().find(|(_, w)| w.is_focused().unwrap_or(false)).map(|(l, _)| l)
}

/// An item was chosen: hand it to the page of the window it is for.
fn chosen<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let Some(item) = ITEMS.iter().find(|i| i.id == id) else { return };
    let Some(to) = route(item.scope, in_front(app).as_deref()) else { return };
    crate::applog("info", &format!("menu: {id} → {to}"));
    if let Err(e) = app.emit_to(EventTarget::webview_window(&to), EVENT, Chosen { id: item.id, chord: item.chord }) {
        crate::applog("warn", &format!("menu: {id} → {to}: {e}"));
    }
}

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

#[cfg(target_os = "macos")]
mod bar {
    use super::{At, Item, CYCLE, IDEAS_AT, ITEMS, LAYOUT};
    use tauri::menu::{IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
    use tauri::{AppHandle, Runtime};

    fn make<R: Runtime>(app: &AppHandle<R>, i: &Item, on: bool) -> tauri::Result<MenuItem<R>> {
        MenuItem::with_id(app, i.id, i.label, on, i.accel)
    }

    fn sub<R: Runtime>(menu: &Menu<R>, title: &str) -> Option<Submenu<R>> {
        let subs: Vec<Submenu<R>> = menu.items().ok()?.iter().filter_map(|k| k.as_submenu().cloned()).collect();
        match title {
            // The app's own menu is titled with the product's name.
            "" => subs.into_iter().next(),
            t => subs.into_iter().find(|s| s.text().is_ok_and(|x| x == t)),
        }
    }

    fn find<R: Runtime>(sub: &Submenu<R>, id: &str) -> Option<(usize, MenuItemKind<R>)> {
        sub.items().ok()?.into_iter().enumerate().find(|(_, k)| k.id() == id)
    }

    /// tauri's default menu (Cut, Copy, Minimize, Quit…) with ours laid in.
    pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
        let menu = Menu::default(app)?;
        for (title, at, ids) in LAYOUT {
            let Some(s) = sub(&menu, title) else {
                crate::applog("warn", &format!("menu: no {title:?} submenu in tauri's default menu"));
                continue;
            };
            let mut pos = match at {
                At::Top => 0,
                At::AfterFirst => 1,
                At::Bottom => s.items()?.len(),
            };
            for id in *ids {
                let item: Box<dyn IsMenuItem<R>> = match ITEMS.iter().find(|i| i.id == *id) {
                    Some(i) => Box::new(make(app, i, true)?),
                    None if *id == CYCLE => Box::new(MenuItem::with_id(app, CYCLE, "Cycle Through Windows", true, Some("Cmd+`"))?),
                    None => Box::new(PredefinedMenuItem::separator(app)?),
                };
                s.insert(item.as_ref(), pos)?;
                pos += 1;
            }
        }
        Ok(menu)
    }

    /// Grey what the window in front cannot do.
    pub fn enable<R: Runtime>(app: &AppHandle<R>, focused: Option<&str>) {
        let Some(menu) = app.menu() else { return };
        for s in menu.items().unwrap_or_default().iter().filter_map(|k| k.as_submenu()) {
            for k in s.items().unwrap_or_default() {
                let (Some(m), Some(i)) = (k.as_menuitem(), ITEMS.iter().find(|i| k.id() == i.id)) else { continue };
                let on = super::live(i.scope, focused);
                if m.is_enabled().unwrap_or(!on) != on {
                    let _ = m.set_enabled(on);
                }
            }
        }
    }

    /// The Ideas beta went on or off: its items are in the menu or not.
    pub fn ideas<R: Runtime>(app: &AppHandle<R>, on: bool, focused: Option<&str>) -> tauri::Result<()> {
        let Some(menu) = app.menu() else { return Ok(()) };
        for (id, title, before) in IDEAS_AT {
            let Some(s) = sub(&menu, title) else { continue };
            match (find(&s, id), on) {
                (None, true) => {
                    let Some(i) = ITEMS.iter().find(|i| i.id == *id) else { continue };
                    let pos = find(&s, before).map_or(0, |(n, _)| n);
                    s.insert(&make(app, i, super::live(i.scope, focused))?, pos)?;
                }
                (Some((_, k)), false) => {
                    if let Some(m) = k.as_menuitem() {
                        s.remove(m)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Add the menu to the builder.
#[cfg(target_os = "macos")]
pub fn install<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.menu(bar::build).on_menu_event(|app, event| {
        if event.id() == CYCLE {
            cycle(app);
        } else {
            chosen(app, event.id().as_ref());
        }
    })
}

/// A window came to the front: grey what it cannot do.
pub fn focused<R: Runtime>(app: &AppHandle<R>, label: &str) {
    #[cfg(target_os = "macos")]
    bar::enable(app, Some(label));
    #[cfg(not(target_os = "macos"))]
    let _ = (app, label);
}

/// The Ideas beta is on or off (the page says; companion.rs never writes it).
pub fn ideas<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return bar::ideas(app, on, in_front(app).as_deref()).map_err(|e| e.to_string());
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, on);
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    // Keep the rest reachable to the compiler on every OS.
    let _ = (cycle::<R>, chosen::<R>);
    let _ = (CYCLE, LAYOUT, IDEAS_AT);
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn an_item_speaks_to_the_window_in_front_and_only_there() {
        for f in ["main", "companion"] {
            assert_eq!(route(Scope::Focused, Some(f)).as_deref(), Some(f), "zoom follows the window in front");
        }
        assert_eq!(route(Scope::Main, Some("main")).as_deref(), Some("main"));
        assert_eq!(route(Scope::Main, Some("companion")), None, "main's items are dead over Ideas");
        assert_eq!(route(Scope::Companion, Some("companion")).as_deref(), Some("companion"));
        assert_eq!(route(Scope::Companion, Some("main")), None);
        // No window of ours in front: nothing to zoom; main's still answer.
        assert_eq!(route(Scope::Focused, None), None);
        assert_eq!(route(Scope::Main, None).as_deref(), Some("main"));
    }

    /// An item is in the menu exactly once, or it is a beta item with a place.
    #[test]
    fn every_item_is_laid_out_once_and_every_accelerator_is_its_own() {
        let laid: Vec<&str> = LAYOUT.iter().flat_map(|(_, _, ids)| ids.iter().copied()).filter(|i| *i != "-" && *i != CYCLE).collect();
        for i in ITEMS {
            let n = laid.iter().filter(|x| **x == i.id).count();
            let beta = IDEAS_AT.iter().filter(|(x, _, _)| *x == i.id).count();
            assert_eq!(n + beta, 1, "{}", i.id);
        }
        assert_eq!(laid.len() + IDEAS_AT.len(), ITEMS.len(), "the layout names an item that does not exist");
        let mut accels: Vec<&str> = ITEMS.iter().filter_map(|i| i.accel).collect();
        accels.sort();
        let all = accels.len();
        accels.dedup();
        assert_eq!(accels.len(), all, "two items share an accelerator");
        for i in ITEMS {
            assert_eq!(i.accel.is_some(), i.chord.is_some(), "{}: an accelerator without a chord, or the reverse", i.id);
        }
    }

    /// The accelerator shown is the chord handed over.
    #[test]
    fn an_accelerator_and_its_chord_are_the_same_keys() {
        for i in ITEMS {
            let (Some(a), Some(c)) = (i.accel, i.chord) else { continue };
            let last = a.rsplit('+').next().unwrap();
            let key = if last == "NumpadAdd" { "+".to_string() } else { last.to_lowercase() };
            assert_eq!(key, c.key, "{}", i.id);
            assert_eq!(a.contains("Shift+"), c.shift, "{}", i.id);
            assert_eq!(a.contains("Alt+"), c.alt, "{}", i.id);
            assert!(a.starts_with("Cmd+"), "{}", i.id);
        }
    }
}

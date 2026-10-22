//! The two PTY surfaces: an attach to a live tmux session, and the dock's own
//! owned shells.
//!
//! Both stream bytes to exactly one client, which is what `ByteSink` is for —
//! Tauri's `Channel<InvokeResponseBody>` on the desktop, a frame writer on the
//! server. ORDER is the contract and it is load-bearing: AGENTS.md records a
//! replay being ANSWERED as input because an out-of-band payload overtook the
//! response it belonged to, and `TerminalPane` mutes `onData` for exactly as
//! long as the replay takes to parse. An implementation that reorders breaks
//! that mute.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use worktrees_core::{sysclock, tmux, Project};

use crate::log::log as applog;
use crate::util::fmt_utc;
use crate::{ByteSink, Ctx};


pub struct Term {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub stop: Arc<AtomicBool>,
    /// The session this client is attached to — what `term_wheel` resolves to
    /// a pane.
    pub session: String,
    /// The pane `term_wheel` last scrolled into tmux history, until the next
    /// keystroke takes it back to live output. See `term_write`.
    pub scrolled: Option<String>,
}

#[derive(Default)]
pub struct Terminals(Mutex<HashMap<u32, Term>>);

/// `worktrees_core::childenv`'s edits and start dir, on a pty child: the rule
/// lives there, beside the std `Command` spawns it also covers, so the two
/// cannot drift. Under an AppImage it keeps the AppDir's PATH/LD_LIBRARY_PATH/…
/// (and its cwd) out of the dock's shells and the tmux client; anywhere else it
/// changes nothing. Call it before the call site's own `env`/`cwd`.
fn scrub_pty(cmd: &mut CommandBuilder) {
    if let Some(d) = worktrees_core::childenv::start_dir() {
        cmd.cwd(d);
    }
    apply_pty(cmd, &worktrees_core::childenv::edits());
}

fn apply_pty(cmd: &mut CommandBuilder, edits: &[worktrees_core::childenv::Edit]) {
    for (k, v) in edits {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        }
    }
}

pub static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// Attach to an EXISTING tmux session inside a PTY and stream its bytes to the
/// frontend. We never create or own a shell — tmux owns the shells, panes, and
/// scrollback; this app is just another tmux client. Closing detaches (the
/// session survives and stays `tmux attach`-able from a bare terminal).
///
/// EXISTING and exactly that name, or an error saying which, before anything
/// spawns. It answered ok for a session that did not exist: the tmux client in
/// the pty printed tmux's complaint and quit, and the caller held an id for
/// nothing. Worse, a tmux target is a PREFIX match, so the attach for a
/// session that had just gone landed on whichever one's name began with it —
/// another place's agent, typed into from this pane. Nothing waits on "ok,
/// attach later": the main pane mounts only for a session `ls` reports up,
/// and writes an error into itself.
pub fn term_open(
    session: String,
    cols: u16,
    rows: u16,
    on_bytes: Arc<dyn ByteSink>,
    terms: &Terminals,
) -> Result<u32, String> {
    if !tmux::session_exists(&session) {
        return Err(format!("no tmux session named {session}"));
    }
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;

    // Sizing: tune the session (window-size latest + aggressive-resize) so a
    // smaller co-attached client can't clamp us — the clamp left stale painted
    // cells ("undeletable" artifacts) outside the redrawn region. Covers
    // sessions that predate the tuning-at-create in ops::launch.
    worktrees_core::tmux::tune_session(&session);
    // -u (global flag, must precede the subcommand): declare this client
    // UTF-8-capable. Without it tmux sniffs LC_ALL/LC_CTYPE/LANG, and a
    // GUI-launched app has none — tmux then draws every non-ASCII cell as "_".
    // Always safe here: the receiving end is xterm.js, which is always UTF-8.
    // -N (`tmux::no_start_args`): an attach under a tty that finds the socket
    // refused or missing would otherwise unlink it and start a second server,
    // orphaning every session in the first.
    // `=`: exactly this session, should it go between the check and here.
    let mut cmd = CommandBuilder::new("tmux");
    scrub_pty(&mut cmd);
    cmd.args(worktrees_core::tmux::no_start_args());
    cmd.args(["-u", "attach-session", "-t", &format!("={session}")]);
    cmd.env("TERM", "xterm-256color");

    let child = pair.slave.spawn_command(cmd).map_err(|e| {
        applog("error", &format!("term_open {session}: tmux attach spawn failed: {e}"));
        e.to_string()
    })?;
    drop(pair.slave); // parent doesn't need the slave handle after spawn

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = stop.clone();

    // Reader thread → frontend. Raw binary (no JSON eval of the byte stream).
    thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            if stop_reader.load(Ordering::Relaxed) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: the tmux client exited (detached)
                Ok(n) => {
                    if on_bytes.send(&buf[..n]).is_err() {
                        break; // frontend gone
                    }
                }
                Err(_) => break,
            }
        }
    });

    // A pane can already be reading history when we attach: scrolled back
    // before a place switch (term_close detaches; copy-mode lives on in tmux),
    // before the app quit, or from a bare `tmux attach`. Without the flag the
    // first keystrokes here would be copy-mode commands — Space selects, Enter
    // copies and leaves, the typed text is lost — so seed it from the pane.
    let scrolled = tmux::tmux(&["display-message", "-p", "-t", &format!("={session}:"), "#{pane_id} #{pane_mode}"])
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| scrolled_at_attach(&String::from_utf8_lossy(&o.stdout)));
    terms.0.lock().unwrap().insert(id, Term { master: pair.master, writer, child, stop, session, scrolled });
    Ok(id)
}

/// `#{pane_id} #{pane_mode}` → the pane, when it is in one of the two modes
/// that hold a pane in history (and that `send-keys -X cancel` leaves).
pub fn scrolled_at_attach(line: &str) -> Option<String> {
    let mut f = line.split_whitespace();
    let (pane, mode) = (f.next()?, f.next().unwrap_or(""));
    (pane.starts_with('%') && is_history_mode(mode)).then(|| pane.to_string())
}

/// copy-mode, and view-mode (tmux's read-only copy-mode, e.g. `run-shell`
/// output). The other modes — choose-tree, the buffer and client choosers —
/// are menus, and neither scroll through history nor take `scroll-up`.
pub fn is_history_mode(mode: &str) -> bool {
    matches!(mode, "copy-mode" | "view-mode")
}

pub fn term_write(id: u32, data: Vec<u8>, terms: &Terminals) -> Result<(), String> {
    let mut map = terms.0.lock().unwrap();
    let term = map.get_mut(&id).ok_or("no such terminal")?;
    // Typing into a pane the wheel scrolled back returns it to live output
    // FIRST. In copy-mode tmux reads keys as copy-mode commands — `/` searches,
    // `q` quits, Enter copies — so without this the first keystrokes after a
    // scroll are eaten, or worse, do something. Done under the map lock on
    // purpose: a second keystroke must not reach the pty before the cancel has.
    // It costs one tmux spawn on the first keystroke after a scroll, never more.
    if let Some(pane) = term.scrolled.take() {
        // `send-keys -X` fails outside a mode, so success means we really did
        // take the pane out of history (it may have left on its own: `-e`
        // exits when scrolled back to the bottom).
        let left = tmux::tmux(&["send-keys", "-X", "-t", &pane, "cancel"]).is_ok_and(|o| o.status.success());
        // Esc while scrolled back means "stop reading history", not "interrupt
        // the agent" — it is pi's and claude's abort key. Swallow it.
        if left && data == [0x1b] {
            return Ok(());
        }
    }
    term.writer.write_all(&data).map_err(|e| e.to_string())?;
    term.writer.flush().map_err(|e| e.to_string())
}

/// What one wheel gesture does to the pane a tmux client is showing.
///
/// Reached only when the pane has NOT asked for the mouse — xterm forwards the
/// wheel as mouse reports otherwise (claude), and this is never called. Without
/// it, xterm.js does what every terminal does for an alternate screen with no
/// mouse mode, and the tmux client's screen always IS alternate: it turns each
/// notch into ↑/↓ (`CoreBrowserTerminal.ts`, "Convert wheel events into up/down
/// events"). For a program on the MAIN screen — pi, a shell — that is not a
/// scroll at all: pi reads ↑ as editor history, zsh as history recall. Its
/// scrollback is tmux's, and only copy-mode can show it.
#[derive(Debug, PartialEq, Eq)]
pub enum Wheel {
    /// Already in copy-mode (or view-mode): move through history.
    Scroll,
    /// Main screen, wheel up: enter copy-mode (`-e`, so scrolling back to the
    /// bottom leaves it) and move up.
    EnterAndScroll,
    /// Alternate screen, no mouse (less, vim, an agent TUI that draws its own
    /// transcript): ↑/↓, exactly what xterm sent before — tmux keeps no history
    /// for such a pane, so there is nothing to scroll to. `send-keys` encodes
    /// the arrows in the pane's OWN cursor-key mode.
    Keys,
    /// Main screen, wheel down, not scrolled back: already live.
    Nothing,
}

/// `mode` is `#{pane_mode}`, empty when the pane is in none. NOT
/// `#{pane_in_mode}`: that is true for choose-tree (C-b w/s) as well, where
/// `scroll-up` fails "not in a mode" — ↑/↓ is what moves a tree's selection,
/// and it is what xterm sent there before.
pub fn wheel_plan(mode: &str, alternate: bool, up: bool) -> Wheel {
    if is_history_mode(mode) {
        return Wheel::Scroll;
    }
    match (mode.is_empty(), alternate, up) {
        (false, _, _) | (true, true, _) => Wheel::Keys,
        (true, false, true) => Wheel::EnterAndScroll,
        (true, false, false) => Wheel::Nothing,
    }
}

/// The ONE tmux invocation that carries a plan out, or None for `Nothing`.
/// `lines` < 0 is up (towards history), matching a WheelEvent's `deltaY`.
pub fn wheel_args(pane: &str, plan: &Wheel, lines: i32) -> Option<Vec<String>> {
    let n = lines.unsigned_abs().to_string();
    let up = lines < 0;
    let scroll = if up { "scroll-up" } else { "scroll-down" };
    let v = |a: &[&str]| Some(a.iter().map(|s| s.to_string()).collect());
    match plan {
        Wheel::Scroll => v(&["send-keys", "-X", "-N", &n, "-t", pane, scroll]),
        Wheel::EnterAndScroll => {
            v(&["copy-mode", "-e", "-t", pane, ";", "send-keys", "-X", "-N", &n, "-t", pane, scroll])
        }
        Wheel::Keys => v(&["send-keys", "-N", &n, "-t", pane, if up { "Up" } else { "Down" }]),
        Wheel::Nothing => None,
    }
}

/// `lines` wheel lines on terminal `id` (< 0 = up). The frontend coalesces a
/// gesture and keeps at most one of these in flight per pane, so this is two
/// tmux spawns per flush, not per notch.
pub fn term_wheel(id: u32, lines: i32, terms: &Terminals) -> Result<(), String> {
    if lines == 0 {
        return Ok(());
    }
    let session = terms.0.lock().unwrap().get(&id).ok_or("no such terminal")?.session.clone();
    // `=name:` — EXACT session, its current window's active pane: the pane this
    // client is showing. A bare name prefix-matches (tmux.rs's `PaneId` note).
    let target = format!("={session}:");
    // pane_mode LAST: it is empty outside a mode, and whitespace-splitting an
    // empty middle field would shift the others.
    let out = tmux::tmux(&["display-message", "-p", "-t", &target, "#{pane_id} #{alternate_on} #{pane_mode}"])
        .map_err(|e| format!("tmux: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut f = text.split_whitespace();
    let (Some(pane), Some(alt)) = (f.next(), f.next()) else {
        return Err(format!("tmux: no pane for {session}"));
    };
    let plan = wheel_plan(f.next().unwrap_or(""), alt == "1", lines < 0);
    let Some(args) = wheel_args(pane, &plan, lines) else { return Ok(()) };
    let history = matches!(plan, Wheel::Scroll | Wheel::EnterAndScroll);
    let flag = |terms: &Terminals| {
        if let Some(t) = terms.0.lock().unwrap().get_mut(&id) {
            t.scrolled = Some(pane.to_string());
        }
    };
    // The flag is set on BOTH sides of the spawn. Before: a keystroke that
    // lands while copy-mode is being entered must find it and cancel. After:
    // that keystroke took the flag, and if its cancel ran before copy-mode was
    // in, the pane would otherwise end up in history with no flag at all —
    // every later key a copy-mode command. A flag left on a pane that is
    // already live costs one failed `cancel`, nothing else.
    if history {
        flag(&terms);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = tmux::tmux(&args).map_err(|e| format!("tmux: {e}"))?;
    if history {
        flag(&terms);
    }
    if !out.status.success() {
        return Err(format!("tmux: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

pub fn term_resize(id: u32, cols: u16, rows: u16, terms: &Terminals) -> Result<(), String> {
    let map = terms.0.lock().unwrap();
    let term = map.get(&id).ok_or("no such terminal")?;
    term.master
        .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

/// Detach, never kill the session. Killing the `tmux attach-session` CLIENT
/// process drops the client → tmux detaches it; the session (and its shells /
/// AI CLI) live on. The killed client also closes the slave, so the reader
/// thread hits EOF and exits.
pub fn term_close(id: u32, terms: &Terminals) -> Result<(), String> {
    if let Some(mut term) = terms.0.lock().unwrap().remove(&id) {
        term.stop.store(true, Ordering::Relaxed);
        let _ = term.child.kill(); // kills the CLIENT = detach, not the session
    }
    Ok(())
}


/// Replay window per shell. Enough for a build log's tail; small enough that a
/// dozen idle tabs don't add up to anything.
pub const SHELL_RING: usize = 256 * 1024;

pub struct Shell {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub stop: Arc<AtomicBool>,
    /// Everything the shell has written, capped. Replayed on re-attach.
    pub ring: Arc<Mutex<VecDeque<u8>>>,
    /// The attached webview channel, if any. `None` = running unwatched.
    pub sink: Arc<Mutex<Option<Arc<dyn ByteSink>>>>,
    /// Attach generation. Detach names the attach it is undoing, because a
    /// STALE detach must be a no-op: under React StrictMode the same pane
    /// mounts, unmounts, and mounts again — and unmount №1's detach can arrive
    /// AFTER mount №2's attach installed its channel. Keyed detach alone would
    /// clear the new sink and freeze a live pane (the tmux design was immune:
    /// every attach had its own id).
    pub gen: u64,
    /// The size the pty is actually at, so a request to be the size it already
    /// is can be dropped. See `next_pty_size`.
    pub size: PtySize,
    /// Has the ring moved since it was last persisted? Lets the slow sampler
    /// skip a tab nobody is using, which is most tabs most of the time.
    pub dirty: Arc<AtomicBool>,
}

/// The size to hand `MasterPty::resize`, or `None` when the pty is already
/// there. `cur` is updated on the way through, so the next request is compared
/// against what was actually applied.
///
/// Resizing a pty to its own size is not the harmless no-op it looks like. It
/// is at best a wasted ioctl; on any platform whose TIOCSWINSZ does not compare
/// first (macOS and Linux both do, today) it is a SIGWINCH — and zsh answers
/// every SIGWINCH by redrawing the prompt and whatever half-typed line is
/// sitting on it, which lands in the replay ring. Measured on a real login
/// zsh: one full redraw per size CHANGE, nothing at all for a repeat.
///
/// That matters because the ring is raw bytes replayed into a FRESH xterm on
/// every re-attach, and those redraws are cursor-relative — rendered at a width
/// other than the one they were computed for they stop overwriting each other
/// and stack up instead. One column off already leaves residue; replaying a
/// 174-column ring into an 80-column terminal stacked a pasted path four deep
/// and left the pane scrolled to the bottom, which is the bug this guards.
pub fn next_pty_size(cur: &mut PtySize, cols: u16, rows: u16) -> Option<PtySize> {
    let want = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
    if *cur == want {
        return None;
    }
    *cur = want;
    Some(want)
}

impl Shell {
    /// Resize the pty unless it is already that size.
    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        match next_pty_size(&mut self.size, cols, rows) {
            Some(want) => self.master.resize(want).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }
}

/// (repo, slug, 1-based tab index) — the webview never names a shell, same rule
/// as the tmux sessions.
pub type ShellKey = (String, String, u32);

#[derive(Default)]
pub struct Shells(pub Mutex<HashMap<ShellKey, Shell>>);

pub fn kill_shell(shells: &Shells, key: &ShellKey) {
    if let Some(mut sh) = shells.0.lock().unwrap().remove(key) {
        sh.stop.store(true, Ordering::Relaxed);
        let _ = sh.child.kill();
    }
}

/// Every dock shell of one place. Called when the place is closed or removed —
/// core's `kill_shell_sidecars` handles the tmux era, but it can't see this map.
pub fn kill_place_shells(shells: &Shells, repo: &str, slug: &str) {
    let keys: Vec<ShellKey> = shells
        .0
        .lock()
        .unwrap()
        .keys()
        .filter(|(r, s, _)| r == repo && s == slug)
        .cloned()
        .collect();
    for k in &keys {
        kill_shell(shells, k);
    }
}

/// What `shell_open` answers: the attach generation `shell_detach` must
/// present, how many bytes were pushed down the channel FIRST, and the grid
/// those bytes were laid out for.
///
/// `replay` is a live shell's ring on a re-attach — and, since tabs started
/// keeping their scrollback between runs, a fresh spawn's RESTORED ring plus its
/// seam. Both are recordings; only a spawn with nothing saved still answers 0.
///
/// The frontend needs it so it can tell the replay from live output.
/// The replay is a recording, and a recording must not be answered: any
/// terminal query in it (vim's DA2 / cursor-position / colour / cursor-blink
/// burst, say — every `git commit` without `-m` leaves one in the ring) is
/// re-issued to a FRESH xterm on every re-attach, and xterm dutifully replies
/// down the pty, where zsh echoes the printable tail of each reply onto the
/// prompt as if typed. The pane mutes its `onData` until the replay is parsed;
/// it cannot key that on "arrived before `shell_open` returned", because Tauri
/// hands a large channel payload to the page through a separate fetch that can
/// land AFTER the invoke's own response. What IS ordered is the channel: the
/// snapshot is its first message whenever `replay > 0`.
///
/// `replay_cols` is the width the recording was written at. Raw bytes do not
/// re-wrap, so replaying them into a different grid stacks every full line — the
/// defect ROADMAP.md parks under "terminal-state serialization". The pane is
/// always attaching a BRAND-NEW xterm, so it can hand the recording the grid it
/// was written for and let xterm reflow it back afterwards.
#[derive(Serialize)]
pub struct ShellAttach {
    pub gen: u64,
    pub replay: usize,
    pub replay_cols: Option<u16>,
}

/// Start (or re-attach to) the dock shell for `index` and stream it to
/// `on_bytes`; see `ShellAttach` for what it answers.
/// Idempotent: a second call for a live shell just swaps the sink and replays —
/// which is exactly what a tab flip or a dock re-open does.
///
/// The registry lock is held for the WHOLE body, spawn included. Two calls for
/// the same key can be in flight at once (StrictMode double-mounts the pane's
/// effect), and the old check-unlock-spawn-insert shape let both see an empty
/// slot: two shells spawned, the second insert winning, the first leaked as an
/// orphan no sweep could see. The spawn is a few ms and there's one webview —
/// serializing every shell command through it is the cheap correct answer.
pub fn shell_open(
    ctx: &Ctx,
    repo: String,
    slug: String,
    index: u32,
    cols: u16,
    rows: u16,
    on_bytes: Arc<dyn ByteSink>,
    shells: &Shells,
) -> Result<ShellAttach, String> {
    // A client's string, about to name a directory a login shell starts in:
    // the shape check here, "is it a place" in `place_session_cwd`, both
    // before anything spawns.
    let slug = worktrees_core::argv::safe_arg(&slug, "slug")?;
    let key: ShellKey = (repo.clone(), slug.clone(), index);
    let mut map = shells.0.lock().unwrap();
    if let Some(sh) = map.get_mut(&key) {
        // The grid the ring was written for, read BEFORE the resize below moves
        // it. This is the pane's reflow input (see `ShellAttach::replay_cols`) —
        // a tab flip after a ⌘B or a window drag is exactly the case where the
        // recording and the terminal about to render it disagree.
        let was_cols = sh.size.cols;
        // ring THEN sink, the same order the reader takes them — otherwise a
        // write landing mid-replay is either sent twice or dropped
        let ring = sh.ring.lock().unwrap();
        let mut sink = sh.sink.lock().unwrap();
        let snapshot: Vec<u8> = ring.iter().copied().collect();
        let replay = snapshot.len();
        if replay > 0 {
            let _ = on_bytes.send(&snapshot);
        }
        *sink = Some(on_bytes);
        drop(sink);
        drop(ring);
        // Replay THEN resize, and only if the size really moved. The order is
        // the ring/sink one above (a resize before the snapshot would let the
        // SIGWINCH redraw land in neither); the suppression is what keeps a tab
        // flip from feeding the ring a redraw per attach.
        let _ = sh.resize(cols, rows);
        sh.gen += 1;
        return Ok(ShellAttach { gen: sh.gen, replay, replay_cols: (replay > 0).then_some(was_cols) });
    }

    let (session, cwd) = place_session_cwd(&repo, &slug)?;
    // One-time cleanup for anyone upgrading: their `<session>~term*` sidecars
    // are orphans now — nothing will ever attach them again. Cheap and
    // idempotent, and only reached when this place has no shell yet. (Runs
    // under the registry lock — a tmux round-trip, but only on first open.)
    if worktrees_core::tmux::have_tmux() {
        worktrees_core::tmux::kill_shell_sidecars(&session);
    }

    let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
    let pair = native_pty_system().openpty(size).map_err(|e| e.to_string())?;

    // A LOGIN shell, like Terminal.app: it sources the user's profile, so the
    // shell has the real PATH even though this process was launched by launchd
    // with a bare one (fixup_gui_path covers our own shell-outs, not this).
    let shell_bin = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = CommandBuilder::new(&shell_bin);
    scrub_pty(&mut cmd);
    cmd.arg("-l");
    // Only the SPAWN path consults the memory: a re-attach above returned long
    // ago, and that shell's cwd is whatever the user has since cd'd to.
    let saved = remembered_dir(&read_cwds(ctx), &repo, &slug, index);
    cmd.cwd(pick_start_dir(saved.as_deref(), &cwd));
    cmd.env("TERM", "xterm-256color");
    // This tab's own command history, where the shell can be pointed at one.
    // Both preferences are read ONCE here: with either on, the tree is resolved
    // (and created); with both off, nothing on disk is touched at all, which is
    // what a switch that is off should mean.
    let want_hist = PER_TAB_HISTORY.load(Ordering::Relaxed);
    let want_scroll = PERSIST_SCROLLBACK.load(Ordering::Relaxed);
    let hist_root = (want_hist || want_scroll).then(|| term_hist_dir(ctx).ok()).flatten();
    if want_hist {
        if let Some(root) = hist_root.as_deref() {
            // Stamped only when a per-tab shell directory was really created:
            // that directory is the one thing here no scrollback flush would
            // write a meta for, and a directory with no meta is one no sweep can
            // ever collect.
            if apply_tab_history_env(&mut cmd, root, &key, &shell_bin).is_some() {
                ensure_meta(root, &key, cols);
            }
        }
    }

    let child = pair.slave.spawn_command(cmd).map_err(|e| {
        applog("error", &format!("shell_open {slug}#{index}: spawn {shell_bin} failed: {e}"));
        e.to_string()
    })?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    // A tab that ran before this app did starts with its own history: the ring
    // saved on the last run, replayed into the fresh xterm exactly as a tab flip
    // replays a live one.
    //
    // Sent BEFORE `sink` exists and before the reader thread is spawned, so
    // nothing the new shell writes can get in front of it — its early output
    // sits in the pty buffer meanwhile with no reader to forward it. The
    // ordering is true by construction here, not by a lock.
    let restored = want_scroll.then(|| hist_root.as_deref().and_then(|root| read_scrollback(root, &key))).flatten();
    let mut seed = VecDeque::<u8>::with_capacity(8192);
    let (mut replay, mut replay_cols) = (0usize, None);
    if let Some((bytes, meta)) = restored {
        let payload = restore_payload(&bytes, meta.at);
        if !payload.is_empty() {
            replay = payload.len();
            replay_cols = Some(meta.cols);
            // Ring and channel get the SAME bytes, seam included — a re-attach
            // replays the ring and nothing else, so anything left out of it is
            // gone the first time the tab is flipped away from. See
            // `restore_payload`.
            seed.extend(payload.iter().copied());
            let _ = on_bytes.send(&payload);
        }
    }

    let ring = Arc::new(Mutex::new(seed));
    let sink = Arc::new(Mutex::new(Some(on_bytes)));
    let stop = Arc::new(AtomicBool::new(false));
    let dirty = Arc::new(AtomicBool::new(false));

    let (r_ring, r_sink, r_stop, r_dirty) =
        (ring.clone(), sink.clone(), stop.clone(), dirty.clone());
    let exit_key = key.clone();
    // The reader outlives this call, so it needs an OWNED `Ctx` — which is
    // cheap, every field being an `Arc` or a `PathBuf`, and is the reason
    // `Ctx` is `Clone` at all.
    let exit_ctx = ctx.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            if r_stop.load(Ordering::Relaxed) {
                return; // closed deliberately — no exit event
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    let mut ring = r_ring.lock().unwrap();
                    ring.extend(chunk.iter().copied());
                    let overflow = ring.len().saturating_sub(SHELL_RING);
                    ring.drain(..overflow);
                    r_dirty.store(true, Ordering::Relaxed);
                    // held together with the ring (see the re-attach comment)
                    let mut sink = r_sink.lock().unwrap();
                    if sink.as_ref().is_some_and(|ch| ch.send(chunk).is_err()) {
                        // The CLIENT is gone, not the shell. Let that client's
                        // sink go — exactly what a detach does — and keep
                        // reading: the ring goes on recording and the next
                        // attach replays it. Breaking here left a live shell
                        // with nobody reading its pty, which under `serve` is
                        // every page reload that happens while a shell prints.
                        *sink = None;
                    }
                }
                Err(_) => break,
            }
        }
        // The shell itself exited (`exit`, or it died). The tab stays — the
        // frontend offers a restart rather than silently vanishing.
        if !r_stop.load(Ordering::Relaxed) {
            // Nothing will be written to the attached channel again, so let
            // it go BEFORE announcing the exit: a host that ends a dropped
            // channel (serve's end frame; Tauri's own on `Channel` drop) then
            // ends it ahead of the event, on the one ordered queue, and the
            // client can release the handler the moment it hears.
            r_sink.lock().unwrap().take();
            let (repo, slug, index) = exit_key;
            exit_ctx.emit("shell:exit", serde_json::json!({ "repo": repo, "slug": slug, "index": index }));
        }
    });

    map.insert(key, Shell { master: pair.master, writer, child, stop, ring, sink, gen: 1, size, dirty });
    Ok(ShellAttach { gen: 1, replay, replay_cols })
}

pub fn shell_write(repo: String, slug: String, index: u32, data: Vec<u8>, shells: &Shells) -> Result<(), String> {
    let mut map = shells.0.lock().unwrap();
    let sh = map.get_mut(&(repo, slug, index)).ok_or("no such shell")?;
    sh.writer.write_all(&data).map_err(|e| e.to_string())?;
    sh.writer.flush().map_err(|e| e.to_string())
}

pub fn shell_resize(repo: String, slug: String, index: u32, cols: u16, rows: u16, shells: &Shells) -> Result<(), String> {
    let mut map = shells.0.lock().unwrap();
    let sh = map.get_mut(&(repo, slug, index)).ok_or("no such shell")?;
    sh.resize(cols, rows)
}

/// Stop streaming; the shell keeps running. This is the unmount path — a tab
/// flip or ⌘J must not kill what you left building. `gen` names the attach
/// being undone: a detach that lost the race to a newer attach is a no-op
/// instead of clearing the newcomer's sink (see `Shell::gen`).
pub fn shell_detach(repo: String, slug: String, index: u32, gen: u64, shells: &Shells) -> Result<(), String> {
    let map = shells.0.lock().unwrap();
    if let Some(sh) = map.get(&(repo, slug, index)) {
        if sh.gen == gen {
            *sh.sink.lock().unwrap() = None;
        }
    }
    Ok(())
}


/// A place's CANONICAL session name + worktree cwd, both derived backend-side
/// from `repo` + `slug`. The dock never supplies a session name or path — that
/// keeps the webview from naming/killing arbitrary tmux sessions or opening a
/// shell outside the workspace, and keeps sidecar names STABLE (a place's
/// canonical name doesn't change when it briefly runs under an adopted session).
///
/// Only for a slug that IS a place, by the index `drop_reference` and the MCP
/// resources read: `place_dir` joins whatever it is given onto `.worktrees/`,
/// and a slug naming nothing became a shell in a directory that is not one.
pub fn place_session_cwd(repo: &str, slug: &str) -> Result<(String, String), String> {
    let p = Project::discover(Path::new(repo)).map_err(|e| e.msg)?;
    if !p.place_index().iter().any(|pl| pl.slug == slug) {
        return Err(format!("no such place: {slug}"));
    }
    Ok((p.session_name(slug), p.place_dir(slug)))
}

/// A place's dock shells, with liveness. The dock restores its Terminal tabs
/// from this; shells don't outlive the app (see `Shells`), so after a restart
/// it's empty and the dock opens a fresh one. `dead` matters because the
/// `shell:exit` event is transient — a shell that exits while the dock is
/// closed has no listener, and without this flag the reopened dock would render
/// the corpse as a live tab (replayed scrollback, EIO on the first keypress).
#[derive(Serialize)]
pub struct ShellTab {
    pub index: u32,
    pub dead: bool,
}

pub fn list_shell_sessions(repo: String, slug: String, shells: &Shells) -> Result<Vec<ShellTab>, String> {
    let mut map = shells.0.lock().unwrap();
    let mut tabs: Vec<ShellTab> = map
        .iter_mut()
        .filter(|((r, s, _), _)| r == &repo && s == &slug)
        .map(|((_, _, i), sh)| ShellTab { index: *i, dead: matches!(sh.child.try_wait(), Ok(Some(_))) })
        .collect();
    tabs.sort_unstable_by_key(|t| t.index);
    Ok(tabs)
}

/// End one shell tab — kills the process, unlike `shell_detach`.
pub fn close_shell_session(
    ctx: &Ctx,
    repo: String,
    slug: String,
    index: u32,
    keep_cwd: Option<bool>,
    shells: &Shells,
) -> Result<(), String> {
    let key: ShellKey = (repo.clone(), slug.clone(), index);
    let keep = keep_cwd.unwrap_or(false);
    // A RESTART keeps this tab's scrollback, so persist it up to the second
    // before the corpse is reaped. This is the one moment where "as of up to 15
    // seconds ago" would be visible in the same breath as the action that caused
    // it — and the dead shell's last output is usually the thing being restarted
    // to get away from.
    if keep {
        flush_one_scrollback(ctx, &shells, &key);
    }
    kill_shell(&shells, &key);
    // A tab the user CLOSED forgets where it was, exactly as it drops its name
    // (App.tsx `closeTab`) — otherwise the next tab to take this index would
    // open in a directory it never visited. A tab being RESTARTED goes through
    // this same command to reap the corpse and asks to KEEP its directory: it
    // is the same tab, and restarting a shell that died in a subdirectory only
    // to land at the place root is the exact papercut this feature removes.
    if !keep {
        edit_cwds(ctx, |map| forget_tab(map, &repo, &slug, index));
        // …and its scrollback and command history with it, for the same reason
        // and with more at stake: the next tab to take this index has never been
        // here, and inheriting someone else's output is worse than inheriting
        // their directory.
        if let Ok(root) = term_hist_dir(ctx) {
            forget_tab_history(&root, &key);
        }
    }
    Ok(())
}


/// `"<repo>|<slug>"` → tab index → last known directory. Same key scheme as the
/// frontend's `term_tab_names`, which this shadows one-for-one.
pub type CwdMap = BTreeMap<String, BTreeMap<u32, String>>;

/// Serialises writers to `shell-cwds.json`: the slow sampler and the shell
/// commands both read-modify-write, and interleaving them would lose an update.
pub static CWD_FILE_LOCK: Mutex<()> = Mutex::new(());

pub fn cwd_key(repo: &str, slug: &str) -> String {
    format!("{repo}|{slug}")
}

pub fn cwd_file(ctx: &Ctx) -> Result<PathBuf, String> {
    let dir = ctx.config_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("shell-cwds.json"))
}

/// A missing or corrupt file is an empty memory, never an error: the worst it
/// can cost is one shell opening at the place root.
pub fn read_cwds_at(path: &Path) -> CwdMap {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The engine's read of the memory, which a spawn does — and so what this
/// engine has SEEN of the file (`unasked_save_ok`).
pub fn read_cwds(ctx: &Ctx) -> CwdMap {
    let Ok(path) = cwd_file(ctx) else { return CwdMap::default() };
    let bytes = std::fs::read(&path).ok();
    note_seen(&path, seen_of(bytes.as_deref()));
    bytes.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Read-modify-write under `CWD_FILE_LOCK`. `f` returns false to skip the write —
/// the sampler runs every few seconds and an idle shell must not churn the disk.
pub fn edit_cwds_at(path: &Path, f: impl FnOnce(&mut CwdMap) -> bool) {
    edit_cwds_as(path, false, f)
}

/// `edit_cwds_at`, as the SAMPLER's save when `unasked`: then only over a file
/// this engine is still the last to have written or read (`unasked_save_ok`).
fn edit_cwds_as(path: &Path, unasked: bool, f: impl FnOnce(&mut CwdMap) -> bool) {
    let _guard = CWD_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if unasked && !unasked_save_ok(path, "shell-cwds") {
        return;
    }
    let mut map = read_cwds_at(path);
    if !f(&mut map) {
        return;
    }
    map.retain(|_, tabs| !tabs.is_empty());
    let json = match serde_json::to_vec_pretty(&map) {
        Ok(j) => j,
        Err(e) => return applog("error", &format!("shell-cwds encode failed: {e}")),
    };
    // Write-then-rename, because readers do NOT take this lock: `shell_open`
    // reads the file while holding the shell registry, and a plain `fs::write`
    // is a truncate followed by a write — a read landing in between gets half a
    // JSON document, which parses as "no memory at all".
    let seen = seen_of(Some(&json));
    match worktrees_core::fsx::write_atomic(path, &json, None) {
        Ok(()) => note_seen(path, seen),
        Err(e) => applog("error", &format!("shell-cwds write failed: {e}")),
    }
}

/// The directory remembered for one tab, if any.
pub fn remembered_dir(map: &CwdMap, repo: &str, slug: &str, index: u32) -> Option<String> {
    map.get(&cwd_key(repo, slug)).and_then(|tabs| tabs.get(&index)).cloned()
}

/// Drop one tab's directory; true if there was one. Empty places are swept by
/// `edit_cwds_at`, so a place whose last tab closes leaves no husk.
pub fn forget_tab(map: &mut CwdMap, repo: &str, slug: &str, index: u32) -> bool {
    map.get_mut(&cwd_key(repo, slug)).is_some_and(|tabs| tabs.remove(&index).is_some())
}

pub fn edit_cwds(ctx: &Ctx, f: impl FnOnce(&mut CwdMap) -> bool) {
    match cwd_file(ctx) {
        Ok(path) => edit_cwds_at(&path, f),
        // A read that can't find the file is just "no memory yet"; a WRITE that
        // can't resolve the config dir is a real failure and has to say so.
        Err(e) => applog("error", &format!("shell-cwds path unavailable: {e}")),
    }
}

/// A live process's working directory, straight from the OS — no subprocess and
/// no shell integration. macOS has no `/proc`, so it goes through libproc
/// (`libc` is already a dependency for the `kill(pid,0)` probes). The shell-side
/// alternative, OSC 7, is a non-starter here: Apple's `/etc/zshrc` only emits it
/// when `TERM_PROGRAM` is `Apple_Terminal`, so we would have to lie about
/// `TERM_PROGRAM` or edit the user's rc files.
pub fn proc_cwd(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let want = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
        // Returns bytes written. Anything short of the whole struct (dead pid,
        // EPERM) means there is no path to be had — not a truncated one.
        let got = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut info as *mut libc::proc_vnodepathinfo).cast(),
                want,
            )
        };
        if got != want {
            return None;
        }
        // libc spells this MAXPATHLEN buffer `[[c_char; 32]; 32]` (a workaround
        // for the old rustc it supports), so flatten it back to bytes.
        let raw = &info.pvi_cdir.vip_path;
        let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), std::mem::size_of_val(raw)) };
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        (end > 0).then(|| PathBuf::from(String::from_utf8_lossy(&bytes[..end]).into_owned()))
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// The pid of a shell that is STILL RUNNING, or `None`.
///
/// The liveness half is not defensive tidiness, it is the whole point. A shell
/// that exits on its own is deliberately left in the registry so the tab can
/// survive and offer a restart — and `list_shell_sessions` calls `try_wait` on
/// every dock mount, which REAPS it. `Child::process_id` then keeps handing
/// back a pid that the OS is free to hand to something else (macOS wraps at
/// 99999, days on a normal machine). Sampling that pid would read a stranger's
/// directory and file it under this tab.
pub fn live_pid(child: &mut (dyn Child + Send + Sync)) -> Option<u32> {
    match child.try_wait() {
        Ok(None) => child.process_id(), // still running
        _ => None,                      // exited, or we cannot tell — either way, don't sample
    }
}

/// Fold a sample into the stored map; true if anything actually moved.
///
/// MERGES rather than replaces, and that is the whole subtlety: after a restart
/// a tab exists as a NAME long before it is re-spawned, so the map holds
/// entries with no live shell behind them. A wholesale rewrite from the live
/// set would wipe exactly the directories this feature exists to keep.
pub fn merge_cwds(map: &mut CwdMap, sample: Vec<(ShellKey, String)>) -> bool {
    let mut changed = false;
    for ((repo, slug, index), path) in sample {
        let tabs = map.entry(cwd_key(&repo, &slug)).or_default();
        if tabs.get(&index) != Some(&path) {
            tabs.insert(index, path);
            changed = true;
        }
    }
    changed
}

/// Record where every live dock shell currently is. Called on a slow tick — so
/// a crash or a force-quit still leaves a recent answer — and once more on the
/// way out, before the exit sweep kills the shells. Both are saves nobody
/// asked for, so both go through `unasked_save_ok`.
pub fn save_shell_cwds(ctx: &Ctx, shells: &Shells) {
    let live: Vec<(ShellKey, u32)> = {
        let mut map = shells.0.lock().unwrap();
        map.iter_mut().filter_map(|(k, sh)| live_pid(&mut *sh.child).map(|pid| (k.clone(), pid))).collect()
    };
    let sample: Vec<(ShellKey, String)> = live
        .into_iter()
        .filter_map(|(key, pid)| proc_cwd(pid).map(|p| (key, p.to_string_lossy().into_owned())))
        .collect();
    if sample.is_empty() {
        return; // no shells (or no cwd to be read) — don't even open the file
    }
    let path = match cwd_file(ctx) {
        Ok(p) => p,
        Err(e) => return applog("error", &format!("shell-cwds path unavailable: {e}")),
    };
    edit_cwds_as(&path, true, |map| {
        // Membership is re-checked HERE, under the file lock, because the
        // registry lock was released before the `proc_cwd` reads above. A tab
        // closed in that gap has already had its entry forgotten, and merging
        // the reading we took a moment earlier would put it straight back — so
        // the next tab to reuse that index would open somewhere it has never
        // been, which is exactly what `close_shell_session` promises cannot
        // happen. (No lock-order hazard: nothing that holds the registry ever
        // waits on this file lock — `shell_open` only READS the file, and reads
        // don't take it.)
        let open = shells.0.lock().unwrap();
        let fresh: Vec<(ShellKey, String)> = sample.into_iter().filter(|(k, _)| open.contains_key(k)).collect();
        drop(open);
        merge_cwds(map, fresh)
    });
}

/// Which `repo|slug` keys no longer name a place on disk. `place_dir` resolves
/// one, or returns `None` when the PROJECT itself is unreachable — a repo that
/// has been deleted or moved takes all of its places with it.
pub fn vanished_keys(keys: &[String], mut place_dir: impl FnMut(&str, &str) -> Option<String>) -> Vec<String> {
    keys.iter()
        .filter(|key| {
            // repo roots are absolute paths and a slug is a directory basename,
            // so the LAST separator is the real one
            match key.rsplit_once('|') {
                Some((repo, slug)) => !place_dir(repo, slug).is_some_and(|d| Path::new(&d).is_dir()),
                None => true, // not a key this app writes — it cannot name a live place
            }
        })
        .cloned()
        .collect()
}

/// Forget every place whose worktree is gone. The app cleans up after its own
/// `remove_place`, but `worktrees rm` from a terminal leaves the entry behind,
/// and the key is only `repo|slug` — so a slug that is later recreated (a
/// recycled branch name) would inherit the previous life's directories wherever
/// those paths still exist in the new checkout. Sweeping at startup shrinks
/// that to the case where the place is removed and recreated with the app
/// closed, which is accepted: what survives is a real directory inside the
/// worktree, so the shell opens somewhere that exists rather than anywhere
/// misleading.
pub fn forget_vanished_places(ctx: &Ctx) {
    let keys: Vec<String> = read_cwds(ctx).keys().cloned().collect();
    if keys.is_empty() {
        return;
    }
    let mut projects: HashMap<String, Option<Project>> = HashMap::new();
    let gone = vanished_keys(&keys, |repo, slug| {
        projects.entry(repo.to_string()).or_insert_with(|| Project::discover(Path::new(repo)).ok());
        projects[repo].as_ref().map(|p| p.place_dir(slug))
    });
    if gone.is_empty() {
        return;
    }
    applog("info", &format!("shell-cwds: forgetting {} place(s) that no longer exist", gone.len()));
    edit_cwds(ctx, |map| {
        let before = map.len();
        map.retain(|k, _| !gone.contains(k));
        map.len() != before
    });
}

/// Where a tab should reopen: its last known directory if that still exists,
/// else the place root. No subtree restriction — if you cd'd out of the
/// worktree, out of the worktree is where you were.
pub fn pick_start_dir(saved: Option<&str>, place_dir: &str) -> String {
    match saved {
        Some(p) if Path::new(p).is_dir() => p.to_string(),
        _ => place_dir.to_string(),
    }
}


/// For the directory NAME only — never for identity (see `read_scrollback`).
/// FNV-1a 64, inline rather than a crate for eight lines. Not a security
/// boundary: it names a file.
pub fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A readable, filesystem-safe stem for one tab. The slug is decoration — it is
/// what makes the directory browsable — so it is clamped hard; the hash carries
/// all of the uniqueness.
pub fn tab_stem(key: &ShellKey) -> String {
    let (repo, slug, index) = key;
    let safe: String = slug
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '-' })
        .take(32)
        .collect();
    format!("{safe}-{index}-{:016x}", fnv1a64(&format!("{repo}|{slug}|{index}")))
}

/// How long an untouched tab's history is kept. This tree holds CONTENT, and
/// unlike the cwd map there is nothing about it that shrinks on its own, so it
/// needs a horizon as well as the sweeps.
pub const TERM_HIST_MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;

/// Serialises writers, exactly as `CWD_FILE_LOCK` does for the directories: the
/// slow sampler, the shell commands and the sweeps all touch this tree.
pub static TERM_HIST_LOCK: Mutex<()> = Mutex::new(());

/// HOST policy the shell path needs (`hostpolicy`): whether a tab's ring is
/// saved and replayed, and whether a new shell gets its own history. Boot
/// loads them from host-policy.json and `set_term_history_opts` is their only
/// other writer — never a page load, and never `ui-state.json`, which the
/// backend must not read for them because it must never write it.
pub static PERSIST_SCROLLBACK: AtomicBool = AtomicBool::new(true);
pub static PER_TAB_HISTORY: AtomicBool = AtomicBool::new(true);

/// `<app config>/term-history`, created 0700 on demand.
pub fn term_hist_dir(ctx: &Ctx) -> Result<PathBuf, String> {
    let dir = ctx.config_dir.join("term-history");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    private_perms(&dir, 0o700);
    Ok(dir)
}

/// Tighten a path we just created. Best-effort: a filesystem that cannot express
/// the mode is not a reason to refuse to remember a tab.
pub fn private_perms(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

pub fn tab_dir(root: &Path, key: &ShellKey) -> PathBuf {
    root.join(tab_stem(key))
}

/// Which tab a directory belongs to, and the WIDTH its bytes were laid out at.
///
/// `cols` is not bookkeeping. Raw bytes replayed into a different grid do not
/// re-wrap, and the cursor-relative redraws in them were computed for a width
/// that no longer exists — which is the defect ROADMAP.md parks under
/// "terminal-state serialization". Recording the width lets the pane hand the
/// recording the grid it was written for and let xterm reflow it back.
///
/// It is the pty's CURRENT width, not the width the oldest byte was written at —
/// a byte ring has no single true width. It converges: quit at 174, restore at
/// 100, and the next save records 100.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct TabMeta {
    pub repo: String,
    pub slug: String,
    pub index: u32,
    pub cols: u16,
    pub at: i64,
}

pub fn read_meta(tab: &Path) -> Option<TabMeta> {
    serde_json::from_slice(&std::fs::read(tab.join("meta.json")).ok()?).ok()
}

/// Write-then-rename, 0600. Readers deliberately do NOT take `TERM_HIST_LOCK` —
/// `shell_open` reads this tree while holding the shell registry, and nothing
/// holding the registry may wait on a file lock — so a plain write (a truncate
/// followed by a write) would let a read land on half a document.
pub fn hist_write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    worktrees_core::fsx::write_atomic(path, bytes, Some(std::fs::Permissions::from_mode(0o600)))
}

/// Drop a leading partial line.
///
/// Only correct for a ring that has actually ROLLED — see `rolled_ring` below,
/// which is the only caller and exists to state that condition.
/// Bounded, so a ring with no newline at all is left alone rather than emptied.
pub fn trim_to_line_start(b: &[u8]) -> &[u8] {
    const LOOK: usize = 4096;
    match b[..b.len().min(LOOK)].iter().position(|&c| c == b'\n') {
        Some(i) => &b[i + 1..],
        None => b,
    }
}

/// What of a ring is worth saving.
///
/// A ring at the CAP has been drained from the front, so it begins wherever
/// `drain` happened to cut — routinely the tail of an escape sequence whose
/// introducer is gone, which replays as a few stray characters at the top. That
/// leading fragment is dropped.
///
/// A ring BELOW the cap has never rolled: it begins where the shell began, and
/// its first line is a real one. Trimming that would throw away a genuine line
/// on every save — and worse, it COMPOUNDS, because the next restore seeds from
/// the trimmed copy and the save after that trims the new first line. One line
/// per restart, silently, off the oldest end. The length test is what separates
/// the two cases, and it is exact: nothing else can put a ring at the cap.
pub fn rolled_ring(bytes: &[u8]) -> &[u8] {
    if bytes.len() >= SHELL_RING {
        trim_to_line_start(bytes)
    } else {
        bytes
    }
}

/// This tab's saved scrollback and the metadata it was saved with, or `None`.
///
/// `meta.json` is the identity, not the directory name: anything that does not
/// name exactly this tab answers `None`. Never an error, and never another tab's
/// output.
///
/// It is also this engine's look at the file, which a spawn takes whatever it
/// answers — "nothing there" included — so the saves after it know what they
/// would be writing over (`unasked_save_ok`).
pub fn read_scrollback(root: &Path, key: &ShellKey) -> Option<(Vec<u8>, TabMeta)> {
    let tab = tab_dir(root, key);
    let ring = tab.join("scrollback.bin");
    let bytes = std::fs::read(&ring).ok();
    note_seen(&ring, seen_of(bytes.as_deref()));
    let meta = read_meta(&tab)?;
    if meta.repo != key.0 || meta.slug != key.1 || meta.index != key.2 {
        return None;
    }
    Some((bytes?, meta))
}

/// Stamp a tab's identity without touching its scrollback.
///
/// Called when a tab's `zdotdir` is generated, so a tab whose shell is closed
/// before the first flush still has the `meta.json` every sweep here reads. A
/// directory with no meta is one nothing can judge, so it would never be
/// collected.
pub fn ensure_meta(root: &Path, key: &ShellKey, cols: u16) {
    let tab = tab_dir(root, key);
    if read_meta(&tab).is_some() {
        return;
    }
    write_meta(&tab, key, cols);
}

pub fn write_meta(tab: &Path, key: &ShellKey, cols: u16) {
    let meta = TabMeta {
        repo: key.0.clone(),
        slug: key.1.clone(),
        index: key.2,
        cols,
        at: sysclock::now_epoch(),
    };
    match serde_json::to_vec_pretty(&meta) {
        Ok(j) => {
            if let Err(e) = hist_write_atomic(&tab.join("meta.json"), &j) {
                applog("error", &format!("term-history meta write failed: {e}"));
            }
        }
        Err(e) => applog("error", &format!("term-history meta encode failed: {e}")),
    }
}

/// Persist one tab's ring at the width it is currently being written at.
pub fn write_scrollback(root: &Path, key: &ShellKey, bytes: &[u8], cols: u16) {
    write_scrollback_as(root, key, bytes, cols, false)
}

/// `write_scrollback`, as the flush's save when `unasked`: then only over a
/// ring this engine is still the last to have written or read (`unasked_save_ok`).
fn write_scrollback_as(root: &Path, key: &ShellKey, bytes: &[u8], cols: u16, unasked: bool) {
    let tab = tab_dir(root, key);
    let ring = tab.join("scrollback.bin");
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if unasked && !unasked_save_ok(&ring, "term-history") {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&tab) {
        return applog("error", &format!("term-history dir failed: {e}"));
    }
    private_perms(&tab, 0o700);
    // bytes THEN meta: meta is what makes the directory readable, so writing it
    // last means a crash mid-save leaves the PREVIOUS pair, never a new meta
    // pointing at a half-written ring.
    let bytes = rolled_ring(bytes);
    if let Err(e) = hist_write_atomic(&ring, bytes) {
        return applog("error", &format!("term-history write failed: {e}"));
    }
    note_seen(&ring, seen_of(Some(bytes)));
    write_meta(&tab, key, cols);
}

/// Everything one tab keeps between runs. Called when the tab is CLOSED — a
/// RESTART keeps it (it is the same tab, and the dead shell's output is usually
/// the thing you wanted to read), and so does closing the place, which keeps tab
/// names and therefore has to keep what they hold.
pub fn forget_tab_history(root: &Path, key: &ShellKey) {
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_dir_all(tab_dir(root, key));
}

/// Every tab of one place. Used when the place is REMOVED.
pub fn forget_place_history(root: &Path, repo: &str, slug: &str) {
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && read_meta(&p).is_some_and(|m| m.repo == repo && m.slug == slug) {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
}

/// Which stored tabs should go: the ones whose place no longer exists, and the
/// ones nobody has touched in a month.
///
/// Split out from the filesystem so the decision can be tested directly, the way
/// `vanished_keys` is. `place_dir` answers `None` when the PROJECT itself is
/// unreachable — a repo that has been deleted or moved takes its places with it.
pub fn stale_history<'a>(
    metas: &'a [(PathBuf, TabMeta)],
    now: i64,
    mut place_dir: impl FnMut(&str, &str) -> Option<String>,
) -> Vec<&'a PathBuf> {
    metas
        .iter()
        .filter(|(_, m)| {
            now - m.at > TERM_HIST_MAX_AGE_SECS
                || !place_dir(&m.repo, &m.slug).is_some_and(|d| Path::new(&d).is_dir())
        })
        .map(|(p, _)| p)
        .collect()
}

/// Cold-start sweep. Same reasoning as `forget_vanished_places` — a
/// `worktrees rm` from a terminal never tells the app — plus the age horizon,
/// because this tree holds content rather than a path.
pub fn sweep_term_history(ctx: &Ctx) {
    let Ok(root) = term_hist_dir(ctx) else { return };
    let Ok(rd) = std::fs::read_dir(&root) else { return };
    let metas: Vec<(PathBuf, TabMeta)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        // no readable meta = nothing to judge it by; left alone rather than
        // guessed at (`ensure_meta` is what keeps that set empty)
        .filter_map(|p| read_meta(&p).map(|m| (p, m)))
        .collect();
    if metas.is_empty() {
        return;
    }
    let mut projects: HashMap<String, Option<Project>> = HashMap::new();
    let gone = stale_history(&metas, sysclock::now_epoch(), |repo, slug| {
        projects.entry(repo.to_string()).or_insert_with(|| Project::discover(Path::new(repo)).ok());
        projects[repo].as_ref().map(|p| p.place_dir(slug))
    });
    if gone.is_empty() {
        return;
    }
    applog("info", &format!("term-history: sweeping {} tab(s)", gone.len()));
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for p in gone {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// One shell's ring, ready to write once the registry lock is gone.
pub type ScrollJob = (ShellKey, Arc<Mutex<VecDeque<u8>>>, u16);

// ── saves nobody asked for ───────────────────────────────────────────────────
//
// The dock's tick and its exit flush write the rings and `shell-cwds.json` on
// their own. With one engine per config dir that is the whole story; with two
// it is not. An app crash left its serve running with no client, a downgraded
// app ran its own engine on the same dir and saved the tab on a clean quit,
// and the orphan's next tick put its own ring and directory back over both
// (rehearsal D7). An app-owned serve now stops itself when no client comes
// back, but its way down IS the exit flush, so the flush has to know better
// as well: it writes only over a file that is still what this engine last
// wrote or read there. A save somebody asked for — a restart, closing a tab —
// is an act in THIS engine, and writes as before.
//
// What is left: a new serve could read a tab in the moment before an old one's
// last write of it, and would then leave that tab alone for its life (logged).
// serve flushes BEFORE its port closes, and nothing starts a new serve while
// the old one's port is open, so between two of ours that ordering cannot
// happen.

/// What this engine saw of a file it saves on its own: a hash of the bytes it
/// last wrote there or read from it, or `None` for "not there".
type Seen = Option<u64>;

fn seen_of(bytes: Option<&[u8]>) -> Seen {
    use std::hash::{Hash, Hasher};
    bytes.map(|b| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        b.hash(&mut h);
        h.finish()
    })
}

/// Per file, what this process last wrote there or read from it. Keyed by
/// the full path, so tests on their own scratch dirs never share an entry.
static SEEN: Mutex<BTreeMap<PathBuf, Seen>> = Mutex::new(BTreeMap::new());

/// The files a refused save has been logged for: once per file per engine
/// life, since an orphan would otherwise say it every 15 s.
static SAID: Mutex<std::collections::BTreeSet<PathBuf>> = Mutex::new(std::collections::BTreeSet::new());

fn note_seen(path: &Path, seen: Seen) {
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), seen);
}

/// May a save nobody asked for write over the file as it is `now`? `was` is
/// what this engine last wrote or read there (`None`: it never looked).
/// Absent is always fine — there is nothing to lose. Otherwise only the very
/// bytes this engine saw: anything else is somebody else's save.
pub fn may_save(was: Option<Seen>, now: Seen) -> bool {
    match (was, now) {
        (_, None) => true,
        (Some(was), now) => was == now,
        (None, Some(_)) => false,
    }
}

/// Whether the refusal for `path` is the first one this engine has had to
/// say.
fn first_refusal(path: &Path) -> bool {
    SAID.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf())
}

/// `may_save` against the file on disk, and the one line when it says no.
fn unasked_save_ok(path: &Path, what: &str) -> bool {
    let now = seen_of(std::fs::read(path).ok().as_deref());
    let was = SEEN.lock().unwrap_or_else(|e| e.into_inner()).get(path).copied();
    if may_save(was, now) {
        return true;
    }
    if first_refusal(path) {
        applog(
            "warn",
            &format!(
                "{what}: not saving over {}: another engine changed it after this one last wrote or read it \
                 (an app with its own engine on the same config dir?)",
                path.display()
            ),
        );
    }
    false
}

/// Persist every live shell's ring.
///
/// Dirty-gated, so a dozen idle tabs cost no I/O at all. The snapshot is taken
/// under the REGISTRY lock and written after it is released: nothing that holds
/// the registry may wait on a file lock, which is the same rule `save_shell_cwds`
/// follows and the reason `shell_open` only ever reads this tree.
///
/// The flag is cleared BEFORE the ring is read, not after it is written. Output
/// landing in that window re-sets it and costs one redundant write next tick;
/// clearing it afterwards would instead drop the last chunk of a shell that then
/// went quiet — the one direction that loses something.
///
/// Deliberately NOT filtered on `live_pid`, unlike `save_shell_cwds`. A shell
/// that exited is kept in the registry precisely so its tab survives and can
/// offer a restart, and its ring is exactly what the user wanted to read; a dead
/// process has no cwd to sample, but it has plenty of scrollback.
pub fn save_shell_scrollback(ctx: &Ctx, shells: &Shells) {
    if !PERSIST_SCROLLBACK.load(Ordering::Relaxed) {
        return;
    }
    let jobs: Vec<ScrollJob> = {
        let map = shells.0.lock().unwrap();
        map.iter()
            .filter(|(_, sh)| sh.dirty.swap(false, Ordering::Relaxed))
            .map(|(k, sh)| (k.clone(), sh.ring.clone(), sh.size.cols))
            .collect()
    };
    if jobs.is_empty() {
        return;
    }
    let Ok(root) = term_hist_dir(ctx) else { return };
    for (key, ring, cols) in jobs {
        let bytes: Vec<u8> = ring.lock().unwrap().iter().copied().collect();
        // Unasked — the tick, or the exit flush — so never over another
        // engine's save of this tab (`unasked_save_ok`).
        write_scrollback_as(&root, &key, &bytes, cols, true);
    }
}

/// Persist ONE tab now, whatever its dirty flag says.
///
/// Used by the restart path: closing a dead shell to make room for a new one in
/// the same tab is the one moment where "up to 15 seconds ago" is visible in the
/// same breath as the action that caused it.
pub fn flush_one_scrollback(ctx: &Ctx, shells: &Shells, key: &ShellKey) {
    if !PERSIST_SCROLLBACK.load(Ordering::Relaxed) {
        return;
    }
    let job = {
        let map = shells.0.lock().unwrap();
        map.get(key).map(|sh| {
            sh.dirty.store(false, Ordering::Relaxed);
            (sh.ring.clone(), sh.size.cols)
        })
    };
    let (Some((ring, cols)), Ok(root)) = (job, term_hist_dir(ctx)) else { return };
    let bytes: Vec<u8> = ring.lock().unwrap().iter().copied().collect();
    write_scrollback(&root, key, &bytes, cols);
}

/// `at` on the machine's LOCAL wall clock.
///
/// The seam below is the one thing this app writes for a person to read where
/// it sits rather than for a log, and app.log's UTC — which CLAUDE.md already
/// warns about when cross-referencing — is the wrong register for it. libc,
/// because chrono is not a dependency and an offset is all `fmt_utc` lacks.
pub fn fmt_local(at: i64) -> String {
    #[cfg(unix)]
    {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let t = at as libc::time_t;
        if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return fmt_utc(at);
        }
        return fmt_utc(at + tm.tm_gmtoff as i64);
    }
    #[cfg(not(unix))]
    fmt_utc(at)
}

/// Opens the seam, and nothing else emits it: `\x18` cancels any control
/// sequence the ring's tail was cut inside, `\x1b\\` closes an unterminated
/// OSC/DCS. Without them a seam can be swallowed as the argument of a
/// half-written sequence — the one failure that reads as "the feature did not
/// run". It doubles as the marker `trim_trailing_seam` finds.
pub const SEAM_HEAD: &[u8] = b"\x18\x1b\\";

/// What separates restored output from the shell about to start under it. Dim,
/// so it reads as chrome rather than as something a command printed.
pub fn restored_seam(at: i64) -> Vec<u8> {
    let mut out = SEAM_HEAD.to_vec();
    out.extend_from_slice(format!("\r\n\x1b[2m── restored · {} ──\x1b[0m\r\n", fmt_local(at)).as_bytes());
    out
}

/// Drop a seam that is the LAST thing in the buffer — one a previous restore
/// wrote and nothing has been typed under since. A seam with a session's output
/// after it is left alone, because it still marks a real boundary.
///
/// The test is exact rather than a search: a seam is FIXED WIDTH (`fmt_utc` is),
/// so the buffer ends with one precisely when the last `n` bytes begin with
/// `SEAM_HEAD`. Scanning a window instead finds a seam that has work under it —
/// which trims a boundary that should have stayed, and which is what the second
/// half of `a_seam_nothing_was_typed_under_is_replaced_not_followed` caught.
pub fn trim_trailing_seam(bytes: &[u8]) -> &[u8] {
    let n = restored_seam(0).len();
    if bytes.len() < n {
        return bytes;
    }
    let cut = bytes.len() - n;
    if &bytes[cut..cut + SEAM_HEAD.len()] == SEAM_HEAD {
        &bytes[..cut]
    } else {
        bytes
    }
}

/// The bytes a restoring tab is handed first: what it had, then a seam saying
/// where that ended. Empty when there is nothing saved, so `replay` stays 0 and
/// the pane keeps treating the shell's own first output as live.
///
/// This goes into the RING as well as down the channel — which is a reversal
/// worth stating, because the tidy-looking rule is the opposite. A re-attach
/// replays the ring and nothing else, so a seam kept out of it is visible only
/// until the first tab flip; under React StrictMode the pane re-attaches before
/// anyone has seen anything, so the seam was effectively dead. Being in the ring
/// makes it persistable, which is why it carries an ABSOLUTE time rather than an
/// age that would freeze at "2h ago" forever, and why a seam still sitting at
/// the very end is REPLACED rather than followed — otherwise three launches with
/// nothing typed between them stack three of them.
pub fn restore_payload(bytes: &[u8], at: i64) -> Vec<u8> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = trim_trailing_seam(bytes).to_vec();
    out.extend_from_slice(&restored_seam(at));
    out
}


pub const SHIM_HEADER: &str = "# Generated by worktrees — per-tab shell history. Edits are overwritten.\n";

/// `$ZDOTDIR/.zshenv`: the load-bearing shim. Runs before anything else of the
/// user's and is the only place that can discover a ZDOTDIR they set themselves.
pub const ZSHENV_SHIM: &str = r#"_WT_Z=${ZDOTDIR}
unset ZDOTDIR
[ -r "$HOME/.zshenv" ] && . "$HOME/.zshenv"
_WT_USER_Z=${ZDOTDIR:-$HOME}
export ZDOTDIR=$_WT_Z
"#;

pub const ZPROFILE_SHIM: &str = r#"[ -r "${_WT_USER_Z:-$HOME}/.zprofile" ] && . "${_WT_USER_Z:-$HOME}/.zprofile"
"#;

/// `$ZDOTDIR/.zshrc`: hand the chain back, source theirs, then take the history.
///
/// `INC_APPEND_HISTORY` is not a nicety. The app SIGHUPs its shells on the way
/// out (portable-pty's `Child::kill()` sends SIGHUP, not SIGKILL), and zsh's
/// default is to flush history only on a clean exit — without this a force-quit
/// takes the whole session's commands with it, which is precisely the case this
/// feature exists to survive. `SHARE_HISTORY` is deliberately NOT set: each tab
/// has its own file, so there is nothing to share, and it would change arrow-key
/// behaviour the user never asked us to touch.
pub const ZSHRC_SHIM: &str = r#"if [ "${_WT_USER_Z:-$HOME}" = "$HOME" ]; then unset ZDOTDIR; else export ZDOTDIR=$_WT_USER_Z; fi
[ -r "${_WT_USER_Z:-$HOME}/.zshrc" ] && . "${_WT_USER_Z:-$HOME}/.zshrc"
[ -n "$WORKTREES_HISTFILE" ] && HISTFILE="$WORKTREES_HISTFILE"
HISTSIZE=10000
SAVEHIST=10000
setopt INC_APPEND_HISTORY
unset _WT_Z _WT_USER_Z
"#;

/// `$ZDOTDIR/.zlogin`: only reached by a NON-interactive login shell, because
/// `.zshrc` has restored ZDOTDIR by the time an interactive one gets here and
/// zsh then finds the user's own directly.
pub const ZLOGIN_SHIM: &str = r#"[ -r "${_WT_USER_Z:-$HOME}/.zlogin" ] && . "${_WT_USER_Z:-$HOME}/.zlogin"
"#;

/// (Re)write the four shims. Regenerated on every spawn so an app upgrade
/// refreshes them and a user who deletes the directory gets it back.
pub fn write_zdotdir(zdot: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(zdot)?;
    private_perms(zdot, 0o700);
    for (name, body) in [
        (".zshenv", ZSHENV_SHIM),
        (".zprofile", ZPROFILE_SHIM),
        (".zshrc", ZSHRC_SHIM),
        (".zlogin", ZLOGIN_SHIM),
    ] {
        std::fs::write(zdot.join(name), format!("{SHIM_HEADER}{body}"))?;
    }
    Ok(())
}

/// Only worth saying once a run — it is a property of the user's `$SHELL`, not
/// of any one tab, and a spawn happens every time a tab is activated.
pub static UNSUPPORTED_SHELL_LOGGED: std::sync::Once = std::sync::Once::new();

/// Point this tab's shell at its own command history. Answers which scheme was
/// applied, for the tests; the caller ignores it.
pub fn apply_tab_history_env(
    cmd: &mut CommandBuilder,
    root: &Path,
    key: &ShellKey,
    shell_bin: &str,
) -> Option<&'static str> {
    let name = Path::new(shell_bin).file_name().and_then(|s| s.to_str()).unwrap_or("");
    let tab = tab_dir(root, key);
    match name {
        "zsh" => {
            let zdot = tab.join("zdotdir");
            if let Err(e) = write_zdotdir(&zdot) {
                applog("error", &format!("per-tab history: {e}"));
                return None;
            }
            cmd.env("WORKTREES_HISTFILE", zdot.join(".zsh_history"));
            cmd.env("ZDOTDIR", &zdot);
            Some("zdotdir")
        }
        // bash reads HISTFILE from the environment and its rc files almost never
        // set it, so the simple lever is the right one here. There is no ZDOTDIR
        // equivalent, and `--rcfile` does not apply to the login shell this must
        // stay.
        "bash" | "sh" => {
            if std::fs::create_dir_all(&tab).is_err() {
                return None;
            }
            private_perms(&tab, 0o700);
            cmd.env("HISTFILE", tab.join(".bash_history"));
            Some("histfile")
        }
        // fish and the rest keep whatever history they already had. Better than a
        // half-working scheme that silently loses commands — and the one log line
        // is what makes "why is there no history here" answerable.
        other => {
            let other = other.to_string();
            UNSUPPORTED_SHELL_LOGGED.call_once(|| {
                applog(
                    "info",
                    &format!("per-tab history: {other} is not supported; shells keep their own history"),
                );
            });
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pty side of `childenv`: a removal reaches the child's environment,
    /// a replacement value too, and no edits (macOS, a .deb) leave the
    /// inherited environment exactly as it was.
    #[test]
    fn a_pty_child_gets_the_childenv_edits_and_nothing_else() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.env("LD_LIBRARY_PATH", "/tmp/.mount_x/usr/lib/:");
        cmd.env("PATH", "/tmp/.mount_x/usr/bin/:/usr/bin:/bin");
        cmd.env("KEEP", "/home/u");
        let before: Vec<(String, String)> =
            cmd.iter_full_env_as_str().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        apply_pty(&mut cmd, &[]);
        let after: Vec<(String, String)> =
            cmd.iter_full_env_as_str().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(before, after);

        apply_pty(&mut cmd, &[("LD_LIBRARY_PATH".into(), None), ("PATH".into(), Some("/usr/bin:/bin".into()))]);
        assert_eq!(cmd.get_env("LD_LIBRARY_PATH"), None);
        assert_eq!(cmd.get_env("PATH").and_then(|v| v.to_str()), Some("/usr/bin:/bin"));
        assert_eq!(cmd.get_env("KEEP").and_then(|v| v.to_str()), Some("/home/u"));
    }
    use std::time::Duration;

    /// The OS read itself, on the one process whose cwd the test already knows.
    #[test]
    fn proc_cwd_reads_a_live_process_directory() {
        let got = proc_cwd(std::process::id()).expect("own cwd");
        // /var vs /private/var on macOS — compare resolved paths.
        assert_eq!(
            std::fs::canonicalize(got).unwrap(),
            std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap()
        );
    }

    fn cwds(entries: &[(&str, u32, &str)]) -> CwdMap {
        let mut m = CwdMap::new();
        for (place, idx, path) in entries {
            m.entry(place.to_string()).or_default().insert(*idx, path.to_string());
        }
        m
    }

    fn key(repo: &str, slug: &str, index: u32) -> ShellKey {
        (repo.to_string(), slug.to_string(), index)
    }

    #[test]
    fn a_remembered_directory_wins_over_the_place_root() {
        let here = std::env::current_dir().unwrap();
        let here = here.to_str().unwrap();
        assert_eq!(pick_start_dir(Some(here), "/place/root"), here);
    }

    /// A worktree that was removed, a directory that was renamed: the memory is
    /// stale, and a shell must still open somewhere real.
    #[test]
    fn a_vanished_directory_falls_back_to_the_place_root() {
        assert_eq!(pick_start_dir(Some("/no/such/dir/anywhere"), "/place/root"), "/place/root");
        assert_eq!(pick_start_dir(None, "/place/root"), "/place/root");
        // a FILE is not somewhere a shell can start either
        let f = std::env::current_dir().unwrap().join("Cargo.toml");
        assert_eq!(pick_start_dir(Some(f.to_str().unwrap()), "/place/root"), "/place/root");
    }

    /// The mechanism this whole feature rests on: a REAL shell on a REAL pty,
    /// told to `cd`, and the directory read back out of the OS by pid. Nothing
    /// mocked — the unit tests above only ever read this process's own cwd,
    /// which would pass just as happily if `proc_cwd` could not follow a child.
    #[test]
    fn proc_cwd_follows_a_live_shell_into_a_new_directory() {
        // Unique per run: two concurrent `cargo test` invocations must not share
        // these, and the test removes them at the end.
        let base = std::env::temp_dir().join(format!("wt-cwd-shell-{}", std::process::id()));
        let start = base.join("start");
        let moved = base.join("moved");
        std::fs::create_dir_all(&start).unwrap();
        std::fs::create_dir_all(&moved).unwrap();
        let real = |p: &Path| std::fs::canonicalize(p).unwrap();

        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        // /bin/sh, not $SHELL -l: the login shell's rc files are the app's
        // concern, not this mechanism's, and they make the test environment-
        // dependent for nothing.
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.cwd(&start);
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        drain(&*pair.master);
        let pid = live_pid(&mut *child).expect("pid of a running shell");

        assert_eq!(real(&proc_cwd(pid).expect("cwd at spawn")), real(&start));

        let mut w = pair.master.take_writer().unwrap();
        write!(w, "cd {}\n", moved.display()).unwrap();
        w.flush().unwrap();

        // The shell needs a moment to read the line; poll rather than sleep a
        // guessed amount.
        // resolved BEFORE the cleanup below — canonicalize needs the directory
        // to still exist, so computing it after the rmdir only ever panics
        let want = real(&moved);
        let mut got = None;
        for _ in 0..100 {
            let now = proc_cwd(pid).map(|p| real(&p));
            if now.as_deref() == Some(want.as_path()) {
                got = now;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        hard_kill(&mut *child);
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(got, Some(want), "shell cd was not visible through proc_cwd");
    }

    /// The reaped-pid guard, which is the difference between sampling this tab
    /// and sampling whatever process the OS later hands that pid to. A shell
    /// that exits is deliberately KEPT in the registry (the tab survives and
    /// offers a restart) and `list_shell_sessions` reaps it on the next dock
    /// mount — after which `process_id()` still answers, and answers wrongly.
    #[test]
    fn a_reaped_shell_reports_no_pid_to_sample() {
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut child = pair.slave.spawn_command(CommandBuilder::new("/bin/sh")).unwrap();
        drop(pair.slave);
        drain(&*pair.master);

        let pid = live_pid(&mut *child).expect("running shell has a pid");
        assert!(proc_cwd(pid).is_some(), "a running shell must have a readable cwd");

        hard_kill(&mut *child); // the reap — try_wait in list_shell_sessions does the same
        assert!(child.process_id().is_some(), "portable-pty still hands back the dangling pid");
        assert_eq!(live_pid(&mut *child), None, "a reaped shell must never be sampled");
    }

    /// …and the same decision on a real PTY, through the method `shell_resize`
    /// and `shell_open`'s re-attach both call. A shell that traps SIGWINCH says
    /// whether the ioctl actually happened.
    ///
    /// Note what this can and cannot catch. macOS (and Linux) compare the new
    /// winsize against the old one inside TIOCSWINSZ and signal only on a
    /// change, so a suppression bug does NOT show up here as a spurious W — the
    /// kernel hides it. What it does catch is the opposite and worse failure:
    /// suppression that swallows a size change, leaving the shell laying its
    /// prompt out for a terminal that is no longer that shape.
    #[test]
    fn a_real_size_change_still_reaches_the_shell() {
        let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).unwrap();
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        // `R` once the handler is installed: a resize sent before that is simply
        // lost, and the test would blame the code for the race.
        cmd.arg("trap 'printf W' WINCH; printf R; while :; do sleep 0.05; done");
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);

        // Drains the master AND is the assertion's eyes — production always has
        // exactly one such reader (see `drain`).
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mine = seen.clone();
        let mut reader = pair.master.try_clone_reader().unwrap();
        thread::spawn(move || {
            let mut buf = [0u8; 256];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => mine.lock().unwrap().extend_from_slice(&buf[..n]),
                }
            }
        });
        let writer = pair.master.take_writer().unwrap();
        let mut sh = Shell {
            master: pair.master,
            writer,
            child,
            stop: Arc::new(AtomicBool::new(false)),
            ring: Arc::new(Mutex::new(VecDeque::new())),
            sink: Arc::new(Mutex::new(None)),
            gen: 1,
            size,
            dirty: Arc::new(AtomicBool::new(false)),
        };
        // Poll rather than sleep a guessed amount; bounded so a shell that never
        // answers fails the test instead of hanging it.
        let count_of = |c: u8| seen.lock().unwrap().iter().filter(|b| **b == c).count();
        let winches = || count_of(b'W');
        let wait = |f: &dyn Fn() -> usize, n: usize| {
            for _ in 0..60 {
                if f() >= n {
                    return true;
                }
                thread::sleep(Duration::from_millis(50));
            }
            false
        };
        let wait_for = |n: usize| wait(&winches, n);

        assert!(wait(&|| count_of(b'R'), 1), "the trap shell never came up");
        assert!(sh.resize(100, 30).is_ok());
        assert!(wait_for(1), "a genuine resize must reach the shell as SIGWINCH");
        assert!(sh.resize(120, 40).is_ok());
        assert!(wait_for(2), "and so must the next one — the recorded size has to move with it");
        assert_eq!((sh.size.cols, sh.size.rows), (120, 40));

        // The repeat: nothing more may arrive. (The kernel would swallow it even
        // without the guard — this pins the assumption, it does not test it.)
        assert!(sh.resize(120, 40).is_ok());
        thread::sleep(Duration::from_millis(300));
        assert_eq!(winches(), 2, "resizing to the size it already has must be silent");

        hard_kill(&mut *sh.child);
    }

    /// Round-trip through the real file, including the two rules a plain
    /// serialize test would miss: an emptied place leaves no husk behind, and a
    /// `false` return writes nothing at all.
    #[test]
    fn the_cwd_file_round_trips_and_prunes() {
        let dir = std::env::temp_dir().join(format!("wt-cwd-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shell-cwds.json");
        let _ = std::fs::remove_file(&file);

        edit_cwds_at(&file, |m| merge_cwds(m, vec![(key("/r", "feat", 1), "/tmp".into())]));
        assert_eq!(read_cwds_at(&file), cwds(&[("/r|feat", 1, "/tmp")]));

        // a no-op edit must not even create noise
        let before = std::fs::read(&file).unwrap();
        edit_cwds_at(&file, |m| merge_cwds(m, vec![(key("/r", "feat", 1), "/tmp".into())]));
        assert_eq!(std::fs::read(&file).unwrap(), before);

        // closing the last tab of a place drops the place, not an empty husk —
        // through the SAME predicate close_shell_session uses, not a copy of it
        edit_cwds_at(&file, |m| forget_tab(m, "/r", "feat", 1));
        assert_eq!(read_cwds_at(&file), CwdMap::new());
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), "{}");

        // a corrupt file reads as "no memory", never as a failure
        std::fs::write(&file, b"{not json").unwrap();
        assert_eq!(read_cwds_at(&file), CwdMap::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `shell_open`'s actual lookup — `remembered_dir` composed with
    /// `pick_start_dir` — over a map that came off disk. The pieces were each
    /// covered; the composition, which is the only thing the feature does for
    /// the user, was not.
    #[test]
    fn a_reopened_tab_starts_in_the_directory_the_file_remembers() {
        let dir = std::env::temp_dir().join(format!("wt-cwd-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shell-cwds.json");
        let _ = std::fs::remove_file(&file);
        let here = std::env::current_dir().unwrap();
        let here = here.to_str().unwrap().to_string();
        let place = "/place/root";

        // tab 1 was left in a real directory, tab 2 in one since deleted
        edit_cwds_at(&file, |m| {
            merge_cwds(
                m,
                vec![
                    (key("/r", "feat", 1), here.clone()),
                    (key("/r", "feat", 2), "/gone/for/good".into()),
                ],
            )
        });
        let map = read_cwds_at(&file);

        let open = |index: u32| pick_start_dir(remembered_dir(&map, "/r", "feat", index).as_deref(), place);
        assert_eq!(open(1), here, "tab 1 must reopen where it was");
        assert_eq!(open(2), place, "a vanished directory falls back to the place root");
        assert_eq!(open(3), place, "a tab with no memory opens at the place root");
        // a DIFFERENT place must not see this one's memory
        assert_eq!(
            pick_start_dir(remembered_dir(&map, "/r", "other", 1).as_deref(), place),
            place
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round-trip through the real files, including the two rules a plain
    /// serialize test would miss: the width travels with the bytes, and a tab
    /// that has never been saved reads as nothing rather than as an error.
    #[test]
    fn a_scrollback_round_trips_with_the_width_it_was_written_at() {
        let root = hist_root("round");
        let k = key("/r", "feat", 3);

        assert!(read_scrollback(&root, &k).is_none(), "an unsaved tab has no history");

        write_scrollback(&root, &k, b"line one\nbuild output\n", 174);
        let (bytes, meta) = read_scrollback(&root, &k).expect("saved tab reads back");
        assert_eq!(bytes, b"line one\nbuild output\n", "a ring that never rolled keeps its first line");
        assert_eq!(meta.cols, 174, "the width travels with the bytes");
        assert_eq!((meta.repo.as_str(), meta.slug.as_str(), meta.index), ("/r", "feat", 3));

        // closing the tab takes everything with it
        forget_tab_history(&root, &k);
        assert!(read_scrollback(&root, &k).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The two halves of `rolled_ring`, and the reason it is not just
    /// `trim_to_line_start`.
    ///
    /// A ring AT the cap begins wherever `drain` cut — routinely mid-escape-
    /// sequence — so its first fragment goes. A ring BELOW the cap begins where
    /// the shell began, and trimming that drops a real line. The second case is
    /// the one that bites, because it COMPOUNDS: the restore seeds from the
    /// trimmed copy, so the next save trims the next line, one per restart.
    #[test]
    fn only_a_ring_that_actually_rolled_loses_its_first_line() {
        let short = b"line one\nline two\n";
        assert_eq!(rolled_ring(short), short, "a ring below the cap is saved whole");

        // at the cap, and cut mid-line the way `drain` leaves it
        let mut full = b"tail-of-a-cut-line".to_vec();
        full.extend(std::iter::repeat(b'x').take(SHELL_RING));
        full.insert(18, b'\n');
        let kept = rolled_ring(&full);
        assert!(kept.len() < full.len(), "a rolled ring drops its leading fragment");
        assert_eq!(kept[0], b'x', "…and resumes at the start of a line");

        // the compounding case, stated directly: saving what a restore seeded
        // must be a fixed point, not another line gone
        let seeded = rolled_ring(&full).to_vec();
        assert_eq!(rolled_ring(&seeded), seeded.as_slice(), "re-saving a restored ring loses nothing");
    }

    /// The directory name is a hash, so it is only a NAME. `meta.json` is the
    /// identity — without that check a collision would hand one tab another
    /// tab's output, which is the one failure worse than opening blank.
    #[test]
    fn a_meta_that_names_another_tab_reads_as_nothing() {
        let root = hist_root("ident");
        let mine = key("/r", "feat", 1);
        write_scrollback(&root, &mine, b"\nmine\n", 80);

        // forge the collision: same directory, someone else's meta
        let tab = tab_dir(&root, &mine);
        let theirs = TabMeta {
            repo: "/other".into(),
            slug: "theirs".into(),
            index: 9,
            cols: 80,
            at: sysclock::now_epoch(),
        };
        std::fs::write(tab.join("meta.json"), serde_json::to_vec(&theirs).unwrap()).unwrap();
        assert!(read_scrollback(&root, &mine).is_none(), "another tab's meta is not mine");

        // and a corrupt one is simply no history, never an error
        std::fs::write(tab.join("meta.json"), b"{not json").unwrap();
        assert!(read_scrollback(&root, &mine).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The restore payload's contracts: the seam comes AFTER the content, an
    /// empty save sends nothing at all (which keeps `replay` at 0 so the pane
    /// treats the new shell's own first output as live), and the seam carries an
    /// absolute time because it is persisted with the ring.
    #[test]
    fn a_seam_follows_the_content_and_an_empty_save_sends_nothing() {
        let at = 1_700_000_000;
        assert!(restore_payload(b"", at).is_empty(), "nothing saved, nothing replayed");

        let out = restore_payload(b"the build log\n", at);
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("the build log\n"), "the content comes first: {text:?}");
        assert!(
            text.find("the build log").unwrap() < text.find("restored").unwrap(),
            "the seam marks the END of what was restored"
        );
        // absolute, not an age: this is seeded into the ring and persisted with
        // it, so "2h ago" would freeze and be wrong forever after
        assert!(text.contains(&fmt_local(at)), "the seam names when it was saved: {text:?}");
        // cancels a control sequence the ring's tail was cut inside, so the seam
        // cannot be swallowed as somebody's argument
        assert!(out.starts_with(SEAM_HEAD) || out.windows(SEAM_HEAD.len()).any(|w| w == SEAM_HEAD));
    }

    /// Three launches with nothing typed between them must not stack three
    /// seams — but a seam with a session's work under it still marks a real
    /// boundary and stays.
    #[test]
    fn a_seam_nothing_was_typed_under_is_replaced_not_followed() {
        let (t1, t2) = (1_700_000_000, 1_700_009_000);
        let once = restore_payload(b"the build log\n", t1);
        let twice = restore_payload(&once, t2);
        let text = String::from_utf8_lossy(&twice);
        assert_eq!(text.matches("── restored · ").count(), 1, "only the newest seam survives: {text:?}");
        assert!(text.contains(&fmt_local(t2)) && !text.contains(&fmt_local(t1)));

        // …but work done after a seam keeps it
        let mut worked = once.clone();
        worked.extend_from_slice(b"$ cargo test\nall good\n");
        let after = restore_payload(&worked, t2);
        let text = String::from_utf8_lossy(&after);
        assert_eq!(text.matches("── restored · ").count(), 2, "a boundary with work under it stays: {text:?}");
    }

    /// Which lever each shell gets. zsh cannot use `HISTFILE` — macOS's
    /// `/etc/zshrc` overwrites it unconditionally — and nothing else can use
    /// ZDOTDIR.
    #[test]
    fn each_shell_gets_the_only_lever_that_works_for_it() {
        let root = hist_root("lever");
        let k = key("/r", "feat", 1);
        let zdot = tab_dir(&root, &k).join("zdotdir");

        let mut zsh = CommandBuilder::new("/bin/zsh");
        assert_eq!(apply_tab_history_env(&mut zsh, &root, &k, "/bin/zsh"), Some("zdotdir"));
        assert_eq!(zsh.get_env("ZDOTDIR"), Some(zdot.as_os_str()));
        assert_eq!(zsh.get_env("WORKTREES_HISTFILE"), Some(zdot.join(".zsh_history").as_os_str()));
        assert!(zsh.get_env("HISTFILE").is_none(), "zsh's HISTFILE is /etc/zshrc's to set");

        let mut bash = CommandBuilder::new("/bin/bash");
        assert_eq!(apply_tab_history_env(&mut bash, &root, &k, "/bin/bash"), Some("histfile"));
        assert!(bash.get_env("ZDOTDIR").is_none(), "ZDOTDIR means nothing to bash");
        assert_eq!(
            bash.get_env("HISTFILE"),
            Some(tab_dir(&root, &k).join(".bash_history").as_os_str())
        );

        let mut fish = CommandBuilder::new("/opt/homebrew/bin/fish");
        assert_eq!(apply_tab_history_env(&mut fish, &root, &k, "/opt/homebrew/bin/fish"), None);
        assert!(fish.get_env("ZDOTDIR").is_none() && fish.get_env("HISTFILE").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The one that proves the MECHANISM rather than the strings: a real login
    /// zsh, with a real `/etc/zshrc` in the way, writing its history where this
    /// feature says it will.
    ///
    /// The string assertions above would pass just as happily if zsh ignored the
    /// shims entirely. This is the sibling of
    /// `proc_cwd_follows_a_live_shell_into_a_new_directory`, and it carries the
    /// same two rules: DRAIN the master (a pty test that does not wedges the
    /// child mid-exit, unkillable) and SIGKILL rather than trusting a signal.
    #[test]
    fn a_real_login_zsh_records_into_the_generated_zdotdir() {
        if !Path::new("/bin/zsh").exists() {
            return; // nothing to prove on a box without zsh
        }
        let root = hist_root("realzsh");
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        // A user rc with something only IT can provide, so "sourced" and
        // "replaced" cannot look the same.
        std::fs::write(home.join(".zshrc"), "export WT_TEST_MARKER=from-the-users-rc\n").unwrap();

        let k = key("/r", "feat", 1);
        let zdot = tab_dir(&root, &k).join("zdotdir");
        let mut cmd = CommandBuilder::new("/bin/zsh");
        cmd.arg("-l");
        cmd.cwd(&root);
        cmd.env("HOME", &home);
        cmd.env("TERM", "xterm-256color");
        assert_eq!(apply_tab_history_env(&mut cmd, &root, &k, "/bin/zsh"), Some("zdotdir"));

        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        drain(&*pair.master);

        // Answer into a FILE rather than parsing the pty: prompts, escape
        // sequences and echo make the stream a poor witness.
        let out = root.join("answers");
        let mut w = pair.master.take_writer().unwrap();
        write!(
            w,
            ": wt-history-probe\n\
             print -r -- \"HISTFILE=$HISTFILE\" > {out}\n\
             print -r -- \"MARKER=$WT_TEST_MARKER\" >> {out}\n\
             print -r -- \"ZDOTDIR=${{ZDOTDIR-unset}}\" >> {out}\n",
            out = out.display()
        )
        .unwrap();
        w.flush().unwrap();

        let hist = zdot.join(".zsh_history");
        let mut answers = String::new();
        for _ in 0..200 {
            if let Ok(s) = std::fs::read_to_string(&out) {
                if s.lines().count() >= 3 {
                    answers = s;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
        // INC_APPEND_HISTORY writes per command, so the probe is already there —
        // this shell is never asked to exit cleanly, which is the point.
        let recorded = std::fs::read_to_string(&hist).unwrap_or_default();
        hard_kill(&mut *child);
        let _ = std::fs::remove_dir_all(&root);

        assert!(!answers.is_empty(), "the shell never answered");
        assert!(
            answers.contains(&format!("HISTFILE={}", hist.display())),
            "HISTFILE is not the tab's: {answers}"
        );
        assert!(
            answers.contains("MARKER=from-the-users-rc"),
            "the user's own .zshrc was not sourced: {answers}"
        );
        assert!(
            answers.contains("ZDOTDIR=unset"),
            "ZDOTDIR must be handed back, so child shells are unaffected: {answers}"
        );
        assert!(
            recorded.contains("wt-history-probe"),
            "a killed shell still recorded its commands (INC_APPEND_HISTORY): {recorded:?}"
        );
    }

    /// The wheel's routing table, and the one invocation each row runs. The
    /// argv is what the throwaway-server probe in the PR was run against.
    #[test]
    fn wheel_scrolls_history_only_where_there_is_history() {
        // pi / a shell: main screen, not scrolled back
        assert_eq!(wheel_plan("", false, true), Wheel::EnterAndScroll);
        assert_eq!(wheel_plan("", false, false), Wheel::Nothing);
        // already reading history: both directions move through it
        assert_eq!(wheel_plan("copy-mode", false, true), Wheel::Scroll);
        assert_eq!(wheel_plan("copy-mode", false, false), Wheel::Scroll);
        assert_eq!(wheel_plan("copy-mode", true, false), Wheel::Scroll);
        assert_eq!(wheel_plan("view-mode", false, true), Wheel::Scroll);
        // alternate screen, no mouse: arrows, as xterm always sent
        assert_eq!(wheel_plan("", true, true), Wheel::Keys);
        assert_eq!(wheel_plan("", true, false), Wheel::Keys);
        // a MENU mode (C-b w / C-b s): ↑/↓ moves its selection; scroll-up
        // would fail "not in a mode" there
        assert_eq!(wheel_plan("tree-mode", false, true), Wheel::Keys);
        assert_eq!(wheel_plan("buffer-mode", true, false), Wheel::Keys);

        let a = |plan, lines| wheel_args("%7", &plan, lines).map(|v| v.join(" "));
        assert_eq!(a(Wheel::EnterAndScroll, -3).as_deref(), Some("copy-mode -e -t %7 ; send-keys -X -N 3 -t %7 scroll-up"));
        assert_eq!(a(Wheel::Scroll, 5).as_deref(), Some("send-keys -X -N 5 -t %7 scroll-down"));
        assert_eq!(a(Wheel::Keys, -2).as_deref(), Some("send-keys -N 2 -t %7 Up"));
        assert_eq!(a(Wheel::Keys, 1).as_deref(), Some("send-keys -N 1 -t %7 Down"));
        assert_eq!(a(Wheel::Nothing, 4), None);
    }

    /// Attaching to a pane that is already reading history must arm the
    /// cancel, or the first keys typed are copy-mode commands.
    #[test]
    fn attach_to_a_scrolled_back_pane_arms_the_cancel() {
        assert_eq!(scrolled_at_attach("%12 copy-mode\n").as_deref(), Some("%12"));
        assert_eq!(scrolled_at_attach("%12 view-mode\n").as_deref(), Some("%12"));
        assert_eq!(scrolled_at_attach("%12 \n"), None, "live pane");
        assert_eq!(scrolled_at_attach("%12\n"), None, "live pane, field trimmed");
        assert_eq!(scrolled_at_attach("%12 tree-mode\n"), None, "a menu, not history");
        assert_eq!(scrolled_at_attach(""), None, "no pane");
    }

    /// The merge exists so a tab that has NOT been re-spawned since the restart
    /// keeps its directory. Sampling only the live tab must leave the other one
    /// exactly as it was.
    #[test]
    fn merge_cwds_leaves_unsampled_tabs_alone() {
        let mut map = cwds(&[("/r|feat", 1, "/r/.worktrees/feat/app"), ("/r|feat", 2, "/r/.worktrees/feat/docs")]);
        let changed = merge_cwds(&mut map, vec![(key("/r", "feat", 1), "/r/.worktrees/feat/crates".into())]);
        assert!(changed);
        assert_eq!(map, cwds(&[("/r|feat", 1, "/r/.worktrees/feat/crates"), ("/r|feat", 2, "/r/.worktrees/feat/docs")]));
    }

    /// An idle shell samples the same path every 15s; that must not be a write.
    #[test]
    fn merge_cwds_reports_no_change_when_nothing_moved() {
        let mut map = cwds(&[("/r|feat", 1, "/r/.worktrees/feat/app")]);
        assert!(!merge_cwds(&mut map, vec![(key("/r", "feat", 1), "/r/.worktrees/feat/app".into())]));
    }

    /// The decision, and the bookkeeping that goes with it. A pane re-attaching
    /// asks to be the size it already is on every tab flip, and the
    /// ResizeObserver's first fire repeats whatever the attach just said — so
    /// "already that size" is the common case, not the corner one. Forgetting to
    /// record what WAS applied is the subtler half: every later request would
    /// then be compared against the size the pty had two resizes ago, and a
    /// repeat would sail through as a change.
    #[test]
    fn a_resize_to_the_size_the_pty_already_has_is_dropped() {
        let mut cur = PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 };
        let applied: Vec<Option<(u16, u16)>> = [(120, 40), (80, 24), (80, 24), (120, 40), (120, 40)]
            .iter()
            .map(|(c, r)| next_pty_size(&mut cur, *c, *r).map(|s| (s.cols, s.rows)))
            .collect();
        assert_eq!(
            applied,
            vec![None, Some((80, 24)), None, Some((120, 40)), None],
            "only the requests that MOVE the pty may reach it"
        );
        assert_eq!((cur.cols, cur.rows), (120, 40), "the record follows what was applied");
    }

    /// The startup sweep's decision. A place removed with `worktrees rm` from a
    /// terminal leaves its entry behind — nothing tells the app — so this is
    /// what stops a recreated slug inheriting the previous life's directories.
    #[test]
    fn only_places_that_no_longer_exist_are_forgotten() {
        let here = std::env::current_dir().unwrap().to_str().unwrap().to_string();
        let keys: Vec<String> = ["/r|alive", "/r|removed", "/dead-repo|any", "no-separator"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let gone = vanished_keys(&keys, |repo, slug| match (repo, slug) {
            ("/r", "alive") => Some(here.clone()),
            ("/r", "removed") => Some("/gone/for/good".into()),
            _ => None, // the project itself is unreachable
        });
        assert_eq!(gone, vec!["/r|removed", "/dead-repo|any", "no-separator"]);
    }

    /// A repo path is absolute and may itself contain the separator; the slug
    /// never does, so the split has to come from the right.
    #[test]
    fn a_repo_path_containing_the_separator_still_splits_at_the_slug() {
        let seen = std::cell::RefCell::new(Vec::new());
        let keys = vec!["/odd|repo|feat".to_string()];
        vanished_keys(&keys, |repo, slug| {
            seen.borrow_mut().push((repo.to_string(), slug.to_string()));
            None
        });
        assert_eq!(seen.into_inner(), vec![("/odd|repo".to_string(), "feat".to_string())]);
    }

    /// A repo path is absolute and may contain anything; two tabs must never
    /// land in the same directory, and the same tab must always land in its own.
    #[test]
    fn every_tab_gets_its_own_stem_and_keeps_it() {
        let a = tab_stem(&key("/odd|repo", "feat", 1));
        assert_eq!(a, tab_stem(&key("/odd|repo", "feat", 1)), "a stem is stable");
        let others = [
            key("/odd|repo", "feat", 2),        // same place, next tab
            key("/odd", "repo|feat", 1),        // the separator moved
            key("/other", "feat", 1),           // another project
        ];
        for o in &others {
            assert_ne!(a, tab_stem(o), "{o:?} collided with /odd|repo|feat|1");
        }
        // a slug that is all path separators still names a file, not a path
        let s = tab_stem(&key("/r", "feat/../../etc", 1));
        assert!(!s.contains('/'), "a stem must never contain a separator: {s}");
    }

    /// The seam is written for a person sitting in front of the terminal, so it
    /// is the machine's own clock — unlike app.log, which is UTC.
    #[test]
    fn the_seam_time_is_local_not_utc() {
        let at = 1_700_000_000;
        let local = fmt_local(at);
        assert_eq!(local.len(), fmt_utc(at).len(), "same shape, different offset");
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let t = at as libc::time_t;
        unsafe { libc::localtime_r(&t, &mut tm) };
        assert_eq!(local, fmt_utc(at + tm.tm_gmtoff as i64));
    }

    /// The cold-start sweep's decision, the way `only_places_that_no_longer_exist_are_forgotten`
    /// covers the cwd map's. Two reasons to drop a tab and one to keep it.
    #[test]
    fn a_tab_is_swept_when_its_place_is_gone_or_it_has_gone_stale() {
        let now = 2_000_000_000;
        let fresh = now - 60;
        let old = now - TERM_HIST_MAX_AGE_SECS - 1;
        let m = |repo: &str, slug: &str, at: i64| TabMeta {
            repo: repo.into(),
            slug: slug.into(),
            index: 1,
            cols: 80,
            at,
        };
        let metas = vec![
            (PathBuf::from("/h/alive"), m("/r", "alive", fresh)),
            (PathBuf::from("/h/removed"), m("/r", "removed", fresh)),
            (PathBuf::from("/h/stale"), m("/r", "alive", old)),
            (PathBuf::from("/h/dead-repo"), m("/dead", "any", fresh)),
        ];
        let here = std::env::current_dir().unwrap().to_str().unwrap().to_string();
        let gone = stale_history(&metas, now, |repo, slug| match (repo, slug) {
            ("/r", "alive") => Some(here.clone()),
            ("/r", "removed") => Some("/gone/for/good".into()),
            _ => None, // the project itself is unreachable
        });
        let gone: Vec<&str> = gone.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(gone, vec!["/h/removed", "/h/stale", "/h/dead-repo"]);
    }

    /// The generated shims, read as text. They are the whole per-tab history
    /// mechanism, and every clause below is load-bearing — see the section
    /// comment for which real configuration each one exists for.
    #[test]
    fn the_zdotdir_shims_hand_the_users_own_config_back_before_taking_the_history() {
        let root = hist_root("zdot");
        let zdot = root.join("zdotdir");
        write_zdotdir(&zdot).unwrap();

        let read = |n: &str| std::fs::read_to_string(zdot.join(n)).unwrap();
        for n in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
            assert!(read(n).starts_with("# Generated by worktrees"), "{n} is unlabelled");
        }

        // .zshenv: their environment while theirs runs, ours back afterwards —
        // otherwise a `~/.zshenv` that exports ZDOTDIR hijacks the whole chain
        let env = read(".zshenv");
        assert!(env.contains("unset ZDOTDIR"), "their .zshenv must see a normal environment");
        assert!(env.contains(r#". "$HOME/.zshenv""#), "their .zshenv is sourced, not replaced");
        assert!(
            env.find("unset ZDOTDIR").unwrap() < env.find("$HOME/.zshenv").unwrap(),
            "ZDOTDIR must be unset BEFORE theirs is sourced"
        );
        assert!(env.contains("export ZDOTDIR=$_WT_Z"), "and taken back after");

        // .zshrc: their ZDOTDIR restored before their rc (which may reference
        // it), our HISTFILE after it (so a user rc that sets HISTFILE loses)
        let rc = read(".zshrc");
        let restore = rc.find("unset ZDOTDIR").expect("restores their ZDOTDIR");
        let source = rc.find("/.zshrc\"").expect("sources their rc");
        let hist = rc.find("HISTFILE=\"$WORKTREES_HISTFILE\"").expect("sets our HISTFILE");
        assert!(restore < source, "their rc must be able to read $ZDOTDIR");
        assert!(source < hist, "our HISTFILE must be set AFTER theirs, or theirs wins");
        // without this a force-quit takes the whole session's commands with it:
        // the app SIGHUPs its shells, and zsh's default flushes only on a clean exit
        assert!(rc.contains("setopt INC_APPEND_HISTORY"));
        assert!(!rc.contains("SHARE_HISTORY"), "each tab has its own file; nothing to share");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Read the master to EOF and throw it away. Production always has one of
    /// these (`shell_open` spawns a reader thread); a pty test that skips it
    /// deadlocks in a way the app never can — the shell's output fills the pty
    /// buffer, and the child then WEDGES MID-EXIT (`ps` shows state `E`, never
    /// reaped) so even SIGKILL + `wait` hangs forever.
    fn drain(master: &(dyn MasterPty + Send)) {
        let mut reader = master.try_clone_reader().unwrap();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
        });
    }

    /// SIGKILL + reap, for tests that must not depend on how a shell reacts to
    /// a signal. `Child::kill` in portable-pty sends SIGHUP (lib.rs:347 of that
    /// crate), and an interactive `/bin/sh` on a pty whose master is still open
    /// survives it. The app gets away with SIGHUP because dropping the `Shell`
    /// closes the master and the EOF finishes the job; a test holding the master
    /// open would wait forever.
    fn hard_kill(child: &mut (dyn Child + Send + Sync)) {
        if let Some(pid) = child.process_id() {
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
        let _ = child.wait();
    }

    /// A scratch `term-history` root, unique per run: two concurrent `cargo
    /// test` invocations must not share one.
    fn hist_root(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-hist-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A sink for tests about what happens before a byte is sent.
    struct Nowhere;
    impl ByteSink for Nowhere {
        fn send(&self, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
    }

    /// A fresh `git init` with one place under `.worktrees/` beside `(main)`:
    /// a directory there is all `place_index` reads, no registration needed.
    /// Returns (scratch root, repo).
    fn repo_with_a_lane(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("wt-shellslug-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join(".worktrees/lane")).unwrap();
        assert!(worktrees_core::git::git_ok(&repo.to_string_lossy(), &["init", "-q"]), "git init");
        (base, repo)
    }

    /// `shell_open` starts a login shell in `<wt_root>/<slug>`, and the slug is
    /// a client's string. It is held to the rule the other place commands
    /// share — core's `safe_arg`, then "is it a place" — and refused before
    /// anything spawns: `--all` and a slug naming nothing each got a shell.
    #[test]
    fn shell_open_refuses_a_slug_that_is_not_a_place_before_anything_spawns() {
        let (base, repo) = repo_with_a_lane("refuse");
        let ctx = Ctx::new(base.join("cfg"), Arc::new(crate::tests::Recorder::default()));
        let shells = Shells::default();
        let repo_s = repo.to_string_lossy().into_owned();
        for (slug, why) in [
            ("--all", "may not begin with '-'"),
            ("-x", "may not begin with '-'"),
            ("nope", "no such place: nope"),
            ("lane/sub", "no such place: lane/sub"),
            ("lane/../..", "not allowed"),
            ("", "slug is required"),
        ] {
            let r = shell_open(&ctx, repo_s.clone(), slug.into(), 0, 80, 24, Arc::new(Nowhere), &shells);
            let e = r.err().unwrap_or_else(|| panic!("`{slug}` was given a shell"));
            assert!(e.contains(why), "`{slug}`: {e}");
        }
        assert!(shells.0.lock().unwrap().is_empty(), "a refused slug left a shell behind");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The other half: a place of the repo still resolves, `(main)` to the
    /// checkout and a `.worktrees/` directory to itself. (The serve socket
    /// tests open `(main)` shells end to end.)
    #[test]
    fn a_place_of_the_repo_still_resolves_to_its_session_and_directory() {
        let (base, repo) = repo_with_a_lane("resolve");
        let repo_s = repo.to_string_lossy().into_owned();
        let (_, main_cwd) = place_session_cwd(&repo_s, "(main)").unwrap();
        assert!(main_cwd.ends_with("/repo"), "{main_cwd}");
        let (session, cwd) = place_session_cwd(&repo_s, "lane").unwrap();
        assert!(cwd.ends_with("/repo/.worktrees/lane"), "{cwd}");
        assert!(session.ends_with("-lane"), "{session}");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── a second engine on the same files (rehearsal D7) ─────────────────────
    // An app crash left its serve running with no client; a downgraded app
    // then ran its own engine on the same config dir, saved its dock tab on a
    // clean quit, and the orphan's next save put its own ring and directory
    // back over both.

    /// A dock shell for the save tests: a real process sitting in `cwd`, so
    /// `proc_cwd` has something to read, with `ring` already in it and dirty.
    /// `sleep` rather than a shell: SIGHUP ends it, which is what `kill_shell`
    /// sends, and it writes nothing, so the pty needs no reader.
    fn parked_shell(cwd: &Path, ring: &[u8]) -> Shell {
        let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).unwrap();
        let mut cmd = CommandBuilder::new("/bin/sleep");
        cmd.arg("60"); // bounded, should a failed assertion skip the kill
        cmd.cwd(cwd);
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let writer = pair.master.take_writer().unwrap();
        Shell {
            master: pair.master,
            writer,
            child,
            stop: Arc::new(AtomicBool::new(false)),
            ring: Arc::new(Mutex::new(ring.iter().copied().collect())),
            sink: Arc::new(Mutex::new(None)),
            gen: 1,
            size,
            dirty: Arc::new(AtomicBool::new(true)),
        }
    }

    /// An engine whose tab `/r|feat#1` was saved by a previous run and READ by
    /// this one's spawn, as `shell_open` reads it, with a live shell for it
    /// sitting in a directory of its own. (ctx, scratch, ring file, cwd file,
    /// the shell's directory as `proc_cwd` will report it.)
    fn engine_with_a_saved_tab(tag: &str) -> (Ctx, PathBuf, PathBuf, PathBuf, String) {
        let base = hist_root(tag);
        let ctx = Ctx::new(base.join("cfg"), Arc::new(crate::tests::Recorder::default()));
        let k = key("/r", "feat", 1);
        let root = term_hist_dir(&ctx).unwrap();
        let ring = tab_dir(&root, &k).join("scrollback.bin");
        let cwd_path = cwd_file(&ctx).unwrap();
        // The previous run was another process: its files are written raw, so
        // that what this engine knows of them is only what its spawn READ.
        std::fs::create_dir_all(tab_dir(&root, &k)).unwrap();
        hist_write_atomic(&ring, b"BEFORE\n").unwrap();
        write_meta(&tab_dir(&root, &k), &k, 80);
        std::fs::write(&cwd_path, serde_json::to_vec(&cwds(&[("/r|feat", 1, "/before")])).unwrap()).unwrap();
        let _ = read_cwds(&ctx);
        let _ = read_scrollback(&root, &k);
        let here = base.join("where-the-shell-is");
        std::fs::create_dir_all(&here).unwrap();
        let here = std::fs::canonicalize(&here).unwrap().to_string_lossy().into_owned();
        ctx.shells.0.lock().unwrap().insert(k, parked_shell(Path::new(&here), b"ORPHAN-2\n"));
        (ctx, base, ring, cwd_path, here)
    }

    /// D7 itself, at the exit flush — the one every orphan now ends on, since
    /// an app-owned serve stops itself when no client comes back: another
    /// engine saved this tab after this one read it, and stopping must leave
    /// the other's ring and directory exactly as it wrote them.
    #[test]
    fn the_exit_flush_leaves_a_ring_and_a_directory_another_engine_saved() {
        let (ctx, base, ring, cwd_path, _) = engine_with_a_saved_tab("d7-other");
        // The other engine's save, its own way: an atomic ring, a plain write.
        hist_write_atomic(&ring, b"DOWNGRADE-2\n").unwrap();
        std::fs::write(&cwd_path, serde_json::to_vec(&cwds(&[("/r|feat", 1, "/other")])).unwrap()).unwrap();
        crate::host::stop(&ctx);
        assert_eq!(
            String::from_utf8_lossy(&std::fs::read(&ring).unwrap()),
            "DOWNGRADE-2\n",
            "the exit flush overwrote the ring another engine saved"
        );
        assert_eq!(read_cwds_at(&cwd_path), cwds(&[("/r|feat", 1, "/other")]), "…and its directory");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// And its saves follow each other: what one wrote is what the next one
    /// finds, so a tick after a tick still saves, and so does a tick after
    /// the user's own edit. If a write did not count as seen, the second tick
    /// would find a stranger's file and an engine would save each tab once.
    #[test]
    fn this_engines_saves_follow_its_own_saves() {
        let (ctx, base, ring, cwd_path, here) = engine_with_a_saved_tab("d7-again");
        let k = key("/r", "feat", 1);
        save_shell_scrollback(&ctx, &ctx.shells);
        save_shell_cwds(&ctx, &ctx.shells);
        {
            let map = ctx.shells.0.lock().unwrap();
            let sh = map.get(&k).unwrap();
            sh.ring.lock().unwrap().extend(b"MORE\n");
            sh.dirty.store(true, Ordering::Relaxed);
        }
        save_shell_scrollback(&ctx, &ctx.shells);
        assert_eq!(String::from_utf8_lossy(&std::fs::read(&ring).unwrap()), "ORPHAN-2\nMORE\n", "the second tick did not save");
        edit_cwds(&ctx, |m| forget_tab(m, "/r", "feat", 1));
        save_shell_cwds(&ctx, &ctx.shells);
        assert_eq!(read_cwds_at(&cwd_path), cwds(&[("/r|feat", 1, here.as_str())]), "the tick after an edit did not save");
        crate::host::stop(&ctx);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The rule on its own. Absent is always fine; otherwise only the bytes
    /// this engine last saw; a file it never looked at is not its to replace.
    #[test]
    fn an_unasked_save_writes_only_over_what_this_engine_saw() {
        let (a, b) = (seen_of(Some(b"mine")), seen_of(Some(b"theirs")));
        assert!(may_save(None, None), "nothing there, never looked");
        assert!(may_save(Some(a), None), "removed since (a clear, a closed tab)");
        assert!(may_save(Some(a), a), "still what this engine saw");
        assert!(may_save(Some(None), None));
        assert!(!may_save(Some(a), b), "somebody else saved it since");
        assert!(!may_save(Some(None), a), "somebody else made it since this engine saw nothing there");
        assert!(!may_save(None, a), "never looked, and something is there");
    }

    /// An orphan refused every 15 s for two minutes would say so every 15 s.
    #[test]
    fn a_refused_save_is_said_once_per_file() {
        let p = hist_root("said").join("scrollback.bin");
        assert!(first_refusal(&p));
        assert!(!first_refusal(&p));
        assert!(!first_refusal(&p));
        assert!(first_refusal(&p.with_file_name("shell-cwds.json")), "per file, not once in all");
    }

    /// The other direction, so the guard cannot be a lid: with nobody else
    /// writing, the exit flush saves what this engine's own shell did, over
    /// the previous run's save it read — and a tab nothing was ever saved for.
    #[test]
    fn the_exit_flush_still_saves_this_engines_own_tabs() {
        let (ctx, base, ring, cwd_path, here) = engine_with_a_saved_tab("d7-own");
        let fresh = key("/r", "feat", 2);
        let fresh_ring = tab_dir(&term_hist_dir(&ctx).unwrap(), &fresh).join("scrollback.bin");
        let _ = read_scrollback(&term_hist_dir(&ctx).unwrap(), &fresh);
        ctx.shells.0.lock().unwrap().insert(fresh, parked_shell(Path::new(&here), b"FRESH\n"));
        crate::host::stop(&ctx);
        assert_eq!(String::from_utf8_lossy(&std::fs::read(&ring).unwrap()), "ORPHAN-2\n");
        assert_eq!(String::from_utf8_lossy(&std::fs::read(&fresh_ring).unwrap()), "FRESH\n");
        assert_eq!(read_cwds_at(&cwd_path), cwds(&[("/r|feat", 1, here.as_str()), ("/r|feat", 2, here.as_str())]));
        let _ = std::fs::remove_dir_all(&base);
    }
}

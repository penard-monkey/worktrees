//! Thin wrappers over the `tmux` CLI. Subprocess (there's no native lib), which
//! also keeps the bats fake-tmux PATH shim intercepting the compiled binary.

use std::process::{Command, Output};

pub fn have_tmux() -> bool {
    Command::new("tmux").arg("-V").output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn tmux(args: &[&str]) -> std::io::Result<Output> {
    Command::new("tmux").args(args).output()
}

/// Does a session named EXACTLY `name` exist? (`list-sessions` + exact match, not
/// `has-session -t` which prefix-matches — so `rm api` can't hit `api-fix`.)
pub fn session_exists(name: &str) -> bool {
    match tmux(&["list-sessions", "-F", "#{session_name}"]) {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).lines().any(|l| l == name)
        }
        _ => false,
    }
}

/// Single-quote `s` for embedding in a shell `-c`/`-ic` string (bash `sq`).
pub fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A session already living in worktree dir `wt` (a pane cwd'd there), so `open`
/// reuses an AI pane running under any name. Prefers a pane whose command looks
/// like the configured AI CLI (`ai_word`) or `node`; else the first match.
pub fn worktree_session(wt: &str, ai_word: &str) -> Option<String> {
    if !have_tmux() {
        return None;
    }
    let o = tmux(&[
        "list-panes",
        "-a",
        "-F",
        "#{session_name}\t#{pane_current_path}\t#{pane_current_command}",
    ])
    .ok()?;
    if !o.status.success() {
        return None;
    }
    let mut best: Option<String> = None;
    let prefix = format!("{wt}/");
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        let mut it = line.splitn(3, '\t');
        let sess = it.next().unwrap_or("");
        let path = it.next().unwrap_or("");
        let cmd = it.next().unwrap_or("");
        if sess.is_empty() || !(path == wt || path.starts_with(&prefix)) {
            continue;
        }
        if cmd.contains(ai_word) || cmd == "node" {
            return Some(sess.to_string());
        }
        if best.is_none() {
            best = Some(sess.to_string());
        }
    }
    best
}

/// `new-session -d -s <session> -c <wt> -P -F '#{pane_id}' <pane0>` → pane id.
pub fn new_session(session: &str, wt: &str, pane0: &str) -> Option<String> {
    let o = tmux(&["new-session", "-d", "-s", session, "-c", wt, "-P", "-F", "#{pane_id}", pane0]).ok()?;
    if o.status.success() {
        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        None
    }
}

pub fn split_window(pane_id: &str, wt: &str, pane1: &str) {
    let _ = tmux(&["split-window", "-h", "-t", pane_id, "-c", wt, pane1]);
}

pub fn select_pane(pane_id: &str) {
    let _ = tmux(&["select-pane", "-t", pane_id]);
}

/// Attach (or switch-client if already in tmux). stdio inherited so the tty
/// reaches tmux; failure ignored (headless CI has no tty).
pub fn attach_or_switch(session: &str) {
    use std::process::Command;
    let in_tmux = std::env::var("TMUX").map(|v| !v.is_empty()).unwrap_or(false);
    let sub = if in_tmux { "switch-client" } else { "attach" };
    let _ = Command::new("tmux").args([sub, "-t", session]).status();
}

/// Kill EXACTLY `name`: `-t =name` (exact-match), falling back to `-t name`.
pub fn kill_session(name: &str) {
    let eq = format!("={name}");
    if tmux(&["kill-session", "-t", &eq]).map(|o| o.status.success()).unwrap_or(false) {
        return;
    }
    let _ = tmux(&["kill-session", "-t", name]);
}

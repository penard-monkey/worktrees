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

/// Sorted, newline-joined live session names — a cheap change signal the app
/// polls (empty when tmux is down / no sessions). Sessions come and go as places
/// are opened/closed even from a bare terminal, so a change here is worth a
/// UI refresh; an unchanged value lets the poll skip the full git sweep.
pub fn session_fingerprint() -> String {
    match tmux(&["list-sessions", "-F", "#{session_name}"]) {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut names: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
            names.sort_unstable();
            names.join("\n")
        }
        _ => String::new(),
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
    worktree_session_excluding(wt, ai_word, None)
}

/// `worktree_session` with a subtree EXCLUSION — required when `wt` is the MAIN
/// checkout: worktree dirs live UNDER it (`<main_root>/.worktrees/<slug>`), so
/// without excluding `.worktrees/` any worktree pane would falsely count as
/// main's session and main would adopt (and attach to!) a worktree's session.
pub fn worktree_session_excluding(wt: &str, ai_word: &str, exclude_under: Option<&str>) -> Option<String> {
    PaneList::fetch()?.session_in(wt, ai_word, exclude_under)
}

/// Snapshot of `list-panes -a` — every live pane as
/// `(session, pane_current_path, pane_current_command)`. Fetched ONCE per
/// caller (one tmux shell-out) and reused: `ls`/`place_json` resolves adopted
/// sessions for many worktrees against this instead of shelling out per place.
pub struct PaneList {
    panes: Vec<(String, String, String)>,
}

impl PaneList {
    /// One `list-panes -a` shell-out. `None` when tmux is absent or errors —
    /// callers then behave as if no adopted session exists.
    pub fn fetch() -> Option<PaneList> {
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
        let panes = String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|line| {
                let mut it = line.splitn(3, '\t');
                (
                    it.next().unwrap_or("").to_string(),
                    it.next().unwrap_or("").to_string(),
                    it.next().unwrap_or("").to_string(),
                )
            })
            .collect();
        Some(PaneList { panes })
    }

    /// Same selection as `worktree_session` but over the prefetched panes: a
    /// pane cwd'd in `wt` (exact or a subdir), preferring one whose command
    /// looks like the AI CLI (`ai_word`) or `node`, else the first match.
    /// `exclude_under` skips panes in that subtree — pass the project's
    /// `.worktrees/` root when `wt` is the MAIN checkout, because worktree dirs
    /// nest under it and would otherwise false-match as main's session.
    pub fn session_in(&self, wt: &str, ai_word: &str, exclude_under: Option<&str>) -> Option<String> {
        let mut best: Option<String> = None;
        let prefix = format!("{wt}/");
        let excl = exclude_under.map(|e| (e.to_string(), format!("{e}/")));
        for (sess, path, cmd) in &self.panes {
            if sess.is_empty() || !(path == wt || path.starts_with(&prefix)) {
                continue;
            }
            if let Some((eroot, eprefix)) = &excl {
                if path == eroot || path.starts_with(eprefix.as_str()) {
                    continue;
                }
            }
            if cmd.contains(ai_word) || cmd == "node" {
                return Some(sess.clone());
            }
            if best.is_none() {
                best = Some(sess.clone());
            }
        }
        best
    }
}

/// Multi-client sizing: by default tmux clamps a window to its SMALLEST
/// attached client, and only redraws that intersection — a larger client (the
/// app's embedded terminal next to a bare `tmux attach`) keeps stale painted
/// cells outside the region ("undeletable" artifacts). `window-size latest` +
/// `aggressive-resize` make OUR sessions follow the most recently active
/// client instead. Session-scoped: the user's global tmux config is untouched.
pub fn tune_session(session: &str) {
    let _ = tmux(&["set-option", "-t", session, "aggressive-resize", "on"]);
    let _ = tmux(&["set-option", "-w", "-t", session, "window-size", "latest"]);
}

/// `new-session -d -s <session> -c <wt> -P -F '#{pane_id}' <pane0>` → pane id.
/// `Err(reason)` carries tmux's own stderr (or a spawn error) so the caller can
/// surface WHY the session failed instead of a silent `None`.
pub fn new_session(session: &str, wt: &str, pane0: &str) -> Result<String, String> {
    let o = tmux(&["new-session", "-d", "-s", session, "-c", wt, "-P", "-F", "#{pane_id}", pane0])
        .map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
        Err(if err.is_empty() { format!("tmux new-session exited {}", o.status.code().unwrap_or(-1)) } else { err })
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

/// Kill EXACTLY `name` (`-t =name`). NO bare fallback: on tmux ≥ 2.1 the exact
/// form only fails when the session is already gone, so a bare `-t name` retry
/// could only ever PREFIX-match a sibling (api → api-fix) — the precise case
/// the `=` guard exists to prevent.
pub fn kill_session(name: &str) {
    let eq = format!("={name}");
    let _ = tmux(&["kill-session", "-t", &eq]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(rows: &[(&str, &str, &str)]) -> PaneList {
        PaneList {
            panes: rows.iter().map(|(s, p, c)| (s.to_string(), p.to_string(), c.to_string())).collect(),
        }
    }

    #[test]
    fn session_in_prefers_ai_word_then_node_then_first() {
        // exact-dir + subdir both count; AI pane wins over an earlier plain shell
        let list = pl(&[
            ("shellsess", "/wt/foo", "zsh"),
            ("aisess", "/wt/foo/sub", "claude"),
        ]);
        assert_eq!(list.session_in("/wt/foo", "claude", None).as_deref(), Some("aisess"));

        // no AI/node → first matching session
        let list = pl(&[("first", "/wt/foo", "zsh"), ("second", "/wt/foo", "vim")]);
        assert_eq!(list.session_in("/wt/foo", "claude", None).as_deref(), Some("first"));

        // node counts as an AI-ish pane
        let list = pl(&[("a", "/wt/foo", "bash"), ("b", "/wt/foo", "node")]);
        assert_eq!(list.session_in("/wt/foo", "claude", None).as_deref(), Some("b"));
    }

    #[test]
    fn session_in_ignores_other_dirs_and_prefix_false_matches() {
        // `/wt/foobar` must NOT match worktree `/wt/foo` (prefix guard uses `foo/`)
        let list = pl(&[("other", "/wt/foobar", "claude"), ("mine", "/wt/foo", "zsh")]);
        assert_eq!(list.session_in("/wt/foo", "claude", None).as_deref(), Some("mine"));

        // nothing cwd'd in the worktree → None
        let list = pl(&[("elsewhere", "/other", "claude")]);
        assert_eq!(list.session_in("/wt/foo", "claude", None), None);
    }

    #[test]
    fn session_in_exclusion_stops_main_adopting_worktree_sessions() {
        // (a) a worktree pane nests UNDER the main root — with the .worktrees/
        // exclusion it must NOT be adopted as main's session
        let list = pl(&[("wtsess", "/repo/.worktrees/feat", "claude")]);
        assert_eq!(list.session_in("/repo", "claude", Some("/repo/.worktrees")), None);
        // pane exactly AT the excluded root is skipped too
        let list = pl(&[("atroot", "/repo/.worktrees", "zsh")]);
        assert_eq!(list.session_in("/repo", "claude", Some("/repo/.worktrees")), None);

        // (b) a genuine main subdir still adopts under the same exclusion
        let list = pl(&[
            ("wtsess", "/repo/.worktrees/feat", "claude"),
            ("mainsess", "/repo/src", "zsh"),
        ]);
        assert_eq!(
            list.session_in("/repo", "claude", Some("/repo/.worktrees")).as_deref(),
            Some("mainsess")
        );

        // (c) no exclusion → old behavior unchanged (worktree pane matches /repo)
        let list = pl(&[("wtsess", "/repo/.worktrees/feat", "claude")]);
        assert_eq!(list.session_in("/repo", "claude", None).as_deref(), Some("wtsess"));

        // exclusion prefix is a real path boundary: /repo/.worktrees-backup is
        // NOT under /repo/.worktrees and must still adopt
        let list = pl(&[("backup", "/repo/.worktrees-backup", "zsh")]);
        assert_eq!(
            list.session_in("/repo", "claude", Some("/repo/.worktrees")).as_deref(),
            Some("backup")
        );
    }
}

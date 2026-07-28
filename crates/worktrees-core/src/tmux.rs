//! Thin wrappers over the `tmux` CLI. Subprocess (there's no native lib), which
//! also keeps the bats fake-tmux PATH shim intercepting the compiled binary.

use std::process::{Command, Output};

/// Marker for dock scratch-shell SIDECAR sessions. The dock's Terminal tab can
/// hold several shells per place: the first is `<place-session>~term`, extra
/// tabs are `<place-session>~term~2`, `~3`, … Each is a persistent,
/// `tmux attach`-able shell cwd'd in the worktree. All are excluded from
/// worktree adoption (`session_in`) so a bare shell can never masquerade as the
/// place's AI session.
///
/// The `~` is deliberate and load-bearing: a place session is `<prefix>-<slug>`
/// where the slug is a slugified git ref — git ref names forbid `~` (and tmux
/// only forbids `.`/`:` in session names), so NO real place session can ever
/// contain `~term`. That makes the marker collision-proof: without it, a place
/// on branch "long-term" (session `<prefix>-long-term`) would be byte-identical
/// to the sidecar of a place named "long", and closing one could kill the
/// other's live Claude session.
pub const SHELL_SIDECAR_MARKER: &str = "~term";

/// The sidecar session name for a place's (canonical) session + a 1-based tab
/// index. Index ≤1 is the bare `~term`; 2+ append `~N`.
pub fn shell_sidecar_name(session: &str, index: u32) -> String {
    if index <= 1 { format!("{session}{SHELL_SIDECAR_MARKER}") }
    else { format!("{session}{SHELL_SIDECAR_MARKER}~{index}") }
}

/// The `<session>~term` stem every sidecar of a place shares (for enumeration).
pub fn shell_sidecar_prefix(session: &str) -> String {
    format!("{session}{SHELL_SIDECAR_MARKER}")
}

/// If `name` is a sidecar of `session`, its 1-based tab index (bare `~term` → 1,
/// `~term~<n>` → n). `None` when `name` isn't this place's sidecar.
pub fn shell_sidecar_index(session: &str, name: &str) -> Option<u32> {
    let stem = shell_sidecar_prefix(session);
    if name == stem { return Some(1); }
    name.strip_prefix(&stem)?.strip_prefix('~').and_then(|d| d.parse::<u32>().ok())
}

/// Is `name` any place's shell sidecar? True for any name carrying the `~term`
/// marker — since `~` can't appear in a real place session, the marker alone is
/// proof (so `session_in` skips every dock shell).
pub fn is_shell_sidecar(name: &str) -> bool {
    name.contains(SHELL_SIDECAR_MARKER)
}

/// All live session names (empty when tmux is down / errors).
pub fn session_names() -> Vec<String> {
    match tmux(&["list-sessions", "-F", "#{session_name}"]) {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

/// End EVERY shell sidecar of `canonical_session` — the dock's shells for a
/// place. Called on close/remove (core, so the CLI cleans up too). Best-effort.
pub fn kill_shell_sidecars(canonical_session: &str) {
    for n in session_names() {
        if shell_sidecar_index(canonical_session, &n).is_some() {
            kill_session(&n);
        }
    }
}

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

/// A session already living in place dir `wt` (a pane cwd'd there), so `open`
/// reuses an AI pane running under any name — including one started under a
/// prefix this repo has since changed (`ops::live_session`). Prefers a pane whose
/// command looks like the configured AI CLI (`ai_word`) or `node`; else the first
/// match.
///
/// `exclude_under` skips a subtree, and is required when `wt` is the MAIN
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

    /// Does a session named EXACTLY `name` exist, per this snapshot? The
    /// prefetched answer to `session_exists`, for a caller that has to ask once
    /// per place: every live session has at least one pane, so `list-panes -a`
    /// names them all and one shell-out replaces N `list-sessions` calls.
    pub fn has_session(&self, name: &str) -> bool {
        self.panes.iter().any(|(s, _, _)| s == name)
    }

    /// Same selection as `worktree_session_excluding` but over the prefetched panes: a
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
            // A dock scratch-shell sidecar (`<place-session>-term`) is cwd'd in
            // the worktree and runs a bare shell — never let it be adopted AS the
            // place's session (that would attach the AI view to a plain shell and
            // skip launching Claude). It's addressed by its exact name instead.
            if is_shell_sidecar(sess) {
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
    fn has_session_answers_exactly_like_session_exists() {
        // Exact match, never a prefix one — `api` must not answer for `api-fix`,
        // the same guard `session_exists` exists for.
        let list = pl(&[("api-fix", "/wt/a", "zsh"), ("main", "/repo", "zsh")]);
        assert!(list.has_session("api-fix"));
        assert!(list.has_session("main"));
        assert!(!list.has_session("api"));
        assert!(!list.has_session(""));
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

    #[test]
    fn session_in_skips_dock_shell_sidecar() {
        // A `<session>~term[~N]` sidecar (a dock scratch shell) is cwd'd in the
        // worktree but must NEVER be adopted as the place's AI session — else the
        // AI view would attach to a bare shell and Claude wouldn't launch.
        let list = pl(&[("repo-foo~term", "/wt/foo", "zsh")]);
        assert_eq!(list.session_in("/wt/foo", "claude", None), None);
        // indexed tabs are skipped too
        let list = pl(&[("repo-foo~term~3", "/wt/foo", "bash")]);
        assert_eq!(list.session_in("/wt/foo", "claude", None), None);
        // the real AI session still wins even alongside its sidecars
        let list = pl(&[
            ("repo-foo~term", "/wt/foo", "zsh"),
            ("repo-foo~term~2", "/wt/foo", "bash"),
            ("repo-foo", "/wt/foo", "claude"),
        ]);
        assert_eq!(list.session_in("/wt/foo", "claude", None).as_deref(), Some("repo-foo"));
    }

    #[test]
    fn shell_sidecar_naming_and_index() {
        assert_eq!(shell_sidecar_name("repo-foo", 1), "repo-foo~term");
        assert_eq!(shell_sidecar_name("repo-foo", 0), "repo-foo~term"); // clamp ≤1
        assert_eq!(shell_sidecar_name("repo-foo", 2), "repo-foo~term~2");
        assert_eq!(shell_sidecar_index("repo-foo", "repo-foo~term"), Some(1));
        assert_eq!(shell_sidecar_index("repo-foo", "repo-foo~term~5"), Some(5));
        // not this place's sidecar / not a sidecar at all
        assert_eq!(shell_sidecar_index("repo-foo", "repo-foo"), None);
        assert_eq!(shell_sidecar_index("repo-foo", "repo-bar~term"), None);
        assert_eq!(shell_sidecar_index("repo-foo", "repo-foo~term~x"), None);
        // COLLISION-PROOFING: a real place on branch "long-term" gets session
        // `repo-long-term`, which must NOT read as a sidecar of place "repo-long"
        // (the `-term` hyphen scheme this replaced would have — cross-place kill).
        assert_eq!(shell_sidecar_index("repo-long", "repo-long-term"), None);
        assert!(!is_shell_sidecar("repo-long-term"));
        assert!(is_shell_sidecar("x~term"));
        assert!(is_shell_sidecar("x~term~9"));
        assert!(!is_shell_sidecar("x-terminal"));
    }
}

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
pub const CODEX_SIDECAR_MARKER: &str = "~agent~codex";
pub const CLAUDE_SIDECAR_MARKER: &str = "~agent~claude";

/// Codex's managed session name. A provider switch ends the other agent's
/// session first; `~` cannot occur in a git ref, so this cannot collide with
/// a place's canonical name.
pub fn codex_session_name(canonical: &str) -> String {
    format!("{canonical}{CODEX_SIDECAR_MARKER}")
}

pub fn claude_session_name(canonical: &str) -> String {
    format!("{canonical}{CLAUDE_SIDECAR_MARKER}")
}

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

    /// Recognize an older Codex pane launched under the canonical place name.
    /// Keep this strict: `node` and version-like names identify Claude on some
    /// installs, so they cannot distinguish the two providers here.
    pub fn session_is_codex(&self, name: &str) -> bool {
        self.panes.iter().any(|(s, _, cmd)| s == name && cmd.rsplit('/').next() == Some("codex"))
    }

    /// Running provider panes in one place, including sessions left under an
    /// older prefix. Callers may close only names they own; an adopted session
    /// must be surfaced for explicit handling instead of silently killed.
    pub fn agents_in(&self, wt: &str, exclude_under: Option<&str>) -> Vec<(String, &'static str)> {
        let prefix = format!("{wt}/");
        let mut found = Vec::new();
        for (session, path, cmd) in &self.panes {
            if is_shell_sidecar(session) || !(path == wt || path.starts_with(&prefix)) { continue; }
            if exclude_under.is_some_and(|dir| path == dir || path.starts_with(&format!("{dir}/"))) { continue; }
            let provider = if cmd.rsplit('/').next() == Some("codex") { "codex" }
                else if is_ai_command(cmd, "claude") { "claude" } else { continue };
            if !found.iter().any(|(name, _)| name == session) {
                found.push((session.clone(), provider));
            }
        }
        found
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
            if is_shell_sidecar(sess) || sess.contains(CODEX_SIDECAR_MARKER) || sess.contains(CLAUDE_SIDECAR_MARKER) {
                continue;
            }
            if let Some((eroot, eprefix)) = &excl {
                if path == eroot || path.starts_with(eprefix.as_str()) {
                    continue;
                }
            }
            if is_ai_command(cmd, ai_word) {
                return Some(sess.clone());
            }
            if best.is_none() {
                best = Some(sess.clone());
            }
        }
        best
    }
}

pub fn session_is_codex(name: &str) -> bool {
    PaneList::fetch().is_some_and(|panes| panes.session_is_codex(name))
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
/// A tmux pane id (`%7`). A newtype, and the ONLY thing `paste_commands` will
/// accept as a target.
///
/// Not decoration. A tmux target given as a NAME prefix-matches — `-t api`
/// resolves to `api-fix` when that is the only session — so a reference could
/// be pasted into a different worktree's Claude. The first fix for that was a
/// test asserting the target starts with `%`, and it did not hold: the target
/// is chosen inside `paste_to_ai`, while the test called `paste_commands`
/// directly with a literal, so swapping the argument back to
/// `format!("{session}:0.0")` passed the whole suite. The constructor is
/// private to this module and only `ai_pane` builds one, so the wrong target
/// can no longer be spelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneId(String);

impl PaneId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The pane in `session` that is running the AI, as a stable `%id`.
///
/// NOT `:0.0`. Pane 0 is where `new_session` launches the AI, and that is the
/// only thing true about it — three ordinary situations break the assumption:
///
///   * `set -g base-index 1` in a user's `~/.tmux.conf` (common) means there is
///     no window 0 at all, and `tmux` answers `can't find window: 0`. Nothing
///     here pins `base-index`, and `tune_session` does not either;
///   * closing pane 0 and opening another renumbers the survivors, so index 0
///     becomes whatever is left — typically the shell in what was pane 1;
///   * an ADOPTED session was not created by `new_session`, so its pane 0 is
///     whatever the user happened to start first.
///
/// Matched on the pane's command against the AI word, the same rule
/// `PaneList::session_in` uses for adoption, and returned as a `%id` because
/// those are globally unique and never renumber.
pub fn ai_pane(session: &str, ai_word: &str) -> Option<PaneId> {
    // `=` anchors the session name: a tmux target PREFIX-matches, so `-t api`
    // resolves to `api-fix` when that is the only session — which would drop a
    // reference into a DIFFERENT worktree's Claude. `session_exists` documents
    // the same trap for `has-session`.
    let target = format!("={session}");
    let o = tmux(&[
        "list-panes",
        "-t",
        &target,
        // `pane_start_command` LAST: it is a shell command line and the only
        // field here that can contain anything, so it gets the tail.
        "-F",
        "#{pane_id}\t#{pane_current_command}\t#{pane_start_command}",
    ])
    .ok()?;
    if !o.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let panes: Vec<(&str, &str, &str)> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let id = it.next()?;
            // A pane id is always `%N`. The guard is cheap and makes the
            // invariant `PaneId` exists for true at the one place ids enter —
            // it also closes a pre-3.2 tmux corner, where `pane_start_command`
            // was the raw string and an embedded newline could split a line.
            id.starts_with('%').then_some((id, it.next()?, it.next().unwrap_or("")))
        })
        .collect();
    pick_ai_pane(&panes, ai_word).map(|id| PaneId(id.to_string()))
}

/// Which of a session's panes is the AI — the whole rule, as a pure function.
///
/// `ai_pane` is reachable only from the app (no CLI path, so no bats), which
/// makes this the only coverage the selection will ever get.
///
/// The start command is a DISAMBIGUATOR, never evidence on its own.
/// `ops::launch` builds pane 0 as `<ai_cmd>; exec "${SHELL}"`, so when Claude
/// exits — `/exit`, ctrl-D, a crash — the pane lives on as a shell while
/// `pane_start_command` still says `claude`. Trusting it alone put the token on
/// that shell's prompt and reported success, which is the exact outcome
/// `paste_to_ai` exists to refuse. Verified: a pane started as
/// `exec sh -ic 'true claude; exec sh'` lists as `cmd=[bash] start=["…claude…"]`.
fn pick_ai_pane<'a>(panes: &[(&'a str, &'a str, &'a str)], ai_word: &str) -> Option<&'a str> {
    panes
        .iter()
        // Both signals: launched as the AI AND still running it.
        .find(|(_, cmd, start)| start.contains(ai_word) && is_ai_command(cmd, ai_word))
        // An ADOPTED pane was never launched with a command, so its foreground
        // process is all the evidence there is.
        .or_else(|| panes.iter().find(|(_, cmd, _)| is_ai_command(cmd, ai_word)))
        .map(|(id, _, _)| *id)
}

/// `%0=zsh %1=vim`, so a refusal can be diagnosed from one log line.
fn pane_summary(session: &str) -> String {
    let target = format!("={session}");
    tmux(&["list-panes", "-t", &target, "-F", "#{pane_id}=#{pane_current_command}"])
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "none".to_string())
}

/// Is this `pane_current_command` the AI? Byte-for-byte the rule `session_in`
/// adopts by, in one place so the two cannot drift — and keyed on the
/// configured `ai_word`, not on the literal "claude", because the AI command is
/// a setting (`config::resolve_ai_cmd`).
pub fn is_ai_command(cmd: &str, ai_word: &str) -> bool {
    // `node`: claude is a node program, and tmux reports the interpreter when
    // the binary is a wrapper script.
    cmd.contains(ai_word) || cmd == "node" || is_version_like(cmd)
}

/// `2.1.277` — a bare dotted version, which is what a natively-installed Claude
/// reports as its process name.
///
/// It is NOT a process rename. `~/.local/bin/claude` is a symlink to
/// `~/.local/share/claude/versions/2.1.277`, and XNU sets `p_comm` from the
/// RESOLVED last path component while argv[0] stays `claude`. Measured:
///
/// ```text
/// ps -o ucomm,comm -p 69362   ->   2.1.277   claude
/// pane_current_command        ->   2.1.277
/// ```
///
/// Three things follow. Searching for a process TITLE finds nothing, because
/// nothing was retitled. It is macOS-specific — tmux's Linux backend reads
/// `/proc/<pid>/cmdline` and reports `claude`. And an npm or brew install
/// reports `node`/`claude` and never reaches this rule.
///
/// Deliberately strict: three or more numeric components and nothing else.
fn is_version_like(cmd: &str) -> bool {
    let parts: Vec<&str> = cmd.split('.').collect();
    parts.len() >= 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Put `text` into the AI's pane in `session` as a PASTE, without a newline.
///
/// **`paste-buffer -p`, never `send-keys -l`.** `-p` wraps the text in
/// bracketed-paste markers when the program in that pane has requested mode
/// 2004, which Claude's input does — so the text arrives as a PASTE, which is
/// a thing a TUI handles deliberately, rather than as a burst of keystrokes it
/// cannot distinguish from typing. That is what keeps a drop landing during a
/// "do you want to allow this?" prompt from reading as an answer to it.
///
/// An earlier version of this comment claimed a permission prompt does not
/// request 2004 — that was a guess, and almost certainly wrong, since it is the
/// same program with the mode already set. The paste/keystroke distinction is
/// the part that holds; the behaviour during a prompt is step 5 of the drag's
/// manual checks precisely because it is not verified here.
/// Do not add a `send-keys` fallback.
///
/// The buffer is NAMED and deleted afterwards so this never disturbs the
/// user's own paste stack, and `--` ends option parsing so a reference that
/// begins with `-` cannot be read as a flag.
pub fn paste_to_ai(session: &str, ai_word: &str, text: &str) -> Result<(), String> {
    // An honest failure. The alternative — pasting into whatever pane happens
    // to be at index 0 — puts the token on a shell prompt and still reports
    // success, which is worse than saying nothing happened.
    // Name what was there. The last time this rule was wrong it took a survey of
    // 21 live sessions to find out why; the next time should be one log line.
    let pane = ai_pane(session, ai_word)
        .ok_or_else(|| format!("no Claude running in session {session} (panes: {})", pane_summary(session)))?;
    let buf = format!("worktrees-drop-{}", std::process::id());
    let [set_argv, paste_argv, del_argv] = paste_commands(&buf, &pane, text);
    let set = tmux(&set_argv).map_err(|e| e.to_string())?;
    if !set.status.success() {
        return Err(String::from_utf8_lossy(&set.stderr).trim().to_string());
    }
    let out = tmux(&paste_argv);
    // Delete the buffer whatever happened to the paste — a named buffer left
    // behind would accumulate one entry per failed drop for the tmux server's
    // whole life.
    let _ = tmux(&del_argv);
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// The three tmux invocations a drop makes, as argv.
///
/// Split out so the exact shape is testable without a tmux server or a PATH
/// shim — `PATH` is process-global and `cargo test` runs tests in threads, so
/// a shim would race every other test that shells out. The shape is the whole
/// safety argument (`-p`, the pane id, `--`, and the cleanup), so it is worth
/// pinning directly.
fn paste_commands<'a>(buf: &'a str, pane: &'a PaneId, text: &'a str) -> [Vec<&'a str>; 3] {
    [
        // `--` so a reference that begins with `-` is not read as a flag.
        vec!["set-buffer", "-b", buf, "--", text],
        // `-p` = bracketed paste IF the pane's program asked for mode 2004.
        // A `%id`, which is globally unique and cannot prefix-match another
        // session the way a NAME target can.
        vec!["paste-buffer", "-b", buf, "-p", "-t", pane.as_str()],
        vec!["delete-buffer", "-b", buf],
    ]
}

#[cfg(test)]
mod paste_tests {
    use super::*;

    /// The shape IS the safety argument, so it is asserted rather than assumed.
    #[test]
    fn a_drop_targets_a_pane_id_and_cleans_up_after_itself() {
        let pane = PaneId("%7".into());
        let [set, paste, del] = paste_commands("buf1", &pane, "-@worktrees:place://x ");

        assert_eq!(set, ["set-buffer", "-b", "buf1", "--", "-@worktrees:place://x "]);
        assert_eq!(
            set.iter().position(|a| *a == "--").unwrap(),
            set.len() - 2,
            "`--` must be the LAST option, or a reference starting with `-` is read as a flag"
        );

        assert!(paste.contains(&"-p"), "without -p this is an unbracketed injection: {paste:?}");
        let t = paste.iter().position(|a| *a == "-t").expect("-t");
        assert!(
            paste[t + 1].starts_with('%'),
            "the target must be a pane ID. A NAME target PREFIX-matches, so `-t api` \
             resolves to `api-fix` when that is the only session, and the reference \
             lands in another worktree's Claude (got {:?})",
            paste[t + 1]
        );

        assert_eq!(del, ["delete-buffer", "-b", "buf1"]);
        assert_eq!(del[2], set[2], "the buffer deleted must be the one written");

        for argv in [&set, &paste, &del] {
            assert!(!argv.contains(&"send-keys"), "send-keys can type into a confirmation dialog");
        }
    }

    /// Pane 0 is NOT "the AI". `base-index 1` in a user's tmux.conf means there
    /// is no window 0 at all; closing a pane renumbers the survivors; and an
    /// adopted session was never laid out by `new_session`. So the pane is found
    /// by what it RUNS, using the same rule adoption uses.
    /// The strings here were measured on the live session where the drop failed
    /// with "no Claude running": a natively-installed Claude reports its
    /// VERSION as the process name, so neither the AI word nor `node` appears
    /// anywhere in `pane_current_command`.
    #[test]
    fn a_version_named_claude_is_still_recognised() {
        assert!(
            is_ai_command("2.1.277", "claude"),
            "the bug that made every drop report no Claude in the session"
        );
        assert!(is_version_like("10.0.1"), "not just single digits");
        assert!(!is_version_like("zsh"));
        assert!(!is_version_like("node"));
        assert!(!is_version_like("2.1"), "two components is not a version here");
        assert!(!is_version_like("v2.1.3"), "a leading v is not a bare version");
        assert!(!is_version_like("a.b.c"));
        assert!(!is_version_like("..."), "empty components are not digits");
        assert!(!is_version_like(""));
    }

    /// The SELECTION, which is where the interesting failures live — the
    /// predicate above says what a command looks like, this says which pane wins.
    #[test]
    fn the_start_command_disambiguates_but_never_vouches() {
        let pick = |panes: &[(&str, &str, &str)]| pick_ai_pane(panes, "claude").map(str::to_string);

        // THE REGRESSION. `ops::launch` builds pane 0 as `claude; exec $SHELL`,
        // so a pane whose Claude has exited keeps a start command saying
        // `claude` while running a shell. Trusting the start command alone
        // pasted the token onto that prompt and called it success.
        assert_eq!(
            pick(&[("%0", "zsh", "exec sh -ic 'claude --name x; exec sh'")]),
            None,
            "a pane whose Claude has EXITED must not be chosen"
        );

        // Both signals present: the ordinary running case.
        assert_eq!(
            pick(&[("%0", "2.1.277", "exec sh -ic 'claude --name x; exec sh'")]),
            Some("%0".to_string())
        );

        // The start command breaks a tie: pane 1 is a node dev server, pane 0 is
        // the AI. Without the ordering, `node` in pane 1 could win.
        assert_eq!(
            pick(&[
                ("%1", "node", ""),
                ("%0", "2.1.277", "exec sh -ic 'claude; exec sh'"),
            ]),
            Some("%0".to_string()),
            "a launched-and-running AI beats a bare `node` elsewhere"
        );

        // Adopted: nothing launched it with a command, so the foreground
        // process is the only evidence there is.
        assert_eq!(pick(&[("%3", "2.1.277", "")]), Some("%3".to_string()));
        assert_eq!(pick(&[("%3", "claude", "")]), Some("%3".to_string()));

        // Nothing resembling the AI: refuse. There is deliberately NO
        // sole-pane fallback — the app creates sessions single-pane, so "one
        // pane" is equally the shape of a live session and of one whose Claude
        // has exited, and the fallback would fire exactly where it is wrong.
        assert_eq!(pick(&[("%0", "zsh", "")]), None);
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn the_ai_pane_is_found_by_command_not_by_position() {
        assert!(is_ai_command("claude", "claude"));
        assert!(is_ai_command("node", "claude"), "claude is a node program behind a wrapper");
        assert!(!is_ai_command("zsh", "claude"), "a shell must never receive a dropped reference");
        assert!(!is_ai_command("bash", "claude"));
        assert!(!is_ai_command("vim", "claude"));
        // The AI command is a SETTING, so the word is too.
        assert!(is_ai_command("aider", "aider"));
        assert!(!is_ai_command("claude", "aider"));
    }

    #[test]
    fn each_drop_uses_its_own_buffer_name() {
        // Named, so a drop never disturbs the user's own paste stack.
        let buf = format!("worktrees-drop-{}", std::process::id());
        assert!(buf.starts_with("worktrees-drop-"), "{buf}");
        assert_ne!(buf, "", "an empty -b would mean tmux's default buffer");
    }
}

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
    fn agents_in_finds_adopted_providers_without_neighboring_places_or_shells() {
        let list = pl(&[
            ("old-prefix", "/repo/src", "claude"),
            ("codex-sidecar", "/repo", "codex"),
            ("repo~term", "/repo", "zsh"),
            ("worktree-agent", "/repo/.worktrees/feat", "codex"),
            ("neighbor", "/repository", "claude"),
        ]);
        assert_eq!(list.agents_in("/repo", Some("/repo/.worktrees")), vec![
            ("old-prefix".into(), "claude"),
            ("codex-sidecar".into(), "codex"),
        ]);
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

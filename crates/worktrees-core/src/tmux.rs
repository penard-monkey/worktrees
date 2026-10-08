//! Thin wrappers over the `tmux` CLI. Subprocess (there's no native lib), which
//! also keeps the bats fake-tmux PATH shim intercepting the compiled binary.

use std::collections::HashMap;
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
    tmux_version().is_some()
}

/// `tmux -V`'s output, or `None` when tmux cannot be run. Probed every call on
/// purpose: the app re-checks after refreshing PATH, so "not installed" must
/// never stick. Only the `-N` verdict below is cached, and only once a probe
/// succeeded.
fn tmux_version() -> Option<String> {
    let o = Command::new("tmux").arg("-V").output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

static NO_START: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// The global options every NON-launching tmux call carries: `["-N"]` on a
/// tmux that has it (≥ 3.2), else nothing (today's behaviour).
///
/// **Why.** A tmux client whose command may start a server (`new-session`,
/// and `attach-session` under a tty — measured on 3.7c), on finding the socket
/// refused or missing, takes the lock, UNLINKS the socket and starts a fresh
/// server. A live server that was only too busy to accept (full listen
/// backlog) is then orphaned with every session in it, and the next `open`
/// relaunches agents that are still running. `-N` makes the client fail
/// instead. Only `tmux_launch` may leave it off.
pub fn no_start_args() -> &'static [&'static str] {
    let supported = match NO_START.get() {
        Some(b) => *b,
        None => match tmux_version() {
            Some(v) => *NO_START.get_or_init(|| supports_no_start(&v)),
            None => false,
        },
    };
    if supported { &["-N"] } else { &[] }
}

/// Does this `tmux -V` line name a tmux with `-N` (added in 3.2)? `master` is
/// a source build, newer than any release. Anything unparsable is `false`:
/// an unknown tmux keeps the behaviour it has always had.
pub fn supports_no_start(version: &str) -> bool {
    let Some(v) = version.trim().strip_prefix("tmux ") else { return false };
    let v = v.trim();
    if v == "master" {
        return true;
    }
    let v = v.strip_prefix("next-").unwrap_or(v);
    let mut it = v.splitn(2, '.');
    let (Some(major), Some(rest)) = (it.next(), it.next()) else { return false };
    let minor: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    match (major.parse::<u32>(), minor.parse::<u32>()) {
        (Ok(major), Ok(minor)) => (major, minor) >= (3, 2),
        _ => false,
    }
}

/// Run tmux for anything that must NOT bring a server up — which is every call
/// but `new-session`. Carries `no_start_args()`, so a new call site is safe by
/// default; when the server is down it fails exactly as before ("no server
/// running" / "error connecting to …"), so callers that treat a failure as
/// "no sessions" are unchanged.
pub fn tmux(args: &[&str]) -> std::io::Result<Output> {
    Command::new("tmux").args(no_start_args()).args(args).output()
}

/// Run tmux for a command that is MEANT to start a server when none is up
/// (`new-session`). The only caller that may omit `-N`; see `no_start_args`.
pub fn tmux_launch(args: &[&str]) -> std::io::Result<Output> {
    wait_out_refusal();
    Command::new("tmux").args(args).output()
}

/// How long `tmux_launch` waits for a server that is REFUSING connections.
const REFUSED_TRIES: u32 = 5;
const REFUSED_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

/// Before a launch: if the socket is there but refuses (a busy server's full
/// backlog — or a dead server's leftover socket), wait a moment. A launch that
/// meets a refusal unlinks the socket and starts a second server, orphaning
/// the first; a busy server usually accepts again within milliseconds. After
/// ~1s it is most likely a stale socket, and the launch goes ahead so tmux's
/// own stale-socket recovery still works. Narrows the race, cannot close it —
/// only a `-N` probe can be refused without consequences.
fn wait_out_refusal() {
    if no_start_args().is_empty() {
        return;
    }
    for _ in 0..REFUSED_TRIES {
        match tmux(&["start-server"]) {
            Ok(o) if !o.status.success() && is_refusal(&String::from_utf8_lossy(&o.stderr)) => {
                std::thread::sleep(REFUSED_WAIT)
            }
            _ => return,
        }
    }
}

/// tmux's client spells ECONNREFUSED — the socket exists and nobody accepted —
/// as `no server running on <path>`, and every other connect error as
/// `error connecting to <path> (<strerror>)` (measured on 3.7c). A MISSING
/// socket is the latter, and means "no server": what a launch is for.
fn is_refusal(stderr: &str) -> bool {
    stderr.contains("no server running on ")
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

/// One pane of a `list-panes -a` snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pane {
    pub session: String,
    /// `#{pane_current_path}`.
    pub path: String,
    /// `#{pane_current_command}`.
    pub cmd: String,
    /// `#{pane_pid}` — what tells two launches under the same session NAME
    /// apart (a close + open recreates `~agent~pi`). `None` from a tmux row
    /// that printed no fourth field.
    pub pid: Option<String>,
    /// `#{pane_tty}`, normalised to how `ps` spells it (`ttys009`, `pts/3`).
    pub tty: Option<String>,
    /// The program word of the tty's FOREGROUND process-group leader, from
    /// `ps` — resolved only for a pane `foreground_wanted` picks, else `None`.
    pub fg: Option<String>,
}

impl Pane {
    /// The program word attribution keys on. tmux names a wrapper's
    /// INTERPRETER: pi (and npm's codex) read as `node`, which by itself says
    /// nothing about which harness it is. pi sets its process title, so the
    /// tty's foreground leader names it (`ps` comm `pi`). The leader counts
    /// only when it names a HARNESS: an `npx`-started Claude leads with `npm`,
    /// and reading that would take the pane away from the `node` heuristic
    /// that has always called it Claude. Everything else is the pane's own
    /// command, as before.
    pub fn program(&self) -> &str {
        match &self.fg {
            Some(fg) if is_wrapper(&self.cmd) && crate::provider::by_word(fg).is_some() => fg,
            _ => self.cmd.rsplit('/').next().unwrap_or(&self.cmd),
        }
    }
}

/// `node`: what tmux shows for any program run through a JS wrapper.
fn is_wrapper(cmd: &str) -> bool {
    cmd.rsplit('/').next() == Some("node")
}

/// A tty as `ps` names it: tmux says `/dev/ttys009` (Linux `/dev/pts/3`), ps
/// says `ttys009` (`pts/3`). Without this the two never meet.
pub fn normalize_tty(tty: &str) -> &str {
    tty.trim().strip_prefix("/dev/").unwrap_or(tty.trim())
}

/// `list-panes -a` rows (`session\tpath\tcmd[\tpid[\ttty]]`) as panes. A row
/// with fewer fields is a tmux, or a test shim, that printed fewer: the
/// missing ones are `None`, never an error.
pub fn parse_pane_rows(text: &str) -> Vec<Pane> {
    text.lines()
        .map(|line| {
            let mut it = line.splitn(5, '\t');
            let mut field = || it.next().unwrap_or("").to_string();
            let (session, path, cmd) = (field(), field(), field());
            let some = |s: String| (!s.trim().is_empty()).then_some(s);
            let pid = some(field());
            let tty = some(field()).map(|t| normalize_tty(&t).to_string());
            Pane { session, path, cmd, pid, tty, fg: None }
        })
        .collect()
}

/// `ps -A -o tty=,pid=,tpgid=,comm=` → each tty's foreground process-group
/// leader (`pid == tpgid`), as a program word. `comm` is the last column and
/// may hold spaces (`npm exec x`) or a path (`/bin/zsh`): the word is the
/// basename of its first token. Ttys with no controlling terminal (`??`, `?`)
/// never match a pane's, so they are not filtered here.
pub fn parse_foreground(ps: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in ps.lines() {
        let mut it = line.split_whitespace();
        let (Some(tty), Some(pid), Some(tpgid), Some(comm)) = (it.next(), it.next(), it.next(), it.next()) else {
            continue;
        };
        if pid != tpgid || pid.parse::<u32>().is_err() {
            continue;
        }
        let word = comm.rsplit('/').next().unwrap_or(comm).trim_start_matches('-');
        if !word.is_empty() {
            out.insert(normalize_tty(tty).to_string(), word.to_string());
        }
    }
    out
}

/// Whether `pane`'s foreground leader is worth a `ps`: a wrapper (`node`) in a
/// session whose NAME does not already say what runs there. A provider
/// sidecar names its harness (`for_pane` rule 2), and a dock shell is never an
/// agent, so neither costs a spawn — the common case is no `ps` at all.
fn foreground_wanted(pane: &Pane) -> bool {
    pane.tty.is_some()
        && is_wrapper(&pane.cmd)
        && !crate::provider::is_sidecar(&pane.session)
        && !is_shell_sidecar(&pane.session)
}

/// Fill `fg` on the panes that want it, from ONE `ps` run — and none when no
/// pane wants it. `ps` is a seam so the tests can count it; it is handed the
/// wanted ttys (normalised, sorted, once each).
fn resolve_foreground(panes: &mut [Pane], ps: impl FnOnce(&[&str]) -> Option<String>) {
    let mut ttys: Vec<&str> = panes.iter().filter(|p| foreground_wanted(p)).filter_map(|p| p.tty.as_deref()).collect();
    if ttys.is_empty() {
        return;
    }
    ttys.sort_unstable();
    ttys.dedup();
    let Some(text) = ps(&ttys) else { return };
    let fg = parse_foreground(&text);
    for pane in panes.iter_mut().filter(|p| foreground_wanted(p)) {
        pane.fg = pane.tty.as_deref().and_then(|t| fg.get(t)).cloned();
    }
}

/// `ps` over just the wanted ttys (`-t a,b`, which macOS and procps both
/// take): ~9ms here against ~125ms for the whole table. But `-t` fails the
/// WHOLE call — exit 1, no rows — when any listed tty is gone (macOS: "No such
/// file or directory"), and a pane can close between `list-panes` and this.
/// Then, and only then, one `ps -A`.
fn ps_foreground(ttys: &[&str]) -> Option<String> {
    let run = |sel: &[&str]| {
        let o = Command::new("ps").args(sel).args(["-o", "tty=,pid=,tpgid=,comm="]).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    run(&["-t", &ttys.join(",")]).or_else(|| run(&["-A"]))
}

/// Snapshot of `list-panes -a` — every live pane, one `Pane` row each.
/// Fetched ONCE per caller (one tmux shell-out, plus one `ps` only when a
/// `node` pane needs naming) and reused: `ls`/`place_json` resolves adopted
/// sessions for many worktrees against this instead of shelling out per
/// place, and every question asked of it afterwards is pure.
pub struct PaneList {
    panes: Vec<Pane>,
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
            "#{session_name}\t#{pane_current_path}\t#{pane_current_command}\t#{pane_pid}\t#{pane_tty}",
        ])
        .ok()?;
        if !o.status.success() {
            return None;
        }
        let mut panes = parse_pane_rows(&String::from_utf8_lossy(&o.stdout));
        resolve_foreground(&mut panes, ps_foreground);
        Some(PaneList { panes })
    }

    /// A snapshot from rows a caller already holds, as
    /// `(session, pane_current_path, pane_current_command)`.
    pub fn from_rows(panes: Vec<(String, String, String)>) -> PaneList {
        PaneList::from_panes(panes.into_iter().map(|(session, path, cmd)| Pane { session, path, cmd, ..Pane::default() }).collect())
    }

    /// A snapshot from whole rows, foreground included — for a caller (a test)
    /// that already knows what `ps` would have said.
    pub fn from_panes(panes: Vec<Pane>) -> PaneList {
        PaneList { panes }
    }

    /// Which LAUNCH of session `name` this is: its first pane's pid. A session
    /// closed and reopened under the same name is a new process, so this is
    /// what "the same session as last time" has to compare — the name alone
    /// cannot tell a relaunch from a re-list.
    pub fn session_launch(&self, name: &str) -> Option<&str> {
        self.panes.iter().filter(|p| p.session == name).find_map(|p| p.pid.as_deref())
    }

    /// Does a session named EXACTLY `name` exist, per this snapshot? The
    /// prefetched answer to `session_exists`, for a caller that has to ask once
    /// per place: every live session has at least one pane, so `list-panes -a`
    /// names them all and one shell-out replaces N `list-sessions` calls.
    pub fn has_session(&self, name: &str) -> bool {
        self.panes.iter().any(|p| p.session == name)
    }

    /// Whether `name` exists and some pane in it is running something other
    /// than a shell. An agent session is launched as `<agent> …; exec
    /// "$SHELL"`, so it OUTLIVES the agent: when codex exits (or is killed —
    /// nothing then writes `turn_aborted`), the session is still there with a
    /// bare shell in it. Keyed on "not a shell" rather than "is codex": an npm
    /// install runs codex under `node`, which must not read as exited.
    pub fn session_runs_program(&self, name: &str) -> bool {
        self.program_in(name).is_some()
    }

    /// The program word (`Pane::program`) of the first pane in `name` that is
    /// running something other than a shell.
    pub fn program_in(&self, name: &str) -> Option<&str> {
        self.panes.iter().find(|p| p.session == name && !is_shell_command(&p.cmd)).map(Pane::program)
    }

    /// Which harness owns canonical session `name`: a non-default harness
    /// whose program word a pane shows (`codex`, or `pi` named by its tty's
    /// foreground leader), else Claude — legacy canonical sessions default to
    /// Claude, including a bare shell.
    pub fn canonical_provider(&self, name: &str) -> &'static crate::provider::Provider {
        crate::provider::PROVIDERS.iter().filter(|p| !p.canonical_default)
            .find(|p| self.panes.iter().any(|pane| pane.session == name && pane.program() == p.match_word))
            .unwrap_or(crate::provider::CLAUDE)
    }

    /// Running provider panes in one place, including sessions left under an
    /// older prefix. Callers may close only names they own; an adopted session
    /// must be surfaced for explicit handling instead of silently killed.
    pub fn agents_in(&self, wt: &str, exclude_under: Option<&str>) -> Vec<(String, &'static str)> {
        let prefix = format!("{wt}/");
        let mut found = Vec::new();
        for pane in &self.panes {
            let (session, path) = (&pane.session, &pane.path);
            if is_shell_sidecar(session) || !(path == wt || path.starts_with(&prefix)) { continue; }
            if exclude_under.is_some_and(|dir| path == dir || path.starts_with(&format!("{dir}/"))) { continue; }
            let Some(provider) = crate::provider::for_pane(session, pane.program()) else { continue };
            let provider = provider.id;
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
        for Pane { session: sess, path, cmd, .. } in &self.panes {
            if sess.is_empty() || !(path == wt || path.starts_with(&prefix)) {
                continue;
            }
            // A dock scratch-shell sidecar (`<place-session>-term`) is cwd'd in
            // the worktree and runs a bare shell — never let it be adopted AS the
            // place's session (that would attach the AI view to a plain shell and
            // skip launching Claude). It's addressed by its exact name instead.
            if is_shell_sidecar(sess) || crate::provider::is_sidecar(sess) {
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

/// `#{pane_current_command}` names an interactive shell (`-zsh` for a login
/// shell): what an agent pane shows once the agent itself has gone.
pub fn is_shell_command(cmd: &str) -> bool {
    let base = cmd.rsplit('/').next().unwrap_or(cmd).trim_start_matches('-');
    matches!(base, "zsh" | "bash" | "sh" | "fish" | "dash" | "ksh" | "tcsh" | "csh" | "nu" | "elvish" | "xonsh")
}

pub fn canonical_provider(name: &str) -> &'static crate::provider::Provider {
    PaneList::fetch().map(|p| p.canonical_provider(name)).unwrap_or(crate::provider::CLAUDE)
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

/// The pane in `session` that is the place's agent, as a `%id`: a pane of THAT
/// session, whose current path is the place (`place_path`, or under it — but
/// never under `exclude_under`, which the main checkout passes as its
/// `.worktrees/` root), and whose command is the agent (`is_ai_command`).
///
/// All three, because a session NAME alone proves little: two clones of one
/// repo share a prefix, so `<prefix>-feat~agent~codex` can belong to the other
/// clone; and a session whose codex exited can have a split pane running vim,
/// which "the first program pane" would have typed into. No fallback — no
/// match is a refusal.
pub fn agent_pane(session: &str, place_path: &str, exclude_under: Option<&str>, ai_word: &str) -> Option<PaneId> {
    let target = format!("={session}");
    let o = tmux(&[
        "list-panes",
        "-t",
        &target,
        "-F",
        "#{pane_id}\t#{pane_current_path}\t#{pane_current_command}",
    ])
    .ok()?;
    if !o.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let rows: Vec<(&str, &str, &str)> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            Some((it.next()?, it.next()?, it.next()?))
        })
        .collect();
    pick_agent_pane(&rows, place_path, exclude_under, ai_word).map(|id| PaneId(id.to_string()))
}

/// `agent_pane`'s rule, pure: `(pane_id, path, command)` rows of one session.
fn pick_agent_pane<'a>(
    rows: &[(&'a str, &str, &str)],
    place_path: &str,
    exclude_under: Option<&str>,
    ai_word: &str,
) -> Option<&'a str> {
    let under = |p: &str, dir: &str| p == dir || p.starts_with(&format!("{dir}/"));
    rows.iter()
        .find(|(id, path, cmd)| {
            id.starts_with('%')
                && under(path, place_path)
                && !exclude_under.is_some_and(|x| under(path, x))
                && is_ai_command(cmd, ai_word)
        })
        .map(|(id, _, _)| *id)
}

/// `send-keys` argv that types `text` LITERALLY into `pane`. `-l` makes every
/// character literal (no key names) and `--` ends option parsing, so text that
/// begins with `-` is not a flag.
///
/// tmux's own command parser still eats a TRAILING `;` as a command separator
/// — even after `-l --` (measured) — so a message ending in one lost it. A
/// trailing `\;` is tmux's escape for a literal semicolon there.
pub fn send_literal_args(pane: &str, text: &str) -> Vec<String> {
    let text = match text.strip_suffix(';') {
        Some(head) => format!("{head}\\;"),
        None => text.to_string(),
    };
    leave_mode(pane)
        .into_iter()
        .chain(["send-keys", "-t", pane, "-l", "--"])
        .map(|s| s.to_string())
        .chain(std::iter::once(text))
        .collect()
}

/// The head of every write into an agent's pane: leave copy-mode first. A pane
/// the user scrolled back in the app sits in copy-mode, where `send-keys` input
/// is eaten outright and a paste lands hidden behind the history being read
/// (both measured on tmux 3.7c). `-q` exits 0 with no mode to leave, which
/// matters because a `;` list stops at the first failing command. Part of the
/// SAME invocation, so nothing can re-enter the mode in between.
fn leave_mode(pane: &str) -> [&str; 5] {
    ["copy-mode", "-q", "-t", pane, ";"]
}

fn run_ok(args: &[&str]) -> Result<(), String> {
    let o = tmux(args).map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// Type `text` into `pane` as KEYSTROKES, without Enter — `send-keys -l`, the
/// opposite choice from `paste_to_ai`, on purpose: this is a message an agent
/// is meant to act on as if typed, and Codex's composer queues typed input
/// mid-turn. The caller presses Enter separately (`press_enter`), after a
/// pause and a fresh look at the screen, so a TUI's paste-burst detection does
/// not read it as a newline and a modal that appeared meanwhile is not
/// answered by it.
pub fn send_literal(pane: &PaneId, text: &str) -> Result<(), String> {
    let args = send_literal_args(pane.as_str(), text);
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    run_ok(&argv)
}

pub fn press_enter(pane: &PaneId) -> Result<(), String> {
    run_ok(&press_enter_args(pane.as_str()))
}

fn press_enter_args(pane: &str) -> Vec<&str> {
    leave_mode(pane).into_iter().chain(["send-keys", "-t", pane, "Enter"]).collect()
}

/// The visible screen of `pane`, or `None` when tmux cannot say.
pub fn capture(pane: &PaneId) -> Option<String> {
    let o = tmux(&["capture-pane", "-p", "-t", pane.as_str()]).ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
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
        .ok_or_else(|| format!("no {ai_word} running in session {session} (panes: {})", pane_summary(session)))?;
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
        leave_mode(pane.as_str()).into_iter().chain(["paste-buffer", "-b", buf, "-p", "-t", pane.as_str()]).collect(),
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

    /// A pane the user scrolled back in the app sits in copy-mode, which eats
    /// `send-keys` input outright and leaves a paste hidden behind history
    /// (both measured). Every write into an agent's pane leaves the mode first,
    /// in the SAME tmux invocation so nothing can re-enter it in between, and
    /// with `-q`, which exits 0 when there is no mode to leave — a `;` list
    /// stops at the first failing command.
    #[test]
    fn a_send_leaves_copy_mode_first() {
        let pane = PaneId("%7".into());
        let leave = ["copy-mode", "-q", "-t", "%7", ";"];
        let [_, paste, _] = paste_commands("buf1", &pane, "x");
        assert_eq!(paste[..5], leave, "{paste:?}");
        assert_eq!(paste[5], "paste-buffer");
        let typed = send_literal_args("%7", "run the tests");
        assert_eq!(typed[..5], leave, "{typed:?}");
        assert_eq!(typed[5], "send-keys");
        let enter = press_enter_args("%7");
        assert_eq!(enter[..5], leave, "{enter:?}");
        assert_eq!(enter[5..], ["send-keys", "-t", "%7", "Enter"]);
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
    let o = tmux_launch(&["new-session", "-d", "-s", session, "-c", wt, "-P", "-F", "#{pane_id}", pane0])
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

/// Whether an attach/switch may happen at all: only for a person at a terminal.
/// An agent's shell has no tty on stdin/stdout, yet inherits `$TMUX` from its
/// pane; a bare `switch-client` there resolves to whichever client tmux deems
/// current for that session, which is the APP's embedded one, and the lane
/// took over the user's view. No `-c` is added for the interactive case: with
/// a tty the caller is a human in their own client, and tmux's pick is theirs.
pub fn may_attach(stdin_tty: bool, stdout_tty: bool) -> bool {
    stdin_tty && stdout_tty
}

/// Attach (or switch-client if already in tmux). stdio inherited so the tty
/// reaches tmux; failure ignored. Returns false (doing nothing) when there is
/// no interactive terminal, so the caller can print the detached line.
pub fn attach_or_switch(session: &str) -> bool {
    use std::io::IsTerminal;
    use std::process::Command;
    if !may_attach(std::io::stdin().is_terminal(), std::io::stdout().is_terminal()) {
        return false;
    }
    let in_tmux = std::env::var("TMUX").map(|v| !v.is_empty()).unwrap_or(false);
    let sub = if in_tmux { "switch-client" } else { "attach" };
    let _ = Command::new("tmux").args(no_start_args()).args([sub, "-t", session]).status();
    true
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

    #[test]
    fn no_start_is_offered_only_from_tmux_3_2() {
        assert!(supports_no_start("tmux 3.7c"));
        assert!(supports_no_start("tmux 3.7c\n"));
        assert!(supports_no_start("tmux 3.2a"));
        assert!(supports_no_start("tmux 3.2"));
        assert!(supports_no_start("tmux 4.0"));
        assert!(supports_no_start("tmux 3.10"), "minor compared as a number, not text");
        assert!(supports_no_start("tmux next-3.6"));
        assert!(supports_no_start("tmux master"));
        assert!(!supports_no_start("tmux 3.1c"));
        assert!(!supports_no_start("tmux 2.9a"));
        assert!(!supports_no_start("tmux 1.9"));
        assert!(!supports_no_start("tmux next-3.1"));
        // Unknown means today's behaviour, never a flag an old tmux rejects.
        assert!(!supports_no_start(""));
        assert!(!supports_no_start("tmux"));
        assert!(!supports_no_start("tmux openbsd-7.4"));
        assert!(!supports_no_start("garbage 3.7"));
        assert!(!supports_no_start("tmux 3"));
        assert!(!supports_no_start("tmux x.y"));
    }

    #[test]
    fn only_a_refused_socket_delays_a_launch() {
        assert!(is_refusal("no server running on /private/tmp/tmux-501/default\n"));
        assert!(!is_refusal("error connecting to /private/tmp/tmux-501/x (No such file or directory)\n"));
        assert!(!is_refusal(""));
    }

    #[test]
    fn only_a_person_at_a_terminal_may_attach() {
        assert!(may_attach(true, true));
        assert!(!may_attach(false, true), "an agent's stdin is not a tty");
        assert!(!may_attach(true, false), "piped output is not a person watching");
        assert!(!may_attach(false, false));
    }

    #[test]
    fn provider_session_names_are_persistent() {
        for (provider, owner, sidecar, expected) in [
            (crate::provider::CLAUDE, "claude", false, "repo-feature"),
            (crate::provider::CLAUDE, "claude", true, "repo-feature~agent~claude"),
            (crate::provider::CLAUDE, "codex", false, "repo-feature~agent~claude"),
            (crate::provider::CODEX, "claude", false, "repo-feature~agent~codex"),
            (crate::provider::CODEX, "codex", false, "repo-feature"),
            (crate::provider::CODEX, "codex", true, "repo-feature"),
        ] {
            assert_eq!(provider.session_name("repo-feature", owner, sidecar), expected);
        }
        assert_eq!(crate::provider::CODEX.sidecar_name("repo-feature"), "repo-feature~agent~codex");
        assert_eq!(crate::provider::CLAUDE.sidecar_name("repo-feature"), "repo-feature~agent~claude");
        let panes = pl(&[("repo-feature", "/repo", "codex")]);
        assert_eq!(crate::activity::codex_session_for(&panes, "repo-feature"), "repo-feature");
        let panes = pl(&[("repo-feature", "/repo", "claude")]);
        assert_eq!(crate::activity::codex_session_for(&panes, "repo-feature"), "repo-feature~agent~codex");
    }

    #[test]
    fn the_agent_pane_must_be_in_the_place_and_running_the_agent() {
        let w = "/r/.worktrees/feat";
        assert_eq!(pick_agent_pane(&[("%1", w, "zsh"), ("%2", w, "codex")], w, None, "codex"), Some("%2"));
        assert_eq!(pick_agent_pane(&[("%1", "/r/.worktrees/feat/src", "codex")], w, None, "codex"), Some("%1"));
        // A split window running vim after codex exited: no agent, no pane.
        assert_eq!(pick_agent_pane(&[("%1", w, "zsh"), ("%2", w, "vim")], w, None, "codex"), None);
        // Same session NAME, another clone's path: not ours.
        assert_eq!(pick_agent_pane(&[("%1", "/other/.worktrees/feat", "codex")], w, None, "codex"), None);
        assert_eq!(pick_agent_pane(&[("%1", "/r/.worktrees/feature", "codex")], w, None, "codex"), None);
        // The main checkout does not own panes in its worktrees.
        assert_eq!(pick_agent_pane(&[("%1", w, "codex")], "/r", Some("/r/.worktrees"), "codex"), None);
        assert_eq!(pick_agent_pane(&[("0", w, "codex")], w, None, "codex"), None, "only %N ids enter a PaneId");
    }

    #[test]
    fn a_trailing_semicolon_is_escaped_for_tmux() {
        let a = send_literal_args("%3", "run the tests;");
        assert_eq!(a[a.len() - 6..], ["send-keys", "-t", "%3", "-l", "--", "run the tests\\;"]);
        assert_eq!(send_literal_args("%3", "a;b").last().unwrap(), "a;b", "only the TRAILING one is eaten");
        assert_eq!(send_literal_args("%3", "-x").last().unwrap(), "-x");
    }

    fn pl(rows: &[(&str, &str, &str)]) -> PaneList {
        PaneList::from_rows(rows.iter().map(|(s, p, c)| (s.to_string(), p.to_string(), c.to_string())).collect())
    }

    /// A codex session is `codex …; exec "$SHELL"`: once codex dies the
    /// session stays up with a bare shell, and must stop reading as a running
    /// agent. `node` (npm's codex) and `codex` both count as running.
    #[test]
    fn a_session_left_with_only_a_shell_runs_no_program() {
        let list = pl(&[
            ("p~agent~codex", "/wt/p", "zsh"),
            ("q~agent~codex", "/wt/q", "codex"),
            ("r~agent~codex", "/wt/r", "node"),
            ("s~agent~codex", "/wt/s", "-zsh"),
        ]);
        assert!(!list.session_runs_program("p~agent~codex"));
        assert!(list.session_runs_program("q~agent~codex"));
        assert!(list.session_runs_program("r~agent~codex"));
        assert!(!list.session_runs_program("s~agent~codex"), "a login shell is a shell");
        assert!(!list.session_runs_program("missing"));
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

    /// npm's codex (and any node-based harness) reports `node`. In a provider
    /// sidecar the session name says whose it is; before `for_pane` looked at
    /// it, both of these read as Claude.
    #[test]
    fn agents_in_reads_a_node_pane_by_its_sidecar() {
        let list = pl(&[
            ("repo-feat~agent~codex", "/wt/feat", "node"),
            ("repo-feat~agent~pi", "/wt/feat", "node"),
            ("repo-feat~agent~opencode", "/wt/feat", "node"),
            ("repo-feat", "/wt/feat", "node"),
        ]);
        assert_eq!(list.agents_in("/wt/feat", None), vec![
            ("repo-feat~agent~codex".into(), "codex"),
            ("repo-feat~agent~pi".into(), "pi"),
            ("repo-feat".into(), "claude"),
        ]);
    }

    /// Adoption finds a place's CANONICAL session by pane cwd. A provider
    /// sidecar is never it — including one for a harness this build does not
    /// know, which runs `node` and would otherwise be the preferred match.
    #[test]
    fn session_in_never_adopts_a_sidecar_known_or_not() {
        let list = pl(&[
            ("repo-feat~agent~opencode", "/wt/feat", "node"),
            ("repo-feat~agent~pi", "/wt/feat", "node"),
            ("repo-feat-hand", "/wt/feat", "zsh"),
        ]);
        assert_eq!(list.session_in("/wt/feat", "claude", None).as_deref(), Some("repo-feat-hand"));
        let only = pl(&[("repo-feat~agent~opencode", "/wt/feat", "node")]);
        assert_eq!(only.session_in("/wt/feat", "claude", None), None);
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

    /// What `ps -A -o tty=,pid=,tpgid=,comm=` printed on macOS, with pi typed
    /// into a zsh on ttys009 (captured 2026-10-02): the shell's own row has
    /// tpgid = pi's pid, a detached daemon has `??`, a comm can be a path or
    /// carry spaces.
    const PS_MACOS: &str = "\
??           1     1 /sbin/launchd
ttys009  84255 94379 /bin/zsh
ttys009  94379 94379 pi
ttys000  78177 84274 /bin/zsh
ttys000  84274 84274 claude
ttys000  84388 84274 npm exec chrome-devtools-mcp@latest
ttys004  50001 50001 -zsh
";
    /// The same shape from procps on Linux: `pts/N`, `?` for no tty.
    const PS_LINUX: &str = "\
?            1     -1 systemd
pts/3     2100   2240 bash
pts/3     2240   2240 pi
pts/5     3001   3007 bash
pts/5     3007   3007 npm exec x
";

    #[test]
    fn the_foreground_leader_is_read_from_both_platforms_ps() {
        let mac = parse_foreground(PS_MACOS);
        assert_eq!(mac.get("ttys009").map(String::as_str), Some("pi"), "the shell under pi is not the leader");
        assert_eq!(mac.get("ttys000").map(String::as_str), Some("claude"));
        assert_eq!(mac.get("ttys004").map(String::as_str), Some("zsh"), "a login shell's dash is not part of the word");
        let linux = parse_foreground(PS_LINUX);
        assert_eq!(linux.get("pts/3").map(String::as_str), Some("pi"));
        assert_eq!(linux.get("pts/5").map(String::as_str), Some("npm"), "a comm with spaces is its first word");
        assert!(parse_foreground("").is_empty());
        assert!(parse_foreground("ttys1 x x pi\nttys2 7\n").is_empty(), "a malformed row names nothing");
    }

    /// tmux spells a tty `/dev/ttys009`, ps `ttys009` — the first cut of this
    /// compared them raw, and a live pi stayed Claude.
    #[test]
    fn a_tty_is_compared_the_way_ps_spells_it() {
        assert_eq!(normalize_tty("/dev/ttys009"), "ttys009");
        assert_eq!(normalize_tty("/dev/pts/3"), "pts/3");
        assert_eq!(normalize_tty("ttys009"), "ttys009");
        let rows = parse_pane_rows("s\t/w\tnode\t9\t/dev/ttys009");
        assert_eq!(rows[0].tty.as_deref(), Some("ttys009"));
        assert_eq!(rows[0].tty.as_deref().and_then(|t| parse_foreground(PS_MACOS).get(t).cloned()).as_deref(), Some("pi"));
    }

    #[test]
    fn pane_rows_parse_with_three_four_or_five_fields() {
        let rows = parse_pane_rows("a\t/w/a\tzsh\nb\t/w/b\tnode\t42\nc\t/w/c\tnode\t43\t/dev/pts/7\nd\t/w/d\tvim\t\t\n");
        let pane = |session: &str, path: &str, cmd: &str, pid: Option<&str>, tty: Option<&str>| Pane {
            session: session.into(),
            path: path.into(),
            cmd: cmd.into(),
            pid: pid.map(Into::into),
            tty: tty.map(Into::into),
            fg: None,
        };
        assert_eq!(
            rows,
            vec![
                pane("a", "/w/a", "zsh", None, None),
                pane("b", "/w/b", "node", Some("42"), None),
                pane("c", "/w/c", "node", Some("43"), Some("pts/7")),
                pane("d", "/w/d", "vim", None, None),
            ]
        );
        let list = PaneList::from_panes(rows);
        assert_eq!(list.session_launch("b"), Some("42"));
        assert_eq!(list.session_launch("a"), None);
    }

    /// One `ps` per snapshot, and only when a `node` pane outside a sidecar
    /// needs naming — a sidecar's name already says whose it is, and the app
    /// fetches a snapshot every 3s.
    #[test]
    fn ps_runs_once_and_only_for_a_node_pane_a_name_does_not_explain() {
        let run = |text: &str| {
            let mut panes = parse_pane_rows(text);
            let mut calls = 0;
            let mut asked = Vec::new();
            resolve_foreground(&mut panes, |ttys| {
                calls += 1;
                asked = ttys.iter().map(|t| t.to_string()).collect();
                Some(PS_MACOS.to_string())
            });
            (calls, panes, asked)
        };
        assert_eq!(run("p\t/w\tzsh\t1\t/dev/ttys009\nq\t/w\t2.1.287\t2\t/dev/ttys000").0, 0, "no node pane, no ps");
        assert_eq!(run("p~agent~pi\t/w\tnode\t1\t/dev/ttys009").0, 0, "a sidecar names its harness");
        assert_eq!(run("p~term\t/w\tnode\t1\t/dev/ttys009").0, 0, "a dock shell is never an agent");
        assert_eq!(run("p\t/w\tnode\t1").0, 0, "no tty to look up");
        let (calls, panes, asked) = run("p\t/w\tnode\t1\t/dev/ttys009\nq\t/w\tnode\t2\t/dev/ttys000\nr\t/w\tzsh\t3\t/dev/ttys004\ns~agent~pi\t/w\tnode\t4\t/dev/ttys005\nt\t/w\tnode\t5\t/dev/ttys009");
        assert_eq!(calls, 1, "one ps for every pane that wants one");
        assert_eq!(asked, ["ttys000", "ttys009"], "only the wanted ttys, as ps spells them, once each");
        let fg: Vec<Option<&str>> = panes.iter().map(|p| p.fg.as_deref()).collect();
        assert_eq!(fg, [Some("pi"), Some("claude"), None, None, Some("pi")], "only the panes that wanted it");
    }

    /// The real `ps`: a tty that is gone fails `-t` outright (macOS: exit 1,
    /// no rows), so the lookup must fall back to the whole table rather than
    /// come back empty for every pane in the snapshot.
    #[test]
    fn a_vanished_tty_falls_back_to_the_whole_table() {
        let rows = ps_foreground(&["ttys-gone-999"]).unwrap_or_default();
        assert!(rows.lines().count() > 1, "fell back to ps -A: {rows:?}");
    }

    fn fg_pane(session: &str, path: &str, cmd: &str, fg: Option<&str>) -> Pane {
        Pane { session: session.into(), path: path.into(), cmd: cmd.into(), fg: fg.map(Into::into), ..Pane::default() }
    }

    /// pi typed into a place's own session: tmux says `node`, the tty's
    /// foreground leader says `pi`. Before, the `node` heuristic handed it to
    /// Claude, and every reader (dots, place_status, wait, send, `open --ai
    /// pi`) looked for it in a `~agent~pi` that does not exist.
    #[test]
    fn a_node_pane_whose_foreground_is_pi_is_pi() {
        let list = PaneList::from_panes(vec![fg_pane("repo-feat", "/w/feat", "node", Some("pi"))]);
        assert_eq!(list.canonical_provider("repo-feat").id, "pi");
        assert_eq!(list.agents_in("/w/feat", None), vec![("repo-feat".to_string(), "pi")]);
        assert_eq!(list.program_in("repo-feat"), Some("pi"));
        assert_eq!(crate::activity::pi_session_for(&list, "repo-feat"), "repo-feat");
        assert_eq!(crate::activity::codex_session_for(&list, "repo-feat"), "repo-feat~agent~codex");
        // Unresolved (no ps answer) — the old reading, unchanged.
        let list = PaneList::from_panes(vec![fg_pane("repo-feat", "/w/feat", "node", None)]);
        assert_eq!(list.canonical_provider("repo-feat").id, "claude");
        assert_eq!(list.agents_in("/w/feat", None), vec![("repo-feat".to_string(), "claude")]);
        assert_eq!(crate::activity::pi_session_for(&list, "repo-feat"), "repo-feat~agent~pi");
    }

    /// A leader that names no harness leaves the pane to the old heuristic:
    /// an npx-started Claude leads with `npm`, and must not stop being Claude.
    /// And a leader only counts on a wrapper pane — tmux's own word wins
    /// everywhere else.
    #[test]
    fn a_foreground_leader_counts_only_when_it_names_a_harness_on_a_wrapper() {
        let list = PaneList::from_panes(vec![fg_pane("s", "/w", "node", Some("npm"))]);
        assert_eq!(list.agents_in("/w", None), vec![("s".to_string(), "claude")]);
        assert_eq!(list.program_in("s"), Some("node"));
        let list = PaneList::from_panes(vec![fg_pane("s", "/w", "codex", Some("pi"))]);
        assert_eq!(list.canonical_provider("s").id, "codex");
    }
}

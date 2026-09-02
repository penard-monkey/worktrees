//! Write ops — new/co, switch, open, rm — ported 1:1 from the bash cmd_* funcs.
//! Each `cmd_*` parses its raw args, emits every message via `Ui`, and returns
//! an exit code (guards → 1). git/tmux are shelled out. The bats suite gates
//! this against the bash CLI byte-for-byte.

use std::path::Path;

use crate::config::sanitize_prefix;
use crate::diag::{Code, Finding, Report, Severity};
use crate::git;
use crate::init;
use crate::materialize;
use crate::projcfg::{self, ProjectConfig};
use crate::provision::{self, Outcome, ProvisionError, ENV_FILE};
use crate::tmux;
use crate::ui::{fmt, Ui};
use crate::Project;

fn basename(p: &str) -> String {
    Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}
fn slugify(s: &str) -> String {
    s.replace('/', "-")
}
fn strip_origin(s: &str) -> &str {
    s.strip_prefix("origin/").unwrap_or(s)
}
/// Indent every line by 4 spaces (bash `sed 's/^/    /'`).
fn indent(s: &str) -> String {
    s.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
}

// ── session adoption — the prefix is not a stable identity ───────────────────
//
// `session_name` is `<prefix>-<slug>`, and since §5's `[project] prefix` a repo
// can CHANGE the prefix by committing a config. Nothing about a running tmux
// session changes when it does: `close`, `rm` and `ls` would look up a name that
// no live session answers to, report "nothing to close" for a session that is
// very much alive, and leave it orphaned when the worktree is deleted.
//
// The fix records nothing. A session is found by PANE CWD — `PaneList::session_in`
// (tmux.rs:106) already does exactly that for `open`/`ls` — because the cwd of a
// running process is a fact no config edit can invalidate, while any name the
// tool wrote down would be one more thing to keep in sync with the truth (and
// would go missing exactly when `.worktrees.places.json` is lost, which that
// store's contract says is safe). New sessions get the new name; a session
// started under the old one is adopted until it is closed, and `doctor` reports
// the mismatch so the drift is visible rather than mysterious.

/// The tmux session actually LIVING in `wt`, whatever it is named: the canonical
/// name when it exists, else any session with a pane cwd'd there.
///
/// `slug == "(main)"` must exclude `.worktrees/` — worktree dirs nest under the
/// main root, so without it main would adopt (and `close`/`rm` would kill) a
/// worktree's session. Same exclusion `launch` and `place_json` apply.
///
/// `panes` is an optional PREFETCHED `list-panes -a`. Sweeps that ask about many
/// places (`doctor`) pass one and pay a single tmux shell-out for the whole run
/// instead of up to three per place; one-shot callers (`close`, `rm`) pass
/// `None` and it fetches. One function either way, because a finding that
/// resolved sessions by a different rule than `close` could name a session
/// `close` would fail to find.
fn live_session(p: &Project, slug: &str, wt: &str, panes: Option<&tmux::PaneList>) -> Option<String> {
    let canonical = p.session_name(slug);
    let exists = match panes {
        Some(pl) => pl.has_session(&canonical),
        None => tmux::session_exists(&canonical),
    };
    if exists {
        return Some(canonical);
    }
    let exclude = if slug == "(main)" { Some(p.wt_root_dir()) } else { None };
    let ai_word = crate::project::adopt_ai_word();
    match panes {
        Some(pl) => pl.session_in(wt, &ai_word, exclude),
        None => tmux::worktree_session_excluding(wt, &ai_word, exclude),
    }
}

/// THE seam where a profile turns a bare `ai_cmd` into the real launch shape.
///
/// One function, called by every launch path (`new`, `open`, and the app's
/// direct main-checkout launch), so the CLI and the app cannot diverge on which
/// profile a place gets — a `worktrees open` from a bare terminal must produce
/// the same session the app would, or an unprofiled CLI session gets silently
/// ADOPTED by the app and displayed as though it were profiled.
///
/// Phase 4 is deliberately behaviour-preserving: it returns the plain launch and
/// leaves the env/flag composition to the claude adapter. What it establishes is
/// that `env` and `match_word` travel BESIDE the command instead of being parsed
/// back out of it.
pub fn ai_launch_for(p: &Project, ui: &mut dyn Ui, wt: &str, ai_cmd: &str) -> crate::profile::AiLaunch {
    let plain = crate::profile::AiLaunch::plain(ai_cmd);
    // Reads the same flag the PROBE side reads, so the two cannot get out of
    // step — `claude_config_dir_for_repo` returns `~/.claude` while this is off.
    if !crate::profile::launch_honors_profiles() {
        return plain;
    }
    // `ai_cmd = none` (plain shell), or a different AI tool. The profile model
    // is tool-agnostic but the recipe is claude's; anything else launches
    // exactly as it does today rather than being handed flags it never had.
    if plain.cmd.is_empty() || plain.match_word != "claude" {
        return plain;
    }
    let Some(prof) = crate::profile::resolve_profile(&p.main_root) else {
        return plain;
    };
    match crate::profile::materialize(&prof, wt, &p.main_root) {
        Ok(m) => {
            // Never swallowed: a skipped skill or a missing worktrees binary
            // changes what the session can do, so the user has to see it.
            for w in &m.warnings {
                ui.warn(&format!("profile '{}': {w}", prof.name));
            }
            crate::profile::claude_launch(&plain, &prof, &m)
        }
        Err(e) => {
            // FAIL CLOSED. Launching unprofiled claude here would be the worst
            // outcome available: a profile is frequently RESTRICTIVE — it is how
            // `--strict-mcp-config` removes a dangerous global server, and where
            // settings deny tools — so "couldn't apply your profile" must never
            // silently mean "ran without your restrictions". Every visible signal
            // (a pane running claude, the activity dots) would have said the
            // profile applied.
            //
            // It also keeps the probe/launch seam honest: an unprofiled fallback
            // writes its conversation to ~/.claude while the probes keep reading
            // the profile dir, so auto-resume would lose it — exactly the
            // divergence `launch_honors_profiles` exists to prevent.
            //
            // The pane still opens, on a plain shell with the reason printed, so
            // nothing is stranded and the user can run claude by hand if they
            // want it anyway.
            let msg = format!(
                "worktrees: profile '{}' could not be prepared: {e}\\nNOT launching claude — your profile's rules and MCP settings are not in effect.\\nFix the profile (or unset it) and reopen; run `claude` here to start an unprofiled session anyway.",
                prof.name
            );
            ui.error(&format!("profile '{}' could not be prepared ({e}) — claude not launched", prof.name));
            crate::profile::AiLaunch {
                profile: None,
                env: Vec::new(),
                cmd: format!("printf '%s\\n' {} >&2", crate::profile::shell_quote(&msg)),
                match_word: plain.match_word,
                opener: None,
            }
        }
    }
}

// ── (re)open a worktree's tmux session, then attach ──────────────────────────
// Returns 0 on success (session live / adopted / attached), 1 when tmux refuses
// to create the session (the reason is surfaced via ui.error — the app shows it
// and the CLI exits nonzero, instead of the old silent "Session ready" lie).
pub fn launch(p: &Project, ui: &mut dyn Ui, wt: &str, session_in: &str, install_cmd: &str, ai: &crate::profile::AiLaunch, do_attach: bool, spare_shell: bool) -> i32 {
    let keep = "exec \"${SHELL:-/bin/sh}\"";
    // The word tmux adoption matches on. It comes from the RESOLVED PROGRAM, not
    // from the string we are about to build: a profiled launch prefixes
    // `CLAUDE_CONFIG_DIR=… ` onto that string, and deriving the word from it
    // would yield `CLAUDE_CONFIG_DIR=…`. `session_in` substring-matches this
    // against `pane_current_command`, so getting it wrong does not error — it
    // silently downgrades adoption to "first pane in the worktree" and switches
    // the app's auto-resume off.
    let ai_word = ai.match_word.clone();
    let ai_cmd = ai.cmd.as_str();
    let mut session = session_in.to_string();
    if !tmux::session_exists(&session) {
        // Adopting MAIN must skip panes under `.worktrees/` — worktree dirs nest
        // inside the main root, so without the exclusion opening main could
        // adopt (and attach to) a worktree's session instead of creating main's.
        let exclude = if wt == p.main_root { Some(p.wt_root.as_str()) } else { None };
        if let Some(existing) = tmux::worktree_session_excluding(wt, &ai_word, exclude) {
            session = existing;
        }
    }
    if tmux::session_exists(&session) {
        // INFO, not warn. Finding the session already up is the normal, healthy
        // outcome of reopening a place — it is what a durable place IS — so it is
        // not something the user has to act on. At Warn severity it rode out
        // through `CaptureUi::warnings()` and `run_op` logged it on EVERY app
        // `open` (that path logs warnings even for rc=0), which is the whole of
        // what made the app log unreadable.
        //
        // The tail follows `do_attach` because only one of the two callers
        // attaches. The app passes false and embeds the session in its own PTY —
        // "— attaching." was simply not true there.
        let tail = if do_attach { "attaching." } else { "reusing it." };
        ui.info(&format!("tmux session '{session}' already in this worktree — {tail}"));
    } else {
        ui.header(&format!("Opening tmux session '{session}'"));
        // Env assignments go INSIDE the -ic string, ahead of the command, rather
        // than through `tmux new-session -e`: env baked into the process survives
        // detach, reattach and a tmux server restart by definition, and the bats
        // fake-tmux shim parses argv positionally with no `-e` case. Assignments
        // first also keeps the program itself as pane0's foreground process, so
        // `pane_current_command` stays `claude`.
        let pane0 = if !ai_cmd.is_empty() {
            format!("exec \"${{SHELL:-/bin/sh}}\" -ic {}", tmux::sq(&ai.pane0_body_for(keep, &session)))
        } else {
            keep.to_string()
        };
        // The spare shell is a SECOND pane next to pane0 (AI). The CLI keeps it
        // (it's where deps install; `new` and `open` both split by default,
        // unless `--no-spare`). The app opens
        // single-pane (`spare_shell=false`) so Claude gets full width — its
        // scratch shell lives in the right dock's Terminal tab instead. An
        // install_cmd is only ever passed WITH the spare shell.
        let pane1 = if !install_cmd.is_empty() {
            format!("{install_cmd} && echo '✓ deps ready'; {keep}")
        } else {
            keep.to_string()
        };
        match tmux::new_session(&session, wt, &pane0) {
            Ok(pid) => {
                tmux::tune_session(&session);
                // Stamp WHICH profile this session started with — including
                // "none at all". Only here, in the branch that actually creates a
                // session: an attach reuses whatever the running process already
                // loaded, so stamping there would clear a legitimate "restart to
                // apply" badge.
                //
                // Writing the CLEARED case matters as much as the set one. A
                // stamp that is only ever written and never cleared outlives the
                // binding: unbind a profile, relaunch, and the badge would keep
                // naming a profile the session is demonstrably not running — the
                // badge lying is the exact failure this feature exists to avoid.
                let slug = if wt == p.main_root { "(main)".to_string() } else { basename(wt) };
                let stamp = ai.profile.clone();
                // Skip the write entirely when there is nothing to record and
                // nothing to clear. Otherwise every unprofiled CLI launch would
                // rewrite `.worktrees.places.json` — bumping updated_epoch and
                // creating empty entries — for people who never use profiles.
                let already_clear = stamp.is_none()
                    && crate::store::read_lenient(&p.main_root)
                        .places
                        .get(&slug)
                        .map(|d| d.profile_id.is_none() && d.profile_epoch.is_none())
                        .unwrap_or(true);
                if already_clear {
                    // nothing to do
                } else if let Err(e) = crate::store::edit(&p.main_root, &slug, |d| match &stamp {
                    Some((id, epoch)) => {
                        d.profile_id = Some(id.clone());
                        d.profile_epoch = Some(*epoch);
                    }
                    None => {
                        d.profile_id = None;
                        d.profile_epoch = None;
                    }
                }) {
                    // Not fatal — the session is up — but silence here means the
                    // badge simply never appears, with nothing to explain why.
                    ui.warn(&format!("could not record which profile this session started with: {e}"));
                }
                if spare_shell {
                    tmux::split_window(&pid, wt, &pane1);
                    tmux::select_pane(&pid);
                }
            }
            Err(reason) => {
                // Loud-guard: a failed new-session must NOT read as "ready". The
                // app surfaces this in the error banner + app.log; the CLI exits
                // nonzero. (Fake shims always succeed → bats never sees this path.)
                ui.error(&format!("Failed to open tmux session '{session}': {reason}"));
                return 1;
            }
        }
    }
    if !do_attach {
        ui.info(&format!("Session ready (detached). Attach with: tmux attach -t {session}"));
        return 0;
    }
    tmux::attach_or_switch(&session);
    0
}

// ── do_switch — move a registered worktree to another branch (DWIM) ──────────
// Ok(()) on success/no-op; Err(code) on a guard failure (message already printed).
pub fn do_switch(p: &Project, ui: &mut dyn Ui, wt: &str, branch: &str, base: Option<&str>, do_fetch: bool, force: bool) -> Result<(), i32> {
    let cur = p.wt_branch(wt);
    if cur == branch {
        ui.info(&format!("Already on '{branch}' — nothing to do."));
        return Ok(());
    }
    let dirty = p.wt_dirty(wt);
    if !dirty.is_empty() && !force {
        ui.warn(&format!("Worktree '{}' has uncommitted changes:", basename(wt)));
        ui.plain(&indent(&dirty));
        ui.error("Refusing to switch. Commit/stash, or pass --force (git will still refuse on conflicts).");
        return Err(1);
    }
    let main = &p.main_root;
    // `reason` is git's own captured stderr — surface it FIRST so the real cause
    // (the app's inherited stderr otherwise vanishes into the launchd void)
    // replaces the generic "checked out elsewhere?" guess.
    let switch_fail = |ui: &mut dyn Ui, reason: &str| -> Result<(), i32> {
        ui.error(reason);
        ui.error("git switch failed. If the branch is checked out in another worktree:");
        if let Some(list) = git::git_out(main, &["worktree", "list"]) {
            ui.plain(&indent(&list));
        }
        Err(1)
    };

    if git::git_ok(main, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]) {
        ui.info("Branch exists locally — switching.");
        if let Err(e) = git::git_status_captured(wt, &["switch", branch]) {
            return switch_fail(ui, &e);
        }
    } else {
        if do_fetch && !git::git_ok(main, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Fetching origin/{branch}..."));
            let _ = git::git(main, &["fetch", "--quiet", "origin", &format!("refs/heads/{branch}:refs/remotes/origin/{branch}")]);
        }
        if git::git_ok(main, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Tracking remote branch origin/{branch}."));
            if let Err(e) = git::git_status_captured(wt, &["switch", branch]) {
                return switch_fail(ui, &e);
            }
        } else {
            let base = base.map(|s| s.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| p.default_base());
            if do_fetch {
                let _ = git::git(main, &["fetch", "--quiet", "origin", &base]);
            }
            let mut start = base.clone();
            if git::git_ok(main, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{base}")]) {
                start = format!("origin/{base}");
            }
            ui.info(&format!("Creating new branch '{branch}' off '{start}'."));
            if let Err(e) = git::git_status_captured(wt, &["switch", "-c", branch, &start]) {
                return switch_fail(ui, &e);
            }
        }
    }
    ui.info(&format!("was '{cur}' → now '{branch}'. Session and deps untouched — keep working."));
    Ok(())
}

// ── new / co ─────────────────────────────────────────────────────────────────

/// Where a place's brief lives, relative to the worktree. `.planning/` is the
/// planning-with-files working dir every repo of ours already gitignores, and
/// the close-out ritual archives it — so the task an agent was given is kept
/// with the rest of that session's working memory.
pub const BRIEF_PATH: &str = ".planning/brief.md";
/// The prompt claude is launched on when a brief was written. Fixed text: the
/// brief itself never travels through argv, only this pointer to it does.
pub const BRIEF_OPENER: &str = "Read .planning/brief.md and begin.";

fn write_brief(wt: &str, text: &str) -> std::io::Result<()> {
    let path = Path::new(wt).join(BRIEF_PATH);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut body = text.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    std::fs::write(path, body)
}

pub fn cmd_new(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut do_install, mut do_tmux, mut do_attach, mut do_fetch, mut resume) = (true, true, true, true, false);
    let (mut branch, mut base, mut name, mut ai_flag) = (String::new(), String::new(), None::<String>, None::<String>);
    // `--brief <text>`: the agent's task, written to BRIEF_PATH in the new
    // worktree; claude then opens on BRIEF_OPENER. This is how an orchestrator
    // (the MCP `create_worktree` tool) hands a place its work.
    let mut brief: Option<String> = None;
    // Default keeps the CLI's spare shell (pane 1) — that's where deps install.
    // The app passes --no-spare so its embedded view is single-pane (Claude
    // full-width); deps install by hand in the dock's Terminal tab.
    let mut spare_shell = true;
    let mut expect = "";
    for arg in args {
        if !expect.is_empty() {
            // A brief is prose and is consumed WHOLE as the value — a markdown
            // list starts with `- `, and refusing it would refuse the most
            // natural brief there is. Nothing in it is ever read as a flag.
            if arg.starts_with('-') && expect != "brief" {
                ui.error(&format!("--{expect} needs a value (got '{arg}')"));
                return 1;
            }
            match expect {
                "name" => name = Some(arg.clone()),
                "ai" => ai_flag = Some(arg.clone()),
                "brief" => brief = Some(arg.clone()),
                _ => {}
            }
            expect = "";
            continue;
        }
        match arg.as_str() {
            "--no-install" => do_install = false,
            "--no-tmux" => do_tmux = false,
            "--no-attach" => do_attach = false,
            "--no-spare" => spare_shell = false,
            "--no-fetch" => do_fetch = false,
            "-r" | "--resume" => resume = true,
            "--name" => expect = "name",
            s if s.starts_with("--name=") => name = Some(s["--name=".len()..].to_string()),
            "--ai" => expect = "ai",
            s if s.starts_with("--ai=") => ai_flag = Some(s["--ai=".len()..].to_string()),
            "--brief" => expect = "brief",
            s if s.starts_with("--brief=") => brief = Some(s["--brief=".len()..].to_string()),
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => {
                if branch.is_empty() {
                    branch = s.to_string();
                } else if base.is_empty() {
                    base = s.to_string();
                } else {
                    ui.error(&format!("Too many args: {s}"));
                    return 1;
                }
            }
        }
    }
    if !expect.is_empty() {
        ui.error(&format!("--{expect} needs a value"));
        return 1;
    }
    if branch.is_empty() {
        ui.error("Branch name required.  e.g. worktrees new feat/foo");
        return 1;
    }
    let branch = strip_origin(&branch).to_string();
    // A freshly `git init`ed repo has an UNBORN HEAD: the branch ref exists in
    // name only, with no commit behind it, so every start-point is unresolvable
    // and `git worktree add` dies with "fatal: not a valid object name: 'main'".
    // Nothing downstream can recover, and git's message names neither the cause
    // nor the fix — so refuse here, with the one command that unblocks it.
    if !git::has_commits(&p.main_root) {
        ui.error("This repo has no commits yet — git cannot create a worktree off an unborn branch.");
        ui.error(&format!("Make the first commit, then retry:  git -C {} commit --allow-empty -m \"Initial commit\"", p.main_root));
        return 1;
    }
    if base.is_empty() {
        base = p.default_base();
    }
    if do_tmux && !tmux::have_tmux() {
        ui.warn("tmux not found — continuing with --no-tmux");
        do_tmux = false;
    }

    let mut slug = slugify(name.as_deref().unwrap_or(&branch));
    let mut wt = format!("{}/{}", p.wt_root_dir(), slug);

    if !Path::new(&wt).exists() {
        if let Some(holder) = p.wt_for_branch(&branch) {
            if name.is_some() {
                ui.error(&format!("Branch '{branch}' is already checked out in worktree '{}' — can't also put it in '{slug}'.", basename(&holder)));
                ui.error(&format!("Use: worktrees open {}   (or switch that worktree off the branch first)", basename(&holder)));
                return 1;
            }
            slug = basename(&holder);
            wt = holder;
            ui.info(&format!("Branch '{branch}' already lives in worktree '{slug}' — using that."));
        }
    }

    // §1.1 — either the tool fully provisions a project it recognizes, or it
    // refuses to create there. A config that does not PARSE is knowable before any
    // side effect (unlike a materialization failure, which needs the worktree to
    // exist), so it is refused here, ahead of `ensure_excluded` — the first thing
    // below that touches anything. Parsed once and reused: the apply step further
    // down takes this value rather than re-reading the file (§8's ordering for
    // that step is unchanged — it still runs after the git create block).
    let project_cfg = match load_project_config(p, ui) {
        Ok(c) => c,
        Err(code) => {
            ui.error("Refusing to create a worktree this repo's config cannot provision — fix .worktrees.toml first.");
            return code;
        }
    };

    let session = p.session_name(&slug);
    p.ensure_excluded();

    ui.header(&format!("Worktree for '{branch}'"));
    ui.info(&format!("repo: {}  (prefix: {})", p.main_root, p.prefix));
    ui.info(&format!("dir : {wt}"));

    let mut already = false;
    if p.is_registered(&wt) {
        already = true;
        let cur = p.wt_branch(&wt);
        if cur == branch {
            ui.warn(&format!("Worktree already exists on '{branch}' — reusing it (skipping create/install)."));
        } else {
            ui.warn(&format!("Worktree '{slug}' exists but is on '{cur}' — switching to '{branch}'."));
            if do_switch(p, ui, &wt, &branch, Some(&base), do_fetch, false).is_err() {
                return 1;
            }
        }
    } else if Path::new(&wt).exists() {
        ui.error(&format!("{wt} exists but is not a registered worktree. Remove it or pick another branch."));
        return 1;
    } else if git::git_ok(&p.main_root, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]) {
        ui.info(&format!("Branch '{branch}' exists locally — checking it out."));
        if let Err(e) = git::git_status_captured(&p.main_root, &["worktree", "add", &wt, &branch]) {
            ui.error(&e);
            ui.error(&format!("Failed to add worktree for '{branch}' at {wt}."));
            return 1;
        }
    } else {
        // ONE round trip, not two. This block has two questions for the remote —
        // "does origin/<branch> exist?" and "is origin/<base> current?" — and it
        // used to pay a separate fetch for each. Worse, the FIRST of them asked
        // for `refs/heads/<branch>` on a branch the user is typically inventing,
        // so it could not succeed: `fatal: couldn't find remote ref`, one full
        // network wait spent to learn nothing. Measured here, that pair was
        // ~1.6s of a 2.07s `new` — most of the delay that made the app look hung.
        //
        // A default `git fetch origin` answers both at once, and both `show-ref`
        // calls below then read purely local refs. Narrowing to be aware of: this
        // honours the repo's CONFIGURED refspec, so in a `--single-branch` clone
        // an unfetched remote branch no longer materializes the way the explicit
        // refspec forced it to. Creating a worktree for a branch such a clone was
        // deliberately set up not to see is the rarer case by far, and forcing
        // `+refs/heads/*:…` on every user to serve it would fetch refs those
        // clones exist to avoid.
        //
        // The guard is the SAME one the old first fetch carried, and keeping it is
        // not optional: when `origin/<branch>` is already on disk (a background
        // fetcher, a previous `new`, an `rm` that kept the tracking ref) the old
        // path took the tracking checkout below without touching the network at
        // all. Dropping it would turn that case from zero round trips into one —
        // and offline or on a flaky link, into a DNS/TCP timeout before producing
        // the identical worktree. Fewer round trips was the point; this keeps the
        // floor at zero and only collapses the 2 into 1.
        if do_fetch && !git::git_ok(&p.main_root, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info("Fetching origin...");
            let _ = git::git(&p.main_root, &["fetch", "--quiet", "origin"]);
        }
        if git::git_ok(&p.main_root, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Checking out remote branch origin/{branch} (tracking)."));
            if let Err(e) = git::git_status_captured(&p.main_root, &["worktree", "add", "--track", "-b", &branch, &wt, &format!("origin/{branch}")]) {
                ui.error(&e);
                ui.error(&format!("Failed to add tracking worktree for origin/{branch} at {wt}."));
                return 1;
            }
        } else {
            let mut start = base.clone();
            if git::git_ok(&p.main_root, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{base}")]) {
                start = format!("origin/{base}");
            }
            ui.info(&format!("Creating new branch '{branch}' off '{start}'."));
            if let Err(e) = git::git_status_captured(&p.main_root, &["worktree", "add", "-b", &branch, &wt, &start]) {
                ui.error(&e);
                ui.error(&format!("Failed to create branch '{branch}' off '{start}' at {wt}."));
                return 1;
            }
        }
    }

    // Materialize the declared files and allocate the port slot here — after the
    // worktree exists, BEFORE install detection and therefore before `launch`
    // starts pane 1 (§8). The ordering is not optional: pane 1 runs the install
    // command, and `pnpm install` racing `.env` into existence is exactly the
    // silent-failure class this fixes. The same argument covers ports — the tmux
    // panes must be able to see `.worktree.env`, since the repo's dev script
    // treats its absence as "not a worktree" (§1.1). A failure is a loud guard
    // like the session one below: report, keep the worktree, propagate the rc —
    // never roll back.
    let mat_rc = match project_cfg {
        None => {
            // §9's passive nudge, and the ONLY thing a config-less repo gains
            // here. Detection runs solely on this branch, so §2.4's
            // byte-identical guarantee still holds for every repo that has
            // nothing to link.
            hint_init(p, ui);
            0
        }
        Some((cfg, findings)) => {
            report_findings(ui, &findings);
            let files = materialize_place(p, ui, &cfg, &wt, false);
            let ports = provision_place(p, ui, &cfg, &wt, &slug, false);
            // Same precedence as the `--all` loops: a hard failure outranks
            // findings, whichever of the two steps produced it.
            worse_rc(files, ports)
        }
    };

    // The brief, once the worktree exists and its declared files are in place.
    // Written on a reused worktree too: a second `--brief` is a re-brief, and
    // the file says what the agent was last told. A failure here is loud and
    // fatal — a session launched on "read the brief" with no brief would sit
    // asking what to do, which is exactly the silent shape this exists to end.
    if let Some(text) = brief.as_deref() {
        if let Err(e) = write_brief(&wt, text) {
            ui.error(&format!("could not write {BRIEF_PATH}: {e}"));
            return 1;
        }
        ui.info(&format!("brief: {BRIEF_PATH}"));
        // Belongs to the session, never to the branch. Real git answers this
        // (the bats harness runs real git), so the warning is exact, not a guess.
        if !git::git_ok(&wt, &["check-ignore", "-q", BRIEF_PATH]) {
            ui.warn(".planning/ is not ignored by git in this repo — add `.planning/` to .gitignore (or .git/info/exclude) so the brief never lands in a commit");
        }
    }

    let install_cmd = if do_install && !already { detect_install_cmd(&wt) } else { String::new() };
    let mut ai_cmd = crate::config::resolve_ai_cmd(ai_flag.as_deref());
    if resume && !ai_cmd.is_empty() {
        ai_cmd = format!("{ai_cmd} {}", crate::config::resolve_ai_resume_arg());
    }

    if !do_tmux {
        ui.header("Done (no tmux)");
        ui.info(&format!("cd {wt}"));
        if !install_cmd.is_empty() {
            ui.info(&format!("then: {install_cmd}"));
        }
        return mat_rc;
    }
    // The worktree already exists at this point — a failed session is a partial
    // success the user MUST see (loud-guard). Propagate launch's rc so cmd_new
    // returns nonzero. (Fake shims always succeed → bats success path unchanged.)
    // `new` keeps the spare shell (pane 1) unless --no-spare — that's where deps
    // install, so suppressing it also suppresses the install command (launch's
    // contract: an install_cmd only ever rides along WITH the spare shell). The
    // detected command isn't lost silently — it's echoed as a hint, same as the
    // --no-tmux branch above.
    if !spare_shell && !install_cmd.is_empty() {
        ui.info(&format!("then: {install_cmd}"));
    }
    let pane1_install = if spare_shell { install_cmd.as_str() } else { "" };
    let mut ai = ai_launch_for(p, ui, &wt, &ai_cmd);
    if brief.is_some() && !ai.cmd.is_empty() {
        ai.opener = Some(BRIEF_OPENER.to_string());
    }
    let rc = launch(p, ui, &wt, &session, pane1_install, &ai, do_attach, spare_shell);
    if rc != 0 {
        rc
    } else {
        mat_rc
    }
}

fn detect_install_cmd(dir: &str) -> String {
    let has = |f: &str| Path::new(&format!("{dir}/{f}")).exists();
    if has("pnpm-lock.yaml") {
        "pnpm install".into()
    } else if has("bun.lockb") || has("bun.lock") {
        "bun install".into()
    } else if has("yarn.lock") {
        "yarn".into()
    } else if has("package-lock.json") || has("npm-shrinkwrap.json") {
        "npm install".into()
    } else {
        String::new()
    }
}

// ── switch ───────────────────────────────────────────────────────────────────
pub fn cmd_switch(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut force, mut do_fetch, mut yes) = (false, true, false);
    let mut pos: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--force" => force = true,
            "--no-fetch" => do_fetch = false,
            "-y" | "--yes" => yes = true,
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => pos.push(s.to_string()),
        }
    }

    let phys = std::env::current_dir().ok().and_then(|d| std::fs::canonicalize(d).ok()).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let wt_root_slash = format!("{}/", p.wt_root_dir());
    let cwd_topic = phys.strip_prefix(&wt_root_slash).map(|rest| rest.split('/').next().unwrap_or("").to_string()).filter(|s| !s.is_empty());

    let (topic, branch, base);
    if pos.len() >= 2 && Path::new(&format!("{}/{}", p.wt_root_dir(), slugify(&pos[0]))).is_dir() {
        topic = slugify(&pos[0]);
        branch = pos[1].clone();
        base = pos.get(2).cloned();
        if pos.len() > 3 {
            ui.error("Too many args.");
            return 1;
        }
        if let Some(cwd) = &cwd_topic {
            if &topic != cwd && !yes {
                ui.warn(&format!("You're inside '{cwd}' but this targets worktree '{topic}' (branch '{branch}')."));
                if !ui.confirm(&format!("Switch '{topic}'? [y/N] ")) {
                    ui.info("Aborted.");
                    return 0;
                }
            }
        }
    } else {
        let t;
        if let Some(cwd) = &cwd_topic {
            t = cwd.clone();
            if pos.len() >= 2 {
                ui.warn(&format!("No worktree '{}' — treating args as <branch> <base> for '{t}' (from cwd).", pos[0]));
            }
        } else if pos.len() >= 2 {
            ui.error(&format!("No worktree '{}' under .worktrees/. See: worktrees ls", pos[0]));
            return 1;
        } else {
            ui.error("Not inside a worktree — name one: worktrees switch <worktree> <branch>");
            return 1;
        }
        topic = t;
        branch = pos.first().cloned().unwrap_or_default();
        base = pos.get(1).cloned();
        if pos.len() > 2 {
            ui.error("Too many args.");
            return 1;
        }
    }
    if branch.is_empty() {
        ui.error("switch needs a branch.  e.g. worktrees switch messaging feat/next");
        return 1;
    }
    let branch = strip_origin(&branch).to_string();
    let wt = format!("{}/{}", p.wt_root_dir(), topic);

    if !p.is_registered(&wt) {
        ui.error(&format!("'{topic}' exists but is not a registered worktree — refusing (a switch would hit the main checkout). Clean it up: worktrees rm {topic}"));
        return 1;
    }
    ui.header(&format!("Switching '{topic}' → '{branch}'"));
    match do_switch(p, ui, &wt, &branch, base.as_deref(), do_fetch, force) {
        Ok(()) => 0,
        Err(c) => c,
    }
}

// ── open ─────────────────────────────────────────────────────────────────────
pub fn cmd_open(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut name, mut ai_flag, mut resume, mut do_attach) = (String::new(), None::<String>, false, true);
    // Default keeps the CLI's spare shell (pane 1). The app passes --no-spare so
    // its embedded view is single-pane (Claude full-width); the scratch shell
    // moves to the dock's Terminal tab.
    let mut spare_shell = true;
    let mut expect = false;
    for a in args {
        if expect {
            if a.starts_with('-') {
                ui.error(&format!("--ai needs a value (got '{a}')"));
                return 1;
            }
            ai_flag = Some(a.clone());
            expect = false;
            continue;
        }
        match a.as_str() {
            "--no-attach" => do_attach = false,
            "--no-spare" => spare_shell = false,
            "-r" | "--resume" => resume = true,
            "--ai" => expect = true,
            s if s.starts_with("--ai=") => ai_flag = Some(s["--ai=".len()..].to_string()),
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => {
                if name.is_empty() {
                    name = s.to_string();
                } else {
                    ui.error(&format!("Too many args: {s}"));
                    return 1;
                }
            }
        }
    }
    if expect {
        ui.error("--ai needs a value");
        return 1;
    }
    if name.is_empty() {
        ui.error("open needs a worktree (slug or branch). See: worktrees ls");
        return 1;
    }
    if !tmux::have_tmux() {
        ui.error("tmux not found");
        return 1;
    }
    let mut slug = slugify(&name);
    let mut wt = format!("{}/{}", p.wt_root_dir(), slug);
    if !Path::new(&wt).is_dir() {
        match p.wt_for_branch(strip_origin(&name)) {
            Some(holder) => {
                slug = basename(&holder);
                wt = holder;
                ui.info(&format!("Branch '{name}' lives in worktree '{slug}' — opening that."));
            }
            None => {
                ui.error(&format!("No worktree '{slug}' under .worktrees/.  Create it: worktrees new {name}"));
                return 1;
            }
        }
    }
    let session = p.session_name(&slug);
    let mut ai_cmd = crate::config::resolve_ai_cmd(ai_flag.as_deref());
    if resume && !ai_cmd.is_empty() {
        ai_cmd = format!("{ai_cmd} {}", crate::config::resolve_ai_resume_arg());
    }
    let ai = ai_launch_for(p, ui, &wt, &ai_cmd);
    launch(p, ui, &wt, &session, "", &ai, do_attach, spare_shell)
}

// ── close ────────────────────────────────────────────────────────────────────
/// End a place's tmux session; the worktree, branch, and declared state all
/// stay. The inverse of `open` — the place goes dormant, ready to re-enter.
pub fn cmd_close(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let mut names: Vec<String> = Vec::new();
    let mut yes = false;
    let mut expect: Option<String> = None;
    let mut want_val = false;
    for a in args {
        if want_val {
            if a.starts_with('-') {
                ui.error(&format!("--session needs a value (got '{a}')"));
                return 1;
            }
            expect = Some(a.clone());
            want_val = false;
            continue;
        }
        match a.as_str() {
            // Same convention as `rm`: `CaptureUi::confirm` always answers no, so
            // a programmatic caller (the app, a script) that means it passes `-y`
            // rather than being silently skipped.
            "-y" | "--yes" => yes = true,
            // The session the caller was SHOWN when it collected the user's word.
            // See close_one: consent is bound to this name, not to whatever is
            // live by the time the answer comes back.
            "--session" => want_val = true,
            s if s.starts_with("--session=") => expect = Some(s["--session=".len()..].to_string()),
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => names.push(s.to_string()),
        }
    }
    if want_val {
        ui.error("--session needs a value");
        return 1;
    }
    if names.is_empty() {
        ui.error("close needs a worktree (slug or branch), or 'main'. See: worktrees ls");
        return 1;
    }
    // One name per session name: `--session` answers for ONE place, and spreading
    // it over several would mean the same consent standing in for sessions the
    // user never saw — the exact thing it exists to prevent.
    if expect.is_some() && names.len() > 1 {
        ui.error("--session answers for ONE worktree — pass a single name.");
        return 1;
    }
    if !tmux::have_tmux() {
        ui.error("tmux not found");
        return 1;
    }
    let mut rc = 0;
    for n in &names {
        // A hard failure OUTRANKS a needs-confirmation stop: across several
        // names the caller must see "something broke" (1) rather than "ask the
        // user" (EXIT_NEEDS_CONFIRM), which is the code every existing caller
        // already understands. `worse_rc` is that ranking, written down once.
        if let Err(code) = close_one(p, ui, n, yes, expect.as_deref()) {
            rc = worse_rc(rc, code);
        }
    }
    rc
}

/// The session a `close` would act on for `slug`: the canonical name when it is
/// live, else whatever session was ADOPTED by pane cwd, else `None` (dormant).
/// Public because the app has to NAME that session in its own confirmation UI,
/// and digging the name back out of `cmd_close`'s prose would put an English
/// sentence on the wire between core and the frontend.
pub fn place_session(p: &Project, slug: &str) -> Option<String> {
    live_session(p, slug, &p.place_dir(slug), None)
}

/// `expect` is the session the CALLER already showed the user — the arm label in
/// the app, and the only session its click consented to kill. It is checked
/// against what resolves HERE, because those are two different moments: the app's
/// confirm is a round-trip (and its ctx-menu arm can sit open), so between the
/// question and the answer the named session can exit and another one can adopt
/// the place by pane cwd. Without the binding, `-y` would kill whatever is live
/// at execution time under a consent collected for something else.
fn close_one(p: &Project, ui: &mut dyn Ui, name: &str, yes: bool, expect: Option<&str>) -> Result<(), i32> {
    let s = slugify(name);
    if name != "(main)" && (s.is_empty() || s == "." || s == "..") {
        ui.error(&format!("Invalid worktree name '{name}'."));
        return Err(1);
    }
    // Resolve like `open`/`rm`: the slug DIR wins first, so a worktree literally
    // named "main" is closed by name (parity with open/rm). `(main)` — the slug
    // the app passes — always means the main checkout; bare `main` is a CLI
    // convenience that falls through to the checkout only when no worktree
    // shadows it. Then the branch's holder worktree, like `open`.
    let slug = if name == "(main)" {
        "(main)".to_string()
    } else if Path::new(&format!("{}/{}", p.wt_root_dir(), s)).is_dir() {
        s
    } else if name == "main" {
        "(main)".to_string()
    } else if let Some(holder) = p.wt_for_branch(strip_origin(name)) {
        let holder_slug = basename(&holder);
        ui.info(&format!("Branch '{name}' lives in worktree '{holder_slug}' — closing that."));
        holder_slug
    } else {
        ui.error(&format!("No worktree '{s}' under .worktrees/. See: worktrees ls"));
        return Err(1);
    };
    let canonical = p.session_name(&slug);
    // `open` ADOPTS any session with a pane cwd'd in the place (tmux::
    // worktree_session_excluding), so close must be its inverse or an adopted
    // session — including one left under a PREVIOUS prefix (§5) — becomes
    // unclosable. Unlike before, `(main)` adopts too: it is a place whose
    // canonical name a prefix change moves like any other.
    let dir = p.place_dir(&slug);
    let Some(session) = live_session(p, &slug, &dir, None) else {
        // No AI session, but the dock may still hold scratch shells for this
        // place (canonical-named) — sweep them so `close` leaves nothing behind.
        tmux::kill_shell_sidecars(&canonical);
        ui.info(&format!("no live session for '{slug}' ({canonical}) — nothing to close."));
        return Ok(());
    };
    // The consent is bound to a NAME. If what is living here now is not the
    // session the caller named in its own prompt, nothing dies: the answer was
    // about a session that is gone, and killing the newcomer would take a whole
    // session nobody was ever asked about. The caller gets a FRESH
    // needs-confirmation (not a failure, not a silent no-op) so it can re-ask
    // naming what is actually there.
    if let Some(want) = expect.filter(|w| *w != session) {
        ui.warn(&format!(
            "{} is no longer the session in {} — {} is.",
            fmt::yellow(want),
            fmt::cyan(&dir),
            fmt::yellow(&session)
        ));
        ui.warn(&format!(
            "Nothing was killed: you confirmed {want}, not {session}. Confirm again to kill {session}."
        ));
        return Err(crate::diag::EXIT_NEEDS_CONFIRM);
    }
    // An ADOPTED session is one this tool did not name, found only by pane cwd —
    // so it may just as well be a personal session with ONE pane cwd'd here (or
    // in a nested unrelated repo under this dir), and `kill-session` takes the
    // WHOLE session with every window in it. `rm` resolves and names the session
    // before its prompt for exactly this reason; `close` was the odd one out and
    // killed first, printing "closed adopted tmux X" afterwards.
    //
    // A CANONICAL-name close still asks nothing: that name is one only this tool
    // writes, so there is no doubt about whose session it is.
    //
    // `-y` ALONE still skips this entirely — `worktrees close -y feat-x` is a
    // person (or a script) saying "don't ask" in the same breath as the command,
    // with no gap between the intent and the kill for the world to change in.
    // The gap is what `--session` closes, and it is the app — whose consent
    // travels back over a round-trip — that must pass one.
    if session != canonical && !yes {
        ui.warn(&format!(
            "tmux {} was not opened under this repo's name ({}) — it was adopted because a pane is cwd'd in {}.",
            fmt::yellow(&session),
            fmt::cyan(&canonical),
            fmt::cyan(&dir)
        ));
        ui.plain("Closing it kills the WHOLE session — every window and pane in it, not just that one.");
        // A Ui that cannot ask has not said no — nobody was asked. Reporting the
        // decline as success is what made the app's Close button a no-op: it got
        // "Skipped … left running" and exit 0, and showed neither. The caller
        // gets a code it can act on, and re-runs with `-y` once it has the word.
        if !ui.can_confirm() {
            ui.warn(&format!("Skipped {slug} — {session} left running; confirm to kill it."));
            return Err(crate::diag::EXIT_NEEDS_CONFIRM);
        }
        if !ui.confirm(&format!("Kill {session}? [y/N] ")) {
            ui.info(&format!("Skipped {slug} — {session} left running."));
            return Ok(());
        }
    }
    tmux::kill_session(&session);
    tmux::kill_shell_sidecars(&canonical); // the dock's scratch shells die with the place
    match (session == canonical, slug.as_str()) {
        (true, "(main)") => ui.info(&format!("closed tmux {session} — checkout untouched.")),
        (true, _) => {
            ui.info(&format!("closed tmux {session} — worktree kept. Reopen: worktrees open {slug}"))
        }
        (false, "(main)") => ui.info(&format!(
            "closed adopted tmux {session} (session living in the main checkout) — checkout untouched."
        )),
        (false, _) => ui.info(&format!(
            "closed adopted tmux {session} (session living in '{slug}') — worktree kept."
        )),
    }
    Ok(())
}

// ── rm ───────────────────────────────────────────────────────────────────────
pub fn cmd_rm(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut del_branch, mut force, mut yes) = (false, false, false);
    let mut names: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--branch" => del_branch = true,
            "--force" => force = true,
            "-y" | "--yes" => yes = true,
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => names.push(s.to_string()),
        }
    }
    if names.is_empty() {
        ui.error("rm needs a worktree name (slug or branch). See: worktrees ls");
        return 1;
    }
    let mut rc = 0;
    for n in &names {
        ui.header(&format!("Removing {n}"));
        if remove_one(p, ui, n, del_branch, force, yes).is_err() {
            rc = 1;
        }
    }
    rc
}

fn remove_one(p: &Project, ui: &mut dyn Ui, name: &str, del_branch: bool, force: bool, yes: bool) -> Result<(), i32> {
    let slug = slugify(name);
    let path = format!("{}/{}", p.wt_root_dir(), slug);
    if slug.is_empty() || slug == "." || slug == ".." {
        ui.error(&format!("Invalid worktree name '{name}'."));
        return Err(1);
    }
    if !Path::new(&path).is_dir() {
        ui.error(&format!("No worktree '{slug}' under .worktrees/ (looked for {path})"));
        return Err(1);
    }
    let phys = std::env::current_dir().ok().and_then(|d| std::fs::canonicalize(d).ok()).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    if phys == path || phys.starts_with(&format!("{path}/")) {
        ui.error(&format!("You're inside {slug} — cd elsewhere first."));
        return Err(1);
    }

    let reg = p.is_registered(&path);
    let (mut branch, dirty) = if reg {
        let mut b = p.wt_branch(&path);
        if b == "(detached)" {
            b = String::new();
        }
        (b, p.wt_dirty(&path))
    } else {
        ui.warn(&format!("'{slug}' is not a registered worktree (stale dir) — will plain-delete it."));
        (String::new(), String::new())
    };
    // Resolved BEFORE the prompt, not at kill time: the prompt names what will
    // actually die, and after a prefix change (§5) that is not always the
    // canonical name. Falls back to the canonical name for display when nothing
    // is running, which is what the line meant all along.
    let live = live_session(p, &slug, &path, None);
    let session = live.clone().unwrap_or_else(|| p.session_name(&slug));

    if !dirty.is_empty() && !force {
        ui.warn(&format!("Worktree '{slug}' has uncommitted changes:"));
        ui.plain(&indent(&dirty));
        ui.error("Refusing to remove. Commit/stash, or pass --force.");
        return Err(1);
    }

    if !yes {
        let branch_part = if del_branch && !branch.is_empty() {
            format!(" · branch {}", fmt::yellow(&branch))
        } else {
            String::new()
        };
        ui.plain(&format!("Remove {} → tmux {} · worktree dir{branch_part}", fmt::cyan(&slug), fmt::yellow(&session)));
        if !ui.confirm("Proceed? [y/N] ") {
            ui.info(&format!("Skipped {slug}."));
            return Ok(());
        }
    }

    // §8: the compose stack comes down HERE — after the confirmation, before the
    // session dies. Anything after the `worktree remove` below is too late: the
    // directory (and the compose file, and `.worktree.env` naming the project)
    // is gone, and the containers are orphaned under a name nothing records.
    compose_down(p, ui, &slug, &path);

    if let Some(session) = &live {
        tmux::kill_session(session);
        ui.info(&format!("killed tmux {session}"));
    }
    tmux::kill_shell_sidecars(&session); // dock shells die with the worktree (past the refusal guards)
    if reg {
        if git::git_status(&p.main_root, &["worktree", "remove", "--force", &path]) {
            ui.info(&format!("removed worktree {slug}"));
        }
    } else if std::fs::remove_dir_all(&path).is_ok() {
        ui.info(&format!("deleted stale dir {slug}"));
    }
    let _ = git::git(&p.main_root, &["worktree", "prune"]);

    if del_branch && !branch.is_empty() {
        let flag = if force { "-D" } else { "-d" };
        if git::git(&p.main_root, &["branch", flag, &branch]).map(|o| o.status.success()).unwrap_or(false) {
            ui.info(&format!("deleted branch {branch}"));
        } else {
            ui.warn(&format!("branch '{branch}' not deleted (unmerged? use --force to force)"));
        }
    } else if !branch.is_empty() {
        ui.info(&format!("kept branch {branch} (use --branch to delete)"));
    }
    let _ = &mut branch;
    Ok(())
}

// ── relink / doctor — the `[[file]]` surface (proposal §7) ───────────────────
// Both go through materialize's probe → plan → (apply | report). ONE code path:
// `relink` applies the plan, `doctor` reads the same plan as a report, and
// `cmd_new` runs it once at creation. Anything that diverges here is a bug.

/// Read `<main_root>/.worktrees.toml`. `Ok(None)` = no config, which is the
/// byte-identical-behavior guarantee (§2.4): every caller does nothing at all.
/// `Err(1)` = present but invalid — a guard failure, already reported, because
/// partial application is how you get a half-provisioned worktree (§4).
fn load_project_config(p: &Project, ui: &mut dyn Ui) -> Result<Option<(ProjectConfig, Vec<Finding>)>, i32> {
    match projcfg::load(Path::new(&p.main_root)) {
        Ok((Some(cfg), findings)) => Ok(Some((cfg, findings))),
        Ok((None, _)) => Ok(None),
        Err(e) => {
            ui.error(&e.to_string());
            Err(1)
        }
    }
}

fn report_findings(ui: &mut dyn Ui, findings: &[Finding]) {
    for f in findings {
        match f.severity {
            Severity::Error => ui.error(&f.message),
            Severity::Warn => ui.warn(&f.message),
            Severity::Info => ui.info(&f.message),
        }
    }
}

/// probe → plan → apply for ONE place. The only place ops touches the filesystem
/// for declared files.
fn materialize_place(p: &Project, ui: &mut dyn Ui, cfg: &ProjectConfig, wt: &str, force: bool) -> i32 {
    let (main, wt) = (Path::new(&p.main_root), Path::new(wt));
    let facts = materialize::probe(cfg, main, wt);
    let plan = materialize::plan(cfg, main, wt, &facts, force);
    materialize::apply(&plan, ui)
}

/// Sorted, REGISTERED worktree dirs under `.worktrees/` — `--all`'s target list.
/// Read here rather than on `Project`: the config deliberately does not live on
/// the hot discovery path (§8). Stale (unregistered) dirs are skipped: writing a
/// credential into one is not "repairing a worktree", it is littering.
fn place_dirs(p: &Project) -> Vec<String> {
    let mut dirs: Vec<String> = match std::fs::read_dir(p.wt_root_dir()) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| format!("{}/{}", p.wt_root_dir(), e.file_name().to_string_lossy()))
            .collect(),
        Err(_) => Vec::new(),
    };
    dirs.retain(|d| p.is_registered(d));
    dirs.sort();
    dirs
}

/// Resolve a user-supplied name like `open`/`close` do: the slug DIR wins, then
/// the branch's holder worktree. The main checkout is refused — it is the SOURCE
/// of every declared file, so materializing into it would link a file to itself.
fn resolve_place(p: &Project, ui: &mut dyn Ui, name: &str) -> Result<String, i32> {
    let slug = slugify(name);
    if slug.is_empty() || slug == "." || slug == ".." {
        ui.error(&format!("Invalid worktree name '{name}'."));
        return Err(1);
    }
    if name == "(main)" || (name == "main" && !Path::new(&format!("{}/{slug}", p.wt_root_dir())).is_dir()) {
        ui.error("The main checkout is the SOURCE of the declared files — nothing to materialize there.");
        return Err(1);
    }
    let dir = format!("{}/{slug}", p.wt_root_dir());
    let dir = if Path::new(&dir).is_dir() {
        dir
    } else {
        match p.wt_for_branch(strip_origin(name)) {
            Some(holder) => holder,
            None => {
                ui.error(&format!("No worktree '{slug}' under .worktrees/. See: worktrees ls"));
                return Err(1);
            }
        }
    };
    // Same registration gate as `switch` (ops.rs:386): a stale dir is not a
    // place, and materializing into one would write credentials nowhere useful.
    if !p.is_registered(&dir) {
        ui.error(&format!("'{slug}' exists but is not a registered worktree — refusing. Clean it up: worktrees rm {slug}"));
        return Err(1);
    }
    Ok(dir)
}

// ── relink ───────────────────────────────────────────────────────────────────
/// Re-apply the file plan to existing worktrees. Without this, any config change
/// strands every worktree that already exists — which is the bug §1.2 describes.
pub fn cmd_relink(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut all, mut force) = (false, false);
    let mut names: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--all" => all = true,
            "--force" => force = true,
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => names.push(s.to_string()),
        }
    }
    if all && !names.is_empty() {
        ui.error("relink takes either <worktree> or --all, not both.");
        return 1;
    }
    if !all && names.is_empty() {
        ui.error("relink needs a worktree (slug or branch), or --all. See: worktrees ls");
        return 1;
    }

    // Targets resolve BEFORE the config loads: a bad worktree name is a usage
    // guard, and it must report as one whether or not the repo has a config.
    let targets: Vec<String> = if all {
        place_dirs(p)
    } else {
        let mut out = Vec::with_capacity(names.len());
        for n in &names {
            match resolve_place(p, ui, n) {
                Ok(d) => out.push(d),
                Err(code) => return code,
            }
        }
        out
    };

    let Some((cfg, findings)) = (match load_project_config(p, ui) {
        Ok(c) => c,
        Err(code) => return code,
    }) else {
        ui.info("No .worktrees.toml in this repo — nothing to materialize.");
        return 0;
    };
    report_findings(ui, &findings);
    if cfg.files.is_empty() {
        ui.info(".worktrees.toml declares no [[file]] entries — nothing to materialize.");
        return 0;
    }
    if targets.is_empty() {
        ui.info("No worktrees (.worktrees/ is empty).");
        return 0;
    }

    let mut rc = 0;
    for dir in &targets {
        ui.header(&format!("Relinking {}", basename(dir)));
        rc = worse_rc(rc, materialize_place(p, ui, &cfg, dir, force));
    }
    rc
}

/// Fold one multi-target loop's per-place exit codes into the process's.
///
/// Last-nonzero-wins would make the reported CLASS depend on the order `--all`
/// happened to walk `.worktrees/`: places [A → 1, B → 2] exits 2, rename the dirs
/// and the same run exits 1. A hard failure OUTRANKS findings, because `1` means
/// the operation did not happen while `2` means it happened and something needs
/// attention. `EXIT_NEEDS_CONFIRM` sits below both (`close`'s loop): the op did
/// not happen, but it stopped to ASK — a caller with a real breakage in the same
/// run must see that instead. Any other nonzero is treated as a hard failure — an
/// unrecognized code is not something to downgrade. The ladder is spelled out so
/// a future code cannot be silently mis-ranked by an ad-hoc comparison.
fn worse_rc(acc: i32, one: i32) -> i32 {
    let rank = |rc: i32| match rc {
        0 => 0,
        crate::diag::EXIT_NEEDS_CONFIRM => 1,
        crate::diag::EXIT_FINDINGS => 2,
        _ => 3,
    };
    if rank(one) > rank(acc) {
        one
    } else {
        acc
    }
}

// ── provision — ports + .worktree.env (proposal §6) ──────────────────────────
// The mirror of relink: one place at a time, idempotent, and the ONLY thing that
// writes `.worktree.env`. The slot is derived from the sibling scan, so there is
// no release step and nothing is ever recorded in the places store.

/// Allocate (or adopt) one place's slot and write its `.worktree.env`.
/// `0` clean · `1` a guard only the user can resolve (conflict, exhaustion, a
/// lock someone else holds). Silent when the project declares no `[ports]` —
/// that is §2.4's byte-identical-behavior guarantee.
fn provision_place(p: &Project, ui: &mut dyn Ui, cfg: &ProjectConfig, wt: &str, slug: &str, reallocate: bool) -> i32 {
    if cfg.ports.is_none() {
        return 0;
    }
    let req = provision::Request {
        main_root: Path::new(&p.main_root),
        wt_root: Path::new(p.wt_root_dir()),
        wt: Path::new(wt),
        prefix: &p.prefix,
        slug,
        reallocate,
    };
    let n = cfg.ports.as_ref().map(|x| x.base.len()).unwrap_or(0);
    match provision::provision(&req, cfg) {
        Ok(Outcome::NoPorts) => 0,
        Ok(Outcome::Unchanged { slot }) => {
            ui.info(&format!("slot {slot} — {ENV_FILE} already current"));
            0
        }
        Ok(Outcome::Updated { slot }) => {
            ui.info(&format!("slot {slot} kept — rewrote {ENV_FILE} ({n} ports)"));
            warn_if_tracked(ui, wt);
            0
        }
        Ok(Outcome::Allocated { slot }) => {
            ui.info(&format!("slot {slot} — wrote {ENV_FILE} ({n} ports)"));
            warn_if_tracked(ui, wt);
            0
        }
        // §6's conflict policy: REFUSE, never silently re-allocate. Rewriting a
        // slot under a running stack orphans containers on the old project name
        // and moves ports the developer has bookmarked. Name both paths, print
        // both remedies.
        Err(ProvisionError::Conflict { slot, this, other }) => {
            ui.error(&format!("slot {slot} is claimed by BOTH:"));
            ui.error(&format!("    {}", this.display()));
            ui.error(&format!("    {}", other.display()));
            ui.error(&format!(
                "Refusing to move a slot under a running stack. Either edit {}/{ENV_FILE} to a free slot by hand,",
                this.display()
            ));
            ui.error(&format!(
                "or stop that stack and run: worktrees provision {slug} --reallocate"
            ));
            1
        }
        Err(e) => {
            ui.error(&e.to_string());
            1
        }
    }
}

/// §6 step 6: the file is worthless if git can see it — `wt_dirty` would be true
/// forever and `switch`/`rm` would refuse without `--force`. `ensure_excluded`
/// has already run; this catches the repo that un-ignores it on purpose.
fn warn_if_tracked(ui: &mut dyn Ui, wt: &str) {
    if !git::git_ok(wt, &["check-ignore", "-q", "--", ENV_FILE]) {
        ui.warn(&format!(
            "{ENV_FILE} is not gitignored in this worktree — `git status` will show it, and \
             switch/rm will refuse without --force"
        ));
    }
}

/// `worktrees provision [<wt>|--all] [--reallocate]`. Same argument shape and
/// same resolve-then-load ordering as `relink`, so a bad worktree name is a
/// usage guard whether or not the repo has a config.
pub fn cmd_provision(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut all, mut reallocate) = (false, false);
    let mut names: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--all" => all = true,
            "--reallocate" => reallocate = true,
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => names.push(s.to_string()),
        }
    }
    if all && !names.is_empty() {
        ui.error("provision takes either <worktree> or --all, not both.");
        return 1;
    }
    if !all && names.is_empty() {
        ui.error("provision needs a worktree (slug or branch), or --all. See: worktrees ls");
        return 1;
    }

    let targets: Vec<String> = if all {
        place_dirs(p)
    } else {
        let mut out = Vec::with_capacity(names.len());
        for n in &names {
            match resolve_place(p, ui, n) {
                Ok(d) => out.push(d),
                Err(code) => return code,
            }
        }
        out
    };

    let Some((cfg, findings)) = (match load_project_config(p, ui) {
        Ok(c) => c,
        Err(code) => return code,
    }) else {
        ui.info("No .worktrees.toml in this repo — nothing to provision.");
        return 0;
    };
    report_findings(ui, &findings);
    if cfg.ports.is_none() {
        ui.info(".worktrees.toml declares no [ports] — nothing to provision.");
        if cfg.compose.is_some() {
            // COMPOSE_PROJECT_NAME rides along with a slot; on its own there is
            // no per-worktree file to write, and `rm` falls back to expanding the
            // template for teardown.
            ui.warn("[compose] without [ports]: the project name is derived at teardown, not written to a file.");
        }
        return 0;
    }
    if targets.is_empty() {
        ui.info("No worktrees (.worktrees/ is empty).");
        return 0;
    }
    // The file must be invisible to git before it is written, not after.
    p.ensure_excluded();

    let mut rc = 0;
    for dir in &targets {
        let slug = basename(dir);
        ui.header(&format!("Provisioning {slug}"));
        rc = worse_rc(rc, provision_place(p, ui, &cfg, dir, &slug, reallocate));
    }
    rc
}

// ── [compose] teardown (proposal §5, §7) ─────────────────────────────────────

/// `docker compose … down -v --remove-orphans` for a place being removed.
///
/// The one behavior in the audited 761-line consumer script that is not
/// expressible as file/ports config, so it is first-class DATA here and the tool
/// assembles the argv (§5). Never a command string from the repo.
///
/// Everything about this is best-effort: a missing docker, a broken config, or a
/// failing stack is a WARNING. The removal must still proceed — refusing to
/// delete a worktree because docker is not running would be a worse bug than the
/// containers it leaves behind.
fn compose_down(p: &Project, ui: &mut dyn Ui, slug: &str, wt: &str) {
    let cfg = match projcfg::load(Path::new(&p.main_root)) {
        Ok((Some(cfg), _)) => cfg,
        Ok((None, _)) => return,
        Err(e) => {
            ui.warn(&format!("{e} — skipping compose teardown"));
            return;
        }
    };
    let Some(compose) = cfg.compose.as_ref() else { return };
    let wt_path = Path::new(wt);
    // Layer B, once more, on EVERY declared file: only a real file that resolves
    // INSIDE this worktree is handed to docker. A symlinked ancestor could
    // otherwise point a compose file (and its bind mounts) anywhere.
    //
    // All-or-nothing, and in the declared order: docker itself fails on a missing
    // `-f`, and a teardown that silently dropped one file would run `down -v`
    // against a subset — which is the volume-leak this list exists to fix.
    let root = std::fs::canonicalize(wt_path).ok();
    let mut files = Vec::with_capacity(compose.files.len());
    for rel in &compose.files {
        let file = wt_path.join(rel.as_str());
        let inside = match (std::fs::canonicalize(&file), root.as_ref()) {
            (Ok(f), Some(root)) => f.starts_with(root) && f.is_file(),
            _ => false,
        };
        if !inside {
            ui.warn(&format!(
                "[compose] file `{}` is missing here — skipping teardown (containers may be left running)",
                rel.as_str()
            ));
            return;
        }
        files.push(file);
    }
    // The place's OWN file first: the running containers are named after what the
    // stack was STARTED with, and the template may have changed since.
    let project = provision::compose_project_for(wt_path, compose, &p.prefix, slug);
    if project.is_empty() {
        ui.warn("[compose] project name is empty — skipping teardown");
        return;
    }
    ui.info(&format!("docker compose -p {project} down -v --remove-orphans"));
    match provision::compose_down(&files, &project) {
        Ok(()) => ui.info(&format!("compose stack '{project}' is down")),
        Err(e) => ui.warn(&format!("compose teardown skipped: {e}")),
    }
}

// ── doctor ───────────────────────────────────────────────────────────────────
/// Report drift. Ships WITH relink, not after: creation is deliberately
/// non-transactional, "materialized" is not a state anything records, and a
/// feature whose own failure mode is silent does not fix a silent-failure bug
/// (§7). Exit: 0 clean · 1 usage/guard · 2 findings present.
pub fn cmd_doctor(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut json, mut strict, mut config_only) = (false, false, false);
    let mut names: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            "--strict" => strict = true,
            "--config-only" => config_only = true,
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => names.push(s.to_string()),
        }
    }
    if names.len() > 1 {
        ui.error("doctor takes at most one worktree.");
        return 1;
    }
    if config_only && !names.is_empty() {
        ui.error("--config-only checks the repo's config against git — it takes no worktree.");
        return 1;
    }
    // Same env override as `ls --json` (main.rs:64-73), so a JSON-mode caller
    // sets it once for every command.
    let json = json || std::env::var("WORKTREES_JSON").ok().as_deref() == Some("1");

    // Same ordering rule as relink: a named place that does not exist is a usage
    // guard (exit 1), decided before the config is even read.
    let targets = if config_only {
        Vec::new()
    } else {
        match names.first() {
            Some(n) => match resolve_place(p, ui, n) {
                Ok(d) => vec![d],
                Err(code) => return code,
            },
            None => place_dirs(p),
        }
    };

    // A fact about the TREE rather than about its config, so it is resolved
    // before the no-config branch below — which returns ahead of every other
    // check, and a copy that arrived on an SSD is not a likely place to find a
    // `.worktrees.toml`. One stat when the repo is an ordinary one.
    let hub_copy = crate::sync::hub_copy_finding(Path::new(&p.main_root));

    // Also a fact about the TREE, and resolved here for the same reason: a
    // project that `sync` ferries need not own a `.worktrees.toml`, and this
    // damage is exactly what a user staring at a wall of `deleted:` needs told.
    //
    // Whole-project runs only (`undeclared_findings`' rule): it covers main AND
    // every linked worktree, so it is not about the one place a name asks about.
    // Never in `--config-only`, which reads no filesystem state at all — on the
    // bare clone that mode is built for, every tracked file is absent. And never
    // beside the hub-copy finding: a hub copy is missing every excluded-tracked
    // file BY CONSTRUCTION (the push skipped them), so absence there is the
    // pushed state, not damage — the remedy this finding names is exactly what
    // the hub-copy guard refuses, and the copy's ferried `.worktrees/*/.git`
    // files point at the origin machine's repo, so the per-worktree status
    // probes would be reading a stranger.
    let skipped = if config_only || !names.is_empty() || hub_copy.is_some() {
        None
    } else {
        let root = Path::new(&p.main_root);
        crate::sync::skipped_files_finding(root, &crate::sync::project_extra_excludes(root))
    };

    let cfg = match load_project_config(p, ui) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let Some((cfg, mut findings)) = cfg else {
        // No config is a healthy repo, not a broken one.
        let report = Report::new(hub_copy.into_iter().chain(skipped).collect());
        if json {
            emit_report(ui, &report);
        } else if report.is_empty() {
            ui.info("No .worktrees.toml in this repo — nothing to check.");
        } else {
            report_findings(ui, &report.findings);
        }
        return report.exit_code();
    };
    // Both go to the front, hub-copy first: one is the answer to "why is nothing
    // I do in here allowed", the other to "why is my git status full of
    // deletions" — and a user reading a doctor report is usually holding one of
    // those two questions.
    if let Some(f) = skipped {
        findings.insert(0, f);
    }
    if let Some(f) = hub_copy {
        findings.insert(0, f);
    }

    // Config-vs-config, so it belongs in `--config-only` too: both sources are
    // COMMITTED files, present on a bare clone with no filesystem state.
    findings.extend(prefix_findings(p, &cfg));

    if config_only {
        // The CI mode: no filesystem state, so it works on a bare clone where
        // every declared (gitignored) source is absent by definition.
        findings.extend(materialize::config_only(&cfg, Path::new(&p.main_root)));
    } else {
        let main = Path::new(&p.main_root);
        for dir in &targets {
            let wt = Path::new(dir);
            let facts = materialize::probe(&cfg, main, wt);
            let plan = materialize::plan(&cfg, main, wt, &facts, false);
            findings.extend(plan.report().findings);
        }
        findings.extend(port_findings(p, &cfg, &targets));
        findings.extend(compose_findings(p, &cfg, &targets));
        // Repo-scoped, and only on a whole-project run — same rule the session
        // scan below follows: a named place is a question about that place, and
        // "the config does not mention apps/api/.env" is not about any of them.
        //
        // Deliberately NOT in `--config-only`: that mode is config-vs-git with no
        // filesystem state, and on the bare clone it is built for every gitignored
        // file is absent by definition, so this check could only ever report
        // nothing there.
        if names.is_empty() {
            findings.extend(undeclared_findings(p, &cfg));
        }
        // The session scan covers MAIN too, unlike the file and port checks:
        // main declares no files and takes no slot, but its session is named
        // from the same prefix and drifts exactly the same way — and `close
        // main` finds (and kills) an old-named session, so `doctor` never
        // mentioning it is the gap. Only on a whole-project run: a named place
        // is a question about that place.
        let mut scanned: Vec<(String, String)> =
            targets.iter().map(|d| (basename(d), d.clone())).collect();
        if names.is_empty() {
            scanned.insert(0, ("(main)".to_string(), p.main_root.clone()));
        }
        findings.extend(session_findings(p, &scanned));
    }

    if strict {
        // `--strict` is what lets a copy the source has moved past — or a
        // credential the config never learned about — fail a run; both stay
        // non-fatal by default, because drift is the expected steady state (§7)
        // and promoting either would break every CI already pinned on exit 0.
        for f in &mut findings {
            if matches!(f.code, Code::CopyStale | Code::Undeclared) && f.severity == Severity::Warn {
                f.severity = Severity::Error;
            }
        }
    }

    let report = Report::new(findings);
    if json {
        emit_report(ui, &report);
    } else if report.is_empty() {
        ui.info("clean — every declared file is materialized.");
    } else {
        for f in &report.findings {
            let line = match &f.place {
                Some(place) => format!("{place}: {}", f.message),
                None => f.message.clone(),
            };
            match f.severity {
                Severity::Error => ui.error(&line),
                Severity::Warn => ui.warn(&line),
                Severity::Info => ui.info(&line),
            }
        }
    }
    report.exit_code()
}

/// Gitignored, untracked files in MAIN that no `[[file]]` entry declares.
///
/// The only check here that runs against the repo rather than against the
/// config, and the reason is that every other one is judged BY the config: a
/// `.worktrees.toml` that stopped being true is invisible to all of them.
/// Detection otherwise happens exactly once, at `init` — see `Code::Undeclared`.
///
/// Repo-scoped (`place` stays `None`) so `doctor` on a project with fifteen
/// worktrees says it once, not fifteen times.
fn undeclared_findings(p: &Project, cfg: &ProjectConfig) -> Vec<Finding> {
    let (files, truncated) = init::undeclared(Path::new(&p.main_root), cfg);
    let mut out: Vec<Finding> = files
        .into_iter()
        .map(|c| {
            // The same split `hint_init` uses, and for the same reason (§12): a
            // missing `.env` breaks the build loudly on the next command, while a
            // missing credential builds fine and dies on a device days later.
            // Only the class that fails SILENTLY earns a warning.
            let (severity, what) = match c.kind {
                init::Kind::Credential => {
                    (Severity::Warn, "a credential, and it fails SILENTLY when missing")
                }
                init::Kind::Env => (Severity::Info, "gitignored and tracked nowhere"),
            };
            let msg = format!(
                "{} is {what} — no [[file]] entry declares it, so every worktree is missing it. \
                 Add it:  worktrees init --diff",
                c.rel
            );
            Finding::new(severity, Code::Undeclared, msg).at_path(c.rel)
        })
        .collect();
    // §9's promise: a bound that was HIT is reported, never swallowed. An empty
    // result from a truncated walk means "I did not look everywhere".
    //
    // Its own wording, NOT `init`'s `TRUNCATED`: that string ends "add anything
    // it missed by hand", where "it" is the config `init` just printed — a
    // referent that does not exist in a doctor report. It is also the one
    // `undeclared` finding that is not about a file, so it says so: a `--json`
    // consumer counting the code as a file count is off by one otherwise, and
    // `path: null` is the only other thing distinguishing it.
    if truncated {
        out.push(Finding::info(
            Code::Undeclared,
            "this scan stopped early (depth or size bound; hidden and vendor dirs are skipped), \
             so it is not a complete answer — a file it never reached would not be listed above.",
        ));
    }
    out
}

/// §5's prefix findings: the two project-scoped sources disagreeing, and — the
/// point of saying it out loud — WHICH one is in effect.
///
/// The legacy `.worktree-prefix` wins so that adding `[project] prefix` to a repo
/// that already has one renames nothing. That is the right default and the
/// surprising one: someone who writes `[project] prefix` expects it to take, and
/// nothing else in the repo would ever tell them it did not.
fn prefix_findings(p: &Project, cfg: &ProjectConfig) -> Vec<Finding> {
    let (Some(file), Some(project)) =
        (crate::project::prefix_file(&p.main_root), cfg.project.prefix.as_deref())
    else {
        return Vec::new();
    };
    // Compare what each source would RESOLVE to, not what it says: `Team.X` and
    // `team-x` are the same prefix once sanitized, and a finding about a
    // difference that changes no session name is noise.
    let (want, got) = (sanitize_prefix(&file), sanitize_prefix(project));
    if want == got {
        return Vec::new();
    }
    // WHICH one is in effect is a question about `p.prefix`, not about these two
    // strings: `$WORKTREES_PREFIX` outranks both project sources (§5). Saying
    // "the file wins" while every session is named from the env value sends the
    // reader hunting a bug that is not there.
    let env = std::env::var("WORKTREES_PREFIX").ok().filter(|s| !s.trim().is_empty());
    let winner = match &env {
        Some(v) => format!("$WORKTREES_PREFIX (`{v}`) outranks both"),
        None => format!("{} wins", init::PREFIX_FILE),
    };
    vec![Finding::warn(
        Code::PrefixMismatch,
        format!(
            "{} says `{want}` but [project] prefix says `{got}` — {winner}, so sessions are \
             named `{}-<slug>`. Delete {} to switch to the config (then: worktrees doctor, \
             to see which sessions are still running under the old name).",
            init::PREFIX_FILE,
            p.prefix,
            init::PREFIX_FILE
        ),
    )
    .at_path(init::PREFIX_FILE)]
}

/// The session half of the prefix hazard (§5): a place whose LIVE session is not
/// named what the current prefix renders.
///
/// Resolved through `live_session` — the same function `close` and `rm` use — so
/// this finding cannot claim a session those two would fail to find.
///
/// The pane list is fetched ONCE for the whole scan (`ls` prefetches for the
/// same reason): per place `live_session` is up to three tmux subprocesses, and
/// a project with fifteen places paid forty-five of them for a read-only report.
///
/// `places` is `(slug, dir)` rather than dirs, because `(main)` is in this scan
/// and its slug is not `basename(dir)`.
fn session_findings(p: &Project, places: &[(String, String)]) -> Vec<Finding> {
    let panes = tmux::PaneList::fetch();
    let mut out = Vec::new();
    for (slug, dir) in places {
        let slug = slug.clone();
        let canonical = p.session_name(&slug);
        let Some(live) = live_session(p, &slug, dir, panes.as_ref()).filter(|s| *s != canonical)
        else {
            continue;
        };
        // `(main)` is the app's slug for the checkout; the CLI spells it `main`,
        // and the parens would not survive a shell anyway.
        let arg = if slug == "(main)" { "main" } else { slug.as_str() };
        out.push(
            Finding::warn(
                Code::SessionDrift,
                format!(
                    "live tmux session is `{live}`, but this repo's prefix now renders \
                     `{canonical}` — the running session is still found (by pane cwd), and the \
                     new name applies the next time it is opened. Rename it now with: \
                     worktrees close {arg} && worktrees open {arg}"
                ),
            )
            .at_place(slug),
        );
    }
    out
}

/// §6's port findings for the places `doctor` was asked about.
///
/// Read-only and deliberately LOCK-FREE: taking the allocation lock here would
/// let a `doctor` (or, later, the app) block a `provision`, and the cost of a
/// torn read is a re-run rather than a wrong write.
fn port_findings(p: &Project, cfg: &ProjectConfig, targets: &[String]) -> Vec<Finding> {
    let Some(ports) = cfg.ports.as_ref() else { return Vec::new() };
    let scan = provision::scan(Path::new(&p.main_root), Path::new(p.wt_root_dir()));
    let mut out = Vec::new();
    for dir in targets {
        let slug = basename(dir);
        let wt = Path::new(dir);
        let env = provision::read_env(wt);
        let Some(slot) = env.as_ref().and_then(|(_, e)| e.slot) else {
            // §1.1 — an Error, and the loudest one in this module. A place with
            // no slot is not "portless": the consumer's dev script reads the
            // file's absence as "not a worktree" and takes the branch that
            // `pkill -9`s the main checkout's whole stack.
            let what = match env {
                Some(_) => format!("`{ENV_FILE}` here declares no WORKTREE_SLOT"),
                None => format!("no `{ENV_FILE}` here"),
            };
            out.push(
                Finding::error(
                    Code::NoSlot,
                    format!(
                        "{what} — this repo declares [ports], and a half-provisioned worktree is \
                         worse than none: scripts that key off that file will treat this as the \
                         MAIN checkout. Fix: worktrees provision {slug}"
                    ),
                )
                .at_place(slug.clone())
                .at_path(ENV_FILE),
            );
            continue;
        };
        // A slot that is present and unique still says nothing about whether the
        // file names every port. The `.worktree.env` files a project accumulated
        // BEFORE a service joined `[ports].base` lack that variable entirely —
        // and a consumer that reads an unset `WEBSITE_PORT` falls back to the
        // default, which is MAIN's port (§1.1 by drift instead of by absence).
        // Warn, not Error: the file is real, and `provision` rewrites it in place
        // keeping the slot it already has.
        let declared = env.as_ref().map(|(_, e)| &e.keys);
        let missing: Vec<String> = ports
            .base
            .keys()
            .map(|n| format!("{n}_PORT"))
            .filter(|v| !declared.is_some_and(|keys| keys.contains(v)))
            .collect();
        if !missing.is_empty() {
            out.push(
                Finding::warn(
                    Code::MissingPort,
                    format!(
                        "`{ENV_FILE}` here declares slot {slot} but not {} — anything reading \
                         {} falls back to its default, which is the MAIN checkout's port. \
                         Fix (the slot is kept): worktrees provision {slug}",
                        missing.join(", "),
                        if missing.len() == 1 { "it" } else { "them" }
                    ),
                )
                .at_place(slug.clone())
                .at_path(ENV_FILE),
            );
        }
        if let Some(other) = scan.other_claimant(slot, wt) {
            out.push(
                Finding::error(
                    Code::SlotConflict,
                    format!(
                        "slot {slot} is also claimed by {} — stop one of the two stacks and run: \
                         worktrees provision {slug} --reallocate",
                        other.display()
                    ),
                )
                .at_place(slug.clone())
                .at_path(ENV_FILE),
            );
        }
        for (name, port) in provision::slot_ports(ports, slot).unwrap_or_default() {
            if !provision::port_free(port) {
                // Info, never Error: the overwhelmingly likely binder is this
                // place's own running stack, which is the healthy state (the same
                // reasoning that keeps copy drift at Info).
                out.push(
                    Finding::info(
                        Code::PortBusy,
                        format!("{name}={port} is already bound — expected while this place's stack is up"),
                    )
                    .at_place(slug.clone()),
                );
            }
        }
    }
    out
}

/// The compose half of §6's "never move a name under a running stack" rule.
///
/// `provision` keeps the `COMPOSE_PROJECT_NAME` a place was started under, so a
/// changed `[compose] project` template leaves the file and the config
/// disagreeing — and that disagreement decides which containers `rm` tears down.
/// A Warn, not an Error: nothing is broken, and `--reallocate` is a deliberate
/// act the user may simply not have taken yet.
fn compose_findings(p: &Project, cfg: &ProjectConfig, targets: &[String]) -> Vec<Finding> {
    let Some(compose) = cfg.compose.as_ref() else { return Vec::new() };
    let mut out = Vec::new();
    for dir in targets {
        let slug = basename(dir);
        let Some(recorded) = provision::recorded_compose_project(Path::new(dir)) else { continue };
        let want = provision::compose_project_name(&compose.project, &p.prefix, &slug);
        if recorded != want {
            out.push(
                Finding::warn(
                    Code::ComposeDrift,
                    format!(
                        "{ENV_FILE} records COMPOSE_PROJECT_NAME={recorded}, but [compose] project \
                         now renders {want} — the recorded name is kept (it is what any running \
                         containers are called). Fix: stop that stack and run: worktrees provision \
                         {slug} --reallocate"
                    ),
                )
                .at_place(slug.clone())
                .at_path(ENV_FILE),
            );
        }
    }
    out
}

/// `doctor --json` follows the `ls --json` template: one compact line with its
/// own `schema_version`, every nullable field an explicit `null`.
fn emit_report(ui: &mut dyn Ui, report: &Report) {
    ui.plain(&serde_json::to_string(report).unwrap_or_default());
}

// ── status ───────────────────────────────────────────────────────────────────
/// `worktrees status <name> [--json]` — one place's health verdict.
///
/// GATHERING lives here (git + the declared store); the JUDGEMENT is
/// `health::assess`, which is pure. The app runs THIS function in-process
/// (`CaptureUi` + `--json`) and deserializes the line it emits, so the CLI and
/// the app can never disagree about what a verdict means.
///
/// ⚠ `WORKTREES_STATUS_NOW` (epoch seconds) overrides the clock. It is a TEST
/// SEAM, not user surface: `test/status.bats` has to age a just-made commit past
/// `STALE_SECS`, and `test/helpers/common.bash` unsets it in `common_setup` so a
/// leaked export cannot warp the rest of the suite. ONE `now` is resolved here
/// and threaded into BOTH `store::reconcile` and `assess` — two clocks would let
/// the lifecycle label and the verdict disagree about what time it is.
///
/// Exit: `0` a report was emitted (ANY verdict — this is a report, not a gate),
/// `1` usage / unknown / unregistered place. There is no exit-2 tier: that is
/// `doctor`'s findings contract, and a health verdict is not a finding.
pub fn cmd_status(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let mut json = false;
    let mut names: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => names.push(s.to_string()),
        }
    }
    // Unlike `doctor`, a name is REQUIRED: a health report is about one place,
    // and "the whole project is at risk" is not a sentence this tool can mean.
    if names.len() != 1 {
        ui.error("usage: worktrees status <name> [--json]");
        return 1;
    }
    // Same env override as `ls --json` / `doctor --json`, so a JSON-mode caller
    // sets it once for every command.
    let json = json || std::env::var("WORKTREES_JSON").ok().as_deref() == Some("1");

    // Resolve exactly like `close` does — the slug DIR wins first (a worktree
    // literally named "main" is its own place), then bare `main` falls through
    // to the checkout, then the branch's holder worktree. `(main)` is a legal
    // target here (unlike `relink`/`doctor`'s resolve_place, which refuses it):
    // main is a place with a health story too, and it is the one place where
    // `behind` means "you need to pull". Bare `main` matters because unquoted
    // parens are a shell syntax error.
    let name = &names[0];
    let s = slugify(name);
    if name != "(main)" && (s.is_empty() || s == "." || s == "..") {
        ui.error(&format!("Invalid worktree name '{name}'."));
        return 1;
    }
    let slug = if name == "(main)" {
        "(main)".to_string()
    } else if Path::new(&format!("{}/{}", p.wt_root_dir(), s)).is_dir() {
        s
    } else if name == "main" {
        "(main)".to_string()
    } else if let Some(holder) = p.wt_for_branch(strip_origin(name)) {
        basename(&holder)
    } else {
        ui.error(&format!("No worktree '{s}' under .worktrees/. See: worktrees ls"));
        return 1;
    };

    let now = std::env::var("WORKTREES_STATUS_NOW")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or_else(crate::sysclock::now_epoch);

    // ONE snapshot: every derived fact this report needs (divergence vs the base,
    // dirty count, last commit, tmux, the claude dir) is already computed there,
    // so there is no second git plumbing layer to drift out of sync with `ls`.
    let ls = p.ls();
    let Some(place) = ls.places.iter().find(|x| x.slug == slug) else {
        ui.error(&format!("No worktree '{slug}' under .worktrees/. See: worktrees ls"));
        return 1;
    };
    // An unregistered place has NO git facts (ls emits nulls). Mapping those to
    // zero would fabricate a clean, in-sync "parked" for a directory git has
    // never heard of — so refuse instead of inventing a verdict.
    if !place.registered {
        ui.error(&format!(
            "'{slug}' exists but is not a registered worktree — refusing. Clean it up: worktrees rm {slug}"
        ));
        return 1;
    }

    let store = crate::store::read_lenient(&p.main_root);
    let decl = store.places.get(&slug);
    // `ls --json` emits LIVE-ONLY lifecycle (model.rs) — recompute with the
    // declared state merged in, the same way the app does.
    let lifecycle_effective = crate::store::reconcile(decl, place.tmux_session.up, now);

    let base_ref = p.base_ref();
    let dir = place.path.as_str();
    let ahead = place.ahead.unwrap_or(0);
    let mut not_on_base: Vec<String> = Vec::new();
    let mut not_on_base_total = 0u32;
    let mut maybe_merged = 0u32;
    let mut true_unpushed: Option<u32> = None;
    // Only when there is something ahead to explain. A place with nothing on top
    // of the base pays three git spawns for three guaranteed-empty answers, and
    // this command is also the app's, run per click. Every failure below reads as
    // "nothing found", never as an error: a health report must still print.
    if ahead > 0 {
        if let Some(out) = git::git_out(dir, &["log", "--oneline", &format!("{base_ref}..HEAD")]) {
            let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
            not_on_base_total = lines.len() as u32;
            not_on_base = lines.iter().take(20).map(|l| l.to_string()).collect();
        }
        if let Some(out) = git::git_out(dir, &["cherry", &base_ref]) {
            maybe_merged = out.lines().filter(|l| l.starts_with("- ")).count() as u32;
        }
        if place.upstream.is_some() {
            // ⚠ A CONFIGURED upstream is not a RESOLVING one: `status
            // --porcelain=v2` reports `branch.<x>.merge`, so a branch set to
            // track origin/feat-x that has never been pushed still names an
            // upstream here while `@{u}` does not resolve. Reading that failure
            // as "all of them unpushed" is the honest answer — the alternative
            // is telling the user their work is "all pushed — safe" to a ref
            // that does not exist.
            true_unpushed = Some(
                git::git_out(dir, &["rev-list", "--count", "@{u}..HEAD"])
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(not_on_base_total),
            );
        }
    }

    let facts = crate::health::HealthFacts {
        slug: slug.clone(),
        branch: place.branch.clone(),
        base: base_ref.clone(),
        created_epoch: place.created_epoch.filter(|e| *e > 0),
        last_commit_epoch: place.last_commit_epoch,
        last_commit_subject: place.last_commit_subject.clone(),
        last_opened_epoch: decl.and_then(|d| d.last_opened_epoch),
        last_worked_epoch: decl.and_then(|d| d.last_worked_epoch),
        claude_last_epoch: place
            .claude_session_dir
            .as_deref()
            .and_then(crate::health::claude_last_epoch),
        dirty_files: place.dirty_files.unwrap_or(0),
        ahead,
        behind: place.behind.unwrap_or(0),
        upstream: place.upstream.clone(),
        tmux_up: place.tmux_session.up,
        lifecycle_effective,
        note: decl.and_then(|d| d.note.clone()),
        title: decl.and_then(|d| d.title.clone()),
        not_on_base,
        not_on_base_total,
        true_unpushed,
        maybe_merged,
    };

    let clock = crate::sysclock::SysClock::detect();
    let (verdict, reasons) = crate::health::assess(&facts, now, &|e| clock.fmt_date(e));
    let report = crate::health::Report {
        schema_version: crate::health::SCHEMA_VERSION,
        verdict,
        reasons,
        facts,
    };

    if json {
        // Same template as `doctor --json` / `ls --json`: ONE compact line.
        ui.plain(&serde_json::to_string(&report).unwrap_or_default());
        return 0;
    }
    emit_status_human(ui, &report, &clock, now);
    0
}

/// The human face of `status`: the verdict, then why, then the receipts.
fn emit_status_human(
    ui: &mut dyn Ui,
    r: &crate::health::Report,
    clock: &crate::sysclock::SysClock,
    now: i64,
) {
    let f = &r.facts;
    let headline = match r.verdict.as_str() {
        "active" => "active",
        "parked" => "parked",
        "at-risk" => "work at risk",
        _ => "cold",
    };
    ui.header(&format!("{} — {headline}", f.title.as_deref().unwrap_or(&f.slug)));
    for reason in &r.reasons {
        ui.info(reason);
    }
    // A verdict with nothing to explain still deserves a sentence — an empty gap
    // under the header reads as a report that failed to run.
    if r.reasons.is_empty() {
        ui.info(match r.verdict.as_str() {
            "active" => "touched recently — this is live work.",
            "cold" => "no session, nothing uncommitted, nothing ahead of the base — nothing here is unique to this worktree.",
            _ => "nothing to report.",
        });
    }

    // Aligned label/value facts. `when` renders "date (age)" for the epochs, so
    // a reader never has to subtract two dates in their head.
    let when = |e: Option<i64>| match e {
        Some(e) if e > 0 => format!("{} ({} ago)", clock.fmt_date(e), clock.ago(e, now)),
        _ => "-".to_string(),
    };
    let mut row = |label: &str, value: String| ui.plain(&format!("  {label:<14}{value}"));
    row("branch", f.branch.clone().unwrap_or_else(|| "(detached)".into()));
    row("created", when(f.created_epoch));
    row(
        "last commit",
        match (&f.last_commit_epoch, &f.last_commit_subject) {
            (Some(_), Some(s)) => format!("{}  {s}", when(f.last_commit_epoch)),
            _ => when(f.last_commit_epoch),
        },
    );
    // Whichever of the two claude signals is newer — "when did anything think in
    // here", which is not the same question as "when was this opened".
    row("last claude", when(f.claude_last_epoch.max(f.last_worked_epoch)));
    row("last entered", when(f.last_opened_epoch));
    row("uncommitted", f.dirty_files.to_string());
    row("ahead", format!("{} not on {}", f.ahead, f.base));
    // Named "base moved", never "behind": the number describes the base, not a
    // failing of this worktree, and the verdict above never counted it.
    row("base moved", format!("{} (informational)", f.behind));
    row(
        "upstream",
        match (&f.upstream, f.true_unpushed) {
            (Some(u), Some(k)) if k > 0 => format!("{u} ({k} not pushed)"),
            (Some(u), _) => u.clone(),
            (None, _) => "none — nothing pushed".to_string(),
        },
    );
    row("session", if f.tmux_up { "up".into() } else { "down".to_string() });
    row("lifecycle", f.lifecycle_effective.clone());
    if let Some(n) = f.note.as_deref().filter(|n| !n.is_empty()) {
        row("note", n.to_string());
    }

    if !f.not_on_base.is_empty() {
        ui.plain("");
        ui.plain(&format!("  commits not on {}:", f.base));
        for line in &f.not_on_base {
            ui.plain(&format!("    {line}"));
        }
        let shown = f.not_on_base.len() as u32;
        if f.not_on_base_total > shown {
            ui.plain(&format!("    …and {} more", f.not_on_base_total - shown));
        }
    }
}

// ── init — the suggestion flow (proposal §9) ─────────────────────────────────

/// `worktrees init [--print] [--diff] [-y] [--force]`.
///
/// Prints the `.worktrees.toml` this repo would get and asks. **Writes nothing
/// without confirmation** — and `CaptureUi::confirm` always answers no, so a
/// programmatic caller (the app, a script) safely declines rather than having a
/// config appear under it.
///
/// `--diff` is the re-run: over an existing config it emits only the `[[file]]`
/// stanzas that config is MISSING. Without it there is no second look at all —
/// this command refuses to run over an existing file, and `--force` re-renders
/// from scratch, which destroys every hand-written `mode = "copy"` and every
/// comment explaining why. Never writing is the point: the fragment goes to
/// stdout for a human to paste.
pub fn cmd_init(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut yes, mut force, mut print_only, mut diff) = (false, false, false, false);
    for a in args {
        match a.as_str() {
            "-y" | "--yes" => yes = true,
            // `--force` also skips the prompt: it is the "I know what I want"
            // flag, and asking after it would be theatre.
            "--force" => {
                force = true;
                yes = true;
            }
            "--print" | "--dry-run" => print_only = true,
            "--diff" => diff = true,
            s => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
        }
    }

    if diff {
        // Refused, not ignored — the same guard shape as `doctor --config-only
        // <name>`. `--diff` writes nothing and already prints to stdout, so
        // every one of these is incoherent with it; `--diff --force` in
        // particular reads as "rewrite the config with the diff applied", and
        // swallowing a destructive flag is the silent skip this codebase does
        // not do.
        if force || yes || print_only {
            ui.error("--diff prints the entries your config is missing; it never writes, so --force, -y and --print do not apply.");
            return 1;
        }
        return init_diff(p, ui);
    }

    let existing = Path::new(&p.main_root).join(projcfg::CONFIG_FILE);
    if existing.exists() && !force && !print_only {
        ui.error(&format!(
            "{} already exists — refusing to overwrite it. Edit it by hand, or: worktrees init --force",
            projcfg::CONFIG_FILE
        ));
        return 1;
    }

    let facts = init::probe(Path::new(&p.main_root), Path::new(p.wt_root_dir()));
    let sug = init::detect(&facts);
    let text = init::render(&sug);
    // The round-trip, enforced at runtime and not only in the unit test: nothing
    // this tool cannot itself read is ever written. `init.rs` keeps every
    // candidate it cannot declare commented out, so reaching this is a bug in the
    // emitter rather than a property of the repo.
    if let Err(e) = projcfg::parse(&text) {
        ui.error(&format!("internal: the generated config does not parse ({e}) — please report this"));
        return 1;
    }

    if sug.is_empty() {
        // `--print` still emits the FILE: its stdout is being redirected into
        // `.worktrees.toml`, and two lines of prose there is precisely the broken
        // config `new` and `doctor` refuse. An all-comments file parses clean.
        if print_only {
            print_config(ui, &text, sug.truncated);
            return 0;
        }
        // Exit 0 and say so plainly: "this repo needs no config" is a healthy
        // answer, not a failure.
        ui.info("Nothing to configure — no gitignored credential file, no compose stack publishing ports.");
        ui.info(&format!(
            "A repo with no {} behaves exactly as it does today. Re-run this after adding one.",
            projcfg::CONFIG_FILE
        ));
        if sug.truncated {
            ui.warn(TRUNCATED);
        }
        return 0;
    }

    // `--print` emits the FILE and nothing else — no header, no commentary — so
    // `worktrees init --print > .worktrees.toml` is a working move rather than a
    // trap that captures a banner into the config.
    if print_only {
        print_config(ui, &text, sug.truncated);
        return 0;
    }
    ui.header(&format!("{} for {}", projcfg::CONFIG_FILE, basename(&p.main_root)));
    for line in text.lines() {
        ui.plain(line);
    }
    describe(ui, &sug);
    if !yes && !ui.confirm(&format!("Write {}? [y/N] ", projcfg::CONFIG_FILE)) {
        ui.info("Nothing written.");
        return 0;
    }
    match init::write_config(Path::new(&p.main_root), &text) {
        Ok(path) => ui.info(&format!("wrote {}", path.display())),
        Err(e) => {
            ui.error(&format!("cannot write {}: {e}", projcfg::CONFIG_FILE));
            return 1;
        }
    }
    // §9's last row: every worktree that already exists predates this config, and
    // without relink they all stay stranded — the exact bug §1.2 describes.
    if !sug.stale_places.is_empty() {
        ui.warn(&format!(
            "{} existing worktree(s) are missing at least one declared file: {}",
            sug.stale_places.len(),
            sug.stale_places.join(", ")
        ));
        ui.info("Repair them now:  worktrees relink --all     (then check: worktrees doctor)");
    }
    if sug.ports.is_some() {
        ui.info("Allocate port slots:  worktrees provision --all");
    }
    ui.info("Commit it — this is project structure, like docker-compose.yml.");
    0
}

/// `worktrees init --diff` — the second look the flow never had.
///
/// Stdout carries the appendable fragment and NOTHING else, on the same rule as
/// `--print`: `worktrees init --diff >> .worktrees.toml` has to be a working
/// move rather than a trap that captures a banner into the config. Every line of
/// prose goes to stderr through `warn_aside`/`info`.
///
/// Writes nothing. `--force` is the only thing that rewrites this file, and it
/// rewrites it wholesale; the entries here are the half a human must place —
/// each one may need `mode = "copy"`, and nothing on disk knows which.
fn init_diff(p: &Project, ui: &mut dyn Ui) -> i32 {
    let cfg = match load_project_config(p, ui) {
        Ok(c) => c,
        Err(code) => return code,
    };
    // Nothing to diff AGAINST: the answer to "what is missing from your config"
    // when there is no config is the whole config, which is what `--print` emits.
    let Some((cfg, _)) = cfg else {
        ui.warn_aside(&format!(
            "No {} in this repo — printing the whole file instead of a diff.",
            projcfg::CONFIG_FILE
        ));
        return cmd_init(p, ui, &["--print".to_string()]);
    };

    let (found, truncated) = init::undeclared(Path::new(&p.main_root), &cfg);
    let (files, rejected) = init::declarable(found);
    if files.is_empty() && rejected.is_empty() {
        ui.info(&format!("{} declares every gitignored file found here.", projcfg::CONFIG_FILE));
        if truncated {
            ui.warn_aside(TRUNCATED);
        }
        return 0;
    }

    let text = init::render_undeclared(&files, &rejected);
    // The same round-trip guard `cmd_init` applies to a whole file: a fragment
    // this tool cannot itself read must never be handed to someone to paste.
    if let Err(e) = projcfg::parse(&text) {
        ui.error(&format!("internal: the generated fragment does not parse ({e}) — please report this"));
        return 1;
    }
    for line in text.lines() {
        ui.plain(line);
    }
    // Only when something is actually appendable. With every candidate rejected
    // the fragment is all comments, and "0 undeclared entries. Append to…" both
    // undercounts (the rejected paths ARE undeclared) and points at nothing.
    if let n @ 1.. = files.len() {
        let noun = if n == 1 { "entry" } else { "entries" };
        ui.warn_aside(&format!(
            "{n} undeclared {noun}. Append to {}, set mode = \"copy\" on anything a script \
             rewrites at runtime, then:  worktrees relink --all",
            projcfg::CONFIG_FILE
        ));
    }
    if !rejected.is_empty() {
        ui.warn_aside(&format!(
            "{} found file(s) cannot be declared — the config refuses the path. They are in the \
             fragment, commented out, with the reason.",
            rejected.len()
        ));
    }
    if truncated {
        ui.warn_aside(TRUNCATED);
    }
    0
}

/// The walk hit a bound. §9's promise is that this is REPORTED, never swallowed
/// — which is why it survives `--print`, where every other line of prose does not.
const TRUNCATED: &str =
    "The search stopped early (depth or size bound; hidden and vendor dirs are skipped) — add anything it missed by hand.";

/// `--print`: the file on stdout, and nothing else on stdout. The truncation
/// warning is the one thing still worth saying, so it goes to stderr rather than
/// nowhere — a caller redirecting stdout still sees it, and the file stays valid.
fn print_config(ui: &mut dyn Ui, text: &str, truncated: bool) {
    for line in text.lines() {
        ui.plain(line);
    }
    if truncated {
        ui.warn_aside(TRUNCATED);
    }
}

/// The human commentary that does NOT belong inside the file: why each section
/// showed up, and what the tool could not read.
fn describe(ui: &mut dyn Ui, sug: &init::Suggestion) {
    ui.plain("");
    match sug.credentials() {
        0 => {}
        1 => ui.warn(
            "1 of these is a credential — it fails SILENTLY when missing (that is the whole reason for this file).",
        ),
        n => ui.warn(&format!(
            "{n} of these are credentials — they fail SILENTLY when missing (that is the whole reason for this file)."
        )),
    }
    if let Some(ports) = &sug.ports {
        match &ports.commented {
            Some(why) => ui.warn(&format!("[ports] is commented out: {why}.")),
            None => ui.info(&format!("[ports] numbers were read out of {}. Check them.", ports.source)),
        }
    }
    if let Some(prefix) = &sug.prefix {
        ui.info(&format!(
            "[project] prefix = \"{prefix}\" was transcribed from {} — which still wins, so nothing is renamed. Delete that file to switch over.",
            init::PREFIX_FILE
        ));
    }
    // Found, and not declarable: §4 refuses these paths for the WHOLE config, so
    // the file lists them commented out. Said out loud too — a credential nobody
    // knows about is the failure this whole flow is for.
    if !sug.rejected.is_empty() {
        ui.warn(&format!(
            "{} found file(s) cannot be declared — the config refuses the path. They are in the file, commented out, with the reason.",
            sug.rejected.len()
        ));
    }
    if sug.truncated {
        ui.warn(TRUNCATED);
    }
}

/// §9's passive nudge, from `new`. One line, ONCE — see `init.rs`'s marker
/// comment for where "once" lives and why.
///
/// ⚠ Only the CREDENTIAL class counts here, not every `.env*`. `init` still
/// suggests both (§9's table has two file rows), but the nudge is unsolicited
/// output on a hot path, and it has to clear a higher bar than "this is a Node
/// repo". The credential class is the one §1.2 is actually about: a missing
/// `.env` breaks the build loudly on the next command, while a missing
/// `google-services.json` builds fine and dies silently on a device days later.
fn hint_init(p: &Project, ui: &mut dyn Ui) {
    let files: Vec<init::Candidate> = init::probe_files(Path::new(&p.main_root))
        .into_iter()
        .filter(|c| c.kind == init::Kind::Credential)
        .collect();
    let n = files.len();
    if n == 0 {
        return;
    }
    // Digested over exactly what the hint REPORTS, so gaining an unrelated
    // `.env.local` does not re-nag with a line that says nothing new.
    let digest = init::digest(&files);
    if init::hint_seen(Path::new(&p.main_root), &digest) {
        return;
    }
    let (noun, verb) = if n == 1 { ("file", "is") } else { ("files", "are") };
    ui.warn(&format!(
        "hint: {n} untracked credential {noun} in main {verb} not linked into this worktree — run 'worktrees init'"
    ));
    init::record_hint(Path::new(&p.main_root), &digest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worse_rc_is_order_independent_and_ranks_a_hard_failure_above_findings() {
        // THE bug: last-nonzero-wins made [1, 2] exit 2 and [2, 1] exit 1, so the
        // class depended on how `--all` sorted the directory.
        assert_eq!(worse_rc(worse_rc(0, 1), 2), 1);
        assert_eq!(worse_rc(worse_rc(0, 2), 1), 1);
        // clean stays clean; findings survive a clean neighbour
        assert_eq!(worse_rc(0, 0), 0);
        assert_eq!(worse_rc(worse_rc(0, 0), 2), 2);
        assert_eq!(worse_rc(worse_rc(0, 2), 0), 2);
        // an unrecognized nonzero is a hard failure, never downgraded to findings
        assert_eq!(worse_rc(2, 7), 7);
        assert_eq!(worse_rc(7, 2), 7);
    }

    #[test]
    fn worse_rc_ranks_a_needs_confirmation_stop_below_everything_nonzero() {
        // `close a b` where one name breaks (1) and the other stops to ask (4):
        // the caller must see the breakage whichever order they were walked in.
        let ask = crate::diag::EXIT_NEEDS_CONFIRM;
        assert_eq!(worse_rc(worse_rc(0, ask), 1), 1);
        assert_eq!(worse_rc(worse_rc(0, 1), ask), 1);
        assert_eq!(worse_rc(worse_rc(0, ask), 2), 2);
        // …but it still outranks a clean neighbour: the question must survive.
        assert_eq!(worse_rc(worse_rc(0, 0), ask), ask);
        assert_eq!(worse_rc(worse_rc(0, ask), 0), ask);
    }
}

//! Write ops — new/co, switch, open, rm — ported 1:1 from the bash cmd_* funcs.
//! Each `cmd_*` parses its raw args, emits every message via `Ui`, and returns
//! an exit code (guards → 1). git/tmux are shelled out. The bats suite gates
//! this against the bash CLI byte-for-byte.

use std::path::Path;

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

// ── (re)open a worktree's tmux session, then attach ──────────────────────────
// Returns 0 on success (session live / adopted / attached), 1 when tmux refuses
// to create the session (the reason is surfaced via ui.error — the app shows it
// and the CLI exits nonzero, instead of the old silent "Session ready" lie).
pub fn launch(p: &Project, ui: &mut dyn Ui, wt: &str, session_in: &str, install_cmd: &str, ai_cmd: &str, do_attach: bool) -> i32 {
    let keep = "exec \"${SHELL:-/bin/sh}\"";
    let ai_word_full = ai_cmd.split_whitespace().next().unwrap_or("");
    let ai_word = {
        let b = if ai_word_full.is_empty() { "claude" } else { ai_word_full };
        basename(b)
    };
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
        ui.warn(&format!("tmux session '{session}' already in this worktree — attaching."));
    } else {
        ui.header(&format!("Opening tmux session '{session}'"));
        let pane0 = if !ai_cmd.is_empty() {
            format!("exec \"${{SHELL:-/bin/sh}}\" -ic {}", tmux::sq(&format!("{ai_cmd}; {keep}")))
        } else {
            keep.to_string()
        };
        let pane1 = if !install_cmd.is_empty() {
            format!("{install_cmd} && echo '✓ deps ready'; {keep}")
        } else {
            keep.to_string()
        };
        match tmux::new_session(&session, wt, &pane0) {
            Ok(pid) => {
                tmux::tune_session(&session);
                tmux::split_window(&pid, wt, &pane1);
                tmux::select_pane(&pid);
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
pub fn cmd_new(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut do_install, mut do_tmux, mut do_attach, mut do_fetch, mut resume) = (true, true, true, true, false);
    let (mut branch, mut base, mut name, mut ai_flag) = (String::new(), String::new(), None::<String>, None::<String>);
    let mut expect = "";
    for arg in args {
        if !expect.is_empty() {
            if arg.starts_with('-') {
                ui.error(&format!("--{expect} needs a value (got '{arg}')"));
                return 1;
            }
            match expect {
                "name" => name = Some(arg.clone()),
                "ai" => ai_flag = Some(arg.clone()),
                _ => {}
            }
            expect = "";
            continue;
        }
        match arg.as_str() {
            "--no-install" => do_install = false,
            "--no-tmux" => do_tmux = false,
            "--no-attach" => do_attach = false,
            "--no-fetch" => do_fetch = false,
            "-r" | "--resume" => resume = true,
            "--name" => expect = "name",
            s if s.starts_with("--name=") => name = Some(s["--name=".len()..].to_string()),
            "--ai" => expect = "ai",
            s if s.starts_with("--ai=") => ai_flag = Some(s["--ai=".len()..].to_string()),
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
        if do_fetch && !git::git_ok(&p.main_root, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Fetching origin/{branch}..."));
            let _ = git::git(&p.main_root, &["fetch", "--quiet", "origin", &format!("refs/heads/{branch}:refs/remotes/origin/{branch}")]);
        }
        if git::git_ok(&p.main_root, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Checking out remote branch origin/{branch} (tracking)."));
            if let Err(e) = git::git_status_captured(&p.main_root, &["worktree", "add", "--track", "-b", &branch, &wt, &format!("origin/{branch}")]) {
                ui.error(&e);
                ui.error(&format!("Failed to add tracking worktree for origin/{branch} at {wt}."));
                return 1;
            }
        } else {
            if do_fetch {
                let _ = git::git(&p.main_root, &["fetch", "--quiet", "origin", &base]);
            }
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
    let rc = launch(p, ui, &wt, &session, &install_cmd, &ai_cmd, do_attach);
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
    launch(p, ui, &wt, &session, "", &ai_cmd, do_attach)
}

// ── close ────────────────────────────────────────────────────────────────────
/// End a place's tmux session; the worktree, branch, and declared state all
/// stay. The inverse of `open` — the place goes dormant, ready to re-enter.
pub fn cmd_close(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let mut names: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => names.push(s.to_string()),
        }
    }
    if names.is_empty() {
        ui.error("close needs a worktree (slug or branch), or 'main'. See: worktrees ls");
        return 1;
    }
    if !tmux::have_tmux() {
        ui.error("tmux not found");
        return 1;
    }
    let mut rc = 0;
    for n in &names {
        if close_one(p, ui, n).is_err() {
            rc = 1;
        }
    }
    rc
}

fn close_one(p: &Project, ui: &mut dyn Ui, name: &str) -> Result<(), i32> {
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
    let session = p.session_name(&slug);
    if tmux::session_exists(&session) {
        tmux::kill_session(&session);
        if slug == "(main)" {
            ui.info(&format!("closed tmux {session} — checkout untouched."));
        } else {
            ui.info(&format!("closed tmux {session} — worktree kept. Reopen: worktrees open {slug}"));
        }
        return Ok(());
    }
    // No canonical session — but `open` ADOPTS any session with a pane cwd'd in
    // the worktree (tmux::worktree_session), so close must be its inverse or an
    // adopted session becomes unclosable. Same ai-word derivation as launch().
    if slug != "(main)" {
        let wt = format!("{}/{}", p.wt_root_dir(), slug);
        let ai_cmd = crate::config::resolve_ai_cmd(None);
        let ai_word_full = ai_cmd.split_whitespace().next().unwrap_or("");
        let ai_word = basename(if ai_word_full.is_empty() { "claude" } else { ai_word_full });
        if let Some(adopted) = tmux::worktree_session(&wt, &ai_word) {
            tmux::kill_session(&adopted);
            ui.info(&format!("closed adopted tmux {adopted} (session living in '{slug}') — worktree kept."));
            return Ok(());
        }
    }
    ui.info(&format!("no live session for '{slug}' ({session}) — nothing to close."));
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
    let session = p.session_name(&slug);

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

    if tmux::session_exists(&session) {
        tmux::kill_session(&session);
        ui.info(&format!("killed tmux {session}"));
    }
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
/// attention. Any other nonzero is treated as a hard failure — an unrecognized
/// code is not something to downgrade.
fn worse_rc(acc: i32, one: i32) -> i32 {
    let rank = |rc: i32| match rc {
        0 => 0,
        crate::diag::EXIT_FINDINGS => 1,
        _ => 2,
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
    let file = wt_path.join(compose.file.as_str());
    // Layer B, once more: only a real file that resolves INSIDE this worktree is
    // handed to docker. A symlinked ancestor could otherwise point the compose
    // file (and its bind mounts) anywhere.
    let inside = match (std::fs::canonicalize(&file), std::fs::canonicalize(wt_path)) {
        (Ok(f), Ok(root)) => f.starts_with(&root) && f.is_file(),
        _ => false,
    };
    if !inside {
        ui.warn(&format!(
            "[compose] file `{}` is missing here — skipping teardown (containers may be left running)",
            compose.file.as_str()
        ));
        return;
    }
    // The place's OWN file first: the running containers are named after what the
    // stack was STARTED with, and the template may have changed since.
    let project = provision::compose_project_for(wt_path, compose, &p.prefix, slug);
    if project.is_empty() {
        ui.warn("[compose] project name is empty — skipping teardown");
        return;
    }
    ui.info(&format!("docker compose -p {project} down -v --remove-orphans"));
    match provision::compose_down(&file, &project) {
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

    let cfg = match load_project_config(p, ui) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let Some((cfg, mut findings)) = cfg else {
        // No config is a healthy repo, not a broken one.
        if json {
            emit_report(ui, &Report::default());
        } else {
            ui.info("No .worktrees.toml in this repo — nothing to check.");
        }
        return 0;
    };

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
    }

    if strict {
        // `--strict` is the only thing that lets a copy the source has moved past
        // fail a run; drift itself stays Info forever (§7).
        for f in &mut findings {
            if f.code == Code::CopyStale && f.severity == Severity::Warn {
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

// ── init — the suggestion flow (proposal §9) ─────────────────────────────────

/// `worktrees init [--print] [-y] [--force]`.
///
/// Prints the `.worktrees.toml` this repo would get and asks. **Writes nothing
/// without confirmation** — and `CaptureUi::confirm` always answers no, so a
/// programmatic caller (the app, a script) safely declines rather than having a
/// config appear under it.
pub fn cmd_init(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let (mut yes, mut force, mut print_only) = (false, false, false);
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
            s => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
        }
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
    if sug.is_empty() {
        // Exit 0 and say so plainly: "this repo needs no config" is a healthy
        // answer, not a failure.
        ui.info("Nothing to configure — no gitignored credential file, no compose stack publishing ports.");
        ui.info(&format!(
            "A repo with no {} behaves exactly as it does today. Re-run this after adding one.",
            projcfg::CONFIG_FILE
        ));
        if sug.truncated {
            ui.warn("(the search stopped early — deep, hidden and vendor directories were not walked)");
        }
        return 0;
    }

    let text = init::render(&sug);
    // The round-trip, enforced at runtime and not only in the unit test: nothing
    // this tool cannot itself read is ever written.
    if let Err(e) = projcfg::parse(&text) {
        ui.error(&format!("internal: the generated config does not parse ({e}) — please report this"));
        return 1;
    }

    // `--print` emits the FILE and nothing else — no header, no commentary — so
    // `worktrees init --print > .worktrees.toml` is a working move rather than a
    // trap that captures a banner into the config.
    if !print_only {
        ui.header(&format!("{} for {}", projcfg::CONFIG_FILE, basename(&p.main_root)));
    }
    for line in text.lines() {
        ui.plain(line);
    }
    if print_only {
        return 0;
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
    if sug.prefix.is_some() {
        ui.info("[project] prefix is commented out — it is not honored yet (changing a prefix renames live tmux sessions).");
    }
    if sug.truncated {
        ui.warn("The search stopped early (depth or size bound; hidden and vendor dirs are skipped) — add anything it missed by hand.");
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
}

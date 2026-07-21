//! Write ops — new/co, switch, open, rm — ported 1:1 from the bash cmd_* funcs.
//! Each `cmd_*` parses its raw args, emits every message via `Ui`, and returns
//! an exit code (guards → 1). git/tmux are shelled out. The bats suite gates
//! this against the bash CLI byte-for-byte.

use std::path::Path;

use crate::git;
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
pub fn launch(p: &Project, ui: &mut dyn Ui, wt: &str, session_in: &str, install_cmd: &str, ai_cmd: &str, do_attach: bool) {
    let _ = p;
    let keep = "exec \"${SHELL:-/bin/sh}\"";
    let ai_word_full = ai_cmd.split_whitespace().next().unwrap_or("");
    let ai_word = {
        let b = if ai_word_full.is_empty() { "claude" } else { ai_word_full };
        basename(b)
    };
    let mut session = session_in.to_string();
    if !tmux::session_exists(&session) {
        if let Some(existing) = tmux::worktree_session(wt, &ai_word) {
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
        if let Some(pid) = tmux::new_session(&session, wt, &pane0) {
            tmux::split_window(&pid, wt, &pane1);
            tmux::select_pane(&pid);
        }
    }
    if !do_attach {
        ui.info(&format!("Session ready (detached). Attach with: tmux attach -t {session}"));
        return;
    }
    tmux::attach_or_switch(&session);
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
    let switch_fail = |ui: &mut dyn Ui| -> Result<(), i32> {
        ui.error("git switch failed. If the branch is checked out in another worktree:");
        if let Some(list) = git::git_out(main, &["worktree", "list"]) {
            ui.plain(&indent(&list));
        }
        Err(1)
    };

    if git::git_ok(main, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]) {
        ui.info("Branch exists locally — switching.");
        if !git::git_status(wt, &["switch", branch]) {
            return switch_fail(ui);
        }
    } else {
        if do_fetch && !git::git_ok(main, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Fetching origin/{branch}..."));
            let _ = git::git(main, &["fetch", "--quiet", "origin", &format!("refs/heads/{branch}:refs/remotes/origin/{branch}")]);
        }
        if git::git_ok(main, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Tracking remote branch origin/{branch}."));
            if !git::git_status(wt, &["switch", branch]) {
                return switch_fail(ui);
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
            if !git::git_status(wt, &["switch", "-c", branch, &start]) {
                return switch_fail(ui);
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
        if !git::git_status(&p.main_root, &["worktree", "add", &wt, &branch]) {
            return 1;
        }
    } else {
        if do_fetch && !git::git_ok(&p.main_root, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Fetching origin/{branch}..."));
            let _ = git::git(&p.main_root, &["fetch", "--quiet", "origin", &format!("refs/heads/{branch}:refs/remotes/origin/{branch}")]);
        }
        if git::git_ok(&p.main_root, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]) {
            ui.info(&format!("Checking out remote branch origin/{branch} (tracking)."));
            if !git::git_status(&p.main_root, &["worktree", "add", "--track", "-b", &branch, &wt, &format!("origin/{branch}")]) {
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
            if !git::git_status(&p.main_root, &["worktree", "add", "-b", &branch, &wt, &start]) {
                return 1;
            }
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
        return 0;
    }
    launch(p, ui, &wt, &session, &install_cmd, &ai_cmd, do_attach);
    0
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
    launch(p, ui, &wt, &session, "", &ai_cmd, do_attach);
    0
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

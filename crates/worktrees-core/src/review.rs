//! The review lane for the `agent-instructions` branch.
//!
//! `rules apply` and `agent-setup fix` commit locally and push only when asked,
//! which used to leave a branch nothing pointed at: no place, a To do row that
//! said "push it and open a PR" in a repository with no remote, and the rules
//! rows hidden behind it. Now the branch gets a PLACE with a brief, and the
//! agent that opens there explains the change and does what the user decides.
//!
//! The brief is a fixed template in this binary (`guidance/review-brief.md`),
//! filled with facts worktrees computes — the base ref, whether `origin`
//! exists, the files the branch changes. Repository text never becomes an
//! instruction (ADR 0001/0002): paths go in a fenced block marked as data, and
//! a path worktrees did not write is called out rather than described.
//!
//! The two decisions are verbs, not prose the agent improvises: `rules land`
//! and `rules abandon`. Both are scoped to `agent-instructions` alone, and both
//! run every git step BEFORE ending the place's tmux sessions — the agent that
//! runs them lives in one of those sessions, and `worktrees rm` kills first,
//! which would cut the removal off halfway.

use crate::agentfiles::{self, FIX_BRANCH};
use crate::git;
use crate::project::Project;
use crate::ui::Ui;

const BRIEF: &str = include_str!("guidance/review-brief.md");

/// The ref the branch is read from: the local branch, else origin's.
fn branch_ref(root: &str) -> Option<String> {
    let has = |r: &str| git::git_ok(root, &["show-ref", "--verify", "-q", r]);
    if has(&format!("refs/heads/{FIX_BRANCH}")) {
        Some(FIX_BRANCH.to_string())
    } else if has(&format!("refs/remotes/origin/{FIX_BRANCH}")) {
        Some(format!("origin/{FIX_BRANCH}"))
    } else {
        None
    }
}

/// One file the branch changes, with what worktrees meant it to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileNote {
    /// `A`, `M`, `D`, `R` (git's letter, similarity dropped).
    pub status: String,
    pub path: String,
    /// The old path of a rename.
    pub from: Option<String>,
    /// What it is, in worktrees' words; `None` = not a file worktrees writes.
    pub what: Option<&'static str>,
}

/// What worktrees writes at `path`, from a fixed table. Anything else on the
/// branch was not written by worktrees.
fn what_is(status: &str, path: &str, from: Option<&str>) -> Option<&'static str> {
    let base = path.rsplit('/').next().unwrap_or(path);
    let renamed_from_claude = from.is_some_and(|f| f.rsplit('/').next() == Some(agentfiles::CLAUDE));
    match (status, base) {
        _ if path == crate::rules::SKILL_PATH => Some("the close-out skill — managed by worktrees, updated by `worktrees rules apply`"),
        _ if path == crate::rules::CONFIG_PATH => Some("this repository's close-out settings — GENERIC defaults: fill in the real gates and paths before applying"),
        ("R", "AGENTS.md") if renamed_from_claude => Some("CLAUDE.md moved to AGENTS.md, so every agent reads it"),
        ("A" | "M", "AGENTS.md") if path == agentfiles::AGENTS => Some("the agent instructions; the worktrees rules block is at its end"),
        ("A" | "M", "AGENTS.md") => Some("the agent instructions for this folder"),
        ("A" | "M", "CLAUDE.md") => Some("a stub that imports AGENTS.md, so Claude reads the same words"),
        ("A", _) if path.starts_with(".agents/skills/") => Some("a link so Codex sees the repository skill of the same name"),
        _ => None,
    }
}

/// A repository path as data: one line, no backticks or control characters.
fn tame(path: &str) -> String {
    path.chars().map(|c| if c.is_control() || c == '`' { '?' } else { c }).collect()
}

/// The files `agent-instructions` changes against the base, per git.
pub fn changed_files(root: &str, base: &str, branch: &str) -> Result<Vec<FileNote>, String> {
    let out = agentfiles::git_env(root, None, &["diff", "--name-status", "-z", "-M", &format!("{base}...{branch}")], None)?;
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    let mut v = Vec::new();
    while let Some(st) = fields.next() {
        let status: String = st.chars().take(1).collect();
        let (from, path) = if status == "R" || status == "C" {
            let from = fields.next().unwrap_or_default().to_string();
            (Some(from), fields.next().unwrap_or_default().to_string())
        } else {
            (None, fields.next().unwrap_or_default().to_string())
        };
        let what = what_is(&status, &path, from.as_deref());
        v.push(FileNote { status, path, from, what });
    }
    Ok(v)
}

/// The filled brief. `Err` when there is no branch to review.
pub fn brief(p: &Project) -> Result<String, String> {
    let root = p.main_root.as_str();
    let Some(branch) = branch_ref(root) else {
        return Err(format!("there is no '{FIX_BRANCH}' branch to review"));
    };
    let base = p.base_ref();
    let default = p.default_base();
    let origin = agentfiles::has_origin(root);
    let files = changed_files(root, &base, &branch)?;
    let commits = git::git_out(root, &["rev-list", "--count", &format!("{base}..{branch}")]).unwrap_or_else(|| "?".into());
    let mut list = String::new();
    for f in &files {
        let path = match &f.from {
            Some(from) => format!("{} -> {}", tame(from), tame(&f.path)),
            None => tame(&f.path),
        };
        let what = f.what.unwrap_or("NOT written by worktrees — inspect it before applying");
        list.push_str(&format!("{}  {path}\n    {what}\n", f.status));
    }
    if list.is_empty() {
        list.push_str("(no file differs from the base)\n");
    }
    let foreign = files.iter().filter(|f| f.what.is_none()).count();
    let foreign = if foreign > 0 {
        format!("**{foreign} of these are not files worktrees writes.** Someone added them to the branch; show the user what they are before anything is applied.\n\n")
    } else {
        String::new()
    };
    let pushed = git::git_ok(root, &["show-ref", "--verify", "-q", &format!("refs/remotes/origin/{FIX_BRANCH}")]);
    let remote = match (origin, pushed) {
        (false, _) => "No `origin` remote: there is nothing to push to and no pull request to open. Applying means merging into the default branch here.".to_string(),
        (true, false) => "An `origin` remote exists; the branch has not been pushed.".to_string(),
        (true, true) => "An `origin` remote exists and the branch is already on it.".to_string(),
    };
    let apply = if origin {
        format!(
            "Commit anything you changed here first. Then run `worktrees rules land`. It pushes this branch to origin and opens a pull request against `{default}` (with the GitHub CLI); this place stays open until the PR merges. Once it is merged, `worktrees rules land` again — or the app's To do row — removes this place and the branch."
        )
    } else {
        format!(
            "Commit anything you changed here first: it refuses a dirty place. Then run `worktrees rules land`. It fast-forwards `{default}` in the repository root to this branch — only when the root is on `{default}`, has no uncommitted changes to tracked files, and this branch is strictly ahead — then deletes the branch, removes this place and ends this session. If it refuses, tell the user its reason; do not work around it."
        )
    };
    let abandon_remote = if origin { " (and its copy on origin, if it is the same commit)" } else { "" };
    Ok(BRIEF
        .replace("{root}", root)
        .replace("{base}", &base)
        .replace("{default}", &default)
        .replace("{commits}", &commits)
        .replace("{remote}", &remote)
        .replace("{files}", list.trim_end())
        .replace("{foreign}", &foreign)
        .replace("{apply}", &apply)
        .replace("{abandon_remote}", abandon_remote))
}

/// The place that holds the branch, if any. Only a place under `.worktrees/`
/// counts: the branch checked out in the root is never removed from here.
pub fn lane_place(p: &Project) -> Option<String> {
    let wt = p.wt_for_branch(FIX_BRANCH)?;
    let under = format!("{}/", p.wt_root_dir());
    wt.starts_with(&under).then_some(wt)
}

/// Open the review lane: a place on `agent-instructions` with the brief.
/// `launch` also starts the agent (the app); without it the place is created
/// with no tmux session and `worktrees open` starts the agent on the brief
/// later (the CLI). Returns `cmd_new`'s rc — `diag::EXIT_LAUNCH_REFUSED` when
/// the machine guard refused the agent and the place is ready without one —
/// and the slug.
pub fn open_lane(p: &Project, ui: &mut dyn Ui, launch: bool, provider: Option<&str>, model: Option<&str>) -> (i32, String) {
    let slug = p.resolve_new_slug(FIX_BRANCH, None);
    let text = match brief(p) {
        Ok(t) => t,
        Err(e) => {
            ui.error(&e);
            return (1, slug);
        }
    };
    let mut args: Vec<String> = vec![FIX_BRANCH.into(), "--no-attach".into(), "--no-spare".into(), "--no-install".into(), "--brief".into(), text];
    if !launch {
        args.push("--no-tmux".into());
    }
    if let Some(pr) = provider {
        args.push("--ai".into());
        args.push(pr.into());
    }
    if let Some(m) = model.filter(|m| !m.trim().is_empty()) {
        args.push("--model".into());
        args.push(m.into());
    }
    (crate::ops::cmd_new(p, ui, &args), slug)
}

/// The CLI's half after `rules apply` / `agent-setup fix` committed: open the
/// lane. Without `launch` it is a place and a brief, and `worktrees open`
/// starts the agent — a CLI run is often a script or an agent in a shell, and
/// a second agent spending quota behind it is the wrong default. Returns the
/// rc to exit with (`cmd_new`'s; the commit itself already succeeded).
pub fn cli_lane(p: &Project, ui: &mut dyn Ui, launch: bool) -> i32 {
    let (rc, slug) = open_lane(p, ui, launch, None, None);
    match (rc, launch) {
        (0, false) => ui.info(&format!("Review lane ready in place '{slug}': `worktrees open {slug}` starts an agent on its brief, which explains the change and lands or abandons it on your word.")),
        (0, true) => ui.info(&format!("Review lane started in place '{slug}': its agent explains the change and lands or abandons it on your word.")),
        (crate::diag::EXIT_LAUNCH_REFUSED, _) => ui.info(&format!("The review place '{slug}' is ready without an agent.")),
        _ => {}
    }
    rc
}

/// What GitHub says about the branch's pull request: `OPEN`, `MERGED`,
/// `CLOSED`, or `None` (no PR, no gh, offline). Asked, never inferred: a
/// squash merge leaves the branch looking unmerged, and a subject that matches
/// main's log proves nothing.
pub fn pr_state(root: &str) -> Option<String> {
    pr_state_with(root, &agentfiles::gh_bin())
}

fn pr_state_with(root: &str, gh: &str) -> Option<String> {
    let mut c = std::process::Command::new(gh);
    c.current_dir(root).args(["pr", "view", FIX_BRANCH, "--json", "state", "--jq", ".state"]);
    let out = agentfiles::run_net(c, "gh pr view").ok()?;
    let s = out.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// The tmux session this process runs in, if any.
fn own_session() -> Option<String> {
    let pane = std::env::var("TMUX_PANE").ok().filter(|v| !v.is_empty())?;
    std::env::var("TMUX").ok().filter(|v| !v.is_empty())?;
    let o = crate::tmux::tmux(&["display-message", "-p", "-t", &pane, "#{session_name}"]).ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !s.is_empty()).then_some(s)
}

/// End `sessions`, this process's own session LAST — killing it ends us.
fn kill_last(sessions: Vec<String>) {
    let own = own_session();
    let (mine, others): (Vec<_>, Vec<_>) = sessions.into_iter().partition(|s| Some(s) == own.as_ref());
    for s in others {
        crate::tmux::kill_session(&s);
    }
    for s in mine {
        crate::tmux::kill_session(&s);
    }
}

/// Remove the place, delete the branch (`-D` when `force`), optionally the
/// origin branch, then end the sessions. Every git step first; the kill last.
/// `farewell` is printed between the two — the last thing the agent sees.
fn retire(p: &Project, ui: &mut dyn Ui, force: bool, delete_origin: bool, farewell: &str) -> i32 {
    let root = p.main_root.as_str();
    let tip = git::git_out(root, &["rev-parse", "--verify", "-q", &format!("refs/heads/{FIX_BRANCH}")]);
    let sessions = match lane_place(p) {
        Some(wt) => match crate::ops::remove_tree(p, ui, &wt) {
            Ok(s) => s,
            Err(rc) => return rc,
        },
        None => Vec::new(),
    };
    let mut done = Vec::new();
    if tip.is_some() {
        let flag = if force { "-D" } else { "-d" };
        if git::git_ok(root, &["branch", flag, FIX_BRANCH]) {
            done.push(format!("deleted branch {FIX_BRANCH}"));
        } else {
            ui.warn(&format!("branch '{FIX_BRANCH}' was not deleted (git branch {flag} refused)"));
        }
    }
    if delete_origin {
        let remote = git::git_out(root, &["rev-parse", "--verify", "-q", &format!("refs/remotes/origin/{FIX_BRANCH}")]);
        match (remote, &tip) {
            (Some(r), Some(t)) if &r == t => match agentfiles::git_net(root, &["push", "--quiet", "origin", "--delete", FIX_BRANCH]) {
                Ok(_) => done.push(format!("deleted origin/{FIX_BRANCH}")),
                Err(e) => ui.warn(&format!("origin/{FIX_BRANCH} was not deleted: {e}")),
            },
            (Some(_), _) => ui.warn(&format!("origin/{FIX_BRANCH} is a different commit from the local branch — left alone")),
            (None, _) => {}
        }
    }
    for d in &done {
        ui.info(d);
    }
    ui.info(farewell);
    kill_last(sessions);
    0
}

/// Refuse unless the branch is reviewable from here: it exists, and any place
/// holding it is a place (not the root).
fn scoped(p: &Project, ui: &mut dyn Ui) -> Result<(), i32> {
    if let Some(msg) = crate::sync::hub_copy_refusal(std::path::Path::new(&p.main_root)) {
        ui.error(&msg);
        return Err(1);
    }
    if branch_ref(&p.main_root).is_none() {
        ui.error(&format!("there is no '{FIX_BRANCH}' branch — nothing to land or abandon"));
        return Err(1);
    }
    let root_head = git::git_out(&p.main_root, &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default();
    if root_head == FIX_BRANCH || (p.wt_for_branch(FIX_BRANCH).is_some() && lane_place(p).is_none()) {
        ui.error(&format!("'{FIX_BRANCH}' is checked out in the repository root (or outside .worktrees/) — switch the root back to its default branch first"));
        return Err(1);
    }
    Ok(())
}

/// `worktrees rules land`: apply the branch.
///
/// - No origin: fast-forward the default branch IN THE ROOT checkout (the
///   branch is checked out there; moving it from a side worktree would leave
///   the root stale), then retire the place and the merged branch.
/// - Origin: push and open a PR, place stays. Once GitHub says MERGED, retire.
pub fn land(p: &Project, ui: &mut dyn Ui) -> i32 {
    land_with(p, ui, &agentfiles::gh_bin())
}

pub fn land_with(p: &Project, ui: &mut dyn Ui, gh: &str) -> i32 {
    if let Err(rc) = scoped(p, ui) {
        return rc;
    }
    let root = p.main_root.as_str();
    let default = p.default_base();
    if let Some(wt) = lane_place(p) {
        // Tracked changes only: the brief itself lives in an untracked
        // `.planning/`, ignored or not depending on the repository.
        let dirty = git::git_out(&wt, &["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
        if !dirty.is_empty() {
            ui.error("the review place has uncommitted changes — commit them on the branch (or discard them) first:");
            ui.plain(&dirty);
            return 1;
        }
    }
    let local = git::git_ok(root, &["show-ref", "--verify", "-q", &format!("refs/heads/{FIX_BRANCH}")]);
    if !agentfiles::has_origin(root) {
        if !local {
            ui.error(&format!("there is no local '{FIX_BRANCH}' branch to merge"));
            return 1;
        }
        let head = git::git_out(root, &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default();
        if head != default {
            ui.error(&format!("the repository root is on '{head}', not '{default}' — switch it back, then run this again"));
            return 1;
        }
        let changes = git::git_out(root, &["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
        if !changes.is_empty() {
            ui.error(&format!("the repository root has uncommitted changes — commit or stash them, then run this again:\n{changes}"));
            return 1;
        }
        let same = git::git_out(root, &["rev-parse", &default]) == git::git_out(root, &["rev-parse", FIX_BRANCH]);
        if same || !git::git_ok(root, &["merge-base", "--is-ancestor", &default, FIX_BRANCH]) {
            ui.error(&format!(
                "'{FIX_BRANCH}' is not strictly ahead of '{default}' — {default} moved since it was built. Abandon it (`worktrees rules abandon --yes`) and apply the rules again"
            ));
            return 1;
        }
        if let Err(e) = git::git_status_captured(root, &["merge", "--ff-only", "--quiet", FIX_BRANCH]) {
            ui.error(&format!("git merge --ff-only refused: {e}"));
            return 1;
        }
        ui.info(&format!("fast-forwarded {default} to {FIX_BRANCH}"));
        return retire(p, ui, false, false, &format!("Applied to {default}. The review place is removed; this session ends now."));
    }
    let pushed = git::git_ok(root, &["show-ref", "--verify", "-q", &format!("refs/remotes/origin/{FIX_BRANCH}")]);
    if pushed {
        match pr_state_with(root, gh).as_deref() {
            Some("MERGED") => return retire_merged_unchecked(p, ui),
            Some("CLOSED") => {
                ui.error("its pull request was closed without merging — `worktrees rules abandon --yes` cleans up");
                return 1;
            }
            Some("OPEN") => {
                if local && git::git_out(root, &["rev-parse", FIX_BRANCH]) != git::git_out(root, &["rev-parse", &format!("origin/{FIX_BRANCH}")]) {
                    match agentfiles::git_net(root, &["push", "--quiet", "origin", &format!("{FIX_BRANCH}:{FIX_BRANCH}")]) {
                        Ok(_) => ui.info("pushed the new commits to the open pull request"),
                        Err(e) => {
                            ui.error(&format!("push failed: {e}"));
                            return 1;
                        }
                    }
                }
                ui.info("The pull request is open — merge it, then run `worktrees rules land` again to remove this place.");
                return 0;
            }
            _ => {}
        }
    }
    if !local {
        ui.error(&format!("there is no local '{FIX_BRANCH}' branch to push"));
        return 1;
    }
    let refspec = format!("refs/heads/{FIX_BRANCH}:refs/heads/{FIX_BRANCH}");
    if let Err(e) = agentfiles::git_net(root, &["push", "--quiet", "-u", "origin", &refspec]) {
        ui.error(&format!("push failed: {e}"));
        return 1;
    }
    ui.info(&format!("pushed {FIX_BRANCH} to origin"));
    let mut c = std::process::Command::new(gh);
    c.current_dir(root).args(["pr", "create", "--base", &default, "--head", FIX_BRANCH, "--fill"]);
    match agentfiles::run_net(c, "gh pr create") {
        Ok(out) => {
            if let Some(u) = out.lines().rev().find(|l| l.starts_with("http")) {
                ui.info(&format!("opened {u}"));
            }
            ui.info("The place stays open until the pull request merges; then `worktrees rules land` again removes it.");
            0
        }
        Err(e) => {
            ui.warn(&format!("pushed, but {e} — open the pull request yourself"));
            0
        }
    }
}

/// Remove the review place once its PR is MERGED — the app's "Remove the
/// place" on the To do row. Re-asks GitHub rather than trusting the row: the
/// branch is unmerged as far as git can tell after a squash, so this deletes
/// it with `-D`, and only GitHub's word justifies that.
pub fn retire_merged(p: &Project, ui: &mut dyn Ui) -> i32 {
    retire_merged_with(p, ui, &agentfiles::gh_bin())
}

pub fn retire_merged_with(p: &Project, ui: &mut dyn Ui, gh: &str) -> i32 {
    if let Err(rc) = scoped(p, ui) {
        return rc;
    }
    match pr_state_with(&p.main_root, gh).as_deref() {
        Some("MERGED") => retire_merged_unchecked(p, ui),
        Some(s) => {
            ui.error(&format!("the pull request for '{FIX_BRANCH}' is {s}, not merged — nothing was removed"));
            1
        }
        None => {
            ui.error(&format!("could not ask GitHub about '{FIX_BRANCH}' (no PR, no gh, or offline) — nothing was removed"));
            1
        }
    }
}

fn retire_merged_unchecked(p: &Project, ui: &mut dyn Ui) -> i32 {
    retire(p, ui, true, true, "The pull request is merged. The review place is removed; this session ends now.")
}

/// `worktrees rules abandon --yes`: delete the branch (force: it is unmerged
/// by definition), remove the place, end the session. Origin's copy goes too,
/// but only when it is the same commit as the local branch.
pub fn abandon(p: &Project, ui: &mut dyn Ui, yes: bool) -> i32 {
    if let Err(rc) = scoped(p, ui) {
        return rc;
    }
    if !yes {
        ui.error(&format!(
            "this deletes '{FIX_BRANCH}' (and origin's copy if it is the same commit), removes its place and ends that place's session — anything uncommitted there is lost. Run again with --yes once the user has said so."
        ));
        return 1;
    }
    retire(p, ui, true, true, &format!("Abandoned {FIX_BRANCH}; redo it any time from Project settings → Worktrees rules. This session ends now."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_names_what_worktrees_writes_and_nothing_else() {
        assert!(what_is("M", "AGENTS.md", None).unwrap().contains("rules block"));
        assert!(what_is("R", "AGENTS.md", Some("CLAUDE.md")).unwrap().contains("moved"));
        assert!(what_is("A", "docs/CLAUDE.md", None).unwrap().contains("stub"));
        assert!(what_is("A", crate::rules::CONFIG_PATH, None).unwrap().contains("GENERIC"));
        assert!(what_is("A", crate::rules::SKILL_PATH, None).is_some());
        assert!(what_is("A", ".agents/skills/close-out", None).is_some());
        assert_eq!(what_is("A", "scripts/run.sh", None), None);
        assert_eq!(what_is("D", "AGENTS.md", None), None, "worktrees never deletes AGENTS.md");
    }

    #[test]
    fn a_path_is_one_line_of_data() {
        assert_eq!(tame("a\nb`c"), "a?b?c");
    }

    fn sh(dir: &std::path::Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args)
            .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
            .output().unwrap();
        assert!(o.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim_end().to_string()
    }

    fn ok(dir: &std::path::Path, args: &[&str]) -> bool {
        std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success()
    }

    /// A repo with the rules applied on `agent-instructions` (local only) and
    /// its review place open with no tmux. `origin` adds a bare remote.
    fn fixture(name: &str, origin: bool) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let t = std::fs::canonicalize(std::env::temp_dir()).unwrap().join(format!("wt-review-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let repo = t.join("myproj");
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "t"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        std::fs::write(repo.join("AGENTS.md"), "# ours\n").unwrap();
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        if origin {
            let o = t.join("origin.git");
            sh(&t, &["init", "-q", "--bare", "-b", "main", o.to_str().unwrap()]);
            sh(&repo, &["remote", "add", "origin", o.to_str().unwrap()]);
            sh(&repo, &["push", "-q", "-u", "origin", "main"]);
        }
        let p = Project::discover(&repo).unwrap();
        let out = crate::rules::apply_with(&p, "/nonexistent/gh", false, false).unwrap();
        assert!(out.commit.is_some(), "{:?}", out.notes);
        let mut ui = crate::ui::CaptureUi::default();
        let (rc, slug) = open_lane(&p, &mut ui, false, None, None);
        assert_eq!(rc, 0, "{:?}", ui.lines);
        let place = repo.join(".worktrees").join(slug);
        assert!(place.join(crate::ops::BRIEF_PATH).is_file());
        (t, repo, place)
    }

    fn fake_gh(t: &std::path::Path) -> (String, std::path::PathBuf) {
        let state = t.join("pr-state");
        let gh = t.join("gh");
        std::fs::write(&gh, format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  'pr view') [ -f '{s}' ] && cat '{s}' || exit 1 ;;\n  'pr create') echo https://example.invalid/pr/1; echo OPEN > '{s}' ;;\nesac\n",
            s = state.display()
        )).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        (gh.to_string_lossy().into_owned(), state)
    }

    /// The brief is the template with computed facts: the files, what each
    /// is, no origin — and a file worktrees did not write, called out.
    #[test]
    fn the_brief_lists_the_files_and_calls_out_a_foreign_one() {
        let (t, repo, place) = fixture("brief", false);
        std::fs::write(place.join("run.sh"), "echo hi\n").unwrap();
        sh(&place, &["add", "run.sh"]);
        sh(&place, &["commit", "-q", "-m", "extra"]);
        let b = brief(&Project::discover(&repo).unwrap()).unwrap();
        assert!(b.contains("No `origin` remote"), "{b}");
        assert!(b.contains("M  AGENTS.md\n    the agent instructions; the worktrees rules block is at its end"), "{b}");
        assert!(b.contains(".claude/close-out.md\n    this repository's close-out settings — GENERIC"), "{b}");
        assert!(b.contains("A  run.sh\n    NOT written by worktrees"), "{b}");
        assert!(b.contains("**1 of these are not files worktrees writes.**"), "{b}");
        assert!(b.contains("Commits on the branch: 2"), "{b}");
        assert!(b.contains("worktrees rules abandon --yes"), "{b}");
        assert!(!b.contains('{'), "an unfilled placeholder: {b}");
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Without origin, land fast-forwards main IN THE ROOT — and only when
    /// the root is on main, clean, and the place has nothing uncommitted.
    #[test]
    fn land_without_origin_refuses_until_safe_then_fast_forwards() {
        let (t, repo, place) = fixture("land", false);
        let p = Project::discover(&repo).unwrap();
        let tip = sh(&repo, &["rev-parse", FIX_BRANCH]);
        let main = sh(&repo, &["rev-parse", "main"]);
        let land = |why: &str| {
            let mut ui = crate::ui::CaptureUi::default();
            let rc = land_with(&p, &mut ui, "/nonexistent/gh");
            (rc, ui.lines.join("\n"), why.to_string())
        };

        std::fs::write(repo.join("README.md"), "edited\n").unwrap();
        let (rc, out, _) = land("dirty root");
        assert!(rc == 1 && out.contains("root has uncommitted changes"), "{out}");
        sh(&repo, &["checkout", "-q", "README.md"]);

        std::fs::write(place.join("README.md"), "edited\n").unwrap();
        let (rc, out, _) = land("dirty place");
        assert!(rc == 1 && out.contains("review place has uncommitted changes"), "{out}");
        sh(&place, &["checkout", "-q", "README.md"]);

        sh(&repo, &["checkout", "-q", "-b", "elsewhere"]);
        let (rc, out, _) = land("root off main");
        assert!(rc == 1 && out.contains("not 'main'"), "{out}");
        sh(&repo, &["checkout", "-q", "main"]);
        assert_eq!(sh(&repo, &["rev-parse", "main"]), main, "every refusal left main alone");

        let (rc, out, _) = land("ok");
        assert_eq!(rc, 0, "{out}");
        assert_eq!(sh(&repo, &["rev-parse", "main"]), tip);
        assert!(sh(&repo, &["status", "--porcelain", "--untracked-files=no"]).is_empty());
        assert!(!ok(&repo, &["show-ref", "--verify", "-q", "refs/heads/agent-instructions"]));
        assert!(!place.exists());
        assert!(out.ends_with("this session ends now."), "the farewell is the last line: {out}");
        let _ = std::fs::remove_dir_all(&t);
    }

    /// A main that moved on cannot be fast-forwarded: land refuses, abandon
    /// asks for --yes, then deletes the unmerged branch and the place.
    #[test]
    fn a_diverged_branch_refuses_land_and_abandon_needs_yes() {
        let (t, repo, place) = fixture("abandon", false);
        let p = Project::discover(&repo).unwrap();
        std::fs::write(repo.join("README.md"), "moved on\n").unwrap();
        sh(&repo, &["commit", "-q", "-am", "moved"]);
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(land_with(&p, &mut ui, "/nonexistent/gh"), 1);
        assert!(ui.lines.join("\n").contains("not strictly ahead"), "{:?}", ui.lines);
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(abandon(&p, &mut ui, false), 1);
        assert!(place.exists() && ok(&repo, &["show-ref", "--verify", "-q", "refs/heads/agent-instructions"]));
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(abandon(&p, &mut ui, true), 0, "{:?}", ui.lines);
        assert!(!place.exists());
        assert!(!ok(&repo, &["show-ref", "--verify", "-q", "refs/heads/agent-instructions"]));
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(land_with(&p, &mut ui, "/nonexistent/gh"), 1, "nothing left to land");
        let _ = std::fs::remove_dir_all(&t);
    }

    /// With origin: land pushes and opens a PR and the place stays; once
    /// GitHub says MERGED, land retires the place and both branches.
    #[test]
    fn land_with_origin_opens_a_pr_then_retires_once_merged() {
        let (t, repo, place) = fixture("origin", true);
        let p = Project::discover(&repo).unwrap();
        let (gh, state) = fake_gh(&t);
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(land_with(&p, &mut ui, &gh), 0, "{:?}", ui.lines);
        assert!(ui.lines.iter().any(|l| l.contains("https://example.invalid/pr/1")), "{:?}", ui.lines);
        assert!(place.exists(), "the place stays until the PR merges");
        assert!(ok(&repo, &["show-ref", "--verify", "-q", "refs/remotes/origin/agent-instructions"]));

        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(land_with(&p, &mut ui, &gh), 0);
        assert!(place.exists(), "OPEN is not MERGED: {:?}", ui.lines);

        std::fs::write(&state, "MERGED\n").unwrap();
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(land_with(&p, &mut ui, &gh), 0, "{:?}", ui.lines);
        assert!(!place.exists());
        assert!(!ok(&repo, &["show-ref", "--verify", "-q", "refs/heads/agent-instructions"]));
        assert!(sh(&repo, &["ls-remote", "origin", "agent-instructions"]).is_empty());
        let _ = std::fs::remove_dir_all(&t);
    }

    /// The app's "Remove the place" re-asks GitHub: OPEN removes nothing.
    #[test]
    fn retire_merged_needs_github_to_say_merged() {
        let (t, repo, place) = fixture("retire", true);
        let p = Project::discover(&repo).unwrap();
        let (gh, state) = fake_gh(&t);
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(retire_merged_with(&p, &mut ui, &gh), 1, "no PR at all");
        assert_eq!(land_with(&p, &mut ui, &gh), 0);
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(retire_merged_with(&p, &mut ui, &gh), 1);
        assert!(place.exists() && ui.lines.join("").contains("OPEN"), "{:?}", ui.lines);
        std::fs::write(&state, "MERGED\n").unwrap();
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(retire_merged_with(&p, &mut ui, &gh), 0, "{:?}", ui.lines);
        assert!(!place.exists());
        let _ = std::fs::remove_dir_all(&t);
    }

    /// The report names the review place and whether there is an origin.
    #[test]
    fn the_report_names_the_review_place() {
        let (t, repo, _place) = fixture("report", false);
        let r = agentfiles::inspect(&Project::discover(&repo).unwrap()).unwrap();
        assert_eq!(r.pending.as_deref(), Some(FIX_BRANCH));
        assert_eq!(r.pending_place.as_deref(), Some("agent-instructions"));
        assert!(!r.has_origin && !r.pending_merged);
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Abandon deletes origin's copy only when it is the same commit.
    #[test]
    fn abandon_keeps_an_origin_branch_that_moved() {
        let (t, repo, place) = fixture("moved", true);
        let p = Project::discover(&repo).unwrap();
        sh(&repo, &["push", "-q", "origin", "agent-instructions"]);
        std::fs::write(place.join("README.md"), "local only\n").unwrap();
        sh(&place, &["commit", "-q", "-am", "local"]);
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(abandon(&p, &mut ui, true), 0);
        assert!(ui.lines.iter().any(|l| l.contains("different commit")), "{:?}", ui.lines);
        assert!(!sh(&repo, &["ls-remote", "origin", "agent-instructions"]).is_empty());
        let _ = std::fs::remove_dir_all(&t);
    }

    /// The branch checked out in the root is never removed from here.
    #[test]
    fn land_refuses_when_the_root_holds_the_branch() {
        let (t, repo, place) = fixture("root", false);
        let p = Project::discover(&repo).unwrap();
        sh(&repo, &["worktree", "remove", "--force", place.to_str().unwrap()]);
        sh(&repo, &["checkout", "-q", "agent-instructions"]);
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(land_with(&p, &mut ui, "/nonexistent/gh"), 1);
        assert!(ui.lines.join("\n").contains("checked out in the repository root"), "{:?}", ui.lines);
        let mut ui = crate::ui::CaptureUi::default();
        assert_eq!(abandon(&p, &mut ui, true), 1);
        assert!(ok(&repo, &["show-ref", "--verify", "-q", "refs/heads/agent-instructions"]));
        let _ = std::fs::remove_dir_all(&t);
    }
}

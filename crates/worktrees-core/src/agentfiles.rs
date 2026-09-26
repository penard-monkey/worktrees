//! Agent instruction files: CLAUDE.md for Claude, AGENTS.md for Codex (and
//! every other agent that reads the AGENTS.md convention).
//!
//! The house layout this module steers a repo toward: **AGENTS.md is the real
//! file; CLAUDE.md is a one-line stub that imports it** (`@AGENTS.md`, which
//! Claude follows and Codex never needs — Codex reads AGENTS.md first, and
//! with `profile::CODEX_DOC_FALLBACK` it would read a lone CLAUDE.md anyway).
//! A symlink is deliberately NOT the fix: a tracked symlink is branch-dependent
//! (checking out a branch that predates it deletes it — CLAUDE.md), where a
//! stub is an ordinary file.
//!
//! Repo skills get the same treatment one level down: Claude reads
//! `.claude/skills/<name>`, Codex reads `.agents/skills/<name>` (measured with
//! `codex debug prompt-input`, 0.157.1 — it never looks in `.claude/skills` and
//! does follow symlinks), so each Claude skill gets a relative symlink.
//!
//! Everything here reads a REF, not a checkout: the fix lands on the default
//! branch by a dedicated PR, so the default branch is what it must be judged
//! against — never whichever place happens to be open. And the fix is built
//! with plumbing (a private index, `commit-tree`, `update-ref`) so no working
//! tree is touched or created: a temporary `git worktree add` would flash up in
//! every app's nav as a stray.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::{Command, Stdio};

pub const CLAUDE: &str = "CLAUDE.md";
pub const AGENTS: &str = "AGENTS.md";
/// The branch the fix is committed to. One name, so a second run finds the
/// first run's PR instead of opening another.
pub const FIX_BRANCH: &str = "agent-instructions";

/// The stub the fix writes. The comment is for the human who opens the file.
pub const STUB: &str = "<!-- The instructions live in AGENTS.md, which Codex and other agents read.\n     Claude loads them through the import below; edit AGENTS.md, not this file. -->\n@AGENTS.md\n";

/// One directory's instruction files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirKind {
    /// Only CLAUDE.md: Codex reads it through the launch fallback, but any other
    /// AGENTS.md tool does not. Fixable.
    ClaudeOnly,
    /// Only AGENTS.md: Claude reads NOTHING here. Fixable (add the stub).
    AgentsOnly,
    /// CLAUDE.md imports AGENTS.md — the target layout.
    Stub,
    /// A symlink is involved (either file, alone or paired). Whatever it
    /// points at, renaming it would carry the LINK and point AGENTS.md at the
    /// new stub — so it is reported and left alone.
    Linked,
    /// Two real files with different content. A person has to merge them.
    Diverged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DirState {
    /// Repo-relative directory, `""` for the root.
    pub dir: String,
    pub kind: DirKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillKind {
    /// `.agents/skills/<name>` is absent — Codex cannot see this skill. Fixable.
    Missing,
    /// Present (a link or a real directory). Nothing to do.
    Present,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SkillState {
    pub name: String,
    pub kind: SkillKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    /// The ref inspected (`origin/main`, or the local base without a remote).
    pub reference: String,
    pub dirs: Vec<DirState>,
    pub skills: Vec<SkillState>,
    /// Something the fix would change.
    pub fixable: bool,
    /// Something only a person can resolve (diverged pairs).
    pub conflicts: bool,
    /// The fix branch already exists (locally or on origin): a previous fix is
    /// waiting to be merged, so nothing should offer another. The base ref does
    /// not move until that PR merges, so without this `fixable` stays true after
    /// a successful fix and the offer would invite a push that can only refuse.
    pub pending: Option<String>,
}

/// One tracked entry from `git ls-tree -r`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub mode: String,
    pub sha: String,
    pub path: String,
}

impl Entry {
    fn is_link(&self) -> bool {
        self.mode == "120000"
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn basename(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, b)| b).unwrap_or(path)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

/// Does this CLAUDE.md import AGENTS.md? A line that is exactly the import,
/// ignoring surrounding whitespace — the form Claude documents.
pub fn imports_agents(claude_md: &str) -> bool {
    claude_md.lines().any(|l| l.trim() == "@AGENTS.md" || l.trim() == "@./AGENTS.md")
}

/// Classify a tree. `read` returns a blob's text (CLAUDE.md contents, or a
/// symlink's target). Pure over its inputs so the rules are testable without a
/// repo.
pub fn classify(entries: &[Entry], read: &dyn Fn(&str) -> Option<String>) -> (Vec<DirState>, Vec<SkillState>) {
    let mut claude: BTreeMap<String, &Entry> = BTreeMap::new();
    let mut agents: BTreeMap<String, &Entry> = BTreeMap::new();
    let mut claude_skills: BTreeSet<String> = BTreeSet::new();
    let mut agent_skills: BTreeSet<String> = BTreeSet::new();
    for e in entries {
        // Skills: the first component under the skills dir is the skill.
        if let Some(rest) = e.path.strip_prefix(".claude/skills/") {
            if let Some(name) = rest.split('/').next().filter(|n| !n.is_empty()) {
                // A skill is a directory; a stray file at the top is not one.
                if rest.contains('/') || e.is_link() {
                    claude_skills.insert(name.to_string());
                }
            }
            continue;
        }
        if let Some(rest) = e.path.strip_prefix(".agents/skills/") {
            if let Some(name) = rest.split('/').next().filter(|n| !n.is_empty()) {
                agent_skills.insert(name.to_string());
            }
            continue;
        }
        // Claude's own config dir (at any depth) can hold a CLAUDE.md that no
        // Codex walk from the repo root would ever reach; not an instruction dir.
        if e.path.split('/').any(|c| c == ".claude") {
            continue;
        }
        match basename(&e.path) {
            CLAUDE => { claude.insert(parent(&e.path).to_string(), e); }
            AGENTS => { agents.insert(parent(&e.path).to_string(), e); }
            _ => {}
        }
    }
    // A hand-made `.agents/skills -> ../.claude/skills` (or a linked `.agents`)
    // already exposes every skill; adding per-skill links under it would fail
    // in git ("appears as both a file and as a directory").
    let skills_dir_linked = entries.iter().any(|e| e.is_link() && (e.path == ".agents/skills" || e.path == ".agents"));
    let dirs_all: BTreeSet<&String> = claude.keys().chain(agents.keys()).collect();
    let mut dirs = Vec::new();
    for dir in dirs_all {
        let (c, a) = (claude.get(dir), agents.get(dir));
        if c.is_some_and(|e| e.is_link()) || a.is_some_and(|e| e.is_link()) {
            dirs.push(DirState { dir: dir.clone(), kind: DirKind::Linked });
            continue;
        }
        let kind = match (c, a) {
            (Some(_), None) => DirKind::ClaudeOnly,
            (None, Some(_)) => DirKind::AgentsOnly,
            (Some(c), Some(a)) => {
                if c.sha == a.sha {
                    // Identical copies: both agents read the same words today,
                    // but they WILL drift. Folding one into a stub is safe.
                    DirKind::ClaudeOnly
                } else if read(&c.sha).map(|t| imports_agents(&t)).unwrap_or(false) {
                    DirKind::Stub
                } else {
                    DirKind::Diverged
                }
            }
            (None, None) => continue,
        };
        dirs.push(DirState { dir: dir.clone(), kind });
    }
    let skills = claude_skills
        .into_iter()
        .map(|name| {
            let kind = if skills_dir_linked || agent_skills.contains(&name) { SkillKind::Present } else { SkillKind::Missing };
            SkillState { name, kind }
        })
        .collect();
    (dirs, skills)
}

pub fn report_from(reference: &str, dirs: Vec<DirState>, skills: Vec<SkillState>) -> Report {
    let fixable = dirs.iter().any(|d| matches!(d.kind, DirKind::ClaudeOnly | DirKind::AgentsOnly))
        || skills.iter().any(|s| s.kind == SkillKind::Missing);
    let conflicts = dirs.iter().any(|d| d.kind == DirKind::Diverged);
    Report { reference: reference.to_string(), dirs, skills, fixable, conflicts, pending: None }
}

// ── git plumbing ─────────────────────────────────────────────────────────────

fn git_env(root: &str, index: Option<&Path>, args: &[&str], input: Option<&[u8]>) -> Result<String, String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).args(args);
    if let Some(ix) = index {
        c.env("GIT_INDEX_FILE", ix);
    }
    c.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("git {}: {e}", args.first().unwrap_or(&"")))?;
    if let Some(bytes) = input {
        use std::io::Write;
        let mut stdin = child.stdin.take().ok_or("git: no stdin")?;
        stdin.write_all(bytes).map_err(|e| format!("git {}: {e}", args[0]))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("git {}: {e}", args[0]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    } else {
        Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// A network command (fetch, push, gh) with the hardening the app's own
/// background fetch has (`lib.rs::fetch_origin_root`): under launchd there is
/// no terminal, so a credential or host-key prompt must fail instead of
/// hanging, and a wedged remote must not hold the sheet on "Pushing…" until TCP
/// gives up. From a terminal the CLI keeps git's prompts.
fn run_net(mut c: Command, what: &str) -> Result<String, String> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        c.env("GIT_TERMINAL_PROMPT", "0").env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes").env("GH_PROMPT_DISABLED", "1");
    }
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("{what}: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(NET_DEADLINE_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{what} timed out after {NET_DEADLINE_SECS}s"));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(format!("{what}: {e}")),
        }
    }
    let out = child.wait_with_output().map_err(|e| format!("{what}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

const NET_DEADLINE_SECS: u64 = 60;

fn git_net(root: &str, args: &[&str]) -> Result<String, String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).args(args);
    run_net(c, &format!("git {}", args.first().unwrap_or(&"")))
}

/// `git ls-tree -r` of `reference`, parsed.
pub fn tree(root: &str, reference: &str) -> Result<Vec<Entry>, String> {
    let out = git_env(root, None, &["ls-tree", "-r", "--full-tree", "-z", reference], None)?;
    let mut v = Vec::new();
    for rec in out.split('\0').filter(|r| !r.is_empty()) {
        // "<mode> <type> <sha>\t<path>"
        let Some((meta, path)) = rec.split_once('\t') else { continue };
        let mut it = meta.split(' ');
        let (Some(mode), Some(_ty), Some(sha)) = (it.next(), it.next(), it.next()) else { continue };
        v.push(Entry { mode: mode.into(), sha: sha.into(), path: path.into() });
    }
    Ok(v)
}

/// The ref the fix is judged against and built on: the remote's default
/// branch when there is one, else the local base.
pub fn reference(p: &crate::project::Project) -> String {
    p.base_ref()
}

/// Classification per (repo, tree id). `ls-tree -r` over a large monorepo is
/// megabytes, and the app asks every five minutes per project and on every
/// sheet open, while the default branch's tree rarely changes between asks.
static CLASSIFIED: std::sync::Mutex<Vec<(String, String, Vec<DirState>, Vec<SkillState>)>> =
    std::sync::Mutex::new(Vec::new());

/// The fix branch, if it exists locally or on origin.
pub fn pending_branch(root: &str) -> Option<String> {
    let exists = |r: String| crate::git::git_ok(root, &["show-ref", "--verify", "-q", &r]);
    (exists(format!("refs/heads/{FIX_BRANCH}")) || exists(format!("refs/remotes/origin/{FIX_BRANCH}")))
        .then(|| FIX_BRANCH.to_string())
}

pub fn inspect(p: &crate::project::Project) -> Result<Report, String> {
    let reference = reference(p);
    let Some(tree_id) = crate::git::git_out(&p.main_root, &["rev-parse", "--verify", "-q", &format!("{reference}^{{tree}}")]) else {
        return Err(format!("{reference} has no commits yet"));
    };
    let cached = {
        let g = CLASSIFIED.lock().unwrap_or_else(|e| e.into_inner());
        g.iter().find(|(r, t, _, _)| *r == p.main_root && *t == tree_id).map(|(_, _, d, k)| (d.clone(), k.clone()))
    };
    let (dirs, skills) = match cached {
        Some(v) => v,
        None => {
            let entries = tree(&p.main_root, &reference)?;
            let root = p.main_root.clone();
            let read = move |sha: &str| git_env(&root, None, &["cat-file", "blob", sha], None).ok();
            let v = classify(&entries, &read);
            let mut g = CLASSIFIED.lock().unwrap_or_else(|e| e.into_inner());
            g.retain(|(r, _, _, _)| *r != p.main_root);
            g.push((p.main_root.clone(), tree_id, v.0.clone(), v.1.clone()));
            v
        }
    };
    let mut r = report_from(&reference, dirs, skills);
    r.pending = pending_branch(&p.main_root);
    Ok(r)
}

/// One change the fix makes, in index terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Move a blob (CLAUDE.md → AGENTS.md keeps its history-friendly identity).
    Rename { from: String, to: String },
    /// Write a new regular file.
    Write { path: String, content: String },
    /// Add a symlink.
    Link { path: String, target: String },
}

/// What the fix does to a classified tree. Diverged dirs are left alone.
pub fn plan(dirs: &[DirState], skills: &[SkillState]) -> Vec<Change> {
    let mut v = Vec::new();
    for d in dirs {
        match d.kind {
            DirKind::ClaudeOnly => {
                v.push(Change::Rename { from: join(&d.dir, CLAUDE), to: join(&d.dir, AGENTS) });
                v.push(Change::Write { path: join(&d.dir, CLAUDE), content: STUB.into() });
            }
            DirKind::AgentsOnly => v.push(Change::Write { path: join(&d.dir, CLAUDE), content: STUB.into() }),
            DirKind::Stub | DirKind::Linked | DirKind::Diverged => {}
        }
    }
    for s in skills.iter().filter(|s| s.kind == SkillKind::Missing) {
        v.push(Change::Link {
            path: format!(".agents/skills/{}", s.name),
            target: format!("../../.claude/skills/{}", s.name),
        });
    }
    v
}

/// The outcome of `fix`, for the CLI and the app.
#[derive(Clone, Debug, Default, Serialize)]
pub struct FixOutcome {
    pub branch: String,
    pub base: String,
    pub commit: Option<String>,
    pub pushed: bool,
    pub pr_url: Option<String>,
    /// Dirs a person must merge by hand.
    pub left_alone: Vec<String>,
    /// Human-readable lines, in order — what happened and what to do next.
    pub notes: Vec<String>,
}

fn has_origin(root: &str) -> bool {
    crate::git::git_out(root, &["remote"]).map(|r| r.lines().any(|l| l == "origin")).unwrap_or(false)
}

/// Build the fix as a commit on `FIX_BRANCH`, push it, and open a PR.
///
/// Refuses when `FIX_BRANCH` already exists locally or on origin — a previous
/// fix is waiting to be merged (or was abandoned), and a second one would race
/// it. The message names both ways out.
pub fn fix(p: &crate::project::Project) -> Result<FixOutcome, String> {
    // `WORKTREES_GH_BIN` is a seam for the bats suite, which must not reach GitHub.
    let gh = std::env::var("WORKTREES_GH_BIN").unwrap_or_else(|_| "gh".into());
    fix_with(p, &gh)
}

pub fn fix_with(p: &crate::project::Project, gh_bin: &str) -> Result<FixOutcome, String> {
    let root = p.main_root.as_str();
    // Here, not in the CLI wrapper: the app calls this in-process and never
    // passes through the CLI's dispatch, and a hub copy is exactly the tree
    // that must never push.
    if let Some(msg) = crate::sync::hub_copy_refusal(Path::new(root)) {
        return Err(msg);
    }
    let origin = has_origin(root);
    if origin {
        // Judge and build against what the remote has NOW, not the last fetch.
        // A failed fetch is not fatal (offline): the last fetch is still a ref.
        let _ = git_net(root, &["fetch", "--quiet", "origin"]);
    }
    let report = inspect(p)?;
    let mut out = FixOutcome { branch: FIX_BRANCH.into(), base: report.reference.clone(), ..Default::default() };
    out.left_alone = report.dirs.iter().filter(|d| d.kind == DirKind::Diverged).map(|d| join(&d.dir, CLAUDE)).collect();
    let changes = plan(&report.dirs, &report.skills);
    if changes.is_empty() {
        out.notes.push(format!("{} already has its agent instructions set up.", report.reference));
        for f in &out.left_alone {
            out.notes.push(format!("{f} and its AGENTS.md differ — merge them by hand, then make CLAUDE.md `@AGENTS.md`."));
        }
        return Ok(out);
    }
    let local = format!("refs/heads/{FIX_BRANCH}");
    let remote = format!("refs/remotes/origin/{FIX_BRANCH}");
    if crate::git::git_ok(root, &["show-ref", "--verify", "-q", &local])
        || crate::git::git_ok(root, &["show-ref", "--verify", "-q", &remote])
    {
        return Err(format!(
            "a branch named '{FIX_BRANCH}' already exists — merge its PR first. If that PR was merged or closed and there is still something to fix, delete the branch and run the fix again: git branch -D {FIX_BRANCH}; git push origin --delete {FIX_BRANCH}"
        ));
    }

    // A private index seeded from the base tree: nothing any checkout uses.
    let ix = std::env::temp_dir().join(format!("worktrees-agentfix-{}-{}.index", std::process::id(), crate::sysclock::now_epoch()));
    let result = (|| -> Result<String, String> {
        git_env(root, Some(&ix), &["read-tree", &report.reference], None)?;
        for c in &changes {
            match c {
                Change::Rename { from, to } => {
                    let line = git_env(root, Some(&ix), &["ls-files", "-s", "--", from], None)?;
                    // "<mode> <sha> <stage>\t<path>"
                    let meta = line.split('\t').next().unwrap_or("");
                    let mut it = meta.split(' ');
                    let (Some(mode), Some(sha)) = (it.next(), it.next()) else {
                        return Err(format!("{from} is not in {}", report.reference));
                    };
                    git_env(root, Some(&ix), &["update-index", "--force-remove", "--", from], None)?;
                    git_env(root, Some(&ix), &["update-index", "--add", "--cacheinfo", &format!("{mode},{sha},{to}")], None)?;
                }
                Change::Write { path, content } => {
                    let sha = git_env(root, None, &["hash-object", "-w", "--stdin"], Some(content.as_bytes()))?;
                    git_env(root, Some(&ix), &["update-index", "--add", "--cacheinfo", &format!("100644,{sha},{path}")], None)?;
                }
                Change::Link { path, target } => {
                    let sha = git_env(root, None, &["hash-object", "-w", "--stdin"], Some(target.as_bytes()))?;
                    git_env(root, Some(&ix), &["update-index", "--add", "--cacheinfo", &format!("120000,{sha},{path}")], None)?;
                }
            }
        }
        let tree = git_env(root, Some(&ix), &["write-tree"], None)?;
        let msg = commit_message(&report, &changes);
        let parent = git_env(root, None, &["rev-parse", &format!("{}^{{commit}}", report.reference)], None)?;
        let commit = git_env(root, None, &["commit-tree", &tree, "-p", &parent, "-F", "-"], Some(msg.as_bytes()))?;
        // Create-only: the zero old-value makes update-ref refuse if the branch
        // appeared since the check above.
        git_env(root, None, &["update-ref", &local, &commit, "0000000000000000000000000000000000000000"], None)?;
        Ok(commit)
    })();
    let _ = std::fs::remove_file(&ix);
    let commit = result?;
    out.commit = Some(commit.clone());
    out.notes.push(format!("Committed {} on branch '{FIX_BRANCH}' (off {}).", &commit[..commit.len().min(7)], report.reference));

    if !origin {
        out.notes.push("No 'origin' remote — merge the branch into your default branch yourself.".into());
        return Ok(out);
    }
    match git_net(root, &["push", "--quiet", "-u", "origin", &format!("{local}:{local}")]) {
        Ok(_) => out.pushed = true,
        Err(e) => {
            out.notes.push(format!("Push failed: {e}. The branch is committed locally; push it and open a PR yourself."));
            return Ok(out);
        }
    }
    let base_branch = report.reference.strip_prefix("origin/").unwrap_or(&report.reference).to_string();
    let body = pr_body(&report, &changes, &out.left_alone);
    let mut gh = Command::new(gh_bin);
    gh.current_dir(root).args(["pr", "create", "--base", &base_branch, "--head", FIX_BRANCH, "--title", "Agent instructions: AGENTS.md for every agent", "--body", &body]);
    match run_net(gh, "gh pr create") {
        Ok(stdout) => {
            let url = stdout.lines().rev().find(|l| l.starts_with("http")).map(str::to_string);
            if let Some(u) = &url {
                out.notes.push(format!("Opened {u} — merge it to finish."));
            }
            out.pr_url = url;
        }
        Err(e) if e.contains("No such file") || e.contains("not found") && e.starts_with("gh pr create:") => {
            out.notes.push(format!("Pushed '{FIX_BRANCH}'. The GitHub CLI (gh) is not installed — open the PR yourself."))
        }
        Err(e) => out.notes.push(format!("Pushed '{FIX_BRANCH}', but {e} — open the PR yourself.")),
    }
    for f in &out.left_alone {
        out.notes.push(format!("{f} and its AGENTS.md differ — left alone; merge them by hand."));
    }
    Ok(out)
}

fn describe(c: &Change) -> String {
    match c {
        Change::Rename { from, to } => format!("- move `{from}` to `{to}`"),
        Change::Write { path, .. } => format!("- add `{path}` as a stub that imports AGENTS.md"),
        Change::Link { path, target } => format!("- link `{path}` → `{target}`"),
    }
}

fn commit_message(r: &Report, changes: &[Change]) -> String {
    let mut m = String::from("chore: AGENTS.md holds the agent instructions\n\n");
    m.push_str("AGENTS.md is read by Codex and other agents; CLAUDE.md becomes a stub that\n");
    m.push_str("imports it, so both read the same file. Repo skills are linked into\n");
    m.push_str(".agents/skills, where Codex looks for them.\n\n");
    for c in changes {
        m.push_str(&describe(c));
        m.push('\n');
    }
    m.push_str(&format!("\nBuilt against {} by worktrees.\n", r.reference));
    m
}

fn pr_body(r: &Report, changes: &[Change], left_alone: &[String]) -> String {
    let mut b = String::from("Makes **AGENTS.md** the one instruction file. Codex and other agents read it directly; **CLAUDE.md** becomes a one-line `@AGENTS.md` import, so Claude reads the same words.\n\n");
    b.push_str("## Changes\n");
    for c in changes {
        b.push_str(&describe(c));
        b.push('\n');
    }
    if !left_alone.is_empty() {
        b.push_str("\n## Left alone\nThese directories have a CLAUDE.md and an AGENTS.md with different content. Merge them by hand, then turn CLAUDE.md into `@AGENTS.md`:\n");
        for f in left_alone {
            b.push_str(&format!("- `{f}`\n"));
        }
    }
    b.push_str(&format!("\nBuilt against `{}` by worktrees. Revert this PR to undo it.\n", r.reference));
    b
}

// ── user-level skills ───────────────────────────────────────────────────────

/// One skill in `~/.claude/skills` and whether Codex can see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UserSkill {
    pub name: String,
    /// `linked` (we or the user already put it in `~/.agents/skills`),
    /// `missing` (Codex cannot see it), or `conflict` (a DIFFERENT skill of that
    /// name is already there — never overwritten).
    pub status: String,
}

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// The user's Claude skills vs the shared `~/.agents/skills` Codex reads.
pub fn user_skills() -> Vec<UserSkill> {
    user_skills_in(&home().join(".claude/skills"), &home().join(".agents/skills"))
}

pub fn user_skills_in(claude: &Path, agents: &Path) -> Vec<UserSkill> {
    let Ok(rd) = std::fs::read_dir(claude) else { return Vec::new() };
    let mut v: Vec<UserSkill> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let src = e.path();
            // A skill is a directory with a SKILL.md, followed through links.
            if !src.join("SKILL.md").is_file() {
                return None;
            }
            let dst = agents.join(&name);
            let status = if std::fs::symlink_metadata(&dst).is_err() {
                "missing"
            } else if same_dir(&src, &dst) {
                "linked"
            } else {
                "conflict"
            };
            Some(UserSkill { name, status: status.into() })
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Link every `missing` skill into `~/.agents/skills`. Returns the names linked.
pub fn link_user_skills() -> Result<Vec<String>, String> {
    link_user_skills_in(&home().join(".claude/skills"), &home().join(".agents/skills"))
}

pub fn link_user_skills_in(claude: &Path, agents: &Path) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    for s in user_skills_in(claude, agents).into_iter().filter(|s| s.status == "missing") {
        std::fs::create_dir_all(agents).map_err(|e| format!("{}: {e}", agents.display()))?;
        // Point at the ~/.claude/skills ENTRY, not its resolved target: if the
        // user re-points their Claude skill, Codex follows.
        let src = claude.join(&s.name);
        let dst = agents.join(&s.name);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&src, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        #[cfg(not(unix))]
        return Err("linking skills needs a unix symlink".into());
        done.push(s.name);
    }
    Ok(done)
}

// ── CLI ──────────────────────────────────────────────────────────────────────

fn kind_line(d: &DirState) -> String {
    let dir = if d.dir.is_empty() { "(root)".to_string() } else { d.dir.clone() };
    let what = match d.kind {
        DirKind::ClaudeOnly => "CLAUDE.md only — fix moves it to AGENTS.md and leaves a stub",
        DirKind::AgentsOnly => "AGENTS.md only — Claude reads nothing here; fix adds the stub",
        DirKind::Stub => "ok (CLAUDE.md imports AGENTS.md)",
        DirKind::Linked => "a symlink is involved — left alone",
        DirKind::Diverged => "CLAUDE.md and AGENTS.md differ — merge them by hand",
    };
    format!("  {dir}: {what}")
}

/// `worktrees agent-setup [status|fix|link-skills] [--json]`.
pub fn cmd_agent_setup(p: &crate::project::Project, ui: &mut dyn crate::ui::Ui, rest: &[String]) -> i32 {
    let json = rest.iter().any(|a| a == "--json");
    let verb = rest.iter().find(|a| !a.starts_with('-')).map(String::as_str).unwrap_or("status");
    match verb {
        "status" => {
            let r = match inspect(p) {
                Ok(r) => r,
                Err(e) => { ui.error(&e); return 1; }
            };
            let skills = user_skills();
            if json {
                let v = serde_json::json!({"repo": r, "user_skills": skills});
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return 0;
            }
            ui.header(&format!("Agent instructions on {}", r.reference));
            if let Some(b) = &r.pending {
                ui.plain(&format!("  a fix is waiting on branch '{b}' — merge its PR"));
            }
            if r.dirs.is_empty() {
                ui.plain("  no CLAUDE.md or AGENTS.md — nothing to fix");
            }
            for d in &r.dirs {
                ui.plain(&kind_line(d));
            }
            for s in &r.skills {
                let st = if s.kind == SkillKind::Missing { "not visible to Codex — fix links it into .agents/skills" } else { "ok" };
                ui.plain(&format!("  skill {}: {st}", s.name));
            }
            ui.header("Your skills (~/.claude/skills → ~/.agents/skills)");
            for s in &skills {
                let st = match s.status.as_str() {
                    "missing" => "not visible to Codex — `worktrees agent-setup link-skills`",
                    "conflict" => "a different skill of that name is in ~/.agents/skills — left alone",
                    _ => "ok",
                };
                ui.plain(&format!("  {}: {st}", s.name));
            }
            0
        }
        "fix" => {
            // `fix` carries the hub-copy guard itself (the app calls it too).
            match fix(p) {
                Ok(o) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
                    } else {
                        for n in &o.notes { ui.info(n); }
                    }
                    0
                }
                Err(e) => { ui.error(&e); 1 }
            }
        }
        "link-skills" => match link_user_skills() {
            Ok(done) => {
                if json {
                    println!("{}", serde_json::to_string(&done).unwrap_or_default());
                } else if done.is_empty() {
                    ui.info("Every skill in ~/.claude/skills is already visible to Codex.");
                } else {
                    ui.info(&format!("Linked into ~/.agents/skills: {}", done.join(", ")));
                }
                0
            }
            Err(e) => { ui.error(&e); 1 }
        },
        other => {
            ui.error(&format!("agent-setup: unknown verb '{other}' (status | fix | link-skills)"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(mode: &str, sha: &str, path: &str) -> Entry {
        Entry { mode: mode.into(), sha: sha.into(), path: path.into() }
    }

    fn reader(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |sha| pairs.iter().find(|(s, _)| *s == sha).map(|(_, t)| t.to_string())
    }

    #[test]
    fn classifies_every_pairing_per_directory() {
        let entries = vec![
            e("100644", "c1", "CLAUDE.md"),                    // root: claude only
            e("100644", "a2", "api/AGENTS.md"),                // api: agents only
            e("100644", "s3", "web/CLAUDE.md"),                // web: stub
            e("100644", "a3", "web/AGENTS.md"),
            e("120000", "l4", "cli/AGENTS.md"),                // cli: linked
            e("100644", "c4", "cli/CLAUDE.md"),
            e("100644", "c5", "db/CLAUDE.md"),                 // db: diverged
            e("100644", "a5", "db/AGENTS.md"),
            e("100644", "same", "ops/CLAUDE.md"),              // ops: identical copies
            e("100644", "same", "ops/AGENTS.md"),
            e("100644", "x", ".claude/CLAUDE.md"),             // not an instruction dir
            e("100644", "x2", "pkg/.claude/CLAUDE.md"),        // …at any depth
            e("120000", "l6", "sub/CLAUDE.md"),                // sub: a LONE link
            e("100644", "y", "README.md"),
        ];
        let read = reader(&[("s3", "# web\n  @AGENTS.md  \n"), ("c5", "# db rules\n")]);
        let (dirs, _) = classify(&entries, &read);
        let got: Vec<(&str, DirKind)> = dirs.iter().map(|d| (d.dir.as_str(), d.kind.clone())).collect();
        assert_eq!(got, vec![
            ("", DirKind::ClaudeOnly),
            ("api", DirKind::AgentsOnly),
            ("cli", DirKind::Linked),
            ("db", DirKind::Diverged),
            ("ops", DirKind::ClaudeOnly),
            ("sub", DirKind::Linked),
            ("web", DirKind::Stub),
        ]);
    }

    #[test]
    fn a_claude_skill_without_an_agents_twin_is_missing() {
        let entries = vec![
            e("100644", "1", ".claude/skills/deploy/SKILL.md"),
            e("100644", "2", ".claude/skills/deploy/run.sh"),
            e("100644", "3", ".claude/skills/review/SKILL.md"),
            e("120000", "4", ".agents/skills/review"),
            e("100644", "5", ".claude/skills/README.md"), // a file, not a skill
        ];
        let (_, skills) = classify(&entries, &|_| None);
        assert_eq!(skills, vec![
            SkillState { name: "deploy".into(), kind: SkillKind::Missing },
            SkillState { name: "review".into(), kind: SkillKind::Present },
        ]);
    }

    #[test]
    fn a_linked_agents_skills_dir_already_exposes_every_skill() {
        let entries = vec![
            e("100644", "1", ".claude/skills/deploy/SKILL.md"),
            e("120000", "2", ".agents/skills"),
        ];
        let (_, skills) = classify(&entries, &|_| None);
        assert_eq!(skills, vec![SkillState { name: "deploy".into(), kind: SkillKind::Present }]);
    }

    #[test]
    fn the_plan_renames_stubs_and_links_but_never_touches_a_diverged_pair() {
        let dirs = vec![
            DirState { dir: "".into(), kind: DirKind::ClaudeOnly },
            DirState { dir: "api".into(), kind: DirKind::AgentsOnly },
            DirState { dir: "db".into(), kind: DirKind::Diverged },
            DirState { dir: "web".into(), kind: DirKind::Stub },
        ];
        let skills = vec![
            SkillState { name: "deploy".into(), kind: SkillKind::Missing },
            SkillState { name: "review".into(), kind: SkillKind::Present },
        ];
        assert_eq!(plan(&dirs, &skills), vec![
            Change::Rename { from: "CLAUDE.md".into(), to: "AGENTS.md".into() },
            Change::Write { path: "CLAUDE.md".into(), content: STUB.into() },
            Change::Write { path: "api/CLAUDE.md".into(), content: STUB.into() },
            Change::Link { path: ".agents/skills/deploy".into(), target: "../../.claude/skills/deploy".into() },
        ]);
        let r = report_from("origin/main", dirs, skills);
        assert!(r.fixable && r.conflicts);
    }

    #[test]
    fn the_stub_is_something_classify_recognises() {
        assert!(imports_agents(STUB));
        assert!(!imports_agents("see @AGENTS.md for more"));
    }

    #[test]
    fn user_skills_link_what_is_missing_and_never_overwrite() {
        let t = std::env::temp_dir().join(format!("wt-userskills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (claude, agents) = (t.join("claude"), t.join("agents"));
        for n in ["mine", "shared", "clash"] {
            std::fs::create_dir_all(claude.join(n)).unwrap();
            std::fs::write(claude.join(n).join("SKILL.md"), n).unwrap();
        }
        std::fs::create_dir_all(claude.join("notaskill")).unwrap();
        std::fs::create_dir_all(agents.join("clash")).unwrap(); // someone else's
        std::os::unix::fs::symlink(claude.join("shared"), agents.join("shared")).unwrap();
        let st: Vec<(String, String)> = user_skills_in(&claude, &agents).into_iter().map(|s| (s.name, s.status)).collect();
        assert_eq!(st, vec![
            ("clash".into(), "conflict".into()),
            ("mine".into(), "missing".into()),
            ("shared".into(), "linked".into()),
        ]);
        assert_eq!(link_user_skills_in(&claude, &agents).unwrap(), vec!["mine".to_string()]);
        assert!(agents.join("mine/SKILL.md").is_file());
        assert!(std::fs::read_dir(agents.join("clash")).unwrap().next().is_none(), "the clash dir was left alone");
        let _ = std::fs::remove_dir_all(&t);
    }

    fn sh(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git").arg("-C").arg(dir).args(args)
            .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
            .output().unwrap();
        assert!(o.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    /// End to end against real git: a CLAUDE.md-only repo with a Claude skill,
    /// an origin to push to, and a fake gh. The fix must land ONLY on the fix
    /// branch (the checkout and main untouched), carry history-preserving
    /// content, and refuse a second run while that branch exists.
    #[test]
    fn fix_commits_on_its_own_branch_pushes_and_leaves_the_checkout_alone() {
        let t = std::fs::canonicalize(std::env::temp_dir()).unwrap()
            .join(format!("wt-agentfix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (origin, repo) = (t.join("origin.git"), t.join("repo"));
        std::fs::create_dir_all(&repo).unwrap();
        sh(&t, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
        sh(&repo, &["init", "-q", "-b", "main"]);
        // In the REPO, not only in `sh`'s env: `fix` runs its own commit-tree,
        // and a CI runner has no global identity (it failed there, not here).
        sh(&repo, &["config", "user.name", "t"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        std::fs::write(repo.join("CLAUDE.md"), "# rules\nbe kind\n").unwrap();
        std::fs::create_dir_all(repo.join(".claude/skills/deploy")).unwrap();
        std::fs::write(repo.join(".claude/skills/deploy/SKILL.md"), "---\nname: deploy\n---\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        sh(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        sh(&repo, &["push", "-q", "-u", "origin", "main"]);
        // A gh that records its argv, so a wrong --base/--head cannot pass.
        let fake_gh = t.join("gh");
        let gh_log = t.join("gh.log");
        std::fs::write(&fake_gh, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho https://example.invalid/pr/1\n", gh_log.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_gh, std::fs::Permissions::from_mode(0o755)).unwrap();

        let p = crate::project::Project::discover(&repo).unwrap();
        let before = inspect(&p).unwrap();
        assert!(before.fixable && !before.conflicts);
        let head_before = sh(&repo, &["rev-parse", "HEAD"]);

        let out = fix_with(&p, fake_gh.to_str().unwrap()).unwrap();
        assert!(out.pushed, "{:?}", out.notes);
        assert_eq!(out.pr_url.as_deref(), Some("https://example.invalid/pr/1"));
        let argv = std::fs::read_to_string(&gh_log).unwrap();
        assert!(argv.contains("--base\nmain\n--head\nagent-instructions\n"), "{argv}");
        // The checkout and main did not move; the working tree is untouched.
        assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), head_before);
        assert_eq!(sh(&repo, &["status", "--porcelain"]), "");
        assert_eq!(std::fs::read_to_string(repo.join("CLAUDE.md")).unwrap(), "# rules\nbe kind\n");
        // The branch carries the layout, and origin has it.
        let b = format!("refs/heads/{FIX_BRANCH}");
        assert_eq!(sh(&repo, &["show", &format!("{b}:AGENTS.md")]), "# rules\nbe kind");
        assert_eq!(sh(&repo, &["show", &format!("{b}:CLAUDE.md")]), STUB.trim_end());
        assert_eq!(sh(&repo, &["ls-tree", &b, ".agents/skills/deploy"]).split_whitespace().next(), Some("120000"));
        assert_eq!(sh(&origin, &["rev-parse", FIX_BRANCH]), sh(&repo, &["rev-parse", &b]));
        // Built on main: one commit ahead of it.
        assert_eq!(sh(&repo, &["rev-list", "--count", &format!("main..{b}")]), "1");
        // The offer retires: the base has not moved, but the fix is pending.
        let after = inspect(&p).unwrap();
        assert!(after.fixable, "main still needs it until the PR merges");
        assert_eq!(after.pending.as_deref(), Some(FIX_BRANCH));
        // A second run refuses rather than racing the open PR.
        assert!(fix_with(&p, fake_gh.to_str().unwrap()).unwrap_err().contains("already exists"));
        let _ = std::fs::remove_dir_all(&t);
    }
}

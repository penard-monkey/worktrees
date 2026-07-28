//! A Project = one opened git repo. Discovery + the read path (`ls`, `ls --json`)
//! ported 1:1 from the bash `cmd_ls`/`emit_*`. git/tmux are shelled out so the
//! stale-dir trap and output match exactly.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::resolve_prefix_from;
use crate::error::{Result, WtError};
use crate::model::{LsJson, Place, TmuxSession, SCHEMA_VERSION};
use crate::projcfg;
use crate::render::{self, Row};
use crate::sysclock::{now_epoch, SysClock};
use crate::{git, tmux};

pub struct Project {
    pub main_root: String,
    pub git_common: String,
    pub wt_root: String,
    pub prefix: String,
    clock: SysClock,
}

fn canon(p: PathBuf) -> Option<String> {
    std::fs::canonicalize(p).ok().map(|c| c.to_string_lossy().into_owned())
}

fn basename(p: &str) -> String {
    Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

/// Recency sort key for a place: last-commit epoch, else creation epoch — the
/// same key the human `ls` used, read from the epochs already inside the Place.
fn recency_key(p: &Place) -> i64 {
    match p.last_commit_epoch {
        Some(c) if c > 0 => c,
        _ => p.created_epoch.unwrap_or(0),
    }
}

impl Project {
    /// git guards + roots + prefix, from `cwd`. Mirrors the top-of-script setup.
    pub fn discover(cwd: &Path) -> Result<Project> {
        if !git::have_git() {
            return Err(WtError::new("git not found"));
        }
        if !git::git_ok(&cwd.to_string_lossy(), &["rev-parse", "--is-inside-work-tree"]) {
            return Err(WtError::new("Not inside a git repository."));
        }
        let cwd_s = cwd.to_string_lossy().into_owned();
        let raw = git::git_out(&cwd_s, &["rev-parse", "--git-common-dir"])
            .ok_or_else(|| WtError::new("cannot resolve --git-common-dir"))?;
        let abs = {
            let p = PathBuf::from(&raw);
            if p.is_absolute() { p } else { cwd.join(p) }
        };
        let git_common = canon(abs).ok_or_else(|| WtError::new("cannot canonicalize git dir"))?;
        let main_root = canon(PathBuf::from(&git_common).join(".."))
            .ok_or_else(|| WtError::new("cannot resolve main checkout"))?;
        let wt_root = format!("{main_root}/.worktrees");
        let prefix = resolve_prefix(&main_root);
        Ok(Project { main_root, git_common, wt_root, prefix, clock: SysClock::detect() })
    }

    pub fn session_name(&self, slug: &str) -> String {
        format!("{}-{}", self.prefix, slug).replace('.', "-")
    }

    fn registrations(&self) -> HashSet<String> {
        git::git_out(&self.main_root, &["worktree", "list", "--porcelain"])
            .map(|s| {
                s.lines()
                    .filter_map(|l| l.strip_prefix("worktree ").map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Sorted (glob-order) worktree subdirs of `.worktrees/`.
    fn worktree_dirs(&self) -> Vec<String> {
        let mut dirs: Vec<String> = match std::fs::read_dir(&self.wt_root) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .map(|e| format!("{}/{}", self.wt_root, e.file_name().to_string_lossy()))
                .collect(),
            Err(_) => Vec::new(),
        };
        dirs.sort();
        dirs
    }

    fn branch_raw(&self, dir: &str) -> String {
        git::git_out(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default()
    }
    fn dirty(&self, dir: &str) -> bool {
        git::git_out(dir, &["status", "--porcelain"]).map(|s| !s.is_empty()).unwrap_or(false)
    }
    fn commit_epoch(&self, dir: &str) -> i64 {
        git::git_out(dir, &["log", "-1", "--format=%ct"]).and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    // ── human `ls` ──────────────────────────────────────────────────────────
    pub fn ls_human(&self) -> String {
        if !Path::new(&self.wt_root).is_dir() {
            return render::info_line("No worktrees (.worktrees/ is empty).");
        }
        let dirs = self.worktree_dirs();
        if dirs.is_empty() {
            return render::info_line("No worktrees (.worktrees/ is empty).");
        }
        let now = now_epoch();
        let reg = self.registrations();
        let mut rows = Vec::new();
        for d in &dirs {
            let slug = basename(d);
            let bepoch = self.clock.stat_birth(d);
            let created = self.clock.fmt_date(bepoch);
            if !reg.contains(d) {
                rows.push(Row {
                    slug: slug.clone(),
                    btext: format!("(rm {slug})"),
                    bcol: render::YELLOW,
                    created,
                    age: "-".to_string(),
                    tmux_cell: "○".to_string(),
                    git_cell: format!("{}stale{}", render::YELLOW, render::NC),
                    key: bepoch,
                });
            } else {
                let raw = self.branch_raw(d);
                let mut branch = if raw == "HEAD" { "(detached)".to_string() } else { raw };
                if branch.is_empty() {
                    branch = "?".to_string();
                }
                let cepoch = self.commit_epoch(d);
                let age = if cepoch > 0 { self.clock.ago(cepoch, now) } else { "-".to_string() };
                let bcol = if branch.replace('/', "-") != slug { render::CYAN } else { "" };
                let tmux_cell = if tmux::session_exists(&self.session_name(&slug)) {
                    format!("{}●{}", render::GREEN, render::NC)
                } else {
                    "○".to_string()
                };
                let git_cell = if self.dirty(d) {
                    format!("{}dirty{}", render::YELLOW, render::NC)
                } else {
                    "clean".to_string()
                };
                let key = if cepoch > 0 { cepoch } else { bepoch };
                rows.push(Row { slug, btext: branch, bcol, created, age, tmux_cell, git_cell, key });
            }
        }
        render::table(rows)
    }

    // ── `ls --json` (live-only: declared=null, lifecycle active|closed) ──────
    /// Typed snapshot — the app consumes this directly (core-as-lib); the CLI
    /// serializes it via `ls_json`.
    pub fn ls(&self) -> LsJson {
        let reg = self.registrations();
        // Prefetch the live tmux panes ONCE per snapshot (one `list-panes -a`),
        // so place_json can detect an ADOPTED (foreign-named) session per place
        // without shelling out per place — the app polls this every ~3s.
        let panes = tmux::PaneList::fetch();
        let ai_word = adopt_ai_word();
        // Build every place's snapshot in parallel — each place shells out to git
        // (status/divergence/log), and summed serially this froze the app on big
        // repos. Main first, then worktrees; recency-sort the worktrees afterwards
        // from the epochs already inside each Place (no second git pass for the key).
        let tasks: Vec<(String, bool)> = std::iter::once((self.main_root.clone(), true))
            .chain(self.worktree_dirs().into_iter().map(|d| (d, false)))
            .collect();
        let mut computed = self.place_json_par(&tasks, &reg, panes.as_ref(), &ai_word);
        let main = computed.remove(0);
        computed.sort_by(|a, b| recency_key(b).cmp(&recency_key(a))); // stable desc, glob-order ties
        let mut places = Vec::with_capacity(computed.len() + 1);
        places.push(main);
        places.extend(computed);
        LsJson {
            schema_version: SCHEMA_VERSION,
            repo: self.main_root.clone(),
            prefix: self.prefix.clone(),
            places_file: format!("{}/.worktrees.places.json", self.main_root),
            places,
        }
    }

    pub fn ls_json(&self) -> String {
        // serde_json compact = same shape/order as the bash emitter; add the
        // trailing newline the bash `printf ']}\n'` produced.
        format!("{}\n", serde_json::to_string(&self.ls()).unwrap_or_default())
    }

    fn place_json(&self, dir: &str, is_main: bool, reg: &HashSet<String>, panes: Option<&tmux::PaneList>, ai_word: &str) -> Place {
        let slug = if is_main { "(main)".to_string() } else { basename(dir) };
        let canonical = self.session_name(&slug);
        // Canonical name first (exact match). If it's down, adopt any session
        // with a pane cwd'd in this dir — the SAME session `open` reuses. Without
        // this an adopted (foreign-named) session read as "down", the app never
        // mounted its terminal, and Enter looked dead. Uses the prefetched pane
        // list (no per-place tmux shell-out). For MAIN, exclude `.worktrees/`:
        // worktree dirs nest under the main root, so without the exclusion any
        // live worktree session would falsely read as main's.
        let exclude = if is_main { Some(self.wt_root.as_str()) } else { None };
        let (session, tmux_up) = if tmux::session_exists(&canonical) {
            (canonical, true)
        } else if let Some(adopted) = panes.and_then(|pl| pl.session_in(dir, ai_word, exclude)) {
            (adopted, true)
        } else {
            (canonical, false)
        };
        let cdir = claude_dir_for(dir);
        let cpresent = claude_has_session(&cdir);
        let bepoch = self.clock.stat_birth(dir);
        let created = self.clock.fmt_date(bepoch);
        let life = if tmux_up { "active" } else { "closed" }.to_string();

        let base = Place {
            schema_version: SCHEMA_VERSION,
            slug,
            path: dir.to_string(),
            is_main,
            registered: true,
            branch: None,
            detached: None,
            dirty: None,
            dirty_files: None,
            ahead: None,
            behind: None,
            upstream: None,
            created: Some(created),
            created_epoch: Some(bepoch),
            last_commit_epoch: None,
            last_commit_subject: None,
            tmux_session: TmuxSession { name: session, up: tmux_up },
            claude_session_present: cpresent,
            claude_session_dir: Some(cdir),
            install_cmd: None,
            stack: None,
            declared: None,
            lifecycle_effective: life,
        };

        if !is_main && !reg.contains(dir) {
            return Place { registered: false, ..base };
        }

        let raw = self.branch_raw(dir);
        let detached = raw.is_empty() || raw == "HEAD";
        let branch = if detached { None } else { Some(raw) };
        let dirtytext = git::git_out(dir, &["status", "--porcelain"]).unwrap_or_default();
        let (dirty, dirty_files) = if dirtytext.is_empty() {
            (false, 0)
        } else {
            (true, dirtytext.lines().filter(|l| !l.is_empty()).count() as u32)
        };
        let upstream = git::git_out(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
            .filter(|s| !s.is_empty());
        let (behind, ahead) = match git::git_out(dir, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]) {
            Some(ab) if !ab.is_empty() => {
                let mut it = ab.split_whitespace();
                (it.next().and_then(|x| x.parse().ok()), it.next().and_then(|x| x.parse().ok()))
            }
            _ => (None, None),
        };
        let cepoch = self.commit_epoch(dir);
        let csubj = git::git_out(dir, &["log", "-1", "--format=%s"]).filter(|s| !s.is_empty());
        let install = detect_install_cmd(dir);

        Place {
            branch,
            detached: Some(detached),
            dirty: Some(dirty),
            dirty_files: Some(dirty_files),
            ahead,
            behind,
            upstream,
            last_commit_epoch: if cepoch > 0 { Some(cepoch) } else { None },
            last_commit_subject: csubj,
            install_cmd: install,
            ..base
        }
    }

    /// Compute many places with bounded fan-out. Each place shells out to git
    /// (status / divergence / last-commit); running them concurrently turns a
    /// sum-of-latencies into ~max. `LANES` caps concurrent git processes so a repo
    /// with many worktrees can't thrash. Order is preserved (caller keeps main first).
    fn place_json_par(&self, tasks: &[(String, bool)], reg: &HashSet<String>, panes: Option<&tmux::PaneList>, ai_word: &str) -> Vec<Place> {
        const LANES: usize = 16;
        let mut out = Vec::with_capacity(tasks.len());
        for chunk in tasks.chunks(LANES) {
            std::thread::scope(|s| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|(dir, is_main)| {
                        let is_main = *is_main;
                        s.spawn(move || self.place_json(dir, is_main, reg, panes, ai_word))
                    })
                    .collect();
                for h in handles {
                    out.push(h.join().unwrap());
                }
            });
        }
        out
    }

    /// Physical dir for a place slug; `(main)` → the main checkout.
    pub fn place_dir(&self, slug: &str) -> String {
        if slug == "(main)" {
            self.main_root.clone()
        } else {
            format!("{}/{}", self.wt_root, slug)
        }
    }
}

// ── helpers used by the write ops (Increment 2) ──────────────────────────────
impl Project {
    pub fn is_registered(&self, dir: &str) -> bool {
        self.registrations().contains(dir)
    }

    /// Dir of the `.worktrees/` worktree currently ON `refs/heads/<branch>`.
    pub fn wt_for_branch(&self, branch: &str) -> Option<String> {
        let out = git::git_out(&self.main_root, &["worktree", "list", "--porcelain"])?;
        let target = format!("refs/heads/{branch}");
        let root = format!("{}/", self.wt_root);
        let mut wt = String::new();
        for line in out.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                wt = p.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                if b == target && wt.starts_with(&root) {
                    return Some(wt);
                }
            }
        }
        None
    }

    /// The FINAL slug `cmd_new` will land a new place on — the single source of
    /// truth for the app, which otherwise re-derives it and guesses wrong.
    /// Mirrors `cmd_new`: `slugify(name || strip_origin(branch))`, but when no
    /// explicit `name` is given and the branch already lives in another worktree,
    /// `cmd_new` REUSES that holder's slug (only if its dir doesn't already exist
    /// under the derived slug). Same conditions as `cmd_new` so they can't diverge.
    pub fn resolve_new_slug(&self, branch: &str, name: Option<&str>) -> String {
        let strip_origin = |s: &str| s.strip_prefix("origin/").unwrap_or(s).to_string();
        let slugify = |s: &str| s.replace('/', "-");
        let branch = strip_origin(branch);
        let mut slug = slugify(name.unwrap_or(&branch));
        let wt = format!("{}/{}", self.wt_root, slug);
        if !Path::new(&wt).exists() && name.is_none() {
            if let Some(holder) = self.wt_for_branch(&branch) {
                slug = basename(&holder);
            }
        }
        slug
    }

    /// Default base for a NEW branch: main, else master, else current HEAD.
    pub fn default_base(&self) -> String {
        for cand in ["main", "master"] {
            if git::git_ok(&self.main_root, &["show-ref", "--verify", "-q", &format!("refs/heads/{cand}")])
                || git::git_ok(&self.main_root, &["show-ref", "--verify", "-q", &format!("refs/remotes/origin/{cand}")])
            {
                return cand.to_string();
            }
        }
        git::git_out(&self.main_root, &["symbolic-ref", "--short", "HEAD"]).unwrap_or_else(|| "main".into())
    }

    /// Branch of worktree `dir`; `(detached)` on a detached HEAD.
    pub fn wt_branch(&self, dir: &str) -> String {
        let b = git::git_out(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
        if b == "HEAD" { "(detached)".to_string() } else { b }
    }

    pub fn wt_dirty(&self, dir: &str) -> String {
        git::git_out(dir, &["status", "--porcelain"]).unwrap_or_default()
    }

    /// Exclude `.worktrees/`, the app sidecar, and the per-worktree port file in
    /// THIS repo (all local, all untracked).
    ///
    /// ⚠ `.worktree.env` is not optional polish (§8). It is untracked, so
    /// `git status --porcelain` reports it `??`, so `wt_dirty` is true forever,
    /// so `switch` (ops.rs:96-102) AND `rm` (ops.rs:642-647) refuse without
    /// `--force` — and the GUI never passes `--force` to switch, which would
    /// leave the app stuck with no remedy. `info/exclude` lives in the git COMMON
    /// dir, so one line here covers every worktree.
    pub fn ensure_excluded(&self) {
        let excl = format!("{}/info/exclude", self.git_common);
        if !Path::new(&excl).exists() {
            let _ = std::fs::File::create(&excl);
        }
        for p in [".worktrees/", ".worktrees.places.json", ".worktree.env"] {
            if git::git_ok(&self.main_root, &["check-ignore", "-q", p]) {
                continue;
            }
            let existing = std::fs::read_to_string(&excl).unwrap_or_default();
            if existing.lines().any(|l| l == p) {
                continue;
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&excl) {
                use std::io::Write;
                let _ = writeln!(f, "{p}");
            }
        }
    }

    pub fn wt_root_dir(&self) -> &str {
        &self.wt_root
    }
}

/// Live prefix resolution — the impure half of `config::resolve_prefix_from`,
/// which owns the order and the sanitization.
fn resolve_prefix(main_root: &str) -> String {
    let env = std::env::var("WORKTREES_PREFIX").ok();
    let file = prefix_file(main_root);
    let project = projcfg::project_prefix(Path::new(main_root));
    // `user_cfg`, not the raw `cfg_get`: `~/.config/worktrees/config.toml` is a
    // tier of the user rung too, and reading only the legacy kv file here was
    // why `prefix` in `config.toml` silently did nothing.
    let cfg = crate::config::user_cfg("prefix");
    resolve_prefix_from(
        env.as_deref(),
        file.as_deref(),
        project.as_deref(),
        cfg.as_deref(),
        &basename(main_root),
    )
}

/// First line of `<main_root>/.worktree-prefix`, whitespace stripped; a file
/// that holds only whitespace is ABSENT, not a prefix of `""`.
///
/// The one reader of that file, on purpose: `init` transcribes it into
/// `[project] prefix` through THIS function (`init::probe`), so the config it
/// writes says what the resolver would produce rather than what the bytes look
/// like — `doctor` comparing the two must not find a mismatch `init` created.
pub(crate) fn prefix_file(main_root: &str) -> Option<String> {
    std::fs::read_to_string(format!("{main_root}/{}", crate::init::PREFIX_FILE))
        .ok()
        .and_then(|c| c.lines().next().map(|l| l.chars().filter(|c| !c.is_whitespace()).collect::<String>()))
        .filter(|s| !s.is_empty())
}

/// The AI-command word used to PREFER an adopted session's AI pane — same
/// derivation as `ops::launch` (first word of the resolved ai_cmd, basename,
/// default `claude`). Computed once per snapshot here; `ops` calls it too, so
/// the two adoption paths cannot drift.
pub(crate) fn adopt_ai_word() -> String {
    let ai_cmd = crate::config::resolve_ai_cmd(None);
    let full = ai_cmd.split_whitespace().next().unwrap_or("");
    let word = if full.is_empty() { "claude" } else { full };
    basename(word)
}

fn detect_install_cmd(dir: &str) -> Option<String> {
    let has = |f: &str| Path::new(&format!("{dir}/{f}")).exists();
    if has("pnpm-lock.yaml") {
        Some("pnpm install".into())
    } else if has("bun.lockb") || has("bun.lock") {
        Some("bun install".into())
    } else if has("yarn.lock") {
        Some("yarn".into())
    } else if has("package-lock.json") || has("npm-shrinkwrap.json") {
        Some("npm install".into())
    } else {
        None
    }
}

/// True if a Claude Code conversation history already exists for this working
/// dir — drives the app's auto-resume (`-r`). Same detection as the per-place
/// `claude_session_present` field.
pub fn claude_session_present(dir: &str) -> bool {
    claude_has_session(&claude_dir_for(dir))
}

fn claude_dir_for(dir: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    // Claude Code names a project dir by replacing EVERY non-alphanumeric char
    // with '-' (not just '/' and '.') — match it or paths with '_' etc. miss.
    let mangled: String = dir.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    format!("{home}/.claude/projects/{mangled}")
}

fn claude_has_session(cdir: &str) -> bool {
    let p = Path::new(cdir);
    if !p.is_dir() {
        return false;
    }
    std::fs::read_dir(p)
        .map(|rd| rd.filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().ends_with(".jsonl")))
        .unwrap_or(false)
}

//! A Project = one opened git repo. Discovery + the read path (`ls`, `ls --json`)
//! ported 1:1 from the bash `cmd_ls`/`emit_*`. git/tmux are shelled out so the
//! stale-dir trap and output match exactly.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::{cfg_get, config_path, sanitize_prefix};
use crate::error::{Result, WtError};
use crate::model::{LsJson, Place, TmuxSession, SCHEMA_VERSION};
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
    pub fn ls_json(&self) -> String {
        let reg = self.registrations();
        let mut places = vec![self.place_json(&self.main_root, true, &reg)];
        // worktrees, recency-sorted like the human ls (stable, glob-order ties)
        let mut keyed: Vec<(i64, String)> = self
            .worktree_dirs()
            .into_iter()
            .map(|d| {
                let bepoch = self.clock.stat_birth(&d);
                let cepoch = if reg.contains(&d) { self.commit_epoch(&d) } else { 0 };
                let key = if cepoch > 0 { cepoch } else { bepoch };
                (key, d)
            })
            .collect();
        keyed.sort_by(|a, b| b.0.cmp(&a.0)); // stable desc
        for (_, d) in keyed {
            places.push(self.place_json(&d, false, &reg));
        }
        let ls = LsJson {
            schema_version: SCHEMA_VERSION,
            repo: self.main_root.clone(),
            prefix: self.prefix.clone(),
            places_file: format!("{}/.worktrees.places.json", self.main_root),
            places,
        };
        // serde_json compact = same shape/order as the bash emitter; add the
        // trailing newline the bash `printf ']}\n'` produced.
        format!("{}\n", serde_json::to_string(&ls).unwrap_or_default())
    }

    fn place_json(&self, dir: &str, is_main: bool, reg: &HashSet<String>) -> Place {
        let slug = if is_main { "(main)".to_string() } else { basename(dir) };
        let session = self.session_name(&slug);
        let tmux_up = tmux::session_exists(&session);
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
}

fn resolve_prefix(main_root: &str) -> String {
    let raw = std::env::var("WORKTREES_PREFIX").ok().filter(|s| !s.is_empty())
        .or_else(|| {
            std::fs::read_to_string(format!("{main_root}/.worktree-prefix"))
                .ok()
                .and_then(|c| c.lines().next().map(|l| l.chars().filter(|c| !c.is_whitespace()).collect::<String>()))
                .filter(|s| !s.is_empty())
        })
        .or_else(|| cfg_get(&config_path(), "prefix").filter(|s| !s.is_empty()))
        .unwrap_or_else(|| basename(main_root));
    sanitize_prefix(&raw)
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

fn claude_dir_for(dir: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mangled: String = dir.chars().map(|c| if c == '/' || c == '.' { '-' } else { c }).collect();
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

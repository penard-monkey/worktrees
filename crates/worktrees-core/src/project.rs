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

/// What one `git status --porcelain=v2 --branch` tells us — see `status_v2`.
struct StatusV2 {
    /// None when detached OR unborn (matches the old `rev-parse` behaviour).
    branch: Option<String>,
    detached: bool,
    dirty_files: u32,
    upstream: Option<String>,
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

    /// The claude config dir in effect for THIS repo — the bound profile's, or
    /// `~/.claude`. Every probe of claude's on-disk state goes through here;
    /// see `profile::claude_config_dir_for_repo` for why that matters.
    pub fn claude_config_root(&self) -> PathBuf {
        crate::profile::claude_config_dir_for_repo(&self.main_root)
    }

    /// True if `dir` already has a Claude Code conversation under this repo's
    /// effective config root — the app's auto-resume test.
    pub fn claude_session_present(&self, dir: &str) -> bool {
        claude_session_present_in(&self.claude_config_root(), dir)
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
    /// Branch, dirty count and upstream from ONE `git status --porcelain=v2
    /// --branch`, replacing three spawns every place paid every poll:
    /// `rev-parse --abbrev-ref HEAD`, `status --porcelain`, `rev-parse @{u}`.
    ///
    /// Semantics are pinned to what those three returned, ugly corners included:
    ///
    /// * **unborn HEAD** — `rev-parse --abbrev-ref HEAD` FAILS on a repo with no
    ///   commits, so the old code saw empty and called it detached. v2 helpfully
    ///   reports the branch-to-be via `# branch.head`; we deliberately throw that
    ///   away (keyed off `# branch.oid (initial)`) so the answer doesn't change.
    ///   `init_repo`/`create_initial_commit` make this a state the app really sees.
    /// * **detached** — v2 says `# branch.head (detached)`, the old call said
    ///   `HEAD`. Both mean "not a branch name".
    /// * **upstream** — subtler than it looks, and caught by diffing `ls --json`
    ///   against v0.9.0. `rev-parse @{u}` reports only an upstream that RESOLVES;
    ///   v2's `# branch.upstream` reports the CONFIGURED one even when its
    ///   remote-tracking ref is gone (branch deleted on origin, never fetched),
    ///   which silently turned `null` into `origin/<branch>` for such a place.
    ///   `# branch.ab` is the free discriminator: git emits it only when the
    ///   upstream resolves well enough to count ahead/behind. Gate on it and the
    ///   old answer is preserved without paying for the extra spawn.
    /// * **not a repo / git failed** — empty output → detached, clean, no upstream,
    ///   exactly as the three separate failures produced.
    ///
    /// Dirty count is the non-header lines (`1`/`2`/`u`/`?`) — the same set v1
    /// listed one per line, with the same untracked defaults, so counts match.
    fn status_v2(&self, dir: &str) -> StatusV2 {
        let text = git::git_out(dir, &["status", "--porcelain=v2", "--branch"]).unwrap_or_default();
        let (mut head, mut oid, mut upstream) = (String::new(), String::new(), None);
        let (mut has_ab, mut dirty_files) = (false, 0u32);
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("# branch.head ") {
                head = rest.to_string();
            } else if let Some(rest) = line.strip_prefix("# branch.oid ") {
                oid = rest.to_string();
            } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
                upstream = Some(rest.to_string());
            } else if line.starts_with("# branch.ab ") {
                has_ab = true;
            } else if !line.is_empty() && !line.starts_with('#') {
                dirty_files += 1;
            }
        }
        let detached = oid == "(initial)" || head.is_empty() || head == "(detached)";
        StatusV2 {
            branch: if detached { None } else { Some(head) },
            detached,
            dirty_files,
            // no branch.ab ⇒ the upstream does not resolve ⇒ what `rev-parse @{u}` called None
            upstream: upstream.filter(|s| !s.is_empty() && has_ab),
        }
    }

    /// Epoch AND subject of the last commit in ONE `git log`. `place_json` wants
    /// both and used to spawn git twice for them; a snapshot pays this per place,
    /// every poll. Tab-joined because a subject can contain anything but a
    /// newline — `splitn(2)` keeps a subject that itself contains tabs intact.
    fn last_commit(&self, dir: &str) -> (i64, Option<String>) {
        let Some(line) = git::git_out(dir, &["log", "-1", "--format=%ct%x09%s"]) else {
            return (0, None);
        };
        let mut it = line.splitn(2, '\t');
        let epoch = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let subj = it.next().filter(|s| !s.is_empty()).map(|s| s.to_string());
        (epoch, subj)
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
        // Which claude config dir this repo's places actually use — `~/.claude`,
        // or the bound profile's dir. Resolved once per snapshot alongside
        // `ai_word` (it reads profiles.json) rather than per place.
        let claude_root = self.claude_config_root();
        // Resolved once per snapshot, not per place — it's a couple of git
        // ref lookups and every place divergence-checks against the same ref.
        let base_ref = self.base_ref();
        // Build every place's snapshot in parallel — each place shells out to git
        // (status/divergence/log), and summed serially this froze the app on big
        // repos. Main first, then worktrees; recency-sort the worktrees afterwards
        // from the epochs already inside each Place (no second git pass for the key).
        let tasks: Vec<(String, bool)> = std::iter::once((self.main_root.clone(), true))
            .chain(self.worktree_dirs().into_iter().map(|d| (d, false)))
            .collect();
        let mut computed = self.place_json_par(&tasks, &reg, panes.as_ref(), &ai_word, &base_ref, &claude_root);
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

    fn place_json(&self, dir: &str, is_main: bool, reg: &HashSet<String>, panes: Option<&tmux::PaneList>, ai_word: &str, base_ref: &str, claude_root: &Path) -> Place {
        let slug = if is_main { "(main)".to_string() } else { basename(dir) };
        let canonical = self.session_name(&slug);
        // Canonical name first (exact match). If it's down, adopt any session
        // with a pane cwd'd in this dir — the SAME session `open` reuses. Without
        // this an adopted (foreign-named) session read as "down", the app never
        // mounted its terminal, and Enter looked dead. For MAIN, exclude
        // `.worktrees/`: worktree dirs nest under the main root, so without the
        // exclusion any live worktree session would falsely read as main's.
        //
        // BOTH lookups answer from the prefetched pane list — zero per-place
        // tmux shell-outs. The canonical check used `session_exists`, which is
        // its own `list-sessions`, so an N-place snapshot paid N of them every
        // poll. `has_session` is exact-match over the same snapshot and every
        // live session has at least one pane, so `list-panes -a` names them all.
        let exclude = if is_main { Some(self.wt_root.as_str()) } else { None };
        let (session, tmux_up) = if panes.is_some_and(|pl| pl.has_session(&canonical)) {
            (canonical, true)
        } else if let Some(adopted) = panes.and_then(|pl| pl.session_in(dir, ai_word, exclude)) {
            (adopted, true)
        } else {
            (canonical, false)
        };
        let cdir = claude_dir_in(claude_root, dir);
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

        let StatusV2 { branch, detached, dirty_files, upstream } = self.status_v2(dir);
        let dirty = dirty_files > 0;
        // Divergence vs the repo's BASE branch, not @{u}. Work flows through
        // worktrees toward main, and that's the question ↑↓ answers on sight:
        // "how far from main". Upstream told a different story — a branch that
        // just merged origin/main in showed ↑hundreds (the merged commits are
        // unpushed) exactly when the user had brought it in sync. It also lit
        // no arrows at all on branches with no upstream, which is most local
        // branches. (main) degenerates to the classic pull counter, since its
        // base ref IS origin/main. The `upstream` field stays informational.
        let (behind, ahead) = match git::git_out(dir, &["rev-list", "--left-right", "--count", &format!("{base_ref}...HEAD")]) {
            Some(ab) if !ab.is_empty() => {
                let mut it = ab.split_whitespace();
                (it.next().and_then(|x| x.parse().ok()), it.next().and_then(|x| x.parse().ok()))
            }
            _ => (None, None),
        };
        let (cepoch, csubj) = self.last_commit(dir);
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
    fn place_json_par(&self, tasks: &[(String, bool)], reg: &HashSet<String>, panes: Option<&tmux::PaneList>, ai_word: &str, base_ref: &str, claude_root: &Path) -> Vec<Place> {
        const LANES: usize = 16;
        let mut out = Vec::with_capacity(tasks.len());
        for chunk in tasks.chunks(LANES) {
            std::thread::scope(|s| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|(dir, is_main)| {
                        let is_main = *is_main;
                        s.spawn(move || self.place_json(dir, is_main, reg, panes, ai_word, base_ref, claude_root))
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

    /// Branch names a place could switch to: every local head, plus every
    /// `origin/*` that has no local counterpart (switching to one of those makes
    /// git track it — see `do_switch`). Sorted, deduped, remotes stripped of the
    /// `origin/` prefix because that is the name `switch` takes.
    ///
    /// A branch already checked out in another worktree is still listed: git
    /// refuses the switch with its own, better message, and hiding the name
    /// would just look like the branch doesn't exist.
    pub fn branch_names(&self) -> Vec<String> {
        let local = git::git_out(
            &self.main_root,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        )
        .unwrap_or_default();
        let mut names: Vec<String> = local.lines().filter(|l| !l.is_empty()).map(String::from).collect();
        let seen: std::collections::HashSet<String> = names.iter().cloned().collect();

        let remote = git::git_out(
            &self.main_root,
            &["for-each-ref", "--format=%(refname:short)", "refs/remotes/origin"],
        )
        .unwrap_or_default();
        for line in remote.lines() {
            // `origin/HEAD` is a symref to the default branch, not a branch
            let Some(short) = line.strip_prefix("origin/") else { continue };
            if short.is_empty() || short == "HEAD" || seen.contains(short) {
                continue;
            }
            names.push(short.to_string());
        }
        names.sort_unstable();
        names.dedup();
        names
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

    /// The ref every place's ↑↓ divergence is measured against: the remote's
    /// view of the default base when a fetch has brought one (origin/main moves
    /// on fetch — exactly what "am I behind?" should track), else the local
    /// base branch (no-remote repos).
    pub fn base_ref(&self) -> String {
        let base = self.default_base();
        if git::git_ok(&self.main_root, &["show-ref", "--verify", "-q", &format!("refs/remotes/origin/{base}")]) {
            format!("origin/{base}")
        } else {
            base
        }
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

/// The AI-command word used to PREFER an adopted session's AI pane.
///
/// The derivation itself lives in `profile::ai_word_of` — it used to be
/// copy-pasted here, in `ops::launch` and in the app, and `tmux::session_in`
/// SUBSTRING-matches the result against `pane_current_command`. One shared
/// function is what keeps a profiled launch (which prefixes env onto the shell
/// line) from quietly turning the match word into `CLAUDE_CONFIG_DIR=…` and
/// degrading adoption to "first pane in the worktree".
pub(crate) fn adopt_ai_word() -> String {
    crate::profile::ai_word_of(&crate::config::resolve_ai_cmd(None))
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
/// dir UNDER `claude_root` — drives the app's auto-resume (`-r`). Same detection
/// as the per-place `claude_session_present` field.
///
/// `claude_root` is the config dir actually in effect (`~/.claude`, or the bound
/// profile's dir). Passing the wrong one is a silent failure: the app decides
/// there is no history, launches cold, and the user's conversation appears lost.
pub fn claude_session_present_in(claude_root: &Path, dir: &str) -> bool {
    claude_has_session(&claude_dir_in(claude_root, dir))
}

fn claude_dir_in(claude_root: &Path, dir: &str) -> String {
    // Claude Code names a project dir by replacing EVERY non-alphanumeric char
    // with '-' (not just '/' and '.') — match it or paths with '_' etc. miss.
    let mangled: String = dir.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    claude_root.join("projects").join(mangled).to_string_lossy().into_owned()
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

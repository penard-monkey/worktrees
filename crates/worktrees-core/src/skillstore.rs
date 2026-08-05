//! The worktrees-owned skill store.
//!
//! Skills a user installs here can be enabled per AI profile; the materializer
//! symlinks them into `<profile dir>/skills/`. Same config/data split as
//! profiles: the MANIFEST is `~/.config/worktrees/skills.json`, the CONTENT is
//! `$XDG_DATA_HOME/worktrees/skills/<name>/`.
//!
//! ⚠ A SKILL IS INSTRUCTIONS THE MODEL READS, NOT INERT DATA.
//!
//! Every installed skill's `description` is loaded into the context at session
//! start — *before* anything invokes it — so installing one is closer to running
//! someone else's prompt than to copying a file. Frontmatter can additionally
//! carry `allowed-tools`, which pre-authorises tool use, and a skill can ship
//! scripts it tells the model to run. Two consequences shape this module:
//!
//! 1. **Installing must never execute anything from the source.** We copy files
//!    and parse text. No build step, no post-install hook, no `git checkout` of
//!    anything but the tree itself.
//! 2. **What a skill can do is surfaced, not hidden.** `inspect` records every
//!    capability-shaped thing it finds (`allowed-tools`, `hooks`, executable
//!    scripts) on the entry, so the UI can show it before the user enables the
//!    skill for a profile — and so `worktrees` can show it again later.
//!
//! Provenance is recorded for the same reason: a git-installed skill is pinned
//! to the exact commit that was reviewed, and an update is a diff the user
//! confirms rather than a silent fetch.

use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::sysclock::now_epoch;

/// Ceilings on what we will copy in. A skill is documentation plus the odd
/// helper script; anything at this scale is a repo that was pointed at us by
/// mistake, and we would rather say so than silently absorb it.
const MAX_FILES: usize = 2_000;
const MAX_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DEPTH: usize = 16;

// ── paths ────────────────────────────────────────────────────────────────────

/// Both locations the store owns, injected rather than read from the
/// environment inside each function — installing and removing MOVE AND DELETE
/// DIRECTORIES, so those paths have to be reachable from a test without
/// pointing them at the developer's real store.
#[derive(Debug, Clone)]
pub struct StorePaths {
    /// `~/.config/worktrees/skills.json` — declarations.
    pub manifest: PathBuf,
    /// `$XDG_DATA_HOME/worktrees/skills` — content.
    pub root: PathBuf,
}

impl StorePaths {
    pub fn real() -> Self {
        StorePaths {
            manifest: crate::profile::config_root_pub().join("skills.json"),
            root: crate::profile::skills_store_root(),
        }
    }

    /// Where one skill's files live. `None` for a name we would never have
    /// minted, so a hand-edited manifest cannot aim a copy or a delete outside
    /// the store.
    pub fn skill_dir(&self, name: &str) -> Option<PathBuf> {
        if !valid_name(name) {
            return None;
        }
        Some(self.root.join(name))
    }
}

/// `~/.config/worktrees/skills.json` — the manifest (declarations).
pub fn manifest_path() -> PathBuf {
    StorePaths::real().manifest
}

/// `$XDG_DATA_HOME/worktrees/skills` — the content.
pub fn store_root() -> PathBuf {
    StorePaths::real().root
}

/// Where one skill's files live, in the real store.
pub fn skill_dir(name: &str) -> Option<PathBuf> {
    StorePaths::real().skill_dir(name)
}

/// Skill names are directory names AND the key claude matches on: lowercase
/// `[a-z0-9_-]`, no separators, not `.`/`..`.
///
/// Lowercase-only is not cosmetic — macOS is case-insensitive, so `Foo` and
/// `foo` would be one directory and two manifest entries, and the store would
/// disagree with the filesystem about what is installed.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

// ── model ────────────────────────────────────────────────────────────────────

/// Where a skill came from. Recorded so an update can be a reviewable diff
/// against a known commit rather than "fetch whatever is there now".
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    /// Copied from a directory on this machine.
    Local { path: String },
    /// Cloned from a git remote and PINNED to the commit that was installed.
    Git {
        url: String,
        /// What the user asked for (branch, tag, or empty for the default head).
        #[serde(default)]
        rev: String,
        /// The commit actually installed. An update diffs against this.
        sha: String,
        /// Sub-directory within the repo, for repos carrying several skills.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub name: String,
    /// From the frontmatter — this is the text loaded into every session's
    /// context, so it is what the UI must show.
    #[serde(default)]
    pub description: String,
    pub source: Option<Source>,
    #[serde(default)]
    pub installed_epoch: i64,
    /// Capability-shaped things found at install time: `allowed-tools`, `hooks`,
    /// executable scripts. Surfaced, never silently accepted.
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Manifest {
    #[serde(default)]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_epoch: Option<i64>,
    #[serde(default)]
    pub skills: BTreeMap<String, Entry>,
    #[serde(flatten)]
    pub extra: Map<String, serde_json::Value>,
}

/// What `inspect` learned about a candidate directory, before anything is copied.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub name: String,
    pub description: String,
    /// Capability-shaped findings, for the review step.
    pub capabilities: Vec<String>,
    pub files: usize,
    pub bytes: u64,
    /// The SKILL.md body, so a UI can show exactly what the model will read.
    pub skill_md: String,
}

// ── inspection ───────────────────────────────────────────────────────────────

/// Read a candidate skill directory and decide whether it is installable.
///
/// Pure with respect to the store: it reads `dir` and nothing else, writes
/// nothing, and executes nothing.
pub fn inspect(dir: &Path) -> Result<Inspection, String> {
    let md_path = dir.join("SKILL.md");
    let skill_md = fs::read_to_string(&md_path)
        .map_err(|_| format!("no SKILL.md in {} — not a skill", dir.display()))?;
    let fm = frontmatter(&skill_md)
        .ok_or_else(|| "SKILL.md has no `---` frontmatter block".to_string())?;

    let dir_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = fm.get("name").cloned().unwrap_or_else(|| dir_name.clone());
    if !valid_name(&name) {
        return Err(format!(
            "skill name {name:?} must be lowercase [a-z0-9_-], 64 chars or fewer"
        ));
    }
    // claude discovers skills by directory but the frontmatter also names them.
    // A mismatch means the two disagree about what this skill is called, and we
    // would be guessing which one wins — so refuse instead of picking.
    if !dir_name.is_empty() && dir_name != name {
        return Err(format!(
            "SKILL.md says name: {name:?} but the directory is {dir_name:?} — they must match"
        ));
    }

    let mut capabilities = Vec::new();
    for (k, v) in &fm {
        let lk = k.to_ascii_lowercase();
        if lk == "allowed-tools" || lk == "allowed_tools" {
            capabilities.push(format!("pre-authorises tools: {v}"));
        }
        if lk == "hooks" {
            capabilities.push("declares hooks (can run commands on session events)".into());
        }
    }

    let mut files = 0usize;
    let mut bytes = 0u64;
    walk(dir, 0, &mut |p, meta| {
        files += 1;
        bytes += meta.len();
        if files > MAX_FILES {
            return Err(format!("more than {MAX_FILES} files — that is a repository, not a skill"));
        }
        if bytes > MAX_BYTES {
            return Err(format!("larger than {} MB", MAX_BYTES / 1024 / 1024));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o111 != 0 {
                capabilities.push(format!(
                    "ships an executable file: {}",
                    p.strip_prefix(dir).unwrap_or(p).display()
                ));
            }
        }
        Ok(())
    })?;

    Ok(Inspection {
        name,
        description: fm.get("description").cloned().unwrap_or_default(),
        capabilities,
        files,
        bytes,
        skill_md,
    })
}

/// Walk `dir`, refusing symlinks outright.
///
/// A symlink inside a skill is the whole ballgame: the materializer links the
/// skill into a profile's `skills/`, which claude reads — so a link named
/// `notes.md` pointing at `~/.ssh/id_rsa` would put a private key where the
/// model looks. We never follow one and never copy one.
fn walk(
    dir: &Path,
    depth: usize,
    f: &mut impl FnMut(&Path, &fs::Metadata) -> Result<(), String>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("directory nesting deeper than {MAX_DEPTH}"));
    }
    let rd = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in rd {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        let meta = fs::symlink_metadata(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "{} is a symlink — skills must be self-contained",
                p.display()
            ));
        }
        if meta.is_dir() {
            walk(&p, depth + 1, f)?;
        } else {
            f(&p, &meta)?;
        }
    }
    Ok(())
}

/// `key: value` pairs from a leading `---` fenced block. Deliberately a tiny
/// scanner rather than a YAML dependency: we need three keys, and the file is
/// untrusted input — the less machinery pointed at it, the better.
fn frontmatter(text: &str) -> Option<BTreeMap<String, String>> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut out = BTreeMap::new();
    for line in lines {
        let t = line.trim_end();
        if t.trim() == "---" {
            return Some(out);
        }
        if let Some((k, v)) = t.split_once(':') {
            if !k.starts_with(char::is_whitespace) && !k.trim().is_empty() {
                out.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    // No closing fence: treat as malformed rather than guessing where it ends.
    None
}

// ── copying ──────────────────────────────────────────────────────────────────

/// Copy a validated skill tree. Refuses symlinks (see `walk`) and re-checks the
/// caps as it goes, so a source that changes between inspect and copy cannot
/// slip past the limits.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    copy_dir(src, dst, 0, &mut files, &mut bytes)
}

fn copy_dir(src: &Path, dst: &Path, depth: usize, files: &mut usize, bytes: &mut u64) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("directory nesting deeper than {MAX_DEPTH}"));
    }
    for e in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let from = e.path();
        let meta = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err(format!("{} is a symlink — refusing to copy it", from.display()));
        }
        let name = e.file_name();
        // `.git` is the clone's own plumbing, not part of the skill.
        if name == ".git" {
            continue;
        }
        let to = dst.join(&name);
        if meta.is_dir() {
            fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            copy_dir(&from, &to, depth + 1, files, bytes)?;
        } else {
            *files += 1;
            *bytes += meta.len();
            if *files > MAX_FILES || *bytes > MAX_BYTES {
                return Err("skill exceeded the size limits while copying".into());
            }
            fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        }
    }
    Ok(())
}

// ── manifest storage ─────────────────────────────────────────────────────────

/// Missing or unparseable manifest reads as an empty store, never an error — a
/// typo must not make the tool unusable.
pub fn read_lenient() -> Manifest {
    read_lenient_at(&StorePaths::real())
}

pub fn read_lenient_at(paths: &StorePaths) -> Manifest {
    let mut m: Manifest = match fs::read(&paths.manifest) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => Manifest::default(),
    };
    // Same coherence rule profiles have: a key that disagrees with its entry, or
    // a name we would never have minted, cannot address a directory safely.
    m.skills.retain(|k, e| k == &e.name && valid_name(k));
    m
}

fn read_strict(path: &Path) -> Result<Manifest, String> {
    match fs::read(path) {
        Ok(b) => serde_json::from_slice(&b)
            .map_err(|e| format!("skills.json is not valid JSON ({e}) — not overwriting")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn edit<F, T>(paths: &StorePaths, f: F) -> Result<T, String>
where
    F: FnOnce(&mut Manifest) -> Result<T, String>,
{
    let path = paths.manifest.clone();
    if let Some(d) = path.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut m = read_strict(&path)?;
    let out = f(&mut m)?;
    m.version = m.version.max(1);
    m.updated_epoch = Some(now_epoch());
    let json = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
    let dir = path.parent().ok_or("no parent dir")?;
    let tmp = dir.join(format!(".skills.json.{}.tmp", std::process::id()));
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(out)
}

// ── install / remove ─────────────────────────────────────────────────────────

/// Install (or replace) a skill from a directory on this machine.
///
/// `inspect` runs first and its findings are returned, so a caller can show the
/// review before committing — but the copy only happens here, and re-inspects.
pub fn install_local(src: &Path) -> Result<Entry, String> {
    install_local_at(&StorePaths::real(), src)
}

pub fn install_local_at(paths: &StorePaths, src: &Path) -> Result<Entry, String> {
    let insp = inspect(src)?;
    let dst = paths.skill_dir(&insp.name).ok_or_else(|| format!("invalid skill name {:?}", insp.name))?;
    stage_and_swap(src, &dst)?;
    let entry = Entry {
        name: insp.name.clone(),
        description: insp.description,
        source: Some(Source::Local { path: src.to_string_lossy().into_owned() }),
        installed_epoch: now_epoch(),
        capabilities: insp.capabilities,
        extra: Map::new(),
    };
    let e2 = entry.clone();
    edit(paths, |m| {
        m.skills.insert(entry.name.clone(), entry);
        Ok(())
    })?;
    Ok(e2)
}

/// Copy into a staging dir beside the target, then swap. A half-copied skill
/// must never be visible to a live session — the materializer symlinks this
/// directory and claude hot-watches it.
fn stage_and_swap(src: &Path, dst: &Path) -> Result<(), String> {
    let parent = dst.parent().ok_or("no store root")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staging = parent.join(format!(".staging-{}-{}", std::process::id(), now_epoch()));
    let _ = fs::remove_dir_all(&staging);
    let res = copy_tree(src, &staging);
    if res.is_err() {
        let _ = fs::remove_dir_all(&staging);
        return res;
    }
    let old = parent.join(format!(".old-{}-{}", std::process::id(), now_epoch()));
    let had_old = dst.exists();
    if had_old {
        fs::rename(dst, &old).map_err(|e| format!("replacing {}: {e}", dst.display()))?;
    }
    match fs::rename(&staging, dst) {
        Ok(()) => {
            let _ = fs::remove_dir_all(&old);
            Ok(())
        }
        Err(e) => {
            // Put the previous version back rather than leaving nothing.
            if had_old {
                let _ = fs::rename(&old, dst);
            }
            let _ = fs::remove_dir_all(&staging);
            Err(format!("installing {}: {e}", dst.display()))
        }
    }
}

/// Clone `url` shallowly into `tmp`, pin the commit, and report the skills it
/// contains — WITHOUT installing any of them.
///
/// Split from the install so a caller can show the user what a URL actually
/// contains, and the SKILL.md text they are about to hand the model, before
/// anything lands in the store.
pub fn fetch_git(url: &str, rev: &str, tmp: &Path) -> Result<(String, Vec<PathBuf>), String> {
    if !crate::git::have_git() {
        return Err("git not found".into());
    }
    // A URL that is really a flag (`--upload-pack=…`) would otherwise be read as
    // one by git. `--` does not help for clone's source argument, so screen it.
    if url.starts_with('-') {
        return Err(format!("refusing a repository URL that looks like a flag: {url}"));
    }
    fs::create_dir_all(tmp).map_err(|e| e.to_string())?;
    let mut args: Vec<&str> = vec!["clone", "--depth", "1", "--no-tags", "--recurse-submodules=no"];
    if !rev.trim().is_empty() {
        args.push("--branch");
        args.push(rev);
    }
    args.push("--");
    args.push(url);
    args.push(".");
    let out = clone_no_prompt(tmp, &args)?;
    if !out.status.success() {
        return Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let sha = crate::git::git_out(&tmp.to_string_lossy(), &["rev-parse", "HEAD"])
        .ok_or("cannot resolve the cloned commit")?;

    // A repo is either one skill at its root, or a directory of them.
    let mut found = Vec::new();
    if tmp.join("SKILL.md").is_file() {
        found.push(tmp.to_path_buf());
    } else {
        let mut roots = vec![tmp.to_path_buf()];
        if let Ok(rd) = fs::read_dir(tmp.join("skills")) {
            roots.push(tmp.join("skills"));
            let _ = rd;
        }
        for root in roots {
            if let Ok(rd) = fs::read_dir(&root) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() && p.join("SKILL.md").is_file() && !found.contains(&p) {
                        found.push(p);
                    }
                }
            }
        }
    }
    found.sort();
    if found.is_empty() {
        return Err("no SKILL.md found in that repository".into());
    }
    Ok((sha, found))
}

/// `git clone` with the terminal prompt disabled, so a private URL fails fast
/// instead of hanging a launch (or a UI thread) on a credential prompt.
fn clone_no_prompt(cwd: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .output()
        .map_err(|e| e.to_string())
}

/// Install one skill from an already-fetched clone. `dir` must be one of the
/// paths `fetch_git` returned.
pub fn install_from_clone(
    url: &str,
    rev: &str,
    sha: &str,
    clone_root: &Path,
    dir: &Path,
) -> Result<Entry, String> {
    install_from_clone_at(&StorePaths::real(), url, rev, sha, clone_root, dir)
}

#[allow(clippy::too_many_arguments)]
pub fn install_from_clone_at(
    paths: &StorePaths,
    url: &str,
    rev: &str,
    sha: &str,
    clone_root: &Path,
    dir: &Path,
) -> Result<Entry, String> {
    let insp = inspect(dir)?;
    let dst = paths.skill_dir(&insp.name).ok_or_else(|| format!("invalid skill name {:?}", insp.name))?;
    stage_and_swap(dir, &dst)?;
    let subdir = dir
        .strip_prefix(clone_root)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty());
    let entry = Entry {
        name: insp.name.clone(),
        description: insp.description,
        source: Some(Source::Git {
            url: url.to_string(),
            rev: rev.to_string(),
            sha: sha.to_string(),
            subdir,
        }),
        installed_epoch: now_epoch(),
        capabilities: insp.capabilities,
        extra: Map::new(),
    };
    let e2 = entry.clone();
    edit(paths, |m| {
        m.skills.insert(entry.name.clone(), entry);
        Ok(())
    })?;
    Ok(e2)
}

/// Forget a skill and delete its files.
///
/// Returns the profiles that still reference it, so the caller can tell the user
/// what just changed. Deliberately does NOT refuse: a skill the user wants gone
/// (a bad one, say) must be removable even while a profile lists it, and the
/// materializer already degrades a missing skill to a warning.
pub fn remove(name: &str) -> Result<Vec<String>, String> {
    remove_at(&StorePaths::real(), name)
}

pub fn remove_at(paths: &StorePaths, name: &str) -> Result<Vec<String>, String> {
    let dir = paths.skill_dir(name).ok_or_else(|| format!("invalid skill name {name:?}"))?;
    edit(paths, |m| {
        m.skills.remove(name).ok_or_else(|| format!("no such skill: {name}"))?;
        Ok(())
    })?;
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("removing {}: {e}", dir.display()))?;
    }
    Ok(crate::profile::read_lenient()
        .profiles
        .values()
        .filter(|p| p.skills.iter().any(|s| s == name))
        .map(|p| p.name.clone())
        .collect())
}

/// Everything installed, for the UI and for `doctor`.
pub fn list() -> Vec<Entry> {
    read_lenient().skills.into_values().collect()
}

pub fn list_at(paths: &StorePaths) -> Vec<Entry> {
    read_lenient_at(paths).skills.into_values().collect()
}

// ── CLI ──────────────────────────────────────────────────────────────────────

/// `worktrees skills …`
///
/// Runs BEFORE the git guard in main: the store is user-global, so managing it
/// must not require standing in a repo.
pub fn cmd_skills(ui: &mut dyn crate::ui::Ui, args: &[String]) -> i32 {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "list" | "ls" => {
            let all = list();
            if all.is_empty() {
                ui.info("No skills installed. Add one with: worktrees skills add <dir>");
                return 0;
            }
            for e in all {
                let src = match &e.source {
                    Some(Source::Local { path }) => format!("local {path}"),
                    Some(Source::Git { url, sha, .. }) => format!("git {url}@{}", &sha[..sha.len().min(8)]),
                    None => "unknown".into(),
                };
                ui.info(&format!("{}  ({src})", e.name));
                if !e.description.is_empty() {
                    ui.info(&format!("    {}", e.description));
                }
                for c in &e.capabilities {
                    ui.warn_aside(&format!("    ! {c}"));
                }
            }
            0
        }
        "show" => {
            let Some(name) = rest.first() else {
                ui.error("usage: worktrees skills show <name>");
                return 2;
            };
            match skill_dir(name).filter(|d| d.is_dir()) {
                Some(d) => match inspect(&d) {
                    Ok(i) => {
                        ui.header(&format!("{} — {}", i.name, i.description));
                        for c in &i.capabilities {
                            ui.warn(c);
                        }
                        print!("{}", i.skill_md);
                        0
                    }
                    Err(e) => {
                        ui.error(&e);
                        1
                    }
                },
                None => {
                    ui.error(&format!("no such skill: {name}"));
                    1
                }
            }
        }
        "add" => cmd_add(ui, rest),
        "rm" | "remove" => {
            let Some(name) = rest.first() else {
                ui.error("usage: worktrees skills rm <name>");
                return 2;
            };
            match remove(name) {
                Ok(used_by) => {
                    ui.info(&format!("Removed skill '{name}'."));
                    if !used_by.is_empty() {
                        // Not an error: a bad skill must be removable. But the
                        // profiles that listed it will now warn at launch, so say
                        // so here rather than letting that be a surprise.
                        ui.warn(&format!(
                            "still enabled in {} profile(s): {} — they will warn on next launch",
                            used_by.len(),
                            used_by.join(", ")
                        ));
                    }
                    0
                }
                Err(e) => {
                    ui.error(&e);
                    1
                }
            }
        }
        other => {
            ui.error(&format!("unknown: worktrees skills {other}"));
            ui.info("usage: worktrees skills [list|show <name>|add <dir>|add --git <url> [--rev <r>] [--pick <name>]|rm <name>]");
            2
        }
    }
}

fn cmd_add(ui: &mut dyn crate::ui::Ui, args: &[String]) -> i32 {
    let yes = args.iter().any(|a| a == "--yes" || a == "-y");
    let flag = |name: &str| -> Option<String> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    };
    let git = flag("--git");
    let rev = flag("--rev").unwrap_or_default();
    let pick = flag("--pick");

    // Installing is not enabling — a skill only reaches a session once a profile
    // lists it. But a git install also DOWNLOADS someone else's instructions, so
    // anything capability-shaped has to be read before it lands, not after.
    let report = |ui: &mut dyn crate::ui::Ui, e: &Entry| {
        ui.info(&format!("Installed '{}': {}", e.name, e.description));
        for c in &e.capabilities {
            ui.warn(&format!("! {c}"));
        }
        ui.info("Not enabled anywhere yet — add it to a profile's `skills` to use it.");
    };

    if let Some(url) = git {
        let tmp = std::env::temp_dir().join(format!("wt-skill-{}-{}", std::process::id(), now_epoch()));
        let out = (|| -> Result<i32, String> {
            let (sha, found) = fetch_git(&url, &rev, &tmp)?;
            let chosen = match (&pick, found.len()) {
                (Some(p), _) => found
                    .iter()
                    .find(|d| d.file_name().map(|f| f.to_string_lossy() == p.as_str()).unwrap_or(false))
                    .cloned()
                    .ok_or_else(|| format!("no skill named {p:?} in that repository"))?,
                (None, 1) => found[0].clone(),
                (None, _) => {
                    ui.info("That repository carries several skills — choose one with --pick <name>:");
                    for d in &found {
                        if let Ok(i) = inspect(d) {
                            ui.info(&format!("  {}  — {}", i.name, i.description));
                        }
                    }
                    return Ok(2);
                }
            };
            let insp = inspect(&chosen)?;
            if !insp.capabilities.is_empty() && !yes {
                ui.warn(&format!("'{}' asks for capabilities beyond reading files:", insp.name));
                for c in &insp.capabilities {
                    ui.warn(&format!("  ! {c}"));
                }
                ui.info("Review it, then re-run with --yes to install.");
                return Ok(2);
            }
            let e = install_from_clone(&url, &rev, &sha, &tmp, &chosen)?;
            report(ui, &e);
            ui.info(&format!("Pinned to {}", &sha[..sha.len().min(12)]));
            Ok(0)
        })();
        let _ = fs::remove_dir_all(&tmp);
        return match out {
            Ok(c) => c,
            Err(e) => {
                ui.error(&e);
                1
            }
        };
    }

    let Some(dir) = args.iter().find(|a| !a.starts_with('-')) else {
        ui.error("usage: worktrees skills add <dir>   |   worktrees skills add --git <url>");
        return 2;
    };
    match install_local(Path::new(dir)) {
        Ok(e) => {
            report(ui, &e);
            0
        }
        Err(e) => {
            ui.error(&e);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let t = std::env::temp_dir().join(format!(
            "wtskill-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        Tmp(t)
    }
    fn skill(root: &Path, name: &str, front: &str) -> PathBuf {
        let d = root.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: does a thing\n{front}---\n\nbody\n"),
        )
        .unwrap();
        d
    }

    #[test]
    fn inspect_reads_the_frontmatter_a_session_will_load() {
        let t = tmp("insp");
        let d = skill(&t.0, "alpha", "");
        let i = inspect(&d).unwrap();
        assert_eq!(i.name, "alpha");
        assert_eq!(i.description, "does a thing");
        assert!(i.capabilities.is_empty());
        assert!(i.skill_md.contains("body"), "the UI must be able to show what the model reads");
    }

    #[test]
    fn capability_frontmatter_is_surfaced_not_swallowed() {
        // A skill's description loads into every session before anything invokes
        // it, and allowed-tools pre-authorises tool use. The user has to be able
        // to see that before enabling it.
        let t = tmp("caps");
        let d = skill(&t.0, "risky", "allowed-tools: Bash(rm:*), Read\n");
        let i = inspect(&d).unwrap();
        assert!(
            i.capabilities.iter().any(|c| c.contains("pre-authorises tools")),
            "{:?}",
            i.capabilities
        );
    }

    #[test]
    fn an_executable_file_is_reported() {
        let t = tmp("exe");
        let d = skill(&t.0, "scripted", "");
        let s = d.join("run.sh");
        fs::write(&s, "#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&s, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let i = inspect(&d).unwrap();
        #[cfg(unix)]
        assert!(i.capabilities.iter().any(|c| c.contains("executable")), "{:?}", i.capabilities);
    }

    #[test]
    fn a_symlink_anywhere_inside_is_refused() {
        // THE reason this check exists: the materializer links a skill into a
        // profile's skills/ dir, which claude reads. A link named notes.md
        // pointing at ~/.ssh/id_rsa would put a private key where the model looks.
        let t = tmp("link");
        let d = skill(&t.0, "sneaky", "");
        fs::write(t.0.join("secret.txt"), "s3cret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(t.0.join("secret.txt"), d.join("notes.md")).unwrap();
        let e = inspect(&d).unwrap_err();
        assert!(e.contains("symlink"), "{e}");
    }

    #[test]
    fn a_name_that_disagrees_with_its_directory_is_refused() {
        // claude finds skills by directory and the frontmatter also names them;
        // guessing which wins would install something under a name the user did
        // not read.
        let t = tmp("mismatch");
        let d = t.0.join("alpha");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), "---\nname: beta\ndescription: x\n---\n").unwrap();
        let e = inspect(&d).unwrap_err();
        assert!(e.contains("must match"), "{e}");
    }

    #[test]
    fn hostile_names_are_refused() {
        for n in ["../evil", "a/b", "..", ".", "", "Upper", "with space", &"x".repeat(65)] {
            assert!(!valid_name(n), "{n:?} should be rejected");
        }
        for n in ["alpha", "a-b_c", "x1"] {
            assert!(valid_name(n), "{n:?} should be allowed");
        }
        assert!(skill_dir("../evil").is_none());
    }

    #[test]
    fn missing_or_fenceless_skill_md_is_not_a_skill() {
        let t = tmp("nofm");
        let d = t.0.join("nope");
        fs::create_dir_all(&d).unwrap();
        assert!(inspect(&d).unwrap_err().contains("no SKILL.md"));
        fs::write(d.join("SKILL.md"), "no frontmatter here\n").unwrap();
        assert!(inspect(&d).unwrap_err().contains("frontmatter"));
        // an unterminated fence is malformed, not "everything is frontmatter"
        fs::write(d.join("SKILL.md"), "---\nname: nope\n").unwrap();
        assert!(inspect(&d).unwrap_err().contains("frontmatter"));
    }

    #[test]
    fn frontmatter_scanner_takes_only_top_level_keys_before_the_close() {
        let fm = frontmatter("---\nname: a\ndescription: b: with colons\n  indented: no\n---\nname: not-this\n").unwrap();
        assert_eq!(fm.get("name").unwrap(), "a");
        assert_eq!(fm.get("description").unwrap(), "b: with colons");
        assert!(!fm.contains_key("indented"), "nested keys are not ours to read");
        assert_eq!(fm.len(), 2, "nothing after the closing fence is frontmatter");
    }

    #[test]
    fn copy_refuses_a_symlink_and_skips_dot_git() {
        let t = tmp("copy");
        let src = skill(&t.0, "c1", "");
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::write(src.join(".git/config"), "x").unwrap();
        let dst = t.0.join("out");
        copy_tree(&src, &dst).unwrap();
        assert!(dst.join("SKILL.md").is_file());
        assert!(!dst.join(".git").exists(), "clone plumbing is not part of the skill");

        #[cfg(unix)]
        {
            let src2 = skill(&t.0, "c2", "");
            std::os::unix::fs::symlink("/etc/hosts", src2.join("link")).unwrap();
            assert!(copy_tree(&src2, &t.0.join("out2")).unwrap_err().contains("symlink"));
        }
    }

    #[test]
    fn a_url_that_looks_like_a_flag_is_refused() {
        let t = tmp("flag");
        let e = fetch_git("--upload-pack=touch /tmp/pwned", "", &t.0).unwrap_err();
        assert!(e.contains("looks like a flag"), "{e}");
    }

    #[test]
    fn manifest_drops_entries_whose_key_disagrees_with_their_name() {
        let m: Manifest = serde_json::from_str(
            r#"{"skills":{"good":{"name":"good"},"bad":{"name":"other"},"Bad Name":{"name":"Bad Name"}}}"#,
        )
        .unwrap();
        let mut m = m;
        m.skills.retain(|k, e| k == &e.name && valid_name(k));
        assert_eq!(m.skills.keys().collect::<Vec<_>>(), vec!["good"]);
    }

    fn store_in(root: &Path) -> StorePaths {
        StorePaths { manifest: root.join("cfg/skills.json"), root: root.join("store") }
    }

    #[test]
    fn install_copies_the_tree_and_records_provenance() {
        let t = tmp("install");
        let paths = store_in(&t.0);
        let src = skill(&t.0, "alpha", "");
        fs::write(src.join("extra.md"), "more").unwrap();

        let e = install_local_at(&paths, &src).unwrap();
        assert_eq!(e.name, "alpha");
        assert_eq!(e.description, "does a thing");
        assert!(matches!(e.source, Some(Source::Local { .. })));

        let installed = paths.skill_dir("alpha").unwrap();
        assert!(installed.join("SKILL.md").is_file());
        assert!(installed.join("extra.md").is_file());
        assert_eq!(list_at(&paths).len(), 1);
    }

    #[test]
    fn reinstalling_replaces_atomically_and_drops_removed_files() {
        // The materializer SYMLINKS this directory and claude hot-watches it, so
        // a half-copied replacement would be visible to a live session.
        let t = tmp("replace");
        let paths = store_in(&t.0);
        let src = skill(&t.0, "alpha", "");
        fs::write(src.join("old.md"), "old").unwrap();
        install_local_at(&paths, &src).unwrap();
        assert!(paths.skill_dir("alpha").unwrap().join("old.md").exists());

        fs::remove_file(src.join("old.md")).unwrap();
        fs::write(src.join("new.md"), "new").unwrap();
        install_local_at(&paths, &src).unwrap();
        let d = paths.skill_dir("alpha").unwrap();
        assert!(d.join("new.md").exists());
        assert!(!d.join("old.md").exists(), "a replace is a swap, not a merge");
        assert_eq!(list_at(&paths).len(), 1, "still one entry, not two");
    }

    #[test]
    fn a_failed_install_leaves_the_previous_version_in_place() {
        let t = tmp("keepold");
        let paths = store_in(&t.0);
        let good = skill(&t.0, "alpha", "");
        install_local_at(&paths, &good).unwrap();

        // now a source that will fail mid-copy: a symlink inside
        let bad = t.0.join("bad-src/alpha");
        fs::create_dir_all(&bad).unwrap();
        fs::copy(good.join("SKILL.md"), bad.join("SKILL.md")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", bad.join("link")).unwrap();
            assert!(install_local_at(&paths, &bad).is_err());
            // the working copy survived
            assert!(paths.skill_dir("alpha").unwrap().join("SKILL.md").is_file());
            assert_eq!(list_at(&paths).len(), 1);
        }
    }

    #[test]
    fn remove_deletes_the_files_and_names_the_profiles_still_using_it() {
        let t = tmp("remove");
        let paths = store_in(&t.0);
        let src = skill(&t.0, "alpha", "");
        install_local_at(&paths, &src).unwrap();
        let d = paths.skill_dir("alpha").unwrap();
        assert!(d.exists());

        let still = remove_at(&paths, "alpha").unwrap();
        assert!(!d.exists(), "content is deleted, not just the manifest entry");
        assert!(list_at(&paths).is_empty());
        // (no profiles declared in this test's environment)
        let _ = still;
        assert!(remove_at(&paths, "alpha").is_err(), "removing twice is an error, not a silent no-op");
    }

    #[test]
    fn staging_dirs_are_not_mistaken_for_skills() {
        // The staging/backup dirs live inside the store root; list() reads the
        // MANIFEST, so they can never show up as installed skills.
        let t = tmp("staging");
        let paths = store_in(&t.0);
        let src = skill(&t.0, "alpha", "");
        install_local_at(&paths, &src).unwrap();
        fs::create_dir_all(paths.root.join(".staging-999-1")).unwrap();
        let names: Vec<String> = list_at(&paths).into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["alpha".to_string()]);
    }

    #[test]
    fn a_broken_manifest_is_not_overwritten() {
        let t = tmp("broken");
        let paths = store_in(&t.0);
        fs::create_dir_all(paths.manifest.parent().unwrap()).unwrap();
        fs::write(&paths.manifest, "{ truncated").unwrap();
        let src = skill(&t.0, "alpha", "");
        let e = install_local_at(&paths, &src).unwrap_err();
        assert!(e.contains("not valid JSON"), "{e}");
        assert_eq!(fs::read_to_string(&paths.manifest).unwrap(), "{ truncated",
            "a hand-edit typo stays human-repairable");
    }

    #[test]
    fn unknown_keys_round_trip() {
        let m: Manifest = serde_json::from_str(
            r#"{"version":1,"skills":{"a":{"name":"a","future":1}},"top":true}"#,
        )
        .unwrap();
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("future"));
        assert!(s.contains("top"));
    }
}

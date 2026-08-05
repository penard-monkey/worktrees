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
        match normalize_key(k).as_str() {
            // The only two keys we understand well enough to call harmless.
            "name" | "description" => {}
            "allowed-tools" => capabilities.push(format!("pre-authorises tools: {v}")),
            "hooks" => capabilities.push("declares hooks (can run commands on session events)".into()),
            // ANYTHING ELSE is reported. This is the whole design: a deny-list of
            // known-dangerous keys can always be spelled around — claude parses
            // this block as real YAML, where `"allowed-tools":` is identical to
            // the bare key, while a naive scanner sees a different string and
            // reports nothing. An allow-list cannot hide a key; at worst it is
            // noisy, and noise is the safe direction for a review gate.
            other => capabilities.push(format!("declares frontmatter `{other}`: {v}")),
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
        // `.git` is skipped by the copy, so counting it here would reject a
        // perfectly good skill directory that happens to be a repo (and the git
        // install path inspects the clone root, which always has one).
        if e.file_name() == ".git" {
            continue;
        }
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

/// Normalize a frontmatter key the way a YAML reader would see it: unwrap
/// surrounding quotes, fold case, and treat `_` and `-` as the same word break.
///
/// Without this, `"allowed-tools"` and `allowed_tools` and `Allowed-Tools` are
/// three different strings to us and one key to claude.
fn normalize_key(k: &str) -> String {
    let t = k.trim();
    let t = t
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| t.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .unwrap_or(t);
    t.trim().to_ascii_lowercase().replace('_', "-")
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
pub fn fetch_git(url: &str, rev: &str, tmp: &Path) -> Result<(String, PathBuf, Vec<PathBuf>), String> {
    if !crate::git::have_git() {
        return Err("git not found".into());
    }
    // A URL that is really a flag (`--upload-pack=…`) would otherwise be read as
    // one by git. `--` does not help for clone's source argument, so screen it.
    if url.starts_with('-') {
        return Err(format!("refusing a repository URL that looks like a flag: {url}"));
    }
    // `ext::<command>` is a git TRANSPORT HELPER: cloning it RUNS that command,
    // before anything here inspects a single file. Whether it is permitted is
    // decided by the user's own git config, which this module does not own — so
    // screen the form as well as pinning the protocol allow-list below.
    // Anchored to the transport-helper shape (`scheme::`) so a legitimate IPv6
    // URL like `https://[::1]/r.git` is unaffected.
    if let Some((scheme, _)) = url.split_once("::") {
        if !scheme.is_empty()
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-')
            && !scheme.contains('/')
        {
            return Err(format!(
                "refusing {scheme}:: — git transport helpers execute a command as part of the clone"
            ));
        }
    }
    // Clone into a subdirectory of the caller's scratch dir. `create_dir` (not
    // create_dir_all) so this fails rather than following a symlink someone
    // planted; the parent is created permissively because the caller owns it —
    // `cmd_add` makes it with `unique_temp_dir`, which is itself fail-if-exists.
    fs::create_dir_all(tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let tmp = &tmp.join("clone");
    fs::create_dir(tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let mut args: Vec<&str> = vec![
        // Pin the protocols HERE rather than trusting the host's git config:
        // `protocol.ext.allow = always` in a user's ~/.gitconfig would otherwise
        // turn a clone into arbitrary command execution.
        "-c", "protocol.ext.allow=never",
        "-c", "protocol.allow=never",
        "-c", "protocol.https.allow=always",
        "-c", "protocol.ssh.allow=always",
        "-c", "protocol.git.allow=always",
        "-c", "protocol.file.allow=always",
        "clone",
        "--depth", "1",
        "--no-tags",
        // The correct spelling of the negation. `--recurse-submodules=no` is read
        // as a PATHSPEC, so it never disabled anything — harmless only because a
        // plain --depth 1 clone does not init submodules on its own.
        "--no-recurse-submodules",
    ];
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
    Ok((sha, tmp.to_path_buf(), found))
}

/// A fresh directory under the system temp dir that did not exist a moment ago.
///
/// `create_dir` (not `create_dir_all`) so an attacker who pre-creates the path —
/// as a symlink, say — loses the race instead of redirecting the clone.
pub fn unique_temp_dir(tag: &str) -> Result<PathBuf, String> {
    let base = std::env::temp_dir();
    for attempt in 0..64u32 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let cand = base.join(format!("{tag}-{}-{nanos}-{attempt}", std::process::id()));
        match fs::create_dir(&cand) {
            Ok(()) => return Ok(cand),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", cand.display())),
        }
    }
    Err("could not create a temporary directory".into())
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

// ── UI-facing preview/install ────────────────────────────────────────────────

/// What a git URL contains, without installing any of it.
#[derive(Serialize, Clone, Debug)]
pub struct GitPreview {
    pub url: String,
    pub rev: String,
    /// The commit that was inspected. Installing pins THIS, so what the user
    /// reviewed is exactly what lands.
    pub sha: String,
    pub skills: Vec<PreviewSkill>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PreviewSkill {
    pub name: String,
    pub description: String,
    /// Everything in the frontmatter we do not positively recognise, plus the
    /// keys we do and consider capability-shaped. The review gate.
    pub capabilities: Vec<String>,
    pub files: usize,
    pub bytes: u64,
    /// The full SKILL.md. This is what the model would read, so the user gets to
    /// read it first.
    pub skill_md: String,
}

/// Clone, inspect, throw the clone away, and report. Installs nothing.
///
/// Deliberately stateless: the alternative (keep the clone alive between a
/// "preview" call and an "install" call) means the app holds a temp directory
/// whose lifetime nobody owns, and a stale one silently installs content the
/// user never reviewed. Installing re-fetches AT THE PREVIEWED SHA instead, so
/// the reviewed bytes are the installed bytes even if the branch moved.
pub fn preview_git(url: &str, rev: &str) -> Result<GitPreview, String> {
    let tmp = unique_temp_dir("wt-skill-preview")?;
    let out = (|| -> Result<GitPreview, String> {
        let (sha, root, found) = fetch_git(url, rev, &tmp)?;
        let mut skills = Vec::new();
        for d in &found {
            let i = inspect(d)?;
            skills.push(PreviewSkill {
                name: i.name,
                description: i.description,
                capabilities: i.capabilities,
                files: i.files,
                bytes: i.bytes,
                skill_md: i.skill_md,
            });
        }
        let _ = root;
        Ok(GitPreview { url: url.to_string(), rev: rev.to_string(), sha, skills })
    })();
    let _ = fs::remove_dir_all(&tmp);
    out
}

/// Install one skill from a git URL, pinned to `sha` — the commit a preview
/// showed the user.
///
/// The sha is REQUIRED, not optional: without it this would fetch whatever the
/// branch points at now, which is not what was reviewed.
pub fn install_git_pinned(url: &str, rev: &str, sha: &str, name: &str) -> Result<Entry, String> {
    if sha.trim().is_empty() {
        return Err("a reviewed commit sha is required".into());
    }
    let tmp = unique_temp_dir("wt-skill-install")?;
    let out = (|| -> Result<Entry, String> {
        let (got, root, found) = fetch_git(url, rev, &tmp)?;
        if got != sha {
            // The branch moved between review and install. Refusing is the whole
            // point of pinning — silently installing the newer commit would mean
            // the review applied to something else.
            return Err(format!(
                "that repository moved since it was reviewed ({} → {}) — review it again",
                &sha[..sha.len().min(8)],
                &got[..got.len().min(8)]
            ));
        }
        let chosen = found
            .iter()
            .find(|d| d.file_name().map(|f| f.to_string_lossy() == *name).unwrap_or(false))
            .or_else(|| if found.len() == 1 { found.first() } else { None })
            .ok_or_else(|| format!("no skill named {name:?} in that repository"))?;
        install_from_clone(url, rev, sha, &root, chosen)
    })();
    let _ = fs::remove_dir_all(&tmp);
    out
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
        let tmp = match unique_temp_dir("wt-skill") {
            Ok(d) => d,
            Err(e) => {
                ui.error(&e);
                return 1;
            }
        };
        let out = (|| -> Result<i32, String> {
            let (sha, clone_root, found) = fetch_git(&url, &rev, &tmp)?;
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
            let e = install_from_clone(&url, &rev, &sha, &clone_root, &chosen)?;
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

    // Skip flags AND the values they consume, so `add --rev main /path` does not
    // resolve `dir` to "main".
    let mut positional: Option<&String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--git" || a == "--rev" || a == "--pick" {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        positional = Some(a);
        break;
    }
    let Some(dir) = positional else {
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
    fn a_capability_cannot_hide_behind_yaml_spelling() {
        // THE finding this design exists to prevent. claude parses this block as
        // real YAML, where `"allowed-tools":` is the same key as the bare form —
        // a scanner that string-matches the bare form reports nothing, the --yes
        // review gate never trips, and the skill pre-authorises tools silently.
        let t = tmp("hide");
        for (tag, front) in [
            ("q1", "\"allowed-tools\": Bash(rm:*)\n"),
            ("q2", "'allowed-tools': Bash(rm:*)\n"),
            ("up", "Allowed-Tools: Bash(rm:*)\n"),
            ("us", "allowed_tools: Bash(rm:*)\n"),
        ] {
            let d = skill(&t.0, tag, front);
            let i = inspect(&d).unwrap();
            assert!(
                !i.capabilities.is_empty(),
                "{tag}: a capability spelled {front:?} slipped past the gate"
            );
        }
    }

    #[test]
    fn an_unrecognised_frontmatter_key_is_reported_rather_than_ignored() {
        // Allow-list, not deny-list: we cannot enumerate every dangerous key a
        // future claude will honour, so anything we do not positively recognise
        // is surfaced for review.
        let t = tmp("unknown");
        let d = skill(&t.0, "odd", "some-future-capability: do-something\n");
        let i = inspect(&d).unwrap();
        assert!(
            i.capabilities.iter().any(|c| c.contains("some-future-capability")),
            "{:?}",
            i.capabilities
        );
    }

    #[test]
    fn a_plain_skill_reports_no_capabilities() {
        // The allow-list must not cry wolf on the ordinary case, or the gate
        // becomes noise the user clicks through.
        let t = tmp("plain");
        let d = skill(&t.0, "plain", "");
        assert!(inspect(&d).unwrap().capabilities.is_empty());
    }

    #[test]
    fn git_transport_helper_urls_are_refused() {
        // `ext::<cmd>` RUNS that command as part of the clone, before anything
        // here inspects a file.
        let t = tmp("ext");
        for url in ["ext::touch /tmp/pwned", "ext::sh -c 'id'"] {
            let d = t.0.join(format!("c{}", url.len()));
            let e = fetch_git(url, "", &d).unwrap_err();
            assert!(e.contains("transport helpers"), "{url} → {e}");
        }
        // …but a legitimate IPv6 URL is not collateral damage
        let d = t.0.join("ipv6");
        let e = fetch_git("https://[::1]/r.git", "", &d).unwrap_err();
        assert!(!e.contains("transport helpers"), "{e}");
    }

    #[test]
    fn inspect_ignores_dot_git_so_a_repo_shaped_skill_is_not_rejected() {
        // copy_dir skips .git; if inspect counted it, a skill directory that is
        // itself a repo (and every git-installed skill, whose clone root has one)
        // could trip the file-count cap.
        let t = tmp("dotgit");
        let d = skill(&t.0, "repoish", "");
        fs::create_dir_all(d.join(".git/objects")).unwrap();
        for i in 0..50 {
            fs::write(d.join(format!(".git/objects/o{i}")), "x").unwrap();
        }
        let i = inspect(&d).unwrap();
        assert_eq!(i.files, 1, "only SKILL.md counts, not the repo plumbing");
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
    fn install_leaves_no_staging_or_backup_dirs_behind() {
        // The previous version of this test asserted that staging dirs do not
        // appear in `list`, which reads the MANIFEST and so could not have failed
        // whatever the store contained. What actually matters is that a
        // successful install cleans up after itself.
        let t = tmp("staging");
        let paths = store_in(&t.0);
        let src = skill(&t.0, "alpha", "");
        install_local_at(&paths, &src).unwrap();
        install_local_at(&paths, &src).unwrap(); // replace → exercises the backup path
        let leftovers: Vec<String> = fs::read_dir(&paths.root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".staging-") || n.starts_with(".old-"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
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
    fn a_pinned_install_refuses_a_repository_that_moved_since_review() {
        // The review gate is only worth anything if the reviewed bytes are the
        // installed bytes. A branch that moved between preview and install must
        // stop, not silently install the newer commit.
        let t = tmp("moved");
        let repo = t.0.join("repo");
        let sk = repo.join("gitskill");
        fs::create_dir_all(&sk).unwrap();
        fs::write(sk.join("SKILL.md"), "---\nname: gitskill\ndescription: v1\n---\nv1\n").unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .expect("git");
        };
        git(&["init", "-q"]);
        git(&["add", "-A"]);
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "one"]);

        let url = format!("file://{}", repo.display());
        let p1 = preview_git(&url, "").unwrap();
        assert_eq!(p1.skills.len(), 1);
        assert_eq!(p1.skills[0].description, "v1");

        // move the branch on
        fs::write(sk.join("SKILL.md"), "---\nname: gitskill\ndescription: v2\n---\nv2\n").unwrap();
        git(&["add", "-A"]);
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "two"]);

        let e = install_git_pinned(&url, "", &p1.sha, "gitskill").unwrap_err();
        assert!(e.contains("moved since it was reviewed"), "{e}");
    }

    #[test]
    fn preview_installs_nothing_and_surfaces_capabilities() {
        let t = tmp("preview");
        let repo = t.0.join("repo");
        let sk = repo.join("risky");
        fs::create_dir_all(&sk).unwrap();
        fs::write(
            sk.join("SKILL.md"),
            "---\nname: risky\ndescription: d\nallowed-tools: Bash(rm:*)\n---\nbody\n",
        )
        .unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&repo).args(args).output().expect("git");
        };
        git(&["init", "-q"]);
        git(&["add", "-A"]);
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "one"]);

        let p = preview_git(&format!("file://{}", repo.display()), "").unwrap();
        assert_eq!(p.skills[0].name, "risky");
        assert!(p.skills[0].capabilities.iter().any(|c| c.contains("pre-authorises")));
        assert!(p.skills[0].skill_md.contains("body"), "the user reads what the model would read");
        assert!(!p.sha.is_empty());
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

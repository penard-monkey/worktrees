//! Will Codex stop at its folder-trust prompt in this directory? Read-only:
//! trust is the user's to give, so this never writes Codex's config and never
//! passes a `-c` that would trust a folder for them.
//!
//! Mirrors the TUI's own decision at upstream tag `rust-v0.160.1`
//! (`tui/src/config_update.rs::read_remote_project_trust`, which
//! `tui/src/onboarding/directory_trust.rs` asks at startup, whatever the
//! approval or sandbox flags), and the loader's `decision_for_dir`
//! (`config/src/loader/mod.rs`), which agrees with it:
//!
//! - the first `[projects."<path>"]` entry that HAS a `trust_level`, looked up
//!   for the cwd, then the nearest project root (`project_root_markers`,
//!   default `.git`), then the MAIN repository's root
//!   (`git-utils/src/trust.rs`, `git_root` below) — each by its canonical
//!   spelling, then the literal one. An entry without a `trust_level` is no
//!   decision and the lookup goes on; a trusted ancestor that is none of
//!   those three counts for nothing;
//! - Codex opens without asking iff that entry says `"trusted"`, or there is
//!   no entry and the directory is in no project at all.
//!
//! Not read: managed config (`/etc/codex`, MDM) and a cloud bundle, which can
//! also hold `projects` — a trust granted only there refuses here, and
//! `force` launches anyway.

use std::path::{Path, PathBuf};

/// `Ok` when Codex will open in `cwd` without asking. `Err` says why not, as
/// a sentence that ends on what the user must do.
pub fn check(cwd: &Path) -> Result<(), String> {
    check_in(cwd, &crate::codexmcp::config_path())
}

pub fn check_in(cwd: &Path, config: &Path) -> Result<(), String> {
    let text = match std::fs::read_to_string(config) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Codex's config {} could not be read ({e}); fix that", config.display())),
    };
    let cfg: toml::Value = toml::from_str(&text)
        .map_err(|e| format!("Codex's config {} does not parse ({}); fix that", config.display(), e.message()))?;
    let markers: Vec<String> = match cfg.get("project_root_markers").and_then(toml::Value::as_array) {
        Some(list) => list.iter().filter_map(toml::Value::as_str).map(String::from).collect(),
        None => vec![".git".into()],
    };
    let project_root = project_root(cwd, &markers);
    let projectless = project_root.is_none() && self::project_root(cwd, &[".git".into()]).is_none();
    let git_root = git_root(cwd);
    let projects = cfg.get("projects").and_then(toml::Value::as_table);
    let saved = |dir: &Path| {
        keys(dir).into_iter().find_map(|key| {
            let level = projects?.get(&key)?.get("trust_level")?.as_str()?;
            Some((key, level.to_string()))
        })
    };
    let found = saved(cwd)
        .or_else(|| saved(project_root.as_deref().unwrap_or(cwd)))
        .or_else(|| git_root.as_deref().and_then(|r| saved(r)));
    let target = match found {
        Some((_, level)) if level == "trusted" => return Ok(()),
        None if projectless => return Ok(()),
        Some((key, _)) => key,
        // What Codex's prompt names: trusting applies to the repository root.
        None => git_root.as_deref().or(project_root.as_deref()).map_or_else(|| keys(cwd).remove(0), |r| keys(r).remove(0)),
    };
    Err(format!(
        "Codex hasn't been told to trust {target} yet; it would stop at its folder-trust prompt. \
         Open Codex in that folder once and choose Trust"
    ))
}

/// A directory's lookup keys, as Codex spells them: canonical first, then the
/// literal path when that differs (`normalized_project_trust_keys`).
fn keys(dir: &Path) -> Vec<String> {
    let literal = dir.to_string_lossy().into_owned();
    match std::fs::canonicalize(dir).map(|p| p.to_string_lossy().into_owned()) {
        Ok(canonical) if canonical != literal => vec![canonical, literal],
        _ => vec![literal],
    }
}

/// The nearest ancestor of `cwd` holding one of `markers`; a `.git` directory
/// counts only with a `HEAD` in it (`discover_project_root`).
fn project_root(cwd: &Path, markers: &[String]) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|dir| {
            markers.iter().any(|m| {
                let at = dir.join(m);
                match std::fs::metadata(&at) {
                    Ok(meta) => m != ".git" || !meta.is_dir() || at.join("HEAD").exists(),
                    Err(_) => false,
                }
            })
        })
        .map(Path::to_path_buf)
}

// Codex resolves this through filesystem metadata, not git rev-parse (which
// would accept separate git dirs and malformed worktree links it rejects).
fn metadata_file(path: &Path) -> Option<Vec<u8>> {
    let m = std::fs::symlink_metadata(path).ok()?;
    if !m.is_file() || m.len() > 64 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() <= 64 * 1024).then_some(bytes)
}

fn file_path(base: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    (!text.is_empty()).then(|| base.join(text))
}

fn gitdir(path: &Path) -> Option<PathBuf> {
    let bytes = metadata_file(path)?;
    file_path(path.parent()?, bytes.trim_ascii().strip_prefix(b"gitdir:")?.trim_ascii())
}

fn same(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

/// The MAIN repository's root for `cwd` (`resolve_root_git_project_for_trust`):
/// a linked worktree resolves to the checkout that owns its common dir, and
/// only when the worktree's back-link and common dir both prove it.
pub(crate) fn git_root(cwd: &Path) -> Option<PathBuf> {
    let base = if cwd.is_dir() { cwd } else { cwd.parent()? };
    let repo = base.ancestors().find(|p| {
        let dot = p.join(".git");
        dot.exists() && (!dot.is_dir() || dot.join("HEAD").exists())
    })?;
    let dot = repo.join(".git");
    if dot.is_dir() {
        return Some(repo.to_path_buf());
    }
    let dir = gitdir(&dot)?;
    if !std::fs::symlink_metadata(&dir).ok()?.is_dir() {
        return None;
    }
    let canonical = std::fs::canonicalize(&dir).ok()?;
    let worktrees = canonical.parent()?;
    if worktrees.file_name()? != "worktrees" {
        return None;
    }
    let common = worktrees.parent()?;
    let registered = file_path(&canonical, &metadata_file(&canonical.join("gitdir"))?)?;
    let linked_common = file_path(&canonical, &metadata_file(&canonical.join("commondir"))?)?;
    if registered.file_name()? != ".git" || !same(registered.parent()?, repo) || !same(&linked_common, common) {
        return None;
    }
    let main = dir.parent()?.parent()?.parent()?;
    let main_dot = main.join(".git");
    let main_git = if main_dot.is_dir() { main_dot } else { gitdir(&main_dot)? };
    same(&main_git, common).then(|| main.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every case Codex 0.160.1 decides one way, against a scratch repo and a
    /// scratch config — which is read, never written, created or touched.
    #[test]
    fn codex_trust_lookup_follows_codex_and_never_writes() {
        let dir = std::env::temp_dir().join(format!("wt-codex-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cwd = dir.join("repo/subdir");
        std::fs::create_dir_all(&cwd).unwrap();
        let repo = cwd.parent().unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let cfg = dir.join("config.toml");
        let why = check_in(&cwd, &cfg).unwrap_err();
        assert!(why.contains(&*keys(repo)[0]), "the prompt names the repository root: {why}");
        assert!(!cfg.exists(), "a missing config must stay missing");
        let entry = |p: &Path, level: Option<&str>| match level {
            Some(l) => format!("[projects.\"{}\"]\ntrust_level = \"{l}\"\n", p.display()),
            None => format!("[projects.\"{}\"]\n", p.display()),
        };
        for (entries, opens) in [
            (entry(&dir, Some("trusted")), false), // an ancestor that is no root of it
            (entry(repo, Some("trusted")), true),
            (entry(repo, Some("untrusted")), false),
            (entry(repo, Some("Trusted")), false),
            // An entry with no level is no decision: the root's still holds.
            (entry(repo, Some("trusted")) + &entry(&cwd, None), true),
            (entry(repo, Some("trusted")) + &entry(&cwd, Some("untrusted")), false),
            (entry(repo, Some("untrusted")) + &entry(&cwd, Some("trusted")), true),
            ("broken = [".into(), false),
        ] {
            std::fs::write(&cfg, &entries).unwrap();
            let modified = std::fs::metadata(&cfg).unwrap().modified().unwrap();
            assert_eq!(check_in(&cwd, &cfg).is_ok(), opens, "{entries}");
            assert_eq!(std::fs::read_to_string(&cfg).unwrap(), entries);
            assert_eq!(std::fs::metadata(&cfg).unwrap().modified().unwrap(), modified, "never rewrite config");
        }
        // A linked worktree (git's own layout, built by hand): its project
        // root is the worktree, and the MAIN checkout's trust covers it.
        let wt = repo.join(".worktrees/feat");
        let admin = repo.join(".git/worktrees/feat");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        std::fs::write(admin.join("HEAD"), "ref: refs/heads/feat\n").unwrap();
        std::fs::write(admin.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        assert_eq!(git_root(&wt).map(|r| keys(&r).remove(0)), Some(keys(repo).remove(0)));
        std::fs::write(&cfg, entry(repo, Some("trusted"))).unwrap();
        assert!(check_in(&wt, &cfg).is_ok(), "the main checkout's trust covers its worktrees");
        std::fs::write(&cfg, entry(&repo.join(".worktrees"), Some("trusted"))).unwrap();
        assert!(check_in(&wt, &cfg).is_err(), "a trusted parent that is no root counts for nothing");
        // A project root nearer than the repository's decides before it.
        std::fs::write(cwd.join("Cargo.toml"), "").unwrap();
        let markers = "project_root_markers = [\"Cargo.toml\"]\n";
        std::fs::write(&cfg, format!("{markers}{}", entry(&cwd, Some("trusted")) + &entry(repo, Some("untrusted")))).unwrap();
        std::fs::create_dir(cwd.join("src")).unwrap();
        assert!(check_in(&cwd.join("src"), &cfg).is_ok(), "its project root's trust covers it, not the repo's");
        // In no project at all, Codex does not ask.
        let loose = dir.join("loose");
        std::fs::create_dir(&loose).unwrap();
        std::fs::write(&cfg, "").unwrap();
        assert!(check_in(&loose, &cfg).is_ok());
        #[cfg(unix)]
        {
            let alias = dir.join("alias");
            std::os::unix::fs::symlink(&cwd, &alias).unwrap();
            let canonical = std::fs::canonicalize(&cwd).unwrap();
            std::fs::write(&cfg, entry(&alias, Some("trusted")) + &entry(&canonical, Some("untrusted"))).unwrap();
            assert!(check_in(&alias, &cfg).is_err(), "canonical spelling wins before the alias");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

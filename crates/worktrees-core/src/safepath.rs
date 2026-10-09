//! Resolving a repo-relative path that came from somewhere we do not control —
//! a request, a config value, a user-chosen show-only plan path — without being
//! walked out of the tree by it.
//!
//! `safe_under` lived in the app's docserver until owned planning needed it in
//! the CLI too (the `plan hook` reads a show-only path through the same rule
//! the tab does; `docs/proposals/owned-planning.md` §2.5.2). The docserver
//! re-exports it, so there is one copy.
//!
//! Why canonicalise rather than lstat each component: these paths have several
//! components, and `symlink_metadata` refuses to follow only the LAST one
//! (AGENTS.md) — an intermediate `a -> elsewhere` is resolved exactly as
//! `metadata` would resolve it. A path built from constants plus ONE plain name
//! (`plan.rs`'s `.planning/<id>/task_plan.md`) can lstat its way down; a path a
//! person typed cannot.

use std::path::{Path, PathBuf};

/// The string half of the check: no empty, `.` or `..` component, nothing
/// absolute, no backslash (a Windows separator that this platform would treat
/// as an ordinary character in a name), no drive letter, no NUL.
fn lexically_ok(rel: &str) -> bool {
    if rel.is_empty() || rel.len() > 1024 || rel.contains('\0') || rel.contains('\\') {
        return false;
    }
    if rel.starts_with('/') || rel.as_bytes().get(1) == Some(&b':') {
        return false;
    }
    rel.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

/// `cand` canonicalised, if that lands under the canonical `root`.
/// Canonical on BOTH sides: `starts_with` compares components, so a root that
/// still contains a symlink (`/tmp` is `/private/tmp` here) would fail to
/// match its own files and refuse everything.
fn contained(root: &Path, cand: &Path) -> Option<PathBuf> {
    let canon = std::fs::canonicalize(cand).ok()?;
    let canon_root = std::fs::canonicalize(root).ok()?;
    canon.starts_with(&canon_root).then_some(canon)
}

/// Resolve a decoded relative path inside `root` to a REGULAR FILE, or refuse
/// it.
///
/// Two layers, and the second is the one that holds. The first is arithmetic on
/// a string (`lexically_ok`). A string cannot show a symlink, so the second
/// layer stats the candidate with `symlink_metadata` (never `metadata`: a link
/// is refused, not followed, even one pointing inside the root) and then
/// canonicalises it, which resolves every parent component, and requires the
/// result to still be under the canonical root.
pub fn safe_under(root: &Path, rel: &str) -> Option<PathBuf> {
    if !lexically_ok(rel) {
        return None;
    }
    let cand = root.join(rel);
    let md = std::fs::symlink_metadata(&cand).ok()?;
    if !md.is_file() {
        return None;
    }
    contained(root, &cand)
}

/// `safe_under`'s sibling for a DIRECTORY: the same string checks, a final
/// component that is a real directory (a symlinked directory is refused), and
/// the canonical result under the canonical root.
pub fn safe_dir_under(root: &Path, rel: &str) -> Option<PathBuf> {
    if !lexically_ok(rel) {
        return None;
    }
    let cand = root.join(rel);
    let md = std::fs::symlink_metadata(&cand).ok()?;
    if !md.is_dir() {
        return None;
    }
    contained(root, &cand)
}

/// A user-entered path, with ONE trailing slash stripped — `docs/plan/` and
/// `docs/plan` mean the same thing. Any other empty component is still refused
/// downstream.
pub fn normalize_entered(rel: &str) -> String {
    let t = rel.trim();
    t.strip_suffix('/').unwrap_or(t).to_string()
}

/// What a show-only path names, or why it names nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    File(PathBuf),
    Dir(PathBuf),
}

/// Why a path was refused, in the words the Plan tab and the CLI print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotFound,
    Outside,
    NotAFile,
}

impl Refusal {
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::NotFound => "not found",
            Refusal::Outside => "outside the project",
            Refusal::NotAFile => "not a file",
        }
    }
}

/// Classify `rel` under `root` for show-only: a file, a directory, or a reason.
/// The ACCEPTING answers come only from `safe_under` / `safe_dir_under`; the
/// rest only chooses which words to say.
pub fn classify(root: &Path, rel: &str) -> Result<Target, Refusal> {
    if let Some(f) = safe_under(root, rel) {
        return Ok(Target::File(f));
    }
    if let Some(d) = safe_dir_under(root, rel) {
        return Ok(Target::Dir(d));
    }
    if !lexically_ok(rel) {
        return Err(Refusal::Outside);
    }
    let cand = root.join(rel);
    match std::fs::symlink_metadata(&cand) {
        Err(_) => Err(Refusal::NotFound),
        Ok(md) if md.file_type().is_symlink() => Err(Refusal::Outside),
        Ok(md) if md.is_file() || md.is_dir() => Err(Refusal::Outside),
        Ok(_) => Err(Refusal::NotAFile),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wtsafe-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_directory_resolves_only_when_it_is_real_and_inside() {
        let base = tmp("dir");
        let root = base.join("root");
        let out = base.join("out");
        fs::create_dir_all(root.join("docs/plan")).unwrap();
        fs::create_dir_all(&out).unwrap();
        symlink(&out, root.join("docs/away")).unwrap();
        symlink(root.join("docs/plan"), root.join("docs/inside")).unwrap();
        fs::write(root.join("docs/f.md"), "x").unwrap();
        assert!(safe_dir_under(&root, "docs/plan").is_some());
        for bad in ["docs/away", "docs/inside", "docs/f.md", "docs/plan/", "../out", "docs/nope"] {
            assert!(safe_dir_under(&root, bad).is_none(), "{bad:?} resolved");
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn classify_names_why_and_accepts_only_what_the_safe_functions_accept() {
        let base = tmp("classify");
        let root = base.join("root");
        let out = base.join("out");
        fs::create_dir_all(root.join("docs/plan")).unwrap();
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("secret.md"), "x").unwrap();
        fs::write(root.join("docs/plan/task_plan.md"), "x").unwrap();
        symlink(&out, root.join("docs/away")).unwrap();
        symlink(out.join("secret.md"), root.join("docs/link.md")).unwrap();
        assert!(matches!(classify(&root, "docs/plan/task_plan.md"), Ok(Target::File(_))));
        assert!(matches!(classify(&root, "docs/plan"), Ok(Target::Dir(_))));
        assert_eq!(classify(&root, "docs/nope.md"), Err(Refusal::NotFound));
        assert_eq!(classify(&root, "../out/secret.md"), Err(Refusal::Outside));
        assert_eq!(classify(&root, "docs/link.md"), Err(Refusal::Outside));
        assert_eq!(classify(&root, "docs/away"), Err(Refusal::Outside));
        // through an intermediate link: the lstat sees a plain file, the
        // containment check does not
        assert_eq!(classify(&root, "docs/away/secret.md"), Err(Refusal::Outside));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn one_trailing_slash_is_stripped_when_entered() {
        assert_eq!(normalize_entered("docs/plan/"), "docs/plan");
        assert_eq!(normalize_entered(" docs/plan "), "docs/plan");
        assert_eq!(normalize_entered("docs/plan//"), "docs/plan/");
    }
}

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

/// The image types a chat shows, by extension: what a place's agent may point
/// `![](path)` at, and the MIME the viewer's `data:` URI needs. SVG is in: an
/// `<img>` never runs a script, and the Files pane shows one the same way.
pub const IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
];

/// A chat image's path, resolved to a regular file INSIDE one place, or why not
/// (`feat/chat-images`). `path` is what an agent wrote in `![](…)`: relative to
/// the PLACE's root, or absolute and then only under that root. Everything
/// else is refused in words that name the path, never dropped: another
/// place's tree, `/tmp`, `..`, a link out, a URL.
///
/// The accepting answer is `safe_under`'s alone (its own two layers: the
/// string rule, then lstat of the last component and canonicalisation of the
/// rest, so a committed `img -> /etc` directory link cannot leak a file). This
/// function only turns an absolute path into the relative one it takes, picks
/// the words, and keeps `(main)` out of the places under its `.worktrees/`:
/// the root of `(main)` contains every other place's tree.
pub fn place_image(root: &Path, is_main: bool, path: &str) -> Result<(PathBuf, &'static str), String> {
    let outside = || format!("outside this place: {path}");
    let p = path.trim();
    if p.is_empty() || p.contains('\0') || p.contains("://") || p.starts_with("data:") {
        return Err(format!("not a file in this place: {path}"));
    }
    let mut rel: String = if p.starts_with('/') {
        let canon = std::fs::canonicalize(root).map_err(|_| outside())?;
        let under = |r: &Path| Path::new(p).strip_prefix(r).ok().map(|x| x.to_string_lossy().into_owned());
        under(root).or_else(|| under(&canon)).ok_or_else(outside)?
    } else {
        p.to_string()
    };
    while let Some(r) = rel.strip_prefix("./") {
        rel = r.to_string();
    }
    let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let Some((_, mime)) = IMAGE_TYPES.iter().find(|(e, _)| *e == ext) else {
        return Err(format!("not an image: {path}"));
    };
    if is_main && rel.split('/').next() == Some(".worktrees") {
        return Err(outside());
    }
    match safe_under(root, &rel) {
        Some(f) => Ok((f, mime)),
        None => Err(match classify(root, &rel) {
            Err(Refusal::NotFound) => format!("not found: {path}"),
            Err(Refusal::NotAFile) => format!("not a file: {path}"),
            _ => outside(),
        }),
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

    /// The chat-image rule, one case per way a path can name a file: inside
    /// (relative, `./`, absolute), and every way out. The place is a nested
    /// folder of `base`, which also holds an "other place" with a real image.
    #[test]
    fn a_chat_image_resolves_only_to_a_regular_image_inside_its_place() {
        let base = tmp("chatimg");
        let root = base.join("place");
        let other = base.join("other");
        fs::create_dir_all(root.join(".worktrees-attachments")).unwrap();
        fs::create_dir_all(root.join("shots")).unwrap();
        fs::create_dir_all(root.join(".worktrees/sib")).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(root.join(".worktrees-attachments/a.PNG"), "x").unwrap();
        fs::write(root.join("shots/b.jpg"), "x").unwrap();
        fs::write(root.join("notes.txt"), "x").unwrap();
        fs::write(root.join(".worktrees/sib/c.png"), "x").unwrap();
        fs::write(other.join("secret.png"), "x").unwrap();
        symlink(other.join("secret.png"), root.join("shots/link.png")).unwrap();
        symlink(&other, root.join("away")).unwrap();
        let canon = fs::canonicalize(&root).unwrap();
        let ok = |p: &str| place_image(&root, false, p).map(|(f, m)| (f.strip_prefix(&canon).unwrap().to_path_buf(), m));
        // inside: relative, dotted, absolute (as given and canonical)
        assert_eq!(ok("shots/b.jpg").unwrap(), (PathBuf::from("shots/b.jpg"), "image/jpeg"));
        assert_eq!(ok("./shots/b.jpg").unwrap().0, PathBuf::from("shots/b.jpg"));
        assert_eq!(ok(".worktrees-attachments/a.PNG").unwrap().1, "image/png");
        assert!(ok(&format!("{}/shots/b.jpg", root.display())).is_ok());
        assert!(ok(&format!("{}/shots/b.jpg", canon.display())).is_ok());
        // outside: absolute elsewhere, `..`, absolute with `..`, links out, a link's directory
        for bad in [
            other.join("secret.png").to_string_lossy().into_owned(),
            "../other/secret.png".into(),
            format!("{}/../other/secret.png", root.display()),
            "shots/link.png".into(),
            "away/secret.png".into(),
            "/etc/hosts.png".into(),
        ] {
            let e = ok(&bad).unwrap_err();
            assert!(e.starts_with("outside this place: ") && e.ends_with(&bad), "{bad}: {e}");
        }
        // never fetched: URLs and data: are not files at all
        for url in ["https://example.com/a.png", "http://x/a.png", "data:image/png;base64,AAAA", "file:///etc/a.png"] {
            assert!(ok(url).unwrap_err().starts_with("not a file in this place: "), "{url}");
        }
        // other refusals say what they are
        assert!(ok("notes.txt").unwrap_err().starts_with("not an image: "));
        assert!(ok("shots/none.png").unwrap_err().starts_with("not found: "));
        assert!(ok("shots.png").unwrap_err().starts_with("not found: "));
        // (main)'s root holds every other place's tree
        assert!(place_image(&root, false, ".worktrees/sib/c.png").is_ok());
        assert!(place_image(&root, true, ".worktrees/sib/c.png").unwrap_err().starts_with("outside this place: "));
        let _ = fs::remove_dir_all(&base);
    }
}

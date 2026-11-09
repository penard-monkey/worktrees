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

/// The most pixels a chat image may hold (`feat/chat-images`, OWASP: image
/// bombs). The 4 MB file cap does not bound it: a solid-colour 150 MP PNG is
/// 175 KB. Measured 2026-11-09: Chromium and Playwright's WebKit decode
/// 16, 40, 64, 100 and 150 MP PNGs without a failure, so the number is about
/// MEMORY (4 bytes a pixel once decoded: 40 MP is 160 MB, 150 MP is 600 MB) and
/// not about what a browser refuses.
pub const MAX_IMAGE_PIXELS: u64 = 40_000_000;

/// A file `place_image` accepted: where it is and the identity it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceImageFile {
    pub path: PathBuf,
    pub mime: &'static str,
    pub dev: u64,
    pub ino: u64,
}

/// Why `rel` names no file under `root`, in ONE voice for everything behind a
/// link: when any component up to the file's own name is a symlink (or `rel`
/// itself is one), the answer is "outside this place" whether or not anything
/// exists on the other side, or `linkdir/x.png` would tell a reader which files
/// exist outside. Only a path of real directories inside the place can be
/// "not found".
fn why_not(root: &Path, rel: &str, path: &str) -> String {
    let outside = || format!("outside this place: {path}");
    let mut cur = root.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Err(_) => return format!("not found: {path}"),
            Ok(md) if md.file_type().is_symlink() => return outside(),
            Ok(md) if i + 1 < parts.len() && !md.is_dir() => return format!("not found: {path}"),
            Ok(md) if i + 1 == parts.len() && !md.is_file() => return format!("not a file: {path}"),
            Ok(_) => {}
        }
    }
    outside()
}

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
/// the words (`why_not`: one voice behind any link), and keeps `(main)` out of
/// the places under its `.worktrees/`, whose root holds every other place's
/// tree. That last one is decided on the RESOLVED file, not on the string: on a
/// case-insensitive volume `.Worktrees/x/y.png` is the same folder, so the
/// check is whether `<root>/.worktrees` is one of the file's own ancestors by
/// identity (dev, ino). The returned identity is what `open_place_image`
/// holds the opened file to.
pub fn place_image(root: &Path, is_main: bool, path: &str) -> Result<PlaceImageFile, String> {
    use std::os::unix::fs::MetadataExt;
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
    let Some(file) = safe_under(root, &rel) else {
        return Err(if lexically_ok(&rel) { why_not(root, &rel, path) } else { outside() });
    };
    let md = std::fs::metadata(&file).map_err(|_| format!("not found: {path}"))?;
    if is_main {
        if let Ok(wt) = std::fs::metadata(std::fs::canonicalize(root).map_err(|_| outside())?.join(".worktrees")) {
            let mut up = file.parent();
            while let Some(d) = up {
                if std::fs::metadata(d).is_ok_and(|m| (m.dev(), m.ino()) == (wt.dev(), wt.ino())) {
                    return Err(outside());
                }
                up = d.parent();
            }
        }
    }
    Ok(PlaceImageFile { path: file, mime, dev: md.dev(), ino: md.ino() })
}

/// What `open_place_image` read.
#[derive(Debug)]
pub struct ImageRead {
    /// The canonical file that was opened.
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    /// More than `cap` bytes are on disk; `bytes` holds the first `cap`.
    pub truncated: bool,
    /// The file's size and mtime (ms) when opened.
    pub on_disk: u64,
    pub mtime_ms: u64,
    pub mime: &'static str,
}

/// `place_image`, then the file OPENED the way that cannot be raced
/// (`feat/chat-images`, OWASP: check/open): `O_NOFOLLOW` on the canonical path,
/// then fstat of the HANDLE, which must be a regular file with the very
/// (dev, ino) `place_image` checked, so a file swapped for a link or another
/// file between the check and the open is refused. Then up to `cap` bytes, and
/// `check_image` over them. `have_etag` (the client's `"<mtime ms>-<size>"`)
/// returns early with no bytes, `unchanged`.
pub fn open_place_image(root: &Path, is_main: bool, path: &str, cap: u64, have_etag: Option<&str>) -> Result<(ImageRead, bool), String> {
    open_place_image_with(root, is_main, path, cap, have_etag, |p| {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(p)
    })
}

/// [`open_place_image`] with the open as a seam: a test swaps the file in the
/// window between `place_image`'s check and the open, which is the window the
/// guard is for.
fn open_place_image_with(
    root: &Path,
    is_main: bool,
    path: &str,
    cap: u64,
    have_etag: Option<&str>,
    open: impl FnOnce(&Path) -> std::io::Result<std::fs::File>,
) -> Result<(ImageRead, bool), String> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let f = place_image(root, is_main, path)?;
    let mut file = open(&f.path).map_err(|e| if e.raw_os_error() == Some(libc::ELOOP) { format!("outside this place: {path}") } else { format!("not found: {path}") })?;
    let md = file.metadata().map_err(|e| e.to_string())?;
    if !md.is_file() || (md.dev(), md.ino()) != (f.dev, f.ino) {
        return Err(format!("outside this place: {path}"));
    }
    let on_disk = md.len();
    let mtime_ms = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
    if have_etag == Some(format!("{mtime_ms}-{on_disk}").as_str()) {
        return Ok((ImageRead { path: f.path, bytes: Vec::new(), truncated: on_disk > cap, on_disk, mtime_ms, mime: f.mime }, true));
    }
    let mut bytes = Vec::new();
    (&mut file).take(cap + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > cap;
    bytes.truncate(cap as usize);
    check_image(&bytes, f.mime, path)?;
    Ok((ImageRead { path: f.path, bytes, truncated, on_disk, mtime_ms, mime: f.mime }, false))
}

/// The pixel size a raster image's HEADER declares, or None when `bytes` is not
/// one it can read. PNG IHDR, GIF logical screen, JPEG SOFn, WebP VP8/VP8L/VP8X,
/// BMP; nothing is decoded.
pub fn image_dims(bytes: &[u8], mime: &str) -> Option<(u64, u64)> {
    let le16 = |i: usize| bytes.get(i..i + 2).map(|b| u64::from(u16::from_le_bytes([b[0], b[1]])));
    let be32 = |i: usize| bytes.get(i..i + 4).map(|b| u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])));
    let le24 = |i: usize| bytes.get(i..i + 3).map(|b| u64::from(b[0]) | u64::from(b[1]) << 8 | u64::from(b[2]) << 16);
    match mime {
        "image/png" => (bytes.get(..8)? == b"\x89PNG\r\n\x1a\n" && bytes.get(12..16)? == b"IHDR").then(|| (be32(16), be32(20))).and_then(|(w, h)| Some((w?, h?))),
        "image/gif" => (bytes.starts_with(b"GIF8")).then(|| (le16(6), le16(8))).and_then(|(w, h)| Some((w?, h?))),
        "image/bmp" => {
            if !bytes.starts_with(b"BM") {
                return None;
            }
            let i32le = |i: usize| bytes.get(i..i + 4).map(|b| i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])).unsigned_abs());
            match le16(14)? | le16(16)? << 16 {
                12 => Some((le16(18)?, le16(20)?)),
                _ => Some((i32le(18)?, i32le(22)?)),
            }
        }
        "image/webp" => {
            if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
                return None;
            }
            match bytes.get(12..16)? {
                b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
                b"VP8L" => {
                    let b = bytes.get(21..25)?;
                    let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    (bytes.get(20)? == &0x2f).then(|| (u64::from(v & 0x3fff) + 1, u64::from((v >> 14) & 0x3fff) + 1))
                }
                b"VP8 " => (bytes.get(23..26)? == [0x9d, 0x01, 0x2a]).then(|| (le16(26).map(|w| w & 0x3fff), le16(28).map(|h| h & 0x3fff))).and_then(|(w, h)| Some((w?, h?))),
                _ => None,
            }
        }
        "image/jpeg" => {
            if bytes.get(..2)? != [0xff, 0xd8] {
                return None;
            }
            let mut i = 2;
            while i + 4 <= bytes.len() {
                if bytes[i] != 0xff {
                    return None;
                }
                let m = bytes[i + 1];
                if m == 0xff {
                    i += 1;
                    continue;
                }
                if (0xd0..=0xd9).contains(&m) || m == 0x01 {
                    i += 2;
                    continue;
                }
                let len = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
                if matches!(m, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                    let h = u64::from(u16::from_be_bytes([*bytes.get(i + 5)?, *bytes.get(i + 6)?]));
                    let w = u64::from(u16::from_be_bytes([*bytes.get(i + 7)?, *bytes.get(i + 8)?]));
                    return Some((w, h));
                }
                i += 2 + len.max(2);
            }
            None
        }
        _ => None,
    }
}

/// Refuse an image the viewer must not be handed: a raster over
/// [`MAX_IMAGE_PIXELS`] by its header (or one whose header says nothing), an
/// SVG that declares a DOCTYPE or an ENTITY (entity expansion: a small file
/// that unfolds to gigabytes in a parser). `path` is only for the words.
pub fn check_image(bytes: &[u8], mime: &str, path: &str) -> Result<(), String> {
    if mime == "image/svg+xml" {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1 << 20)]).to_ascii_lowercase();
        if head.contains("<!doctype") || head.contains("<!entity") {
            return Err(format!("not shown: an SVG with a DOCTYPE or ENTITY is refused: {path}"));
        }
        return Ok(());
    }
    match image_dims(bytes, mime) {
        Some((w, h)) if w.saturating_mul(h) <= MAX_IMAGE_PIXELS => Ok(()),
        Some((w, h)) => Err(format!("not shown: {w}×{h} is {} megapixels, over the {} limit: {path}", w.saturating_mul(h) / 1_000_000, MAX_IMAGE_PIXELS / 1_000_000)),
        None => Err(format!("not shown: the image header cannot be read: {path}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::fs::OpenOptionsExt;

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
        let ok = |p: &str| place_image(&root, false, p).map(|f| (f.path.strip_prefix(&canon).unwrap().to_path_buf(), f.mime));
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
        assert!(ok("nodir/none.png").unwrap_err().starts_with("not found: "), "a real place with no such folder");
        assert!(ok("shots.png").unwrap_err().starts_with("not found: "));
        // (main)'s root holds every other place's tree
        assert!(place_image(&root, false, ".worktrees/sib/c.png").is_ok());
        assert!(place_image(&root, true, ".worktrees/sib/c.png").unwrap_err().starts_with("outside this place: "));
        let _ = fs::remove_dir_all(&base);
    }

    /// (main)'s root holds every other place: `.worktrees/` is refused by what
    /// the file RESOLVES to, so a differently-cased folder name on a
    /// case-insensitive volume (the macOS default) cannot get past a string
    /// compare. On a case-sensitive volume the cased path simply names nothing.
    #[test]
    fn main_refuses_the_places_under_it_by_identity_not_by_spelling() {
        let base = tmp("mainwt");
        let root = base.join("main");
        fs::create_dir_all(root.join(".worktrees/sib")).unwrap();
        fs::write(root.join(".worktrees/sib/c.png"), "x").unwrap();
        fs::write(root.join("own.png"), "x").unwrap();
        assert!(place_image(&root, true, "own.png").is_ok());
        for spelled in [".worktrees/sib/c.png", "./.worktrees/sib/c.png", ".Worktrees/sib/c.png", ".WORKTREES/sib/C.PNG", &format!("{}/.Worktrees/sib/c.png", root.display())] {
            let got = place_image(&root, true, spelled);
            assert!(got.is_err(), "{spelled} got through: {got:?}");
        }
        let insensitive = root.join(".WORKTREES").exists();
        eprintln!("volume is case-{}sensitive", if insensitive { "in" } else { "" });
        if insensitive {
            assert_eq!(place_image(&root, true, ".Worktrees/sib/c.png").unwrap_err(), "outside this place: .Worktrees/sib/c.png");
        }
        let _ = fs::remove_dir_all(&base);
    }

    /// A folder that is a link answers ONE way whatever is behind it, or
    /// `linkdir/x.png` says which files exist outside the place.
    #[test]
    fn behind_a_link_the_answer_is_the_same_whether_or_not_the_file_exists() {
        let base = tmp("oracle");
        let root = base.join("place");
        let out = base.join("out");
        fs::create_dir_all(root.join("real")).unwrap();
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("there.png"), "x").unwrap();
        symlink(&out, root.join("linkdir")).unwrap();
        symlink(base.join("nowhere"), root.join("dangling")).unwrap();
        let said = |p: &str| place_image(&root, false, p).unwrap_err().replace(p, "<p>");
        let there = said("linkdir/there.png");
        assert_eq!(there, "outside this place: <p>");
        assert_eq!(said("linkdir/absent.png"), there, "an absent file behind the link says the same");
        assert_eq!(said("linkdir/deeper/absent.png"), there, "and a deeper one");
        assert_eq!(said("dangling/x.png"), there, "and behind a dangling link");
        assert_eq!(said("dangling"), "not an image: <p>");
        symlink(out.join("there.png"), root.join("real/l.png")).unwrap();
        symlink(base.join("nowhere.png"), root.join("real/d.png")).unwrap();
        assert_eq!(said("real/l.png"), there, "a link to a file outside");
        assert_eq!(said("real/d.png"), there, "a link to nothing");
        // only a path of real folders inside the place can be "not found"
        assert_eq!(said("real/none.png"), "not found: <p>");
        assert_eq!(said("nodir/none.png"), "not found: <p>");
        let _ = fs::remove_dir_all(&base);
    }

    /// The file is opened with O_NOFOLLOW and held to the identity that was
    /// checked: a swap between the check and the open (another file renamed
    /// over it, or a link planted in its place) is refused.
    #[test]
    fn the_file_opened_is_the_file_checked() {
        let base = tmp("race");
        let root = base.join("place");
        let out = base.join("out");
        fs::create_dir_all(root.join("shots")).unwrap();
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("secret.png"), "SECRET").unwrap();
        fs::write(root.join("shots/a.png"), b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x08\0\0\0\x08").unwrap();
        let open = |p: &Path| std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(p);
        // no swap: read
        let (read, unchanged) = open_place_image_with(&root, false, "shots/a.png", 1 << 20, None, open).unwrap();
        assert!(!unchanged && read.bytes.starts_with(b"\x89PNG"), "{read:?}");
        // another file renamed over it in the window
        let swapped = |p: &Path| {
            fs::write(root.join("shots/new.png"), "OTHER").unwrap();
            fs::rename(root.join("shots/new.png"), p).unwrap();
            open(p)
        };
        assert_eq!(open_place_image_with(&root, false, "shots/a.png", 1 << 20, None, swapped).unwrap_err(), "outside this place: shots/a.png");
        // a link planted in its place in the window: O_NOFOLLOW refuses it
        fs::write(root.join("shots/b.png"), "x").unwrap();
        let linked = |p: &Path| {
            fs::remove_file(p).unwrap();
            symlink(out.join("secret.png"), p).unwrap();
            open(p)
        };
        let e = open_place_image_with(&root, false, "shots/b.png", 1 << 20, None, linked).unwrap_err();
        assert_eq!(e, "outside this place: shots/b.png");
        // ... and WITHOUT O_NOFOLLOW it would have read the secret (what the flag is for)
        fs::remove_file(root.join("shots/b.png")).unwrap();
        fs::write(root.join("shots/b.png"), "x").unwrap();
        let follows = |p: &Path| {
            fs::remove_file(p).unwrap();
            symlink(out.join("secret.png"), p).unwrap();
            std::fs::File::open(p)
        };
        let e = open_place_image_with(&root, false, "shots/b.png", 1 << 20, None, follows).unwrap_err();
        assert_eq!(e, "outside this place: shots/b.png", "the identity check alone refuses a followed link too");
        let _ = fs::remove_dir_all(&base);
    }

    fn hdr_png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        v.extend(w.to_be_bytes());
        v.extend(h.to_be_bytes());
        v
    }

    /// Dimensions come from the HEADER of every format the chat shows, and
    /// anything over the pixel limit, an unreadable header, or an SVG with a
    /// DOCTYPE or ENTITY is refused in words.
    #[test]
    fn image_bombs_are_refused_by_their_headers() {
        let gif = |w: u16, h: u16| { let mut v = b"GIF89a".to_vec(); v.extend(w.to_le_bytes()); v.extend(h.to_le_bytes()); v };
        let bmp = |w: i32, h: i32| { let mut v = b"BM".to_vec(); v.extend([0u8; 12]); v.extend(40u32.to_le_bytes()); v.extend(w.to_le_bytes()); v.extend(h.to_le_bytes()); v };
        let jpeg = |w: u16, h: u16| { let mut v = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8]; v.extend(h.to_be_bytes()); v.extend(w.to_be_bytes()); v.extend([1, 1, 0x11, 0]); v };
        let webp = |kind: &[u8; 4], body: Vec<u8>| { let mut v = b"RIFF\0\0\0\0WEBP".to_vec(); v.extend(kind); v.extend([0u8; 4]); v.extend(body); v };
        let vp8x = |w: u32, h: u32| { let mut b = vec![0u8; 4]; b.extend(&(w - 1).to_le_bytes()[..3]); b.extend(&(h - 1).to_le_bytes()[..3]); webp(b"VP8X", b) };
        let vp8l = |w: u32, h: u32| { let mut b = vec![0x2f]; b.extend(((w - 1) | ((h - 1) << 14)).to_le_bytes()); webp(b"VP8L", b) };
        let vp8 = |w: u16, h: u16| { let mut b = vec![0u8; 3]; b.extend([0x9d, 0x01, 0x2a]); b.extend(w.to_le_bytes()); b.extend(h.to_le_bytes()); webp(b"VP8 ", b) };
        assert_eq!(image_dims(&hdr_png(640, 480), "image/png"), Some((640, 480)));
        assert_eq!(image_dims(&gif(30, 20), "image/gif"), Some((30, 20)));
        assert_eq!(image_dims(&bmp(100, -50), "image/bmp"), Some((100, 50)));
        assert_eq!(image_dims(&jpeg(1200, 800), "image/jpeg"), Some((1200, 800)));
        assert_eq!(image_dims(&vp8x(5000, 4000), "image/webp"), Some((5000, 4000)));
        assert_eq!(image_dims(&vp8l(321, 123), "image/webp"), Some((321, 123)));
        assert_eq!(image_dims(&vp8(64, 48), "image/webp"), Some((64, 48)));
        assert_eq!(image_dims(b"not an image", "image/png"), None);
        // at the limit passes, one row over is refused, with the numbers in the note
        assert!(check_image(&hdr_png(8000, 5000), "image/png", "a.png").is_ok(), "40 MP exactly");
        let e = check_image(&hdr_png(8000, 5001), "image/png", "a.png").unwrap_err();
        assert!(e.contains("8000×5001") && e.contains("megapixels") && e.contains("40") && e.ends_with("a.png"), "{e}");
        assert!(check_image(&gif(65535, 65535), "image/gif", "g").is_err());
        assert!(check_image(&bmp(100_000, 100_000), "image/bmp", "b").is_err());
        assert!(check_image(&jpeg(65535, 65535), "image/jpeg", "j").is_err());
        assert!(check_image(&vp8x(16384, 16384), "image/webp", "w").is_err());
        assert!(check_image(&vp8l(16384, 16384), "image/webp", "w").is_err());
        assert!(check_image(&hdr_png(u32::MAX, u32::MAX), "image/png", "p").is_err(), "no overflow");
        assert!(check_image(b"garbage", "image/png", "p").unwrap_err().contains("header cannot be read"));
        // SVG: a DOCTYPE or an ENTITY, whatever the case, is refused; a plain one passes
        let svg = |s: &str| check_image(s.as_bytes(), "image/svg+xml", "x.svg");
        assert!(svg("<svg xmlns='http://www.w3.org/2000/svg'/>").is_ok());
        assert!(svg("<?xml version='1.0'?><!DOCTYPE svg [<!ENTITY a 'aaaa'>]><svg/>").unwrap_err().contains("DOCTYPE or ENTITY"));
        assert!(svg("<!doctype svg><svg/>").is_err() && svg("<svg><!ENTITY x 'y'></svg>").is_err());
        // end to end: a bomb on disk is refused by the reader, with its note
        let base = tmp("bomb");
        fs::write(base.join("big.png"), hdr_png(10_000, 10_000)).unwrap();
        let e = open_place_image(&base, false, "big.png", 1 << 20, None).unwrap_err();
        assert!(e.contains("10000×10000") && e.contains("over the 40 limit"), "{e}");
        let _ = fs::remove_dir_all(&base);
    }
}

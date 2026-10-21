//! Replacing a file so that a reader sees the old bytes or the new ones, and so
//! that nothing already on disk can redirect the write.
//!
//! Every "atomic write" here used to be `fs::write(tmp)` + `rename(tmp, dst)`
//! with `tmp` a FIXED or predictable name beside the target. `fs::write`
//! follows a symlink, and the directory is often a checkout: git checks a
//! tracked `120000` blob out with no check at all, so a cloned repo could
//! commit `.worktrees.places.json.tmp -> ~/.zshrc` and have the next store
//! write go through it, after which `rename` put the link itself in place of
//! the target. A pid suffix is no defence — a repo can commit one link per pid.
//!
//! So the temp is created with `O_CREAT | O_EXCL | O_NOFOLLOW` under a random
//! name: an existing path of ANY kind is refused, and refused means "pick
//! another name", never "remove what is there". The mode is set on the open
//! handle, and the only file ever deleted is a temp this call created.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Names tried before giving up. A random 64-bit name is taken only if
/// something put a file there on purpose; this bounds that, nothing else.
const ATTEMPTS: u32 = 16;

/// Write `bytes` to `path` via a fresh temp in the same directory (so the
/// `rename` stays on one filesystem), then rename it over `path`.
///
/// `mode` is applied with `fchmod` on the temp's handle before the rename.
/// `None` leaves the creation mode — `0o666` less the umask, what `fs::write`
/// gives a new file. A temp that will get a mode is created `0o600`, so its
/// bytes are never readable by more than the mode it ends with allows.
///
/// A symlink AT `path` is replaced, not followed (`rename` never follows),
/// which is what every caller already relied on. A symlink in a PARENT
/// component is followed like any path; callers that take a directory from
/// a repo check it themselves (`materialize::make_dir`, a canonical root).
pub fn write_atomic(path: &Path, bytes: &[u8], mode: Option<Permissions>) -> io::Result<()> {
    write_atomic_named(path, bytes, mode, random_suffix)
}

fn write_atomic_named(
    path: &Path,
    bytes: &[u8],
    mode: Option<Permissions>,
    mut suffix: impl FnMut() -> u64,
) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{}: no file name", path.display())))?;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut taken = None;
    for _ in 0..ATTEMPTS {
        let mut tmp = OsString::from(".");
        tmp.push(name);
        tmp.push(format!(".{:016x}.tmp", suffix()));
        let tmp = dir.join(tmp);
        let file = match create(&tmp, mode.is_some()) {
            Ok(f) => f,
            Err(e) if is_taken(&e) => {
                taken = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        };
        return finish(file, &tmp, path, bytes, mode).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        });
    }
    Err(taken.unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no free temp name")))
}

fn create(tmp: &Path, private: bool) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(if private { 0o600 } else { 0o666 })
        .open(tmp)
}

/// `O_EXCL` answers EEXIST for anything at the name, a link included; with
/// `O_NOFOLLOW` beside it a platform may answer ELOOP for the link instead.
/// Either way the name is someone else's.
fn is_taken(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::AlreadyExists || e.raw_os_error() == Some(libc::ELOOP)
}

fn finish(mut file: File, tmp: &Path, path: &Path, bytes: &[u8], mode: Option<Permissions>) -> io::Result<()> {
    file.write_all(bytes)?;
    if let Some(p) = mode {
        file.set_permissions(p)?;
    }
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}

/// 64 bits a repo author cannot predict: `RandomState`'s keys come from the
/// OS generator, and the counter keeps two calls in one process apart.
fn random_suffix() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(SEQ.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::PathBuf;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let t = std::env::temp_dir().join(format!("wtfsx-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        Tmp(t)
    }
    fn names(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }
    fn mode(p: &Path) -> u32 {
        fs::symlink_metadata(p).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn creates_then_replaces_and_leaves_no_temp() {
        let t = tmp("replace");
        let f = t.0.join("a.json");
        write_atomic(&f, b"one", None).unwrap();
        write_atomic(&f, b"two", None).unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"two");
        assert_eq!(names(&t.0), ["a.json"]);
        assert_eq!(mode(&f) & 0o600, 0o600, "the owner can read and write what it wrote");
    }

    #[test]
    fn a_link_at_the_target_is_replaced_not_followed() {
        let t = tmp("target-link");
        let victim = t.0.join("victim");
        fs::write(&victim, "precious").unwrap();
        let f = t.0.join("a.json");
        symlink(&victim, &f).unwrap();
        write_atomic(&f, b"new", None).unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        assert!(fs::symlink_metadata(&f).unwrap().is_file());
        assert_eq!(fs::read(&f).unwrap(), b"new");
    }

    /// The heart of it: a name that is already taken — by a link to a file
    /// outside, or by someone's regular file — is skipped, neither written
    /// through nor removed, and the next name is used.
    #[test]
    fn a_taken_temp_name_is_skipped_never_followed_or_removed() {
        let t = tmp("taken");
        let victim = t.0.join("victim");
        fs::write(&victim, "precious").unwrap();
        fs::set_permissions(&victim, Permissions::from_mode(0o600)).unwrap();
        let f = t.0.join("a.json");
        let at = |n: u64| t.0.join(format!(".a.json.{n:016x}.tmp"));
        symlink(&victim, at(1)).unwrap();
        fs::write(at(2), "mine").unwrap();
        let mut next = 0;
        write_atomic_named(&f, b"new", Some(Permissions::from_mode(0o755)), || {
            next += 1;
            next
        })
        .unwrap();
        assert_eq!(next, 3, "two taken names, the third used");
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        assert_eq!(mode(&victim), 0o600, "the mode went through the link");
        assert!(fs::symlink_metadata(at(1)).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(at(2)).unwrap(), "mine");
        assert_eq!(fs::read(&f).unwrap(), b"new");
    }

    #[test]
    fn gives_up_when_every_name_is_taken() {
        let t = tmp("exhausted");
        let f = t.0.join("a.json");
        fs::write(t.0.join(format!(".a.json.{:016x}.tmp", 7)), "mine").unwrap();
        let e = write_atomic_named(&f, b"new", None, || 7).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(!f.exists());
        assert_eq!(fs::read_to_string(t.0.join(format!(".a.json.{:016x}.tmp", 7))).unwrap(), "mine");
    }

    /// Set through the handle, so the umask has no say: an exec bit survives,
    /// and a private file is private.
    #[test]
    fn the_mode_is_exactly_the_one_asked_for() {
        let t = tmp("mode");
        for m in [0o755, 0o600, 0o640] {
            let f = t.0.join(format!("f{m:o}"));
            write_atomic(&f, b"x", Some(Permissions::from_mode(m))).unwrap();
            assert_eq!(mode(&f), m);
        }
    }

    /// A rename that fails (the target is a non-empty directory) cleans up the
    /// temp it made and nothing else.
    #[test]
    fn a_failed_rename_removes_only_its_own_temp() {
        let t = tmp("rename-fails");
        let f = t.0.join("a.json");
        fs::create_dir_all(f.join("inside")).unwrap();
        fs::write(t.0.join(".keep.tmp"), "mine").unwrap();
        assert!(write_atomic(&f, b"new", None).is_err());
        assert_eq!(names(&t.0), [".keep.tmp", "a.json"]);
        assert!(f.join("inside").is_dir());
    }

    #[test]
    fn a_path_with_no_file_name_is_refused() {
        let e = write_atomic(Path::new("/"), b"x", None).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "no file name to write");
    }

    // ── the guard: no temp name of its own, anywhere ─────────────────────────

    /// Lines a release build compiles that build a temp-like name without
    /// `fsx`, each with why a repo cannot redirect it:
    /// `(file, text on the line, reason)`.
    const OWN_TEMP_NAMES: &[(&str, &str, &str)] = &[
        ("worktrees-core/src/fsx.rs", "tmp.push(format!", "the helper itself"),
        ("worktrees-core/src/materialize.rs", ".wt-tmp-", "links only: symlink(2) makes the link and refuses an existing name"),
        ("worktrees-core/src/tmux_server.rs", "with_extension(format!(\"{}.{}.tmp\"", "already create_new, mode 0600"),
        ("worktrees-core/src/slots.rs", "format!(\".{}.tmp\", me.pid)", "$XDG_STATE_HOME, and the fd must stay locked across the rename"),
        ("worktrees-core/src/guidance.rs", "format!(\".tmp-{}-{}\"", "a staged DIRECTORY in the user's data dir"),
        ("worktrees-core/src/guidance.rs", "\"wt-guidance-{}-{nanos}\"", "create_dir, which refuses anything already there"),
        ("worktrees-core/src/skillstore.rs", "\".staging-{}-{}\"", "a staged DIRECTORY in the user's skill store"),
        ("worktrees-core/src/skillstore.rs", "\".old-{}-{}\"", "a rename target in the user's skill store"),
        ("worktrees-core/src/skillstore.rs", "format!(\"{tag}-{}-{nanos}-{attempt}\"", "create_dir, which refuses anything already there"),
        ("worktrees-core/src/messages.rs", "format!(\"{:013}-{:x}-{:08x}\"", "a message id; its file is written by fsx"),
        ("worktrees-core/src/messages.rs", "ends_with(\".tmp\")", "matches stranded temps to sweep; builds nothing"),
        ("worktrees-core/src/tmux.rs", "\"worktrees-drop-{}\"", "a tmux buffer name, not a path"),
        ("src-tauri/src/viewer.rs", "\".{name}.{}.{n}.tmp\"", "the app's config dir; a sync per document would cost every derive"),
        ("src-tauri/src/viewer.rs", "join(std::process::id().to_string())", "a directory in the app's config dir"),
        ("src-tauri/src/lib.rs", "\"worktrees-install-{}.sh\"", "temp_dir(), not a checkout: per-user on macOS; Linux /tmp is a local-user race, not a repo plant"),
    ];

    /// Every `src` directory in the workspace: each crate's and the app's.
    fn sources(root: &Path) -> Vec<PathBuf> {
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            for e in fs::read_dir(d).unwrap().map(Result::unwrap) {
                let p = e.path();
                if e.file_type().unwrap().is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut out = Vec::new();
        for c in fs::read_dir(root.join("crates")).unwrap().map(Result::unwrap) {
            if c.path().join("src").is_dir() {
                walk(&c.path().join("src"), &mut out);
            }
        }
        walk(&root.join("app/src-tauri/src"), &mut out);
        out.sort();
        out
    }

    /// The lines a release build compiles: a column-0 `#[cfg(test)]` drops the
    /// item under it — one line ending `;` or `}`, or a block up to the
    /// column-0 line that closes it (rustfmt puts it there). `mod tests` is
    /// one such item, and so are test helpers that live beside it.
    fn live_lines(text: &str) -> Vec<(usize, &str)> {
        let mut out = Vec::new();
        let mut it = text.lines().enumerate();
        while let Some((n, line)) = it.next() {
            if line != "#[cfg(test)]" {
                out.push((n + 1, line));
                continue;
            }
            for (_, l) in it.by_ref() {
                if l.starts_with("#[") || l.starts_with("///") {
                    continue;
                }
                if !(l.ends_with(';') || l.ends_with('}')) {
                    for (_, l) in it.by_ref() {
                        if l.starts_with(['}', ']', ')']) {
                            break;
                        }
                    }
                }
                break;
            }
        }
        out
    }

    /// A string literal naming a temp (`".x.tmp"`, `"wt-tmp"`, `"tmp"` for
    /// `with_extension`) — not `"/tmp"`, which names a directory — or a pid
    /// going into a name. Comments are not code.
    fn builds_temp_name(line: &str) -> bool {
        let t = line.trim_start();
        if t.starts_with("//") {
            return false;
        }
        let temp_like = |s: &str| s == "tmp" || [".tmp", "tmp.", "tmp-", "-tmp", "_tmp", "tmp_"].iter().any(|k| s.contains(k));
        let literal_tmp = t.split('"').skip(1).step_by(2).any(temp_like);
        let pid_name = t.contains("process::id()")
            && ["join(", "with_extension(", "with_file_name(", "format!("].iter().any(|k| t.contains(k));
        literal_tmp || pid_name
    }

    /// The fixed-name temp beside a target is how a cloned repo's committed
    /// symlink got written through (store, `.worktrees.toml`, `.worktree.env`,
    /// the dock editor's save). So, outside test code, no source builds a
    /// temp name of its own: a new write goes through [`write_atomic`] or
    /// earns a line in [`OWN_TEMP_NAMES`] saying why it is safe. A line there
    /// that no longer matches anything fails too, so the list cannot rot into
    /// a blanket pass. Read as bytes: `projcfg.rs` carries a non-UTF-8 byte.
    #[test]
    fn no_source_builds_its_own_temp_name() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut used = vec![false; OWN_TEMP_NAMES.len()];
        let mut hits = Vec::new();
        for f in sources(&root) {
            let rel = f.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
            let text = String::from_utf8_lossy(&fs::read(&f).unwrap()).into_owned();
            for (n, line) in live_lines(&text).into_iter().filter(|(_, l)| builds_temp_name(l)) {
                match OWN_TEMP_NAMES.iter().position(|(file, needle, _)| rel.ends_with(file) && line.contains(needle)) {
                    Some(i) => used[i] = true,
                    None => hits.push(format!("{rel}:{n}: {}", line.trim())),
                }
            }
        }
        assert!(
            hits.is_empty(),
            "a temp name built outside fsx — use fsx::write_atomic, or add it to OWN_TEMP_NAMES with the reason:\n{}",
            hits.join("\n")
        );
        let stale: Vec<_> = OWN_TEMP_NAMES.iter().zip(&used).filter(|(_, u)| !**u).map(|(e, _)| format!("{}: {}", e.0, e.1)).collect();
        assert!(stale.is_empty(), "OWN_TEMP_NAMES entries that match nothing any more:\n{}", stale.join("\n"));
    }
}

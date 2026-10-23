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
//!
//! [`PrivateDir`] covers a file that is about to be RUN rather than renamed
//! into place: a fresh 0700 directory to stage it in.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

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
/// a repo check it themselves ([`make_dir`] per component, a canonical root).
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
        let file = match create(&tmp, if mode.is_some() { 0o600 } else { 0o666 }) {
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

fn create(path: &Path, mode: u32) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(mode)
        .open(path)
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

/// Create ONE directory, and only if it is genuinely missing. A racing `mkdir`
/// is fine; a symlink, or anything else that is not a directory, standing where
/// we expected to create one is not (materialize §4 B6). Never
/// `create_dir_all` under a checkout: it follows a link it meets on the way.
pub fn make_dir(d: &Path) -> io::Result<()> {
    match fs::create_dir(d) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let md = fs::symlink_metadata(d)?;
            if md.file_type().is_symlink() || !md.is_dir() {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not a real directory", d.display()),
                ))
            } else {
                Ok(())
            }
        }
        Err(e) => Err(e),
    }
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

/// A directory only this user can enter, made fresh for one job and removed,
/// with everything in it, when dropped. For a file that something is about
/// to RUN or read back, where [`write_atomic`]'s rename would not help.
///
/// The updater used to stage install.sh as `temp_dir()/worktrees-install-
/// <pid>.sh`. On Linux that is the shared `/tmp`, where any local user can
/// take the name first: with a link, or with a 0666 file of their own that
/// curl writes into and its owner rewrites before bash reads it (measured:
/// the planter's script ran as the victim). Here the name is random and made
/// by `mkdir`, which refuses anything already at it, a link included, so a
/// taken name means "pick another". The new directory is then opened without
/// following a link, given 0700 on that handle and checked on it: a
/// directory, this user's, mode 0700.
///
/// `base` is created 0700 if missing. It is refused if someone else could
/// rename the directory away after that check and put their own in its
/// place: when others can write it and it is not sticky, or when it belongs
/// to another user who is not root. A group-writable base passes. Under
/// umask 002, the user-private-group default on Debian and Ubuntu, the
/// user's own `~/.cache/worktrees` is 0775 already.
#[derive(Debug)]
pub struct PrivateDir(PathBuf);

impl PrivateDir {
    pub fn new(base: &Path, tag: &str) -> io::Result<PrivateDir> {
        PrivateDir::new_named(base, tag, random_suffix)
    }

    fn new_named(base: &Path, tag: &str, mut suffix: impl FnMut() -> u64) -> io::Result<PrivateDir> {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(base)?;
        let b = fs::metadata(base)?;
        if (b.mode() & 0o002 != 0 && b.mode() & 0o1000 == 0) || (b.uid() != euid() && b.uid() != 0) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{}: another user could replace what is made in it", base.display()),
            ));
        }
        let mut taken = None;
        for _ in 0..ATTEMPTS {
            let dir = base.join(format!("{tag}-{:016x}", suffix()));
            match fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    taken = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            }
            return match seal(&dir) {
                Ok(()) => Ok(PrivateDir(dir)),
                Err(e) => {
                    // `remove_dir` takes only an empty directory: the one
                    // just made, never one with someone's files in it.
                    let _ = fs::remove_dir(&dir);
                    Err(e)
                }
            };
        }
        Err(taken.unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no free directory name")))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Create `name` in the directory with `O_CREAT | O_EXCL | O_NOFOLLOW`,
    /// write `bytes` and set exactly `mode` on the handle, which is closed
    /// before this returns. Returns the file's path.
    pub fn create_file(&self, name: &str, bytes: &[u8], mode: u32) -> io::Result<PathBuf> {
        if Path::new(name).file_name() != Some(name.as_ref()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{name}: not a file name")));
        }
        let path = self.0.join(name);
        let mut file = create(&path, 0o600)?;
        file.write_all(bytes)?;
        file.set_permissions(Permissions::from_mode(mode))?;
        Ok(path)
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 0700 on the directory's own handle, then check that handle: a directory
/// (not a link, which `O_NOFOLLOW` refuses to open), this user's, 0700.
fn seal(dir: &Path) -> io::Result<()> {
    let handle = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(dir)?;
    handle.set_permissions(Permissions::from_mode(0o700))?;
    let m = handle.metadata()?;
    if !m.is_dir() || m.uid() != euid() || m.mode() & 0o7777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: not a private directory of this user's after creating it", dir.display()),
        ));
    }
    Ok(())
}

fn euid() -> u32 {
    // SAFETY: geteuid takes nothing and cannot fail.
    unsafe { libc::geteuid() }
}

/// Write `body` to `path` as an executable (0755) that a test is about to RUN
/// (a fake `gh`, `ssh` or `codex`). Do not use `fs::write` + `set_permissions`
/// for one.
///
/// On Linux, `execve` of a file that ANY process holds open for writing fails
/// with ETXTBSY (errno 26). libtest runs tests on parallel threads, and a fork
/// made by one of them inherits every fd the process holds at that instant,
/// keeping each until the child's own exec (`O_CLOEXEC` acts AT the exec, not
/// before). So an in-process write in one test plus a spawn in another opens a
/// window in which this test's exec is refused: `codex_usage` reported
/// `"spawn"` in CI, on Linux only, because macOS runs a file that is open for
/// writing. Here a short-lived `sh` does the write and is reaped before this
/// returns. This process never holds a writable fd on the inode, so there is
/// nothing for a concurrent fork to inherit.
#[cfg(test)]
pub(crate) fn exec_fixture(path: &Path, body: &str) {
    let mut sh = crate::childenv::command("/bin/sh")
        .args(["-c", r#"cat > "$1" && chmod 0755 "$1""#, "sh"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    sh.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
    assert!(sh.wait().unwrap().success(), "exec fixture {}", path.display());
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

    // ── PrivateDir ───────────────────────────────────────────────────────────

    fn uid(p: &Path) -> u32 {
        fs::symlink_metadata(p).unwrap().uid()
    }

    /// Fresh each time, this user's and 0700, under `base`, and gone, with
    /// everything in it, once dropped. A missing base is made private too.
    #[test]
    fn a_private_dir_is_fresh_ours_0700_and_removed_on_drop() {
        let t = tmp("private");
        let base = t.0.join("cache/worktrees");
        let a = PrivateDir::new(&base, "job").unwrap();
        let b = PrivateDir::new(&base, "job").unwrap();
        assert_ne!(a.path(), b.path(), "a fresh name each time");
        for d in [&a, &b] {
            assert_eq!(d.path().parent(), Some(base.as_path()));
            assert!(fs::symlink_metadata(d.path()).unwrap().is_dir());
            assert_eq!(mode(d.path()), 0o700);
            assert_eq!(uid(d.path()), euid());
        }
        assert_eq!(mode(&base), 0o700);
        let f = a.create_file("install.sh", b"echo hi\n", 0o700).unwrap();
        assert_eq!(f.parent(), Some(a.path()));
        assert_eq!(fs::read(&f).unwrap(), b"echo hi\n");
        assert_eq!(mode(&f), 0o700);
        let (pa, pb) = (a.path().to_path_buf(), b.path().to_path_buf());
        drop(a);
        assert!(!pa.exists(), "dropped with its file");
        assert!(pb.is_dir(), "the other one is not touched");
        drop(b);
        assert!(names(&base).is_empty());
    }

    /// A name already taken — by a link to someone's directory, or by a
    /// directory of their own — is skipped, neither entered nor removed, and
    /// the next name is used. Dropping removes only the one it made.
    #[test]
    fn a_taken_private_dir_name_is_skipped_never_entered_or_removed() {
        let t = tmp("private-taken");
        let base = t.0.join("base");
        fs::create_dir(&base).unwrap();
        let victim = t.0.join("victim");
        fs::create_dir(&victim).unwrap();
        fs::set_permissions(&victim, Permissions::from_mode(0o755)).unwrap();
        let at = |n: u64| base.join(format!("job-{n:016x}"));
        symlink(&victim, at(1)).unwrap();
        fs::create_dir(at(2)).unwrap();
        fs::write(at(2).join("theirs"), "x").unwrap();
        let mut next = 0;
        let d = PrivateDir::new_named(&base, "job", || {
            next += 1;
            next
        })
        .unwrap();
        assert_eq!(d.path(), at(3), "two taken names, the third used");
        d.create_file("f", b"y", 0o600).unwrap();
        drop(d);
        assert!(fs::symlink_metadata(at(1)).unwrap().file_type().is_symlink());
        assert!(names(&victim).is_empty(), "nothing was made through the link");
        assert_eq!(mode(&victim), 0o755, "the mode went through the link");
        assert_eq!(fs::read_to_string(at(2).join("theirs")).unwrap(), "x");
        assert_eq!(names(&base), [format!("job-{:016x}", 1), format!("job-{:016x}", 2)]);
    }

    /// A base that others can write without the sticky bit lets them swap the
    /// directory out after the check, so nothing is made there. Sticky, like
    /// `/tmp`, passes, and so does group-writable (umask 002 makes the user's
    /// own cache 0775).
    #[test]
    fn a_base_others_can_write_is_refused_unless_sticky() {
        let t = tmp("private-base");
        let base = t.0.join("shared");
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, Permissions::from_mode(0o777)).unwrap();
        assert_eq!(PrivateDir::new(&base, "job").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(names(&base).is_empty(), "refused before anything was made");
        for ok in [0o1777, 0o775] {
            fs::set_permissions(&base, Permissions::from_mode(ok)).unwrap();
            let d = PrivateDir::new(&base, "job").unwrap();
            assert_eq!(mode(d.path()), 0o700, "under a {ok:o} base");
        }
    }

    /// Inside, a name is created new: never reused, never followed, and
    /// always a single component.
    #[test]
    fn a_private_file_is_created_new_and_never_followed() {
        let t = tmp("private-file");
        let d = PrivateDir::new(&t.0, "job").unwrap();
        d.create_file("a", b"one", 0o600).unwrap();
        assert_eq!(d.create_file("a", b"two", 0o600).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(d.path().join("a")).unwrap(), b"one");
        let victim = t.0.join("victim");
        fs::write(&victim, "precious").unwrap();
        symlink(&victim, d.path().join("l")).unwrap();
        assert!(d.create_file("l", b"x", 0o600).is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        for bad in ["", ".", "..", "a/b", "/abs"] {
            assert_eq!(d.create_file(bad, b"x", 0o600).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
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
        ("worktrees-api/src/viewer.rs", "\".{name}.{}.{n}.tmp\"", "the app's config dir; a sync per document would cost every derive"),
        ("worktrees-api/src/viewer.rs", "join(std::process::id().to_string())", "a directory in the app's config dir"),
        ("worktrees-cli/src/serve/mod.rs", "[{level}] [{}] {msg}", "a serve.log line carrying the pid; names nothing"),
        ("worktrees-cli/src/serve/tests.rs", "std::process::id()", "test code: serve/mod.rs declares the file under #[cfg(test)], which a line scan cannot see"),
        ("worktrees-cli/src/serve/remote/tests.rs", "std::process::id()", "test code: remote/mod.rs declares the file under #[cfg(test)], which a line scan cannot see"),
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

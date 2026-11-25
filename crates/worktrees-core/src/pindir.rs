//! A folder held open, and what is in it reached from that handle: no name is
//! looked up from the root twice, so nothing can be put in the folder's place
//! between a check and the act it was for, and no link is followed at any
//! component.

use std::ffi::{CStr, CString, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

pub struct Dir(OwnedFd);

/// What one name is, never what a link of that name points at.
#[derive(Debug, Clone, Copy)]
pub struct Stat {
    pub mode: u32,
    pub size: u64,
    pub mtime_ms: u64,
    pub changed_ns: i128,
    pub ino: u64,
    pub links: u64,
}

impl Stat {
    pub fn is_dir(&self) -> bool {
        self.mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32
    }
    pub fn is_file(&self) -> bool {
        self.mode & libc::S_IFMT as u32 == libc::S_IFREG as u32
    }
    /// The permission bits.
    pub fn perm(&self) -> u32 {
        self.mode & 0o7777
    }
}

/// One component: no separator, not `.` or `..`.
fn name_of(name: &[u8]) -> io::Result<CString> {
    if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file name"));
    }
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "not a file name"))
}

const OPEN_DIR: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

fn own(fd: libc::c_int) -> io::Result<OwnedFd> {
    // SAFETY: a descriptor the kernel just gave us, or the error.
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(fd) }) }
}

impl Dir {
    /// The folder at `path`, which is not a link itself. Its parents may be:
    /// what is trusted is where the caller starts.
    pub fn open(path: &Path) -> io::Result<Dir> {
        let p = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "not a path"))?;
        // SAFETY: a NUL-terminated path.
        Ok(Dir(own(unsafe { libc::open(p.as_ptr(), OPEN_DIR) })?))
    }

    pub fn fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    fn at(&self, name: &CStr, flags: libc::c_int, mode: libc::c_uint) -> io::Result<OwnedFd> {
        // SAFETY: a live descriptor and a NUL-terminated name.
        own(unsafe { libc::openat(self.fd(), name.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_CLOEXEC, mode) })
    }

    /// The folder `name` in it.
    pub fn sub(&self, name: &str) -> io::Result<Dir> {
        Ok(Dir(self.at(&name_of(name.as_bytes())?, OPEN_DIR, 0)?))
    }

    /// [`Dir::sub`], made when it is missing.
    pub fn make(&self, name: &str) -> io::Result<Dir> {
        let n = name_of(name.as_bytes())?;
        // SAFETY: a live descriptor and a NUL-terminated name.
        if unsafe { libc::mkdirat(self.fd(), n.as_ptr(), 0o777) } != 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::AlreadyExists {
                return Err(e);
            }
        }
        self.sub(name)
    }

    /// The folder `rel` under it, a component at a time.
    pub fn under(&self, rel: &str) -> io::Result<Dir> {
        rel.split('/').filter(|p| !p.is_empty()).try_fold(Dir(self.0.try_clone()?), |d, p| d.sub(p))
    }

    /// `name` open for reading, whatever it is: the caller asks the handle.
    pub fn read(&self, name: &str) -> io::Result<File> {
        Ok(File::from(self.at(&name_of(name.as_bytes())?, libc::O_RDONLY | libc::O_NONBLOCK, 0)?))
    }

    /// A NEW file `name`: an existing path of any kind is refused.
    pub fn create(&self, name: &str, mode: u32) -> io::Result<File> {
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL;
        Ok(File::from(self.at(&name_of(name.as_bytes())?, flags, mode as libc::c_uint)?))
    }

    pub fn rename(&self, from: &str, to_dir: &Dir, to: &str) -> io::Result<()> {
        let (a, b) = (name_of(from.as_bytes())?, name_of(to.as_bytes())?);
        // SAFETY: live descriptors and NUL-terminated names.
        if unsafe { libc::renameat(self.fd(), a.as_ptr(), to_dir.fd(), b.as_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Remove the file or the link `name`.
    pub fn unlink(&self, name: &str) -> io::Result<()> {
        let n = name_of(name.as_bytes())?;
        // SAFETY: a live descriptor and a NUL-terminated name.
        if unsafe { libc::unlinkat(self.fd(), n.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn stat(&self, name: &str) -> io::Result<Stat> {
        self.stat_raw(name.as_bytes())
    }

    fn stat_raw(&self, name: &[u8]) -> io::Result<Stat> {
        let n = name_of(name)?;
        let mut s = std::mem::MaybeUninit::<libc::stat>::zeroed();
        // SAFETY: a live descriptor, a NUL-terminated name, room for one stat.
        if unsafe { libc::fstatat(self.fd(), n.as_ptr(), s.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: filled in by the call that just succeeded.
        Ok(stat_of(&unsafe { s.assume_init() }))
    }

    /// Every name in it, sorted, `.` and `..` left out.
    pub fn names(&self) -> io::Result<Vec<OsString>> {
        // A description of its own: one shared would share its place in it.
        let fd = self.at(c".", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        // SAFETY: the descriptor is handed to the stream, which closes it.
        let d = unsafe { libc::fdopendir(std::os::fd::IntoRawFd::into_raw_fd(fd)) };
        if d.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut out = Vec::new();
        let done = loop {
            // SAFETY: a live stream, read by this thread alone; errno is
            // cleared because a NULL means the end or an error.
            let e = unsafe {
                *errno() = 0;
                libc::readdir(d)
            };
            if e.is_null() {
                let err = io::Error::last_os_error();
                break if err.raw_os_error() == Some(0) { Ok(()) } else { Err(err) };
            }
            // SAFETY: the entry's name is NUL-terminated and lives until the
            // next read.
            let name = unsafe { CStr::from_ptr((*e).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." {
                out.push(OsString::from_vec(name.to_vec()));
            }
        };
        // SAFETY: ours, and not used again.
        unsafe { libc::closedir(d) };
        done.map(|()| {
            out.sort();
            out
        })
    }
}

#[cfg(target_os = "macos")]
unsafe fn errno() -> *mut libc::c_int {
    unsafe { libc::__error() }
}
#[cfg(not(target_os = "macos"))]
unsafe fn errno() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}

// The fields' widths differ by platform.
#[allow(clippy::unnecessary_cast)]
fn stat_of(s: &libc::stat) -> Stat {
    Stat {
        mode: s.st_mode as u32,
        size: s.st_size as u64,
        mtime_ms: (s.st_mtime as i128 * 1000 + s.st_mtime_nsec as i128 / 1_000_000).max(0) as u64,
        changed_ns: s.st_ctime as i128 * 1_000_000_000 + s.st_ctime_nsec as i128,
        ino: s.st_ino as u64,
        links: s.st_nlink as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;
    use std::os::unix::fs::{symlink, MetadataExt};

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wt-pindir-{tag}-{}", crate::ulid::new()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_folder_replaced_after_it_was_opened_is_still_the_one_that_was_opened() {
        let t = tmp("pin");
        fs::create_dir_all(t.join("mem/notes")).unwrap();
        fs::write(t.join("mem/notes/a.md"), "ours").unwrap();
        let d = Dir::open(&t.join("mem")).unwrap();
        fs::rename(t.join("mem"), t.join("was")).unwrap();
        fs::create_dir_all(t.join("out/notes")).unwrap();
        fs::write(t.join("out/notes/a.md"), "theirs").unwrap();
        symlink(t.join("out"), t.join("mem")).unwrap();
        assert!(Dir::open(&t.join("mem")).is_err(), "a link is no folder");
        let mut s = String::new();
        d.under("notes").unwrap().read("a.md").unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "ours");
        d.make("new").unwrap().create("x", 0o644).unwrap();
        assert!(t.join("was/new/x").is_file() && !t.join("out/new").exists());
        let _ = fs::remove_dir_all(&t);
    }

    #[test]
    fn no_link_is_followed_and_no_name_leaves_the_folder() {
        let t = tmp("link");
        fs::create_dir_all(t.join("mem/real")).unwrap();
        fs::write(t.join("secret"), "s").unwrap();
        symlink(&t, t.join("mem/away")).unwrap();
        symlink(t.join("secret"), t.join("mem/l")).unwrap();
        let d = Dir::open(&t.join("mem")).unwrap();
        assert!(d.sub("away").is_err() && d.under("away").is_err() && d.make("away").is_err() && d.read("l").is_err());
        assert!(d.create("l", 0o644).is_err(), "nor replaced");
        for n in ["", ".", "..", "a/b", "/etc", "a\0b"] {
            assert!(d.sub(n).is_err() && d.read(n).is_err() && d.create(n, 0o644).is_err() && d.stat(n).is_err() && d.make(n).is_err(), "{n:?}");
            assert!(d.unlink(n).is_err() && d.rename("real", &d, n).is_err() && d.rename(n, &d, "x").is_err(), "{n:?}");
        }
        assert!(d.under("real/..").is_err());
        assert_eq!(fs::read_to_string(t.join("secret")).unwrap(), "s");
        let _ = fs::remove_dir_all(&t);
    }

    #[test]
    fn names_and_stats_are_the_folders_own() {
        let t = tmp("names");
        for n in ["b", "a", ".hidden", "ünï"] {
            fs::write(t.join(n), n).unwrap();
        }
        symlink("/etc/hosts", t.join("l")).unwrap();
        fs::create_dir(t.join("d")).unwrap();
        let d = Dir::open(&t).unwrap();
        for _ in 0..2 {
            assert_eq!(d.names().unwrap(), [".hidden", "a", "b", "d", "l", "ünï"].map(OsString::from), "and again from the start");
        }
        let md = fs::metadata(t.join("ünï")).unwrap();
        let s = d.stat("ünï").unwrap();
        let ms = md.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        assert_eq!((s.size, s.mtime_ms, s.ino, s.links, s.perm()), (md.len(), ms, md.ino(), 1, md.mode() & 0o7777));
        assert_eq!(s.changed_ns, i128::from(md.ctime()) * 1_000_000_000 + i128::from(md.ctime_nsec()));
        assert!(s.is_file() && d.stat("d").unwrap().is_dir());
        assert!(!d.stat("l").unwrap().is_file() && !d.stat("l").unwrap().is_dir());
        d.unlink("l").unwrap();
        assert!(Path::new("/etc/hosts").exists() && d.stat("l").is_err());
        let _ = fs::remove_dir_all(&t);
    }
}

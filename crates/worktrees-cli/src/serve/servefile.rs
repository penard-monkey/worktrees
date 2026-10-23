//! `<config_dir>/serve.json` — how a client finds this server.
//!
//! `{port, token, pid, version, owner?}`, mode `0600`, rewritten on every launch
//! with a fresh token. `owner` is present only for a serve started with
//! `--owner app`: the desktop app manages that one's life, and an app that
//! finds it after a crash adopts it (stops it on quit) rather than leaving it
//! running unseen. A file without it — the CLI's, the dev stack's, or one from
//! before the field existed — parses with `None`. The desktop app reads it to connect; so does the dev stack.
//! The token is a capability for everything the engine can do on this
//! machine, which is why the file's mode is the first thing the tests check
//! and why it is written under the same discipline `registry.rs` uses: an
//! `flock` on a sidecar, a temp file created with the final mode (so there is
//! no instant in which a world-readable version exists), then a rename.
//!
//! One engine per host is a design rule (`worktrees_api`'s crate doc), so a
//! second `serve` on the same config dir is refused while the first is LIVE —
//! its pid answers `kill(pid, 0)` AND its port accepts a connection. Either
//! check alone is wrong: a pid can be reused by an unrelated process after a
//! crash, and a port can be held by something that is not us. Both together
//! name a running engine; anything less is a stale file and is overwritten.

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "serve.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ServeFile {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub version: String,
    /// Who manages this serve's life (`--owner`); absent means nobody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE)
}

/// The file as it stands, or `None` for absent or unparsable — a broken file
/// is treated as no file, because the only thing to do with it is overwrite it.
pub fn read(config_dir: &Path) -> Option<ServeFile> {
    serde_json::from_slice(&fs::read(path(config_dir)).ok()?).ok()
}

/// A 256-bit token from the OS CSPRNG, as 64 hex characters. Rotated every
/// launch: a token that escaped into a log or a chat window stops working at
/// the next restart.
pub fn new_token() -> String {
    hex32()
}

/// The server's challenge nonce `sn` in handshake v1: the same 32 bytes from
/// the same OS CSPRNG as the token, fresh per connection. A predictable `sn`
/// would let a recorded client proof be replayed.
pub fn nonce() -> String {
    hex32()
}

/// 32 bytes from the OS CSPRNG as 64 lowercase hex characters.
fn hex32() -> String {
    let mut b = [0u8; 32];
    // The OS refusing entropy is not a state this process can serve in. The
    // fallback is deliberately NOT a clock and a pid — a token anyone can
    // reconstruct is not a token.
    getrandom::fill(&mut b).expect("the OS refused entropy");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Run `f` with the sidecar lock held. Opened per call: separate `open()`s are
/// separate open file descriptions, which DO contend under `flock` even inside
/// one process — the same reasoning as `registry::edit_at`.
fn locked<T>(config_dir: &Path, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    with_lock(config_dir, FILE, f)
}

/// [`locked`] for any file of the config dir: `<config_dir>/<file>.lock`,
/// created `0600`. `devices.json` is written under the same discipline.
pub(super) fn with_lock<T>(config_dir: &Path, file: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    fs::create_dir_all(config_dir).map_err(|e| format!("{}: {e}", config_dir.display()))?;
    let lock_path = config_dir.join(format!("{file}.lock"));
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    // SAFETY: flock gets a live descriptor; `lock` outlives the critical
    // section and closing it releases the lock on every exit path.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!("could not lock {}", lock_path.display()));
    }
    let out = f();
    drop(lock);
    out
}

/// Write the file: lock, temp created `0600`, rename — `fsx::write_atomic`,
/// whose temp gets a fresh random name it never opens through a link. The
/// mode is set at CREATION, not after: a `create` + `set_permissions` pair
/// leaves a window in which the token sits in a file the umask decided the
/// mode of.
pub fn write(config_dir: &Path, sf: &ServeFile) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let json = serde_json::to_string_pretty(sf).map_err(|e| e.to_string())?;
    locked(config_dir, || {
        let dst = path(config_dir);
        worktrees_core::fsx::write_atomic(&dst, json.as_bytes(), Some(fs::Permissions::from_mode(0o600)))
            .map_err(|e| format!("{}: {e}", dst.display()))
    })
}

/// Remove the file if it still names `pid` — ours — and leave it alone if a
/// successor has already replaced it. Under the lock, so a successor's write
/// and our removal cannot interleave into "the new server's file is gone".
pub fn remove_if_ours(config_dir: &Path, pid: u32) -> Result<bool, String> {
    locked(config_dir, || {
        if read(config_dir).is_some_and(|sf| sf.pid == pid) {
            fs::remove_file(path(config_dir)).map_err(|e| e.to_string())?;
            return Ok(true);
        }
        Ok(false)
    })
}

/// Is the server the file names still running? Both halves, for the reasons
/// in the module doc. The connect is a plain TCP open and close — it says
/// nothing on the wire, so the server it reaches refuses it silently, which is
/// fine: reaching it was the question.
pub fn live_other(config_dir: &Path, my_pid: u32) -> Option<ServeFile> {
    let sf = read(config_dir)?;
    if sf.pid == my_pid || !pid_alive(sf.pid) {
        return None;
    }
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, sf.port));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500))
        .ok()
        .map(|_| sf)
}

/// `kill(pid, 0)`: `0` means it exists; `EPERM` means it exists and is not
/// ours to signal, which for this question is still "exists".
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // A pid that does not fit `pid_t` cannot be addressed at all, so it is
    // dead without asking — and `pid as pid_t` would have turned it negative,
    // which `kill(2)` reads as a process GROUP: a different question.
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 delivers nothing; it only asks the kernel whether the
    // pid is addressable.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-servefile-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn sf(token: &str) -> ServeFile {
        ServeFile { port: 1, token: token.into(), pid: std::process::id(), version: "t".into(), owner: None }
    }

    /// The file holds a capability for the whole engine, so its mode is the
    /// property, and it is asserted on the FILE, not on the request — a
    /// `create` + `chmod` pair would also end at 0600 and would have been
    /// 0644 for an instant. Measured on macOS 27.0 (Darwin 27, arm64) and on
    /// Linux 6.12 in an Alpine 3.24 (musl) and a Debian 12 (glibc) container,
    /// each under umask 022, 077 and 000: `mode & 0o777` reads 0600 on all
    /// six — open(2) can only CLEAR bits with the umask, and 0600 has none
    /// in group or other to clear. CI's ubuntu runner is not measured
    /// separately; it runs this test.
    #[test]
    fn serve_json_is_written_0600_and_rewritten_with_a_new_token_each_time() {
        let d = tmp("mode");
        let a = new_token();
        write(&d, &sf(&a)).unwrap();
        let mode = fs::metadata(path(&d)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "serve.json mode is {mode:o}");
        assert_eq!(read(&d).unwrap().token, a);

        // A second launch: the token changes and the mode holds.
        let b = new_token();
        assert_ne!(a, b, "two launches drew the same 256-bit token");
        write(&d, &sf(&b)).unwrap();
        assert_eq!(read(&d).unwrap().token, b);
        assert_eq!(fs::metadata(path(&d)).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(a.len(), 64, "256 bits as hex");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_dead_pid_is_a_stale_file_and_ours_is_removable() {
        let d = tmp("stale");
        // A pid no process has, inside `pid_t`: `kill(2_000_000_000, 0)` is
        // ESRCH on macOS 27.0 (xnu's PID_MAX is 99998) and on Linux 6.12
        // (`/proc/sys/kernel/pid_max` was 99999 in the containers measured;
        // the kernel caps it at 4194304 on 64-bit).
        let dead = ServeFile { port: 1, token: "x".into(), pid: 2_000_000_000, version: "t".into(), owner: None };
        write(&d, &dead).unwrap();
        assert!(live_other(&d, std::process::id()).is_none(), "a dead pid must read as stale");
        // And one OUTSIDE `pid_t`, which a corrupt file can carry: dead
        // without asking the kernel, never cast negative into a pgid probe.
        let wide = ServeFile { port: 1, token: "x".into(), pid: 4_000_000_000, version: "t".into(), owner: None };
        write(&d, &wide).unwrap();
        assert!(live_other(&d, std::process::id()).is_none(), "a pid outside pid_t must read as stale");
        write(&d, &dead).unwrap();
        assert!(!remove_if_ours(&d, std::process::id()).unwrap(), "not ours: left alone");
        assert!(path(&d).exists());

        write(&d, &sf("y")).unwrap();
        assert!(live_other(&d, std::process::id()).is_none(), "our own pid is never 'another' server");
        assert!(remove_if_ours(&d, std::process::id()).unwrap());
        assert!(!path(&d).exists());
        let _ = fs::remove_dir_all(&d);
    }

    /// Our own pid is alive but nothing listens on the port: the test's own
    /// process stands in for a server that crashed after writing the file, or
    /// a pid that was reused. Needs a port nothing holds — bind one and drop it.
    #[test]
    fn a_live_pid_with_no_listener_is_stale_too() {
        let d = tmp("noport");
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        // A different live pid than ours: the parent process is alive for the
        // life of this test on both platforms.
        let parent = unsafe { libc::getppid() } as u32;
        write(&d, &ServeFile { port, token: "x".into(), pid: parent, version: "t".into(), owner: None }).unwrap();
        assert!(live_other(&d, std::process::id()).is_none());
        // …and with something listening, the same file names a live server.
        let l = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        assert_eq!(live_other(&d, std::process::id()).map(|s| s.port), Some(port));
        drop(l);
        let _ = fs::remove_dir_all(&d);
    }

    /// `owner` is optional both ways: a file written before the field existed
    /// (or by a serve nobody owns) parses with `None` and is written back
    /// without the key, and an owned one round-trips.
    #[test]
    fn the_owner_field_is_optional_and_round_trips() {
        let d = tmp("owner");
        fs::write(path(&d), r#"{"port": 7, "token": "t", "pid": 9, "version": "0.1.0"}"#).unwrap();
        assert_eq!(read(&d).unwrap().owner, None, "an old file still parses");
        let mut sf = read(&d).unwrap();
        write(&d, &sf).unwrap();
        assert!(!fs::read_to_string(path(&d)).unwrap().contains("owner"), "no owner, no key");
        sf.owner = Some("app".into());
        write(&d, &sf).unwrap();
        assert_eq!(read(&d).unwrap().owner.as_deref(), Some("app"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_broken_file_reads_as_absent() {
        let d = tmp("broken");
        fs::write(path(&d), "{not json").unwrap();
        assert!(read(&d).is_none());
        assert!(live_other(&d, 1).is_none());
        let _ = fs::remove_dir_all(&d);
    }
}

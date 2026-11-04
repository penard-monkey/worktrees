//! What the kernel knows about a running process: its executable, when it
//! started, and the files it holds open. Read-only, no spawn, no signal.
//!
//! The agent restart (`docs/proposals/harness-restart.md`) needs these for
//! Codex, which records no running version anywhere a reader can see: a
//! resumed Codex APPENDS to the thread's rollout, whose `session_meta` keeps
//! the version that started the thread (measured, 0.159.2 → 0.160.1). So a
//! Codex session's age is its process's, and its conversation is the rollout
//! it holds open.
//!
//! Every function answers `None`/empty for a pid that is gone, not ours, or
//! on a platform without the call; callers treat that as "cannot tell".

use std::path::PathBuf;

/// The executable `pid` runs, as the kernel names it. On Linux a replaced
/// file reads `<path> (deleted)`; that suffix is kept, so it never equals the
/// installed path.
#[cfg(target_os = "macos")]
pub fn exe(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    if pid <= 0 {
        return None;
    }
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is as large as the size passed.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(PathBuf::from(std::ffi::OsString::from_vec(buf)))
}

#[cfg(target_os = "linux")]
pub fn exe(pid: i32) -> Option<PathBuf> {
    (pid > 0).then(|| std::fs::read_link(format!("/proc/{pid}/exe")).ok()).flatten()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn exe(_pid: i32) -> Option<PathBuf> {
    None
}

/// When `pid` started, in epoch seconds.
#[cfg(target_os = "macos")]
pub fn start_epoch(pid: i32) -> Option<i64> {
    if pid <= 0 {
        return None;
    }
    // SAFETY: a zeroed plain-data struct, filled by the kernel up to its size.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (n == size).then_some(info.pbi_start_tvsec as i64)
}

#[cfg(target_os = "linux")]
pub fn start_epoch(pid: i32) -> Option<i64> {
    if pid <= 0 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let boot = std::fs::read_to_string("/proc/stat").ok()?;
    // SAFETY: sysconf has no preconditions.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    linux_start(&stat, &boot, ticks as i64)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn start_epoch(_pid: i32) -> Option<i64> {
    None
}

/// `/proc/<pid>/stat`'s `starttime` (field 22, clock ticks since boot) plus
/// `/proc/stat`'s `btime`. The command name (field 2) is parenthesised and
/// may hold spaces or `)`, so fields are counted after its LAST `)`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_start(stat: &str, proc_stat: &str, ticks: i64) -> Option<i64> {
    if ticks <= 0 {
        return None;
    }
    let rest = &stat[stat.rfind(')')? + 1..];
    // After the name: field 3 is the state, so starttime (22) is the 20th.
    let start: i64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    let btime: i64 = proc_stat.lines().find_map(|l| l.strip_prefix("btime "))?.trim().parse().ok()?;
    Some(btime + start / ticks)
}

/// `pid`'s parent.
#[cfg(target_os = "macos")]
pub fn parent(pid: i32) -> Option<i32> {
    if pid <= 0 {
        return None;
    }
    // SAFETY: as `start_epoch`.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (n == size).then_some(info.pbi_ppid as i32)
}

#[cfg(target_os = "linux")]
pub fn parent(pid: i32) -> Option<i32> {
    if pid <= 0 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat[stat.rfind(')')? + 1..].split_whitespace().nth(1)?.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn parent(_pid: i32) -> Option<i32> {
    None
}

/// `pid` and every ancestor up to init, nearest first (bounded, so a cycle
/// read mid-reparent cannot spin).
pub fn lineage(pid: i32) -> Vec<i32> {
    let mut out = vec![pid];
    while let Some(p) = out.last().copied().and_then(parent).filter(|p| *p > 1 && !out.contains(p)) {
        out.push(p);
        if out.len() >= 64 {
            break;
        }
    }
    out
}

/// The paths of the regular files and directories `pid` holds open.
#[cfg(target_os = "macos")]
pub fn open_paths(pid: i32) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    if pid <= 0 {
        return Vec::new();
    }
    #[repr(C)]
    struct ProcFileInfo {
        fi_openflags: u32,
        fi_status: u32,
        fi_offset: libc::off_t,
        fi_type: i32,
        fi_guardflags: u32,
    }
    #[repr(C)]
    struct VnodeFdInfoWithPath {
        pfi: ProcFileInfo,
        pvip: libc::vnode_info_path,
    }
    const PROC_PIDFDVNODEPATHINFO: libc::c_int = 2;
    let fd_size = std::mem::size_of::<libc::proc_fdinfo>();
    // SAFETY: a null buffer asks for the size needed.
    let need = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if need <= 0 {
        return Vec::new();
    }
    // Room for a few fds opened between the two calls.
    let mut fds: Vec<libc::proc_fdinfo> = (0..need as usize / fd_size + 16)
        .map(|_| libc::proc_fdinfo { proc_fd: 0, proc_fdtype: 0 })
        .collect();
    // SAFETY: the buffer holds `fds.len()` entries, the size passed.
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast(), (fds.len() * fd_size) as libc::c_int) };
    if got <= 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for fd in &fds[..(got as usize / fd_size).min(fds.len())] {
        if fd.proc_fdtype != libc::PROX_FDTYPE_VNODE as u32 {
            continue;
        }
        // SAFETY: zeroed plain data, filled by the kernel up to its size.
        let mut vi: VnodeFdInfoWithPath = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<VnodeFdInfoWithPath>() as libc::c_int;
        let n = unsafe { libc::proc_pidfdinfo(pid, fd.proc_fd, PROC_PIDFDVNODEPATHINFO, (&mut vi as *mut VnodeFdInfoWithPath).cast(), size) };
        if n != size {
            continue;
        }
        // SAFETY: `vip_path` is MAXPATHLEN bytes of one NUL-terminated string.
        let raw: &[u8] = unsafe { std::slice::from_raw_parts(vi.pvip.vip_path.as_ptr().cast::<u8>(), 32 * 32) };
        let len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        if len > 0 {
            out.push(PathBuf::from(std::ffi::OsStr::from_bytes(&raw[..len])));
        }
    }
    out
}

#[cfg(target_os = "linux")]
pub fn open_paths(pid: i32) -> Vec<PathBuf> {
    if pid <= 0 {
        return Vec::new();
    }
    let Ok(dir) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { return Vec::new() };
    dir.flatten().filter_map(|e| std::fs::read_link(e.path()).ok()).filter(|p| p.is_absolute()).collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn open_paths(_pid: i32) -> Vec<PathBuf> {
    Vec::new()
}

/// A file's inode-change time, in epoch seconds: when it was last created,
/// replaced or re-linked. The install time of an executable — npm writes
/// every extracted file with an mtime of 1985-10-26, so mtime is not.
pub fn ctime(path: &std::path::Path) -> Option<i64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.ctime())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_reads_back_its_own_facts() {
        let me = std::process::id() as i32;
        let exe_now = std::env::current_exe().unwrap();
        let got = exe(me).expect("own executable");
        assert_eq!(std::fs::canonicalize(&got).unwrap(), std::fs::canonicalize(&exe_now).unwrap());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let start = start_epoch(me).expect("own start time");
        assert!(start <= now && now - start < 24 * 3600, "start {start} vs now {now}");
        let dir = std::env::temp_dir().join(format!("wt-procinfo-{me}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("held.jsonl");
        let held = std::fs::File::create(&path).unwrap();
        let canon = std::fs::canonicalize(&path).unwrap();
        let open: Vec<PathBuf> = open_paths(me).into_iter().map(|p| std::fs::canonicalize(&p).unwrap_or(p)).collect();
        assert!(open.contains(&canon), "{canon:?} not in {open:?}");
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
        let line = lineage(me);
        assert_eq!(line[0], me);
        assert_eq!(line.get(1).copied(), parent(me).filter(|p| *p > 1), "the parent comes next");
        assert_eq!(exe(-1), None);
        assert_eq!(start_epoch(0), None);
        assert!(open_paths(-5).is_empty());
    }

    #[test]
    fn linux_start_counts_fields_after_the_name() {
        // A name with a space and a `)`, as a process may set it.
        let stat = "4242 (my) proc) S 1 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 25000 1000 10";
        let proc_stat = "cpu  1 2 3\nbtime 1762200000\nprocesses 9\n";
        assert_eq!(linux_start(stat, proc_stat, 100), Some(1762200000 + 250));
        assert_eq!(linux_start(stat, "cpu 1\n", 100), None);
        assert_eq!(linux_start(stat, proc_stat, 0), None);
    }
}

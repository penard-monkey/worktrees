//! The one lock a workspace's engine state is changed under, in this process
//! and in any other engine on this data: `<workspaces root>/.engine/<workspace
//! id>.lock`, an flock.
//!
//! **Beside the engine's folder of that workspace, never in it.** That folder
//! moves when the workspace is removed, and a lock that moved with it would
//! let a waiter win one nobody else looks at. Never removed, either.
//!
//! **Order, outermost first:** a workspace's `life_lock`, a chat's
//! `place_lock`, this, the chat record's own flock. Nobody takes an earlier
//! one while holding a later one, and this one is held for check, record and
//! git: never across a launch or a wait for an agent.
//!
//! **Re-entrant on one thread**, so whoever checks something under it can call
//! what takes it itself. A [`Held`] stays on the thread that took it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::marker::PhantomData;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::{fsx, wsfolder};

thread_local! {
    /// What this thread holds, by lock file, and how many times over.
    static MINE: RefCell<HashMap<PathBuf, (fs::File, usize)>> = RefCell::new(HashMap::new());
}

/// The lock, until it is dropped.
#[must_use]
pub struct Held {
    path: PathBuf,
    _thread: PhantomData<Rc<()>>,
}

impl Drop for Held {
    fn drop(&mut self) {
        MINE.with_borrow_mut(|m| {
            let Some((file, n)) = m.get_mut(&self.path) else { return };
            *n -= 1;
            if *n == 0 {
                // Said, and not left to the close: a child another thread has
                // forked and not yet exec'd still has the descriptor.
                // SAFETY: a live descriptor of ours.
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
                m.remove(&self.path);
            }
        });
    }
}

/// Take workspace `id`'s lock, waiting for whoever has it.
pub fn take(root: &Path, id: &str) -> Result<Held, String> {
    lock(root, id, libc::LOCK_EX)?.ok_or_else(|| format!("could not lock workspace {id}"))
}

/// Take it, or `None` when someone else has it.
pub fn try_take(root: &Path, id: &str) -> Result<Option<Held>, String> {
    lock(root, id, libc::LOCK_EX | libc::LOCK_NB)
}

fn lock(root: &Path, id: &str, how: libc::c_int) -> Result<Option<Held>, String> {
    let path = path_at(root, id)?;
    let held = |path| Ok(Some(Held { path, _thread: PhantomData }));
    let again = MINE.with_borrow_mut(|m| m.get_mut(&path).map(|(_, n)| *n += 1).is_some());
    if again {
        return held(path);
    }
    wsfolder::refuse_real(root)?;
    let said = |e: std::io::Error| format!("{}: {e}", path.display());
    let dir = path.parent().expect("under .engine");
    fs::create_dir_all(root).map_err(said)?;
    fsx::make_dir(dir).map_err(said)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .map_err(said)?;
    if !file.metadata().map_err(said)?.is_file() {
        return Err(format!("{}: not a regular file", path.display()));
    }
    loop {
        // SAFETY: a live descriptor, kept in MINE until it is let go.
        if unsafe { libc::flock(file.as_raw_fd(), how) } == 0 {
            break;
        }
        let e = std::io::Error::last_os_error();
        match e.kind() {
            std::io::ErrorKind::Interrupted => continue,
            std::io::ErrorKind::WouldBlock => return Ok(None),
            _ => return Err(said(e)),
        }
    }
    MINE.with_borrow_mut(|m| m.insert(path.clone(), (file, 1)));
    held(path)
}

/// By the root as it resolves, so that one folder under two spellings is one
/// lock to this thread too.
fn path_at(root: &Path, id: &str) -> Result<PathBuf, String> {
    wsfolder::folder_at(root, id)?;
    Ok(wsfolder::resolved(root).join(wsfolder::ENGINE_DIR).join(format!("{id}.lock")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::sync::mpsc;
    use std::time::Duration;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-wslock-{tag}-{}", crate::ulid::new()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Whether another thread, which is another open file as another process
    /// is, can have it now.
    fn free(root: &Path, id: &str) -> bool {
        let (root, id) = (root.to_path_buf(), id.to_string());
        std::thread::spawn(move || try_take(&root, &id).unwrap().is_some()).join().unwrap()
    }

    #[test]
    fn one_holder_at_a_time_and_again_on_its_own_thread() {
        let top = tmp("one");
        let root = top.join("ws");
        let (a, b) = (crate::ulid::new(), crate::ulid::new());
        let one = take(&root, &a).unwrap();
        assert!(!free(&root, &a));
        assert!(free(&root, &b), "another workspace's is another lock");
        let two = take(&root, &a).unwrap();
        let three = try_take(&root, &a).unwrap().unwrap();
        // By another spelling of the same folder too.
        symlink(&root, top.join("link")).unwrap();
        let four = take(&top.join("link"), &a).unwrap();
        // In any order: it is held until the last is let go.
        drop(one);
        drop(three);
        drop(four);
        assert!(!free(&root, &a));
        drop(two);
        assert!(free(&root, &a));
        assert!(root.join(".engine").join(format!("{a}.lock")).is_file(), "and it is never removed");
        let _ = fs::remove_dir_all(&top);
    }

    #[test]
    fn a_waiter_gets_it_when_it_is_let_go_and_not_before() {
        let root = tmp("wait");
        let id = crate::ulid::new();
        let held = take(&root, &id).unwrap();
        let (tx, rx) = mpsc::channel();
        let (r, i) = (root.clone(), id.clone());
        let t = std::thread::spawn(move || {
            let h = take(&r, &i).map(|_| ());
            tx.send(h).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        drop(held);
        rx.recv_timeout(Duration::from_secs(20)).expect("it never got it").unwrap();
        t.join().unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    /// A child another thread has started keeps no lock: its descriptor closes
    /// at the exec, and the holder says it lets go rather than closing.
    #[test]
    fn letting_go_is_said_whatever_else_has_the_descriptor() {
        let root = tmp("dup");
        let id = crate::ulid::new();
        let held = take(&root, &id).unwrap();
        let dup = MINE.with_borrow(|m| m[&held.path].0.try_clone().unwrap());
        drop(held);
        assert!(free(&root, &id));
        drop(dup);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn it_is_never_opened_through_a_link_and_a_failure_holds_nothing() {
        let top = tmp("link");
        let (root, id) = (top.join("ws"), crate::ulid::new());
        fs::create_dir_all(root.join(".engine")).unwrap();
        let out = top.join("out");
        symlink(&out, root.join(".engine").join(format!("{id}.lock"))).unwrap();
        assert!(take(&root, &id).is_err() && take(&root, &id).is_err(), "not even the second time");
        assert!(!out.exists());
        let dir = crate::ulid::new();
        fs::create_dir(root.join(".engine").join(format!("{dir}.lock"))).unwrap();
        assert!(take(&root, &dir).is_err());
        let away = top.join("away");
        fs::create_dir_all(top.join("ws2")).unwrap();
        fs::create_dir_all(&away).unwrap();
        symlink(&away, top.join("ws2/.engine")).unwrap();
        assert!(take(&top.join("ws2"), &id).is_err());
        assert_eq!(fs::read_dir(&away).unwrap().count(), 0);
        assert!(take(&root, "../x").is_err());
        let _ = fs::remove_dir_all(&top);
    }
}

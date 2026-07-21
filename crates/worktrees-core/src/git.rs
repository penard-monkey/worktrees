//! Thin typed wrappers over the `git` CLI (always `git -C <cwd>`), mirroring the
//! bash invocations 1:1. Shelling out (not gix/git2) is deliberate: it preserves
//! git's own worktree-add DWIM and the stale-dir upward-resolution trap exactly,
//! and keeps the bats fake-git PATH shim intercepting the compiled binary.

use std::process::{Command, Output};

pub fn git(cwd: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git").arg("-C").arg(cwd).args(args).output()
}

/// Trimmed stdout on success, else `None` (stderr silenced, like `2>/dev/null`).
pub fn git_out(cwd: &str, args: &[&str]) -> Option<String> {
    let o = git(cwd, args).ok()?;
    if o.status.success() {
        Some(String::from_utf8_lossy(&o.stdout).trim_end_matches('\n').to_string())
    } else {
        None
    }
}

/// Did the command exit 0? (For `show-ref --verify -q`, guards, etc.)
pub fn git_ok(cwd: &str, args: &[&str]) -> bool {
    git(cwd, args).map(|o| o.status.success()).unwrap_or(false)
}

pub fn have_git() -> bool {
    Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

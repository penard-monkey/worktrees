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

/// Run `git -C cwd <args>` inheriting stdio (git's own messages reach the
/// terminal, like the bash CLI's un-redirected mutating commands). Returns
/// whether it exited 0.
pub fn git_status(cwd: &str, args: &[&str]) -> bool {
    Command::new("git").arg("-C").arg(cwd).args(args).status().map(|s| s.success()).unwrap_or(false)
}

/// CAPTURED variant of `git_status` for callers that must surface git's own
/// failure reason (the app's inherited stderr goes to the launchd void).
/// Behaviour is a superset of `git_status`:
///  - SUCCESS: git's stdout/stderr are written straight through to this
///    process's stdout/stderr as RAW BYTES — no reformatting, no added
///    newlines — so the CLI is byte-identical to the inherited-stdio path and
///    bats stays green. Returns `Ok(())`.
///  - FAILURE: nothing is passed through; returns `Err(stderr)` (trimmed; a
///    spawn error or empty stderr yields a short synthetic reason) so the
///    caller can route git's real message through `ui.error`.
pub fn git_status_captured(cwd: &str, args: &[&str]) -> std::result::Result<(), String> {
    use std::io::Write;
    let out = match Command::new("git").arg("-C").arg(cwd).args(args).output() {
        Ok(o) => o,
        Err(e) => return Err(e.to_string()),
    };
    if out.status.success() {
        // Passthrough as raw bytes to preserve the exact terminal output.
        let _ = std::io::stdout().write_all(&out.stdout);
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().write_all(&out.stderr);
        let _ = std::io::stderr().flush();
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("git exited {}", out.status.code().unwrap_or(-1))
        } else {
            err
        })
    }
}

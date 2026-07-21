//! Thin wrappers over the `tmux` CLI. Subprocess (there's no native lib), which
//! also keeps the bats fake-tmux PATH shim intercepting the compiled binary.

use std::process::{Command, Output};

pub fn have_tmux() -> bool {
    Command::new("tmux").arg("-V").output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn tmux(args: &[&str]) -> std::io::Result<Output> {
    Command::new("tmux").args(args).output()
}

/// Does a session named EXACTLY `name` exist? (`list-sessions` + exact match, not
/// `has-session -t` which prefix-matches — so `rm api` can't hit `api-fix`.)
pub fn session_exists(name: &str) -> bool {
    match tmux(&["list-sessions", "-F", "#{session_name}"]) {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).lines().any(|l| l == name)
        }
        _ => false,
    }
}

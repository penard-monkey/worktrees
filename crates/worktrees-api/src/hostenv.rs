//! The process environment a GUI or launchd-started program does not inherit.
//!
//! Not window-local, despite the names: these fix the PROCESS's `PATH` and
//! locale, and `worktrees serve` started by launchd needs both for exactly
//! the reason the app does — no homebrew on the inherited `PATH` means no
//! tmux, and every place this program manages lives in one.

use std::path::Path;

use worktrees_core::proc::run_deadline;

use crate::log::log as applog;

/// GUI-launched apps inherit launchd's bare PATH (/usr/bin:/bin:…) — no
/// homebrew, no ~/.local/bin — so the engine's tmux/git shell-outs fail even
/// though they work in every terminal (tmux is homebrew-installed: every place
/// looks dead and Enter errors). Resolve the user's real PATH from their login
/// shell once at startup (marker-wrapped so chatty profiles can't corrupt it;
/// deadline-guarded so a hung profile can't block launch).
///
/// The usual install dirs are ALWAYS appended, not just when the probe fails:
/// a login shell whose profile never runs `brew shellenv` reports a PATH with no
/// /opt/homebrew/bin in it, and a brew-installed tmux would stay invisible even
/// though the probe "worked". Order is shell PATH → standard dirs → the original
/// PATH as a safety net, so the user's own resolution still wins; duplicate
/// entries are harmless.
///
/// Re-entrant on purpose: `tmux_check(refresh = true)` calls it again to pick up
/// a tmux installed after launch.
pub fn fixup_gui_path() {
    let current = std::env::var("PATH").unwrap_or_default();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    // Through childenv, as every spawn: under an AppImage the login shell
    // would otherwise run the user's profile with the AppDir's libraries,
    // and hand the AppDir's paths straight back in the PATH this reads.
    let mut cmd = worktrees_core::childenv::command(&shell);
    cmd.args(["-lc", r#"printf '\n__WTPATH__%s' "$PATH""#]);
    let from_shell = run_deadline(cmd, 5)
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .rsplit("__WTPATH__")
                .next()
                .map(|p| p.trim().to_string())
        })
        .filter(|p| !p.is_empty());
    let home = std::env::var("HOME").unwrap_or_default();
    let std_dirs = format!("{home}/.local/bin:{home}/bin:/opt/homebrew/bin:/usr/local/bin");
    // dups harmless; `current` kept as the final safety net
    let path = match from_shell {
        Some(p) => format!("{p}:{std_dirs}:{current}"),
        None => format!("{std_dirs}:{current}"),
    };
    std::env::set_var("PATH", path);
    // The Codex VS Code extension bundles a usable CLI but does not always
    // put it on the GUI app's PATH. Its parent directory also lets the tmux
    // launch use the same `codex` command that MCP setup resolves below.
    if worktrees_core::profile::bin_on_path("codex").is_none() {
        if let Some(dir) = worktrees_core::profile::codex_bin().and_then(|p| p.parent().map(Path::to_path_buf)) {
            let current = std::env::var("PATH").unwrap_or_default();
            std::env::set_var("PATH", format!("{}:{current}", dir.display()));
        }
    }
}

/// Is tmux reachable right now? `refresh = true` re-resolves the GUI PATH first,
/// so a tmux installed AFTER the app launched is picked up without a restart
/// (startup resolves PATH exactly once). Async is not optional here: the
/// login-shell probe inside `fixup_gui_path` is deadline-guarded at 5s, and a
/// sync handler would spend all of it frozen on the main thread.
pub fn tmux_check(refresh: bool) -> Result<bool, String> {
    if !refresh {
        return Ok(worktrees_core::tmux::have_tmux());
    }
    let before = worktrees_core::tmux::have_tmux();
    fixup_gui_path();
    let after = worktrees_core::tmux::have_tmux();
    if after && !before {
        applog(
            "info",
            &format!("tmux_check: tmux found after PATH refresh; PATH={}", std::env::var("PATH").unwrap_or_default()),
        );
    }
    Ok(after)
}

/// Same launchd-bare-env problem as PATH, but for locale: GUI-launched apps
/// have no LC_ALL/LC_CTYPE/LANG, so everything we spawn (tmux server via the
/// engine's shell-outs, shells inside sessions) runs locale-less. The embedded
/// tmux client is already covered by `-u` in term_open; this covers the server
/// side when this app is the first tmux invocation.
pub fn fixup_gui_locale() {
    let has_utf8 = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .any(|v| v.to_uppercase().contains("UTF-8") || v.to_uppercase().contains("UTF8"));
    if !has_utf8 {
        std::env::set_var("LANG", "en_US.UTF-8");
    }
}

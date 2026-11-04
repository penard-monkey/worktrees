//! The agent restart end to end, through the real `Live` path, against a
//! PRIVATE tmux server and a scratch repo you set up by hand
//! (docs/proposals/harness-restart.md, "Verification"). Ignored: it needs a
//! running scratch Claude and Codex, and it restarts them.
//!
//! ```sh
//! T=/private/tmp/tmux-$(id -u)/<private name>      # tmux -L <private name> display -p '#{socket_path}'
//! TMUX=$T,0,0 WT_RESTART_E2E_SOCKET=$T WT_RESTART_E2E_REPO=<scratch repo> \
//!   cargo test -p worktrees-api --test restart_e2e -- --ignored --nocapture
//! ```
//!
//! It refuses to act unless tmux, asked through the same wrapper the restart
//! uses, answers from that socket, and it acts only on the scratch repo's
//! places: the owner's server and sessions are out of reach twice over.

use std::sync::{Arc, Mutex};

use worktrees_api::harness_restart::{restart_with, sessions_with, Live, Tool};

fn guarded() -> Option<String> {
    let (Ok(sock), Ok(repo)) = (std::env::var("WT_RESTART_E2E_SOCKET"), std::env::var("WT_RESTART_E2E_REPO")) else {
        eprintln!("skipped: WT_RESTART_E2E_SOCKET and WT_RESTART_E2E_REPO are not set");
        return None;
    };
    let tmux_env = std::env::var("TMUX").unwrap_or_default();
    assert!(tmux_env.starts_with(&format!("{sock},")), "TMUX must name the private socket {sock}, not {tmux_env:?}");
    let o = worktrees_core::tmux::tmux(&["display-message", "-p", "#{socket_path}"]).expect("tmux");
    let answered = String::from_utf8_lossy(&o.stdout).trim().to_string();
    assert_eq!(answered, sock, "tmux answered from {answered:?}: refusing to act");
    assert!(repo.starts_with("/tmp/") || repo.starts_with("/private/tmp/"), "the scratch repo must be under /tmp: {repo}");
    Some(std::fs::canonicalize(&repo).unwrap().to_string_lossy().into_owned())
}

#[test]
#[ignore]
fn restart_a_scratch_claude_and_codex_on_a_private_server() {
    let Some(repo) = guarded() else { return };
    let roots = vec![repo];
    let lock = |_: &str| Arc::new(Mutex::new(()));
    for tool in [Tool::Claude, Tool::Codex] {
        let before = sessions_with(&Live, &roots, tool, true);
        eprintln!("{:?} before: {}", tool, serde_json::to_string(&before).unwrap());
        assert!(!before.sessions.is_empty(), "{tool:?}: the scratch session must run an older build first");
        assert!(before.sessions.iter().all(|s| s.project == roots[0]), "only the scratch repo's places");
        let out = restart_with(&Live, &roots, &lock, tool, false).unwrap();
        eprintln!("{:?} restart: {}", tool, serde_json::to_string(&out).unwrap());
        assert_eq!(out.failed, 0, "{out:?}");
        assert!(out.restarted >= 1, "{out:?}");
        for r in out.results.iter().filter(|r| r.outcome == "restarted") {
            assert_eq!(r.version, out.installed_version, "proved on the installed version: {r:?}");
        }
        let after = sessions_with(&Live, &roots, tool, true);
        eprintln!("{:?} after: {}", tool, serde_json::to_string(&after).unwrap());
        assert!(after.sessions.is_empty(), "{tool:?}: nothing older is left: {after:?}");
    }
}

//! The agent rows end to end against REAL panes on a PRIVATE tmux server —
//! a driver, not a suite: `#[ignore]`d, and it refuses to run unless `$TMUX`
//! names the socket in `WT_E2E_TMUX`, a private server's (`tmux -L <name>`;
//! never the user's, whose agents these rows would answer). One step per run,
//! so a person (or a lane) looks at the pane between steps:
//!
//! ```sh
//! TMUX=/private/tmp/tmux-$UID/<name>,0,0 WT_E2E_TMUX=/private/tmp/tmux-$UID/<name> \
//!   WT_E2E_REPO=<scratch repo> WT_E2E_STEP='prompt' \
//!   cargo test -p worktrees-api --test agents_e2e -- --ignored --nocapture
//! ```
//!
//! `WT_E2E_STEP` is `prompt`, `answer <n>` (the prompt's own fingerprint is
//! read first, as a client would), `text <n> <text…>`, `commands`, `command
//! <name> [args…]` (the session `agent_commands` names), or `turns <n>`.
//! `WT_E2E_SLUG` defaults to `(main)`. The answer is printed as JSON.

use std::sync::Arc;

use worktrees_api::{agents, Ctx, Host};

struct Quiet;

impl Host for Quiet {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

fn private_server() -> bool {
    let want = std::env::var("WT_E2E_TMUX").unwrap_or_default();
    let have = std::env::var("TMUX").unwrap_or_default();
    let socket = have.split(',').next().unwrap_or("");
    !want.is_empty() && socket == want && !socket.ends_with("/default")
}

#[test]
#[ignore]
fn drive_one_step() {
    let (Ok(repo), Ok(step)) = (std::env::var("WT_E2E_REPO"), std::env::var("WT_E2E_STEP")) else {
        eprintln!("set WT_E2E_REPO and WT_E2E_STEP");
        return;
    };
    assert!(private_server(), "TMUX must name the private socket in WT_E2E_TMUX, never the user's server");
    worktrees_core::registry::forbid_real_writes();
    let slug = std::env::var("WT_E2E_SLUG").unwrap_or_else(|_| "(main)".into());
    let dir = std::env::temp_dir().join(format!("wt-agents-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ctx = Ctx::new(dir.clone(), Arc::new(Quiet));
    let words: Vec<&str> = step.splitn(3, ' ').collect();
    let fingerprint = || {
        let p = agents::agent_prompt(&ctx, repo.clone(), slug.clone()).expect("agent_prompt");
        p["fingerprint"].as_str().map(str::to_string).unwrap_or_else(|| panic!("no readable prompt: {p}"))
    };
    let session = || {
        let c = agents::agent_commands(&ctx, repo.clone(), slug.clone(), None).expect("agent_commands");
        c["session"].as_str().unwrap_or_default().to_string()
    };
    let out = match words[..] {
        ["prompt"] => agents::agent_prompt(&ctx, repo.clone(), slug.clone()),
        ["commands"] => agents::agent_commands(&ctx, repo.clone(), slug.clone(), None).map(|mut v| {
            let n = v["commands"].as_array().map_or(0, Vec::len);
            let own: Vec<serde_json::Value> = v["commands"].as_array().into_iter().flatten().filter(|c| c["source"] != "built-in").cloned().collect();
            v["commands"] = serde_json::json!({ "count": n, "not_built_in": own });
            v
        }),
        ["command", name] => agents::agent_command(&ctx, repo.clone(), slug.clone(), session(), name.into(), None),
        ["command", name, args] => agents::agent_command(&ctx, repo.clone(), slug.clone(), session(), name.into(), Some(args.into())),
        ["turns", n] => agents::agent_turns(&ctx, repo.clone(), slug.clone(), None, Some(n.parse().unwrap())),
        ["answer", n] => agents::agent_answer(&ctx, repo.clone(), slug.clone(), n.parse().unwrap(), fingerprint()),
        ["text", n, text] => agents::agent_answer_text(&ctx, repo.clone(), slug.clone(), n.parse().unwrap(), fingerprint(), text.into()),
        _ => panic!("unknown step {step:?}"),
    };
    println!("E2E {}", match out {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap(),
        Err(e) => format!("ERR {e}"),
    });
    let _ = std::fs::remove_dir_all(&dir);
}

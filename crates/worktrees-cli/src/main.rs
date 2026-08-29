//! `worktrees` CLI — thin front-end over worktrees-core.
//!
//! Increment 1: `--version`/help + the read path (`ls`, `ls --json`), gated by
//! the bats suite via the bash shim. Write ops (new/switch/…) land Increment 2.
//! See MIGRATION.md.

use worktrees_core::ops;
use worktrees_core::render::error_line;
use worktrees_core::{CliUi, Project};

mod mcp;

const USAGE: &str = "\
worktrees — one git worktree per branch, one tmux session per worktree.

  worktrees new <branch> [base]         create a worktree + tmux (AI | shell)
  worktrees co  <branch>                checkout a REMOTE branch (fetch if needed)
  worktrees switch [<worktree>] <branch> [base]   move a worktree to another branch
  worktrees open <name>                 reopen a worktree's tmux session
  worktrees close <name> [name...]      end the tmux session (worktree stays; also: main)
                                        (-y to kill an adopted session; --session <s> binds that answer)
  worktrees ls [--json]                 list worktrees + state (--json = machine-readable)
  worktrees rm <name> [name...]         tear one (or more) down
  worktrees relink [<name>|--all]       re-apply .worktrees.toml's files (--force to overwrite)
  worktrees provision [<name>|--all]    allocate a port slot + write .worktree.env (--reallocate)
  worktrees doctor [<name>]             report file drift, declared and un- (--json --strict --config-only)
  worktrees status <name>               health verdict for one worktree (--json)
  worktrees init                        suggest a .worktrees.toml for this repo (--print, -y)
  worktrees init --diff                 print the [[file]] entries the config is MISSING
  worktrees skills [list|show|add|rm]   manage AI-profile skills (user-global, no repo needed)
  worktrees sync push|pull [name]       courier-sync this project to/from an SSD hub (rsync)
  worktrees sync status                 hub + project sync state (--json; no repo needed)
  worktrees mcp [--mutations]           MCP server over stdio (for an AI session; not interactive)
  worktrees -V | --version              print version   (also: help / -h)
  worktrees                             (no args) -> ls";

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // help/version work anywhere — handled BEFORE the git guard (like bash).
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") | Some("help") => {
            println!("{USAGE}");
            return 0;
        }
        Some("-V") | Some("--version") => {
            println!("worktrees {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        // The skill store is USER-GLOBAL, so managing it must not require
        // standing in a git repo — handled here, ahead of the guards below.
        Some("skills") => {
            let mut ui = CliUi;
            return worktrees_core::skillstore::cmd_skills(&mut ui, args.get(1..).unwrap_or(&[]));
        }
        // Also ahead of the guards: a `pull` that ADOPTS a project, and a
        // hub-level `status`, both run on a machine that does not have the repo
        // yet — there is no git worktree to stand in.
        Some("sync") => {
            let mut ui = CliUi;
            return worktrees_core::sync::cmd_sync(&mut ui, args.get(1..).unwrap_or(&[]));
        }
        _ => {}
    }

    // git guards run for every other command (incl. no-args -> ls).
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}", error_line(&e.to_string()));
            return 1;
        }
    };
    let project = match Project::discover(&cwd) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", error_line(&e.msg));
            return e.code;
        }
    };

    let sub = args.first().map(String::as_str).unwrap_or("ls");
    let rest = args.get(1..).unwrap_or(&[]);

    // ONE choke point for the hub-copy guard, ahead of the dispatch that would
    // otherwise carry it into eight cmd_* functions. A tree that arrived on a
    // sync hub is another machine's mirror: its `.git` registers worktrees at
    // paths that exist only there, so a prune here can unregister the wrong
    // repo's worktrees. The read-only commands are deliberately absent from the
    // list — `ls`, `doctor` and `sync status` are how you find out what this
    // tree is, and `doctor` is where the finding is reported.
    const MUTATING: &[&str] = &[
        "new", "create", "co", "checkout", "switch", "sw", "branch", "open", "reopen", "attach",
        "a", "close", "rm", "remove", "delete", "relink", "provision", "init",
    ];
    if MUTATING.contains(&sub) {
        if let Some(msg) =
            worktrees_core::sync::hub_copy_refusal(std::path::Path::new(&project.main_root))
        {
            eprintln!("{}", error_line(&msg));
            return 1;
        }
    }

    let mut ui = CliUi;
    match sub {
        "ls" | "list" => {
            let json = rest.iter().any(|a| a == "--json")
                || std::env::var("WORKTREES_JSON").ok().as_deref() == Some("1");
            if json {
                print!("{}", project.ls_json());
            } else {
                print!("{}", project.ls_human());
            }
            0
        }
        "new" | "create" | "co" | "checkout" => ops::cmd_new(&project, &mut ui, rest),
        "switch" | "sw" | "branch" => ops::cmd_switch(&project, &mut ui, rest),
        "open" | "reopen" | "attach" | "a" => ops::cmd_open(&project, &mut ui, rest),
        "close" => ops::cmd_close(&project, &mut ui, rest),
        "rm" | "remove" | "delete" => ops::cmd_rm(&project, &mut ui, rest),
        "relink" => ops::cmd_relink(&project, &mut ui, rest),
        "provision" => ops::cmd_provision(&project, &mut ui, rest),
        "doctor" => ops::cmd_doctor(&project, &mut ui, rest),
        // Read-only, so deliberately absent from MUTATING above — a health
        // verdict is part of finding out what a tree is, like `ls` and `doctor`.
        "status" => ops::cmd_status(&project, &mut ui, rest),
        "init" => ops::cmd_init(&project, &mut ui, rest),
        // Note: not routed through `ui` — this speaks JSON-RPC on stdout, and
        // anything else written there corrupts the transport.
        "mcp" => mcp::cmd_mcp(rest),
        other => {
            eprintln!("{}", error_line(&format!("Unknown command: {other}")));
            println!();
            println!("{USAGE}");
            1
        }
    }
}

//! `worktrees` CLI — thin front-end over worktrees-core.
//!
//! Increment 0: only `--version` / help build out (proves the binary compiles
//! and matches the bash version string). Command dispatch (ls, new, switch, …)
//! lands in Increment 1+. See MIGRATION.md.

use std::process::exit;

const USAGE: &str = "\
worktrees — one git worktree per branch, one tmux session per worktree.

  worktrees new <branch> [base]         create a worktree + tmux (AI | shell)
  worktrees co  <branch>                checkout a REMOTE branch (fetch if needed)
  worktrees switch [<worktree>] <branch> [base]   move a worktree to another branch
  worktrees open <name>                 reopen a worktree's tmux session
  worktrees ls [--json]                 list worktrees + state (--json = machine-readable)
  worktrees rm <name> [name...]         tear one (or more) down
  worktrees -V | --version              print version   (also: help / -h)
  worktrees                             (no args) -> ls";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-V") | Some("--version") => {
            println!("worktrees {}", env!("CARGO_PKG_VERSION"));
        }
        Some("-h") | Some("--help") | Some("help") => {
            println!("{USAGE}");
        }
        _ => {
            // Increment 0: dispatch not ported yet. The shipped CLI is still the
            // bash bin/worktrees; this binary is not on any real path yet.
            eprintln!("worktrees: command dispatch not yet implemented in the Rust CLI (MIGRATION.md Increment 1)");
            exit(1);
        }
    }
}

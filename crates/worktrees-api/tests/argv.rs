//! R3: a value a client sent is never read as a flag.
//!
//! Core's `cmd_*` parsers tell a flag from a positional by its leading `-`,
//! and this layer used to hand them client strings bare — so `base:
//! "--no-fetch"` WAS `--no-fetch`, and `relink`'s `slug: "--all"` widened one
//! place to every place. Each value below is one a parser consumed as its own
//! flag on the commit before the fix; each must now be refused at the
//! boundary, before any core code runs, with words the UI can show.
//!
//! An integration test on purpose, like `headless.rs`: this is the position
//! `worktrees serve` is in. The fixtures are chosen so that even the UNFIXED
//! code could touch neither tmux nor the tree — an unborn repo stops `new`
//! before its first side effect, and a single-name command whose only name is
//! eaten ends at "needs a worktree".

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use worktrees_api::ops::CmdResult;
use worktrees_api::{Ctx, Host};

struct NoWindow;
impl Host for NoWindow {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

/// A repo of the test's own; `commit: false` leaves HEAD unborn.
fn repo(tag: &str, commit: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wt-r3-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = std::fs::canonicalize(&dir).unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false", "-C"])
            .arg(&dir)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    if commit {
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    }
    dir
}

fn s(p: &PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// Refused at the boundary: a failed result (so the new-worktree form comes
/// back with what was typed), naming the field and why.
#[track_caller]
fn refused(r: &CmdResult, field: &str) {
    assert!(!r.ok, "accepted: {}", r.output);
    assert!(r.output.starts_with(field) && r.output.contains("may not begin with '-'"), "{field}: {}", r.output);
}

#[test]
fn new_place_refuses_a_flag_shaped_branch_base_or_name() {
    let dir = repo("new", false);
    let ctx = Ctx::new(std::env::temp_dir(), Arc::new(NoWindow));
    let new = |branch: &str, base: Option<&str>, name: Option<&str>| {
        worktrees_api::ops::new_place(&ctx, s(&dir), branch.into(), base.map(Into::into), name.map(Into::into), None, None, None)
            .expect("a result, not a thrown error")
    };
    // The base slot: `--no-fetch`, `-r` and `--force` each set the flag.
    for flag in ["--no-fetch", "-r", "--force", "--brief=do something else"] {
        refused(&new("feat", Some(flag), None), "base");
    }
    // The branch slot: `--force` set force and left no branch at all.
    refused(&new("--force", None, None), "branch");
    // The name: the parser already refused this one ("--name needs a value"),
    // but in words about the argv, not about the field the user filled in.
    refused(&new("feat", None, Some("--no-tmux")), "name");
    // Not merely leading-dash: git's own branch rule, at the boundary.
    let r = new("feat bar", None, None);
    assert!(!r.ok && r.output.contains("not a valid branch name"), "{}", r.output);
    // …applied to the name core USES: `new` strips one `origin/`, so this one
    // passes the leading-dash check and would have arrived as `-x`.
    let r = new("origin/-x", None, None);
    assert!(!r.ok && r.output.contains("'-x' is not a valid branch name"), "{}", r.output);
    // A base is a REVISION, not a branch name: these all pass the boundary
    // and reach core, which stops at the unborn HEAD.
    for ok in ["HEAD~1", "v1.2.0", "origin/main", "0123abc"] {
        let r = new("feat", Some(ok), None);
        assert!(r.output.contains("no commits yet"), "{ok}: {}", r.output);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switch_place_refuses_a_flag_shaped_slug_branch_or_base() {
    let dir = repo("switch", true);
    let sw = |slug: &str, branch: &str, base: Option<&str>| {
        worktrees_api::ops::switch_place(s(&dir), slug.into(), branch.into(), base.map(Into::into)).unwrap()
    };
    refused(&sw("lane", "feat", Some("--force")), "base");
    refused(&sw("lane", "--no-fetch", None), "branch");
    refused(&sw("-y", "feat", None), "slug");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_place_refuses_a_flag_shaped_slug() {
    let dir = repo("open", true);
    let ctx = Ctx::new(std::env::temp_dir(), Arc::new(NoWindow));
    refused(&worktrees_api::ops::open_place(&ctx, s(&dir), "--force".into(), None, None, None, None).unwrap(), "slug");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn close_place_refuses_a_flag_shaped_slug() {
    let dir = repo("close", true);
    let cfg = repo("close-cfg", false);
    let ctx = Ctx::new(cfg.clone(), Arc::new(NoWindow));
    refused(&worktrees_api::ops::close_place(s(&dir), "-y".into(), true, None, None, &ctx.shells).unwrap(), "slug");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cfg);
}

#[test]
fn remove_place_refuses_a_flag_shaped_slug() {
    let dir = repo("rm", true);
    let cfg = repo("rm-cfg", false);
    let ctx = Ctx::new(cfg.clone(), Arc::new(NoWindow));
    let r = worktrees_api::prefs::remove_place(&ctx, s(&dir), "--force".into(), false, false, &ctx.shells).unwrap();
    refused(&r, "slug");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cfg);
}

/// `--all` is the widening case: one place became every place, `--force`
/// included, and the result came back OK.
#[test]
fn relink_refuses_a_slug_that_would_widen_to_every_place() {
    let dir = repo("relink", true);
    refused(&worktrees_api::config::relink(s(&dir), Some("--all".into()), true).unwrap(), "slug");
    // A literal name still goes through — past the `--` the layer now puts in
    // front of it, which the parser has to honour or this says "Unknown flag".
    let r = worktrees_api::config::relink(s(&dir), Some("lane".into()), false).unwrap();
    assert!(r.output.contains("No worktree 'lane'"), "{}", r.output);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn provision_refuses_a_slug_that_would_widen_to_every_place() {
    let dir = repo("provision", true);
    refused(&worktrees_api::config::provision(s(&dir), Some("--all".into())).unwrap(), "slug");
    let r = worktrees_api::config::provision(s(&dir), Some("lane".into())).unwrap();
    assert!(r.output.contains("No worktree 'lane'"), "{}", r.output);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doctor_refuses_a_flag_shaped_slug_and_passes_a_name_through() {
    let dir = repo("doctor", true);
    let e = worktrees_api::config::doctor(s(&dir), Some("--config-only".into())).unwrap().error.unwrap_or_default();
    assert!(e.starts_with("slug") && e.contains("may not begin with '-'"), "{e}");
    let d = worktrees_api::config::doctor(s(&dir), Some("nope".into())).unwrap();
    assert!(d.error.as_deref().unwrap_or("").contains("No worktree 'nope'"), "{:?}", d.error);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `place_health` is `status_of`'s caller; so is `ai_status_report`, which
/// reaches the same parser through the same function.
#[test]
fn place_health_refuses_a_flag_shaped_slug_and_passes_a_name_through() {
    let dir = repo("health", true);
    let e = worktrees_api::config::place_health(s(&dir), "-x".into()).unwrap().error.unwrap_or_default();
    assert!(e.starts_with("slug") && e.contains("may not begin with '-'"), "{e}");
    let h = worktrees_api::config::place_health(s(&dir), "nope".into()).unwrap();
    assert!(h.error.as_deref().unwrap_or("").contains("No worktree 'nope'"), "{:?}", h.error);
    let _ = std::fs::remove_dir_all(&dir);
}

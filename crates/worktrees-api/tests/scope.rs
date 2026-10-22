//! R1's SECOND check: a `Ctx` narrowed to a caller's scope answers with the
//! caller's roots (Server Edition Phase 2, §3.3).
//!
//! serve's resolver checks every argument before a body runs; this is the
//! check behind it. Bodies that ENUMERATE roots — `list_workspace`, the path
//! resolvers, the guard — read them through `projects::read_projects`, so a
//! body handed a scoped `Ctx` that forgets a filter fails closed instead of
//! leaking another project.
//!
//! Its own test binary, because it points `XDG_CONFIG_HOME` at a sandbox to
//! own a registry: an environment variable is process-wide, and no other test
//! may run beside it. One `#[test]`, for the same reason.

use std::path::Path;
use std::sync::Arc;

use worktrees_api::projects::{guard_under_projects, project_roots, read_projects};
use worktrees_api::{Ctx, Host};
use worktrees_core::authz::Projects;

struct Quiet;
impl Host for Quiet {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

fn git_init(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let ok = std::process::Command::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap().success();
    assert!(ok, "git init {}", dir.display());
    std::fs::write(dir.join("f.txt"), "x").unwrap();
}

#[test]
fn a_scoped_ctx_sees_only_the_callers_projects() {
    let top = std::env::temp_dir().join(format!("wt-api-scope-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(&top).unwrap();
    let top = std::fs::canonicalize(&top).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", top.join("xdg"));
    std::env::set_var("HOME", top.join("home"));
    let (a, b) = (top.join("a"), top.join("b"));
    git_init(&a);
    git_init(&b);
    let (sa, sb) = (a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned());
    worktrees_core::registry::add(&sa).unwrap();
    worktrees_core::registry::add(&sb).unwrap();

    let cfg = top.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let engine = Ctx::new(cfg, Arc::new(Quiet));
    let phone = engine.scoped(Projects::only([sa.clone()]));

    // The engine's own Ctx — the poll's, the console's — sees every root.
    assert_eq!(read_projects(&engine), vec![sa.clone(), sb.clone()]);
    assert!(engine.scope().is_all());
    // The phone's, only its own.
    assert_eq!(read_projects(&phone), vec![sa.clone()]);
    assert_eq!(project_roots(&phone), vec![a.clone()]);
    assert!(guard_under_projects(&phone, &b.join("f.txt").to_string_lossy()).is_err());
    assert!(guard_under_projects(&phone, &a.join("f.txt").to_string_lossy()).is_ok());
    assert!(guard_under_projects(&engine, &b.join("f.txt").to_string_lossy()).is_ok());
    // A body that walks the registry walks the caller's part of it.
    let ws = worktrees_api::workspace::list_workspace(&phone).unwrap();
    assert_eq!(ws.projects.iter().map(|p| p.root.clone()).collect::<Vec<_>>(), vec![sa.clone()]);
    // Narrowing only: scoping a scoped Ctx again cannot widen it.
    assert_eq!(read_projects(&phone.scoped(Projects::All)), vec![sa.clone()]);
    assert_eq!(read_projects(&phone.scoped(Projects::only([sb.clone()]))), Vec::<String>::new());
    // Every handle is still the engine's: a scoped Ctx is a view, not a copy.
    assert!(Arc::ptr_eq(&engine.terminals, &phone.terminals));
    assert!(Arc::ptr_eq(&engine.shells, &phone.shells));

    let _ = std::fs::remove_dir_all(&top);
}

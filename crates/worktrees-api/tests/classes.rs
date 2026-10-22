//! A path's CLASS reaches the bodies that list and write paths (Server
//! Edition Phase 2, §7.1, slice 2b).
//!
//! serve's resolver raises the capability for the one path a request names.
//! These are the bodies that ENUMERATE paths, which it cannot see into: a
//! directory listing marks the names its caller may not read, and the docs
//! index and the path links drop them. And `write_file`'s forbidden guard
//! judges over EVERY registered root, so a caller's narrower scope cannot
//! shrink its answer.
//!
//! Its own test binary, because it points `XDG_CONFIG_HOME` at a sandbox to
//! own a registry (as `scope.rs` does): an environment variable is
//! process-wide, and no other test may run beside it. One `#[test]`, for the
//! same reason.

use std::path::Path;
use std::sync::Arc;

use worktrees_api::files::{self, FsEntry};
use worktrees_api::{Ctx, Host};
use worktrees_core::authz::{Capability, Caps, Projects};
use worktrees_core::pathclass::Class;

struct Quiet;
impl Host for Quiet {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

fn git_init(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let ok = worktrees_core::childenv::command("git").args(["init", "-q"]).current_dir(dir).status().unwrap().success();
    assert!(ok, "git init {}", dir.display());
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `name` → its mark, for the entries of one listing.
fn marks(entries: &[FsEntry]) -> Vec<(String, Option<Class>)> {
    entries.iter().map(|e| (e.name.clone(), e.restricted)).collect()
}

fn mark(entries: &[FsEntry], name: &str) -> Option<Class> {
    entries.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("{name} is not listed: {:?}", marks(entries))).restricted
}

#[test]
fn a_paths_class_reaches_the_bodies_that_list_and_write_paths() {
    let top = std::env::temp_dir().join(format!("wt-api-classes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(&top).unwrap();
    let top = std::fs::canonicalize(&top).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", top.join("xdg"));
    std::env::set_var("HOME", top.join("home"));
    let a = top.join("a");
    let inner = a.join("vendor/lib");
    git_init(&a);
    git_init(&inner);
    for d in ["src", ".claude", "config", "docs", ".worktrees/feat"] {
        std::fs::create_dir_all(a.join(d)).unwrap();
    }
    for (f, body) in [
        ("src/lib.rs", "x"),
        ("AGENTS.md", "# agents"),
        (".claude/settings.json", "{}"),
        (".env", "TOKEN=1"),
        ("config/master.key", "k"),
        ("docs/guide.md", "# guide"),
        ("docs/keys.md", "# keys"),
        (".worktrees.places.json", "{}"),
        (".worktrees/feat/.env", "TOKEN=2"),
        (".worktrees/feat/AGENTS.md", "# agents"),
        (".worktrees.toml", "[[file]]\npath = \"config/master.key\"\n\n[[file]]\npath = \"docs/keys.md\"\n"),
    ] {
        std::fs::write(a.join(f), body).unwrap();
    }
    std::os::unix::fs::symlink(a.join(".env"), a.join("src/notes.md")).unwrap();
    std::fs::write(inner.join(".worktrees.places.json"), "{}").unwrap();
    worktrees_core::registry::add(&s(&a)).unwrap();
    worktrees_core::registry::add(&s(&inner)).unwrap();

    let cfg = top.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let console = Ctx::new(cfg, Arc::new(Quiet));
    let only_a = Projects::only([s(&a)]);
    // An Operate device scoped to the outer project, and a Drive one.
    let operate = console.scoped(only_a.clone()).granted(Caps::of(&[Capability::Read, Capability::Write]), false);
    let drive = console.scoped(only_a).granted(Caps::of(&[Capability::Read, Capability::Write, Capability::Instruct, Capability::Stream]), false);
    assert!(console.reads_every_class() && !drive.reads_every_class() && !operate.reads_every_class());
    assert!(!console.granted(Caps::ALL, false).reads_every_class(), "narrowing only: not the console stays not the console");

    // ── write_file: the forbidden guard is over EVERY registered root ──────
    // The inner project's store is `Forbidden` to it and a plain file three
    // levels down to the outer one, the only root the device can see. The
    // resolver refuses this first (the inner root is the owner, out of the
    // scope); this is the check behind it.
    let store = inner.join(".worktrees.places.json");
    let e = files::write_file(&operate, s(&store), "{\"x\":1}".into(), None).expect_err("a scoped caller wrote another project's store");
    assert!(e.contains("worktrees' own state"), "{e}");
    assert_eq!(std::fs::read_to_string(&store).unwrap(), "{}");

    // ── list_dir: restricted names are SHOWN, and marked ──────────────────
    let root = files::list_dir(&operate, s(&a), Some(true)).unwrap();
    assert_eq!(mark(&root, ".env"), Some(Class::Secret), "{:?}", marks(&root));
    assert_eq!(mark(&root, ".worktrees.places.json"), Some(Class::Forbidden));
    assert_eq!(mark(&root, ".worktrees"), Some(Class::Forbidden));
    for readable in ["AGENTS.md", ".claude", "src", "config", "docs", ".worktrees.toml"] {
        assert_eq!(mark(&root, readable), None, "{readable}: an Operate device reads it");
    }
    let src = files::list_dir(&operate, s(&a.join("src")), None).unwrap();
    assert_eq!(mark(&src, "notes.md"), Some(Class::Secret), "a link is the class of what it lands on");
    assert_eq!(mark(&src, "lib.rs"), None);
    let place = files::list_dir(&operate, s(&a.join(".worktrees/feat")), Some(true)).unwrap();
    assert_eq!((mark(&place, ".env"), mark(&place, "AGENTS.md")), (Some(Class::Secret), None), "a place is classed as a checkout");
    let config = files::list_dir(&operate, s(&a.join("config")), None).unwrap();
    assert_eq!(mark(&config, "master.key"), Some(Class::Secret), "a declared [[file]]");
    // A device with a shell reads secrets, and still not worktrees' own state.
    let root = files::list_dir(&drive, s(&a), Some(true)).unwrap();
    assert_eq!((mark(&root, ".env"), mark(&root, ".worktrees.places.json")), (None, Some(Class::Forbidden)));
    // The console reads every class, so it marks nothing.
    let root = files::list_dir(&console, s(&a), Some(true)).unwrap();
    assert!(root.iter().all(|e| e.restricted.is_none()), "{:?}", marks(&root));

    // ── the docs index drops what the caller may not read ─────────────────
    let rels = |ctx: &Ctx| files::list_docs(ctx, s(&a), s(&a)).unwrap().entries.into_iter().map(|e| e.rel).collect::<Vec<_>>();
    let seen = rels(&operate);
    assert!(seen.contains(&"docs/guide.md".to_string()) && seen.contains(&"AGENTS.md".to_string()), "{seen:?}");
    assert!(!seen.contains(&"docs/keys.md".to_string()), "a declared secret is not a document to an Operate device: {seen:?}");
    assert!(rels(&console).contains(&"docs/keys.md".to_string()));
    assert!(rels(&drive).contains(&"docs/keys.md".to_string()));

    // ── the path links name only what the caller may read ─────────────────
    let term = |ctx: &Ctx, paths: &[&str]| {
        files::resolve_term_paths(ctx, &ctx.shells, s(&a), None, None, paths.iter().map(|p| p.to_string()).collect()).unwrap()
    };
    let asked = [".env", "src/lib.rs", "AGENTS.md", "src/notes.md", "config/master.key"];
    let got = term(&operate, &asked);
    assert_eq!(got.iter().map(Option::is_some).collect::<Vec<_>>(), [false, true, true, false, false], "{got:?}");
    assert!(term(&console, &asked).iter().all(Option::is_some));
    assert!(term(&drive, &asked).iter().all(Option::is_some));
    let doc = |ctx: &Ctx, paths: &[&str]| {
        let docnames = files::DocNameIndex::default();
        files::resolve_doc_paths(ctx, &docnames, s(&a), Some(s(&a.join("docs/guide.md"))), 0, paths.iter().map(|p| p.to_string()).collect()).unwrap()
    };
    // `master.key` is a bare name, found through the place's name index.
    let asked = ["../.env", "master.key", "../src/lib.rs"];
    let got = doc(&operate, &asked);
    assert_eq!(got.iter().map(|h| h.len()).collect::<Vec<_>>(), [0, 0, 1], "{got:?}");
    assert_eq!(doc(&console, &asked).iter().map(|h| h.len()).collect::<Vec<_>>(), [1, 1, 1]);

    let _ = std::fs::remove_dir_all(&top);
}

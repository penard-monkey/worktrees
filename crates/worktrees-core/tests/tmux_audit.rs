//! Adding a raw subprocess bypasses endpoint routing. New sites require a
//! deliberate adapter review, not another caller-local choice of server.
use std::{fs, path::Path};
fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() { sources(&p, out); }
        else if p.extension().is_some_and(|x| x == "rs") { out.push(p); }
    }
}
#[test]
fn tmux_processes_only_exist_in_the_routing_adapters_and_version_probe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["crates/worktrees-core/src", "crates/worktrees-cli/src", "app/src-tauri/src"] { sources(&root.join(dir), &mut files); }
    let mut sites = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let count = compact.matches("::new(\"tmux\")").count();
        if count > 0 { sites.push((file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"), count)); }
    }
    sites.sort();
    assert_eq!(sites, vec![
        ("app/src-tauri/src/lib.rs".into(), 1), // portable-pty adapter
        ("crates/worktrees-core/src/tmux.rs".into(), 2), // descriptor + -V
        ("crates/worktrees-core/src/tmux_server.rs".into(), 2), // private real-server witness + argv unit test
    ]);
    let core = fs::read_to_string(root.join("crates/worktrees-core/src/tmux.rs")).unwrap();
    assert!(core.contains("server.configure(&mut command)"));
    let app = fs::read_to_string(root.join("app/src-tauri/src/lib.rs")).unwrap();
    for required in ["cmd.args(server.endpoint_args())", "cmd.env(\"TMUX_TMPDIR\", server.socket_root())", "cmd.env_remove(\"TMUX\")"] {
        assert!(app.contains(required), "PTY adapter lost {required}");
    }
}

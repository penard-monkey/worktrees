//! gap 17's api half: a body asks its `Ctx` how git may fetch, so a device's
//! clone and skill preview run under the pin serve's resolver made, and the
//! console's run today's git (`worktrees_core::fetchpolicy`).
//!
//! Its own test binary, because the console's clone REGISTERS the project:
//! `XDG_CONFIG_HOME` and `HOME` point at a sandbox, and an environment
//! variable is process-wide. One `#[test]`, for the same reason.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use worktrees_api::{profiles, workspace, Ctx, Host};
use worktrees_core::clone::CloneErrorKind;
use worktrees_core::fetchpolicy::{Fetcher, Pin, Rules};

struct Quiet;
impl Host for Quiet {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

/// A loopback server answering every request with `answer` (status line and
/// headers), counting what it was asked.
fn server(answer: String) -> (u16, Arc<AtomicUsize>) {
    let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = l.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut conn = conn;
            let _ = conn.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let mut buf = [0u8; 8192];
            let _ = conn.read(&mut buf);
            h.fetch_add(1, Ordering::SeqCst);
            let _ = conn.write_all(format!("{answer}Content-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
        }
    });
    (port, hits)
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn a_body_fetches_as_its_ctx_says() {
    let top = std::env::temp_dir().join(format!("wt-api-fetch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(&top).unwrap();
    let top = std::fs::canonicalize(&top).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", top.join("xdg"));
    std::env::set_var("HOME", top.join("home"));
    let cfg = top.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let engine = Ctx::new(cfg, Arc::new(Quiet));
    let parent = top.join("clones").to_string_lossy().into_owned();
    let mut quiet = |_: worktrees_core::clone::CloneProgress| {};

    // A server standing in for a public one, pinned the way serve's resolver
    // pins (the rules admit loopback and plain http only because no TLS
    // server runs here). It 302s to a witness on the host's loopback.
    let (witness, seen) = server("HTTP/1.1 404 Not Found\r\n".into());
    let (public, asked) = server(format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{witness}/r.git/info/refs\r\n"));
    let url = format!("http://127.0.0.1:{public}/r.git");
    let admit = |ip: &IpAddr| ip.is_loopback();
    let none = |h: &str, _: u16| -> Result<Vec<IpAddr>, String> { panic!("looked up {h}") };
    let rules = Rules { schemes: &["http"], resolve: &none, admit: &admit, git_env: &[] };
    let phone = engine.fetching(Fetcher::Device(vec![Pin::resolve_with(&url, &rules).unwrap()]));

    let e = workspace::clone_project(&phone, "a".into(), url.clone(), parent.clone(), Some("bounced".into()), &mut quiet).err().expect("the clone went through");
    assert_ne!(e.kind, CloneErrorKind::Invalid, "the pinned URL was refused: {}", e.message);
    assert!(asked.load(Ordering::SeqCst) > 0, "the pinned server was never asked");
    assert_eq!(seen.load(Ordering::SeqCst), 0, "clone_project followed the redirect");
    let before = asked.load(Ordering::SeqCst);
    assert!(profiles::skill_preview_git(&phone, url.clone(), None).is_err());
    assert!(asked.load(Ordering::SeqCst) > before, "skill_preview_git never asked the pinned server");
    assert_eq!(seen.load(Ordering::SeqCst), 0, "skill_preview_git followed the redirect");

    // A URL the device was not pinned for is checked afresh, and a local
    // repository on the host's disk is refused before anything is made.
    let src = top.join("src");
    std::fs::create_dir_all(&src).unwrap();
    git(&src, &["init", "-q"]);
    std::fs::write(src.join("f.txt"), "x").unwrap();
    git(&src, &["add", "."]);
    git(&src, &["commit", "-qm", "one"]);
    let local = format!("file://{}", src.display());
    let e = workspace::clone_project(&phone, "b".into(), local.clone(), parent.clone(), None, &mut quiet).err().expect("the clone went through");
    assert_eq!(e.kind, CloneErrorKind::Invalid, "{}", e.message);
    assert!(!Path::new(&parent).join("src").exists(), "a refused clone made its target");
    assert!(profiles::skill_preview_git(&phone, local.clone(), None).is_err());

    // The console clones the same folder, with today's git: nothing kept.
    let done = workspace::clone_project(&engine, "c".into(), local, parent, None, &mut quiet).unwrap();
    let kept = std::process::Command::new("git")
        .args(["-C", &done.dir, "config", "--local", "--get", "http.followRedirects"])
        .output()
        .unwrap();
    assert!(kept.stdout.is_empty(), "the console's clone kept a device's setting");
}

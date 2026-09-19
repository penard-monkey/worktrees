//! The tool-owned documentation viewer — one supervised child, N places.
//!
//! `docs.rs` decides which documents a place has; `derive.rs` decides what each
//! one looks like once a viewer gets hold of it. This is the third part: where
//! the derived tree lands on disk, what process serves it, and what URL the
//! frontend hands to `openUrl`. It is the only part of the feature that holds a
//! port, so it is the only part that can leak a client's signed agreement to a
//! web page — which is why almost every rule below is a refusal.
//!
//! **The viewer is `mo`, patched** (proposal §13.4, §14). Stock `mo` fails the
//! §11.3 gate: it does not validate the `Host` header, so a loopback bind keeps
//! other MACHINES out while leaving the browser on this one — which runs code
//! from strangers and can reach `127.0.0.1` — able to read every document in
//! every group via DNS rebinding. §5.1's "one process for the whole app" makes
//! that blast radius total rather than per place.
//!
//! So the binary is not trusted to be the patched one. **Every spawn proves it**
//! (`probe_refuses_foreign_host`): one request carrying a foreign `Host`, and
//! anything but `403` kills the child and fails the open. That costs one
//! round-trip on a path that is already polling the port, and it is the
//! difference between "we believe we shipped the fixed build" and "this process,
//! now, refuses". A release built from an unpatched upstream, a binary swapped
//! on disk, a `WORKTREES_VIEWER_BIN` pointed at a stock `mo` — all three fail
//! closed, and none of them is detectable any other way.
//!
//! Five more rules, each of them a failure this app or the research already had:
//!
//! 1. **Foreground, under our supervision.** `mo` daemonises by default: spawn
//!    it without `--foreground` and the `Child` we hold is a launcher that has
//!    already exited, `RunEvent::Exit` kills nothing, and an unauthenticated
//!    server keeps the user's documents on a port after the app has quit.
//! 2. **Nothing happens at launch.** The first `open_docs_viewer` spawns; a
//!    missing, quarantined or wrong-architecture binary cannot touch startup,
//!    and the Docs tab (phases 1–2) keeps working with no viewer at all. There
//!    is deliberately no launch probe deciding whether to show the tab — the
//!    blast radius of the whole subsystem is one button.
//! 3. **A bounded deadline, never a sleep.** The spike measured 0.23 s to
//!    listen; `START_DEADLINE` is an order of magnitude over that, polled, and
//!    a child that misses it is killed and reported rather than waited on.
//! 4. **Liveness the way `Shells` does it** — `try_wait` on every open, respawn
//!    on the next one, no health-check timer. Nothing here samples by pid: a
//!    reaped pid keeps answering (`portable_pty`'s `process_id`, CLAUDE.md), and
//!    the `Child` handle is the only honest liveness signal we have.
//! 5. **`mo`'s own state is isolated and wiped.** It keeps a session under
//!    `$XDG_STATE_HOME/mo/` keyed by PORT and RESTORES the previous groups when
//!    a server starts on that port — so a reused port resurrects places the user
//!    removed, and sharing the default directory with a `mo` the user runs by
//!    hand would mix the two. `XDG_STATE_HOME` is honoured (verified, not
//!    inferred: with it set, `~/.local/state/mo` was never created), so it is
//!    pointed inside the app's own config dir and that directory is emptied
//!    before each spawn.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use worktrees_core::derive::{self, Links, Staleness};
use worktrees_core::docs::DocEntry;

/// How long a spawned viewer gets to answer on its port before it is killed.
///
/// The spike measured **0.23 s** from exec to listening with 7 files, and the
/// tree we hand it is registered afterwards rather than on the command line, so
/// startup does not grow with the place. An order of magnitude over the measured
/// figure absorbs a cold page cache and a first-launch Gatekeeper check without
/// ever becoming an unbounded wait — which is the failure mode that matters,
/// because it happens on the UI's click path.
const START_DEADLINE: Duration = Duration::from_millis(3_000);

/// Poll interval while waiting for the port. Small enough that the common case
/// (0.23 s) costs about nine connect attempts, each of which fails instantly on
/// a loopback address nobody is listening on.
const POLL_EVERY: Duration = Duration::from_millis(25);

/// Deadline for the `Host`-header probe and for the registration client. Both
/// talk to a process that has just proved it is listening, so anything slower
/// than this is a hang rather than a slow answer.
const PROBE_TIMEOUT: Duration = Duration::from_millis(2_000);

/// The `Host` a browser would only send if it had been rebound. `.invalid` is
/// reserved by RFC 2606 and can never resolve, so the probe cannot accidentally
/// name something real.
const PROBE_HOST: &str = "worktrees-viewer-probe.invalid";

/// Per-document read cap, matching `read_file`'s. A document over this is not
/// silently dropped — it gets a stub page saying so (§2.7, "fail per file,
/// loudly"), because a row that opens onto nothing is the same silent wrongness
/// the staleness header exists to prevent.
const DOC_MAX_BYTES: u64 = 1_000_000;

/// Longest group name we will build. `mo` serves a group at `/{name}`, so this
/// is a URL path segment; the length is cosmetic, the character set is not.
const GROUP_MAX: usize = 48;

/// Where the viewer binary is looked for, relative to the bundle's resource
/// directory. `release.yml` puts it here and signs it.
const RESOURCE_REL: &str = "viewer/mo";

/// Development / test escape hatch: an absolute path to a viewer binary. This is
/// the USER's environment, never a cloned repo's file, so ADR 0001 is untouched
/// — the repo still cannot name a program to run. It exists because `tauri dev`
/// has no bundle and therefore no resource directory.
const BIN_ENV: &str = "WORKTREES_VIEWER_BIN";

// ── the managed child ────────────────────────────────────────────────────────

/// One running viewer, and the places already registered with it.
struct Proc {
    child: std::process::Child,
    port: u16,
    /// Canonical place root → the `mo` group serving it, for THIS process only.
    ///
    /// Registration is idempotent in `mo` (verified: re-running the client
    /// against the same directory leaves the file count unchanged), so this map
    /// is not correctness — it is the record of which names are taken, which
    /// `group_name` needs in order to keep two places from colliding into one
    /// group and showing each other's documents.
    groups: Vec<(PathBuf, String)>,
}

/// The app's single viewer slot. `None` = nothing has been spawned, or the last
/// one died and has not been replaced yet. Both states are normal.
#[derive(Default)]
pub struct Viewer(Mutex<Option<Proc>>);

/// Kill the viewer, if there is one. Called from `RunEvent::Exit` next to the
/// shells: the child dies with the app deliberately, because what it is holding
/// is an unauthenticated port onto the user's documents.
pub fn kill(v: &Viewer) {
    if let Some(mut p) = v.0.lock().unwrap().take() {
        let _ = p.child.kill();
        let _ = p.child.wait();
    }
}

/// Drop what the viewer left on disk. Called from `RunEvent::Exit` after `kill`.
///
/// The derived trees are copies of the user's documents in a directory that is
/// neither the repo nor covered by its gitignore; the session file is `mo`'s own
/// and would restore removed places onto a reused port. Neither is worth keeping
/// for a viewer that is, by design, dead. Best-effort: a file that will not go is
/// not worth failing a shutdown over, and `spawn` empties both again anyway.
pub fn cleanup(config_dir: &Path) {
    let v = viewer_dir(config_dir);
    empty_dir(&v.join("tree"));
    empty_dir(&v.join("state"));
}

// ── what the command is asked for ────────────────────────────────────────────

/// One `open_docs_viewer` call, as the frontend states it.
pub struct Request<'a> {
    /// The place directory. Already through `guard_under_projects` by the time
    /// it arrives here.
    pub root: &'a Path,
    /// The place's slug. Names the group, and is the first fact in the header.
    pub slug: &'a str,
    /// The document to open, as an absolute path out of the index, or `None`
    /// for the place's landing page.
    pub path: Option<&'a str>,
    /// The staleness facts, as the app already holds them in `Place`.
    pub stale: Staleness,
}

// ── the pure parts, which is where the tests live ────────────────────────────

/// The verdict on the `Host`-header probe, from the response's first line.
///
/// Anything but `403` is a failure, INCLUDING a success: a viewer that answers
/// `200` to a request claiming to be `worktrees-viewer-probe.invalid` is a
/// viewer that would answer a rebound page the same way, and that is the whole
/// of §13.2. A malformed or empty first line is also a failure — "I could not
/// tell" and "it refused" must never collapse into the same branch, because
/// only one of them is safe.
pub fn probe_verdict(first_line: &str) -> Result<(), String> {
    let mut parts = first_line.trim_end().split(' ');
    let version = parts.next().unwrap_or("");
    let code = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        return Err(format!("no HTTP status line from the viewer (got {first_line:?})"));
    }
    match code {
        "403" => Ok(()),
        "" => Err("the viewer sent a status line with no code".into()),
        other => Err(format!(
            "the viewer answered {other} to a request with a foreign Host header; it must answer 403 \
             (an unpatched viewer is readable by any web page the user has open — proposal §13.2)"
        )),
    }
}

/// A place's `mo` group name: the slug, reduced to what can stand in a URL path
/// segment, disambiguated against the names already taken.
///
/// Two places may legitimately share a slug — the same branch name in two
/// projects — and `mo` merges same-named groups rather than rejecting them, so a
/// collision would silently show one project's documents under the other's
/// name. That is §1.1's failure with the axes swapped, so the tie is broken by
/// a hash of the canonical root, which is the one thing that cannot collide.
///
/// The character set is not cosmetic. `mo` reserves `/_/` for its own API, and
/// anything with a `/`, a `?` or a `%` in it would not be the segment we then
/// bake into a `click` directive — so the name is reduced to `[a-z0-9-]`,
/// forced to start with a letter or digit, and never left empty.
pub fn group_name(slug: &str, root: &Path, taken: &[(PathBuf, String)]) -> String {
    if let Some((_, name)) = taken.iter().find(|(r, _)| r == root) {
        return name.clone();
    }
    let mut base = String::new();
    let mut dash = false;
    for c in slug.chars() {
        if c.is_ascii_alphanumeric() {
            if dash && !base.is_empty() {
                base.push('-');
            }
            dash = false;
            base.push(c.to_ascii_lowercase());
        } else {
            dash = true;
        }
        if base.len() >= GROUP_MAX {
            break;
        }
    }
    // A slug of nothing but punctuation, and the place whose slug is literally
    // `_` — `mo`'s own namespace — both land here.
    if base.is_empty() {
        base = "place".to_string();
    }
    if !taken.iter().any(|(_, n)| *n == base) {
        return base;
    }
    let suffix = &id8(root.as_os_str().as_encoded_bytes());
    let keep = GROUP_MAX.saturating_sub(suffix.len() + 1);
    format!("{}-{}", &base[..base.len().min(keep)], suffix)
}

/// `mo`'s file id: the first 8 hex characters of the sha256 of the file's
/// ABSOLUTE path. Verified against a running server, not read off the source —
/// `sha256("…/a.md")[..8]` was `10ca7322` and so was the id in `/_/api/groups`.
fn id8(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// The URL for one derived file, in the form the spike proved is the working
/// one: absolute, with the port, the group and the path hash baked in.
///
/// A relative `./x.md` in a `click` directive misroutes to another group
/// entirely (spike, Test 1), which is why none of this can be committed into the
/// user's own files and why the derived tree is generated at launch.
///
/// `127.0.0.1` rather than `localhost`: it is what `derive::is_loopback_target`
/// will accept on the way out, it needs no resolver, and it is the literal the
/// patched viewer's `Host` check will see.
pub fn file_url(port: u16, group: &str, derived: &Path) -> String {
    format!("http://127.0.0.1:{port}/{group}?file={}", id8(derived.as_os_str().as_encoded_bytes()))
}

/// The URL for a place's landing page — what the "Open this place in the
/// browser" button opens.
pub fn group_url(port: u16, group: &str) -> String {
    format!("http://127.0.0.1:{port}/{group}")
}

/// A relative path we are willing to CREATE under the derived tree, or `None`.
///
/// `DocEntry::rel` comes out of a walk that started at a canonicalised place
/// root and never followed a symlink, so in practice it is already safe. This
/// checks anyway, because the walk's guarantee is about READING and this is the
/// one place in the feature that WRITES: a `..` component here does not list the
/// wrong file, it puts a tool-generated document with a tool-generated
/// `click` directive somewhere outside the tree we serve. The guarantee and the
/// use are far enough apart that the check belongs at the write.
pub fn safe_rel(rel: &str) -> Option<&str> {
    if rel.is_empty() || rel.len() > 1024 {
        return None;
    }
    if rel.starts_with('/') || rel.contains('\\') || rel.contains('\0') {
        return None;
    }
    // A Windows drive letter is not a component the loop below would refuse, and
    // `Path::join` would treat it as absolute.
    if rel.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
    }
    Some(rel)
}

/// The stub page a document gets when it could not be rendered.
///
/// §2.7: *"a page that does not render is a card that says so, beside pages that
/// did"*. The alternative — dropping the row — makes the viewer disagree with
/// the index the user just clicked in, and disagreeing silently is the shape
/// this whole feature exists to remove.
fn stub(entry: &DocEntry, stale: &Staleness, why: &str) -> String {
    let mut s = derive::header(stale);
    s.push_str(&format!("# {}\n\n", entry.title.replace(['\n', '\r'], " ")));
    s.push_str(&format!("*This document was not rendered: {why}.*\n\n"));
    s.push_str(&format!("`{}`\n", entry.rel.replace('`', "'")));
    s
}

// ── generating the tree ──────────────────────────────────────────────────────

/// Read one document and transform it, or produce the stub that says why not.
fn render_one(path: &Path, entry: &DocEntry, stale: &Staleness, links: &Links) -> String {
    let Ok(file) = std::fs::File::open(path) else {
        return stub(entry, stale, "it could not be opened");
    };
    let mut bytes = Vec::new();
    if file.take(DOC_MAX_BYTES + 1).read_to_end(&mut bytes).is_err() {
        return stub(entry, stale, "it could not be read");
    }
    if bytes.len() as u64 > DOC_MAX_BYTES {
        return stub(entry, stale, "it is larger than 1 MB");
    }
    match String::from_utf8(bytes) {
        // Not lossy. A document with a broken byte in it is a document whose
        // author would want to know, and a silently mangled page is indis-
        // tinguishable from the real thing — which is exactly what `mo` does on
        // its own (it skips an invalid-UTF-8 file with no log line at all).
        Err(_) => stub(entry, stale, "it is not valid UTF-8"),
        Ok(text) => derive::document(entry, &text, stale, links),
    }
}

/// Write the whole derived tree for one place, and return the derived path of
/// each entry, in index order.
///
/// Written IN PLACE rather than wiped and recreated. `mo` watches this directory
/// (`-wR`), and a delete-then-create cycle is two events per file on a watcher
/// that is also the thing keeping the user's open tab live; overwriting is one.
/// Files that are no longer in the index are pruned afterwards, which is the
/// only removal that has to happen at all.
fn write_tree(
    dir: &Path,
    entries: &[DocEntry],
    stale: &Staleness,
    port: u16,
    group: &str,
) -> Result<Vec<(usize, PathBuf)>, String> {
    // The derived path of every entry, decided BEFORE anything is written,
    // because `Links::url` has to be able to answer for a page that has not been
    // generated yet — a diagram on the first page links to the last one.
    let mut derived: Vec<(usize, PathBuf)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(rel) = safe_rel(&e.rel) {
            derived.push((i, dir.join(rel)));
        }
    }
    let url_for = |e: &DocEntry| -> Option<String> {
        let (_, p) = derived.iter().find(|(i, _)| entries[*i].path == e.path)?;
        Some(file_url(port, group, p))
    };
    let links = Links { entries, url: &url_for };

    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // A SET, not the Vec: `prune` asks "is this one kept?" once per file on
    // disk, and the index's cap is 2,000 — a linear scan makes that four million
    // path comparisons on a button press, for no reason.
    let mut written: BTreeSet<PathBuf> = BTreeSet::new();
    for (i, out) in &derived {
        let entry = &entries[*i];
        let body = render_one(Path::new(&entry.path), entry, stale, &links);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(out, body).map_err(|e| format!("{}: {e}", out.display()))?;
        written.insert(out.clone());
    }
    prune(dir, &written);
    Ok(derived)
}

/// Remove derived files that are no longer in the index, and the directories
/// that empties. Bounded to `dir`, which this module created and owns — nothing
/// here follows a symlink, and a directory we did not make is not descended.
fn prune(dir: &Path, keep: &BTreeSet<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let p = ent.path();
        let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
        if md.file_type().is_dir() {
            prune(&p, keep);
            // Only if it emptied — `remove_dir` refuses a non-empty directory,
            // so this can never take a file with it.
            let _ = std::fs::remove_dir(&p);
        } else if md.file_type().is_file() && !keep.contains(&p) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

// ── spawning, and proving what was spawned ───────────────────────────────────

/// A free loopback port.
///
/// The OS picks it (`bind` to port 0, read it back, close), then
/// `provision::port_free` — the repo's own probe — confirms it. The window
/// between closing and the viewer binding is real and is deliberately not
/// papered over: a port that was taken in that gap shows up as "started but
/// never listened", which the deadline already handles by killing the child and
/// saying so.
fn pick_port() -> Result<u16, String> {
    for _ in 0..16 {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        let port = l.local_addr().map_err(|e| e.to_string())?.port();
        drop(l);
        if worktrees_core::provision::port_free(port as u32) {
            return Ok(port);
        }
    }
    Err("no free loopback port for the documentation viewer".into())
}

/// Ask the viewer for something, claiming to be a host it is not, and require a
/// refusal.
///
/// This is §11.3's acceptance criterion executed rather than assumed. The threat
/// is not another local process — anything running as this user can read `docs/`
/// without our help — it is a **web page**: script in a tab the user already has
/// open can re-resolve its own domain to `127.0.0.1` and is then same-origin
/// with the viewer, so no CORS check applies and `/_/api/groups` returns every
/// document in every place. The browser always sends the name it believes it is
/// talking to and cannot be made to lie about it, so a viewer that checks `Host`
/// is immune and one that does not is wide open. There is no third state, and no
/// proxy in front fixes it — the page can reach the viewer's own port directly
/// (§13.3).
fn probe_refuses_foreign_host(port: u16) -> Result<(), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(PROBE_TIMEOUT)).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(PROBE_TIMEOUT)).map_err(|e| e.to_string())?;
    // `/_/api/groups` on purpose: it is the endpoint that hands back every
    // document in every group at once, so it is the one the gate is about.
    let req = format!(
        "GET /_/api/groups HTTP/1.1\r\nHost: {PROBE_HOST}\r\nUser-Agent: worktrees-viewer-probe\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    s.flush().map_err(|e| e.to_string())?;
    // The status line and nothing more. A bounded read, because the body of a
    // 200 here would be the documents themselves and this process has no reason
    // to hold them.
    let mut buf = [0u8; 256];
    let n = s.read(&mut buf).map_err(|e| e.to_string())?;
    let head = String::from_utf8_lossy(&buf[..n]);
    probe_verdict(head.lines().next().unwrap_or(""))
}

/// Wait for the port, or for the child to die trying.
fn await_listening(child: &mut std::process::Child, port: u16) -> Result<(), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let deadline = Instant::now() + START_DEADLINE;
    loop {
        // The child first: a viewer that exited is answerable now, and polling a
        // port nobody will ever bind for the whole deadline turns an instant
        // failure into a three-second one on the UI's click path.
        match child.try_wait() {
            Ok(Some(st)) => return Err(format!("the viewer exited before it listened ({st})")),
            Ok(None) => {}
            Err(e) => return Err(format!("the viewer could not be waited on: {e}")),
        }
        if TcpStream::connect_timeout(&addr, POLL_EVERY).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the viewer did not listen on 127.0.0.1:{port} within {} ms",
                START_DEADLINE.as_millis()
            ));
        }
        std::thread::sleep(POLL_EVERY);
    }
}

/// Where the viewer's own state lives, and where the derived trees go.
fn viewer_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("viewer")
}

/// Empty a directory without removing it, ignoring what will not go. Used on the
/// `mo` state root before every spawn: its session file is keyed by PORT and
/// restores whatever that port served last time, so a port the OS hands us twice
/// would resurrect a place the user has since removed.
fn empty_dir(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let p = ent.path();
        let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
        if md.file_type().is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// The viewer binary, by STAT — never by running it.
///
/// "It should be there and it is not" is a `doctor` finding, which already runs
/// off the hot path; executing a candidate to find out would put an exec of an
/// unknown binary on app launch, which is the opposite of what rule 2 asks for.
pub fn binary_path(resource_dir: Option<&Path>) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(BIN_ENV) {
        let p = PathBuf::from(p);
        return std::fs::metadata(&p).ok().filter(|m| m.is_file()).map(|_| p);
    }
    let p = resource_dir?.join(RESOURCE_REL);
    std::fs::metadata(&p).ok().filter(|m| m.is_file()).map(|_| p)
}

/// Spawn a viewer, wait for it, and prove it refuses a foreign `Host`.
///
/// Every failure past the spawn kills the child before returning. A viewer we
/// could not vouch for is worse than no viewer: it is an unauthenticated server
/// holding the user's documents, with nothing in the UI saying so.
fn spawn(bin: &Path, state_dir: &Path, log: &Path) -> Result<Proc, String> {
    std::fs::create_dir_all(state_dir).map_err(|e| format!("{}: {e}", state_dir.display()))?;
    empty_dir(state_dir);
    let port = pick_port()?;

    let mut cmd = std::process::Command::new(bin);
    cmd.args([
        // Without this it daemonises and the handle we keep is a launcher that
        // has already exited — `RunEvent::Exit` would kill nothing.
        "--foreground",
        // We open the URL ourselves, through the same `openUrl` path the rest of
        // the app uses.
        "--no-open",
        "--bind",
        // The literal rather than `localhost`: no resolver, no chance of an
        // `::1`/`127.0.0.1` split, and it is the name the patched Host check
        // sees.
        "127.0.0.1",
        "--port",
    ]);
    cmd.arg(port.to_string());
    // Its session lives where we say, not in `~/.local/state/mo` beside a copy
    // the user runs by hand.
    cmd.env("XDG_STATE_HOME", state_dir);
    cmd.stdin(std::process::Stdio::null());
    // A file rather than a pipe. `mo` logs a JSON line per file event, and a pipe
    // nobody drains fills and wedges the child — the same trap `run_deadline`
    // spawns reader threads for. A file also survives the failure we most need
    // to explain, which is a child that died before it listened.
    let out = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    cmd.stdout(std::process::Stdio::from(out));
    cmd.stderr(std::process::Stdio::from(err));

    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", e_with_path(bin, &e)))?;
    let fail = |child: &mut std::process::Child, msg: String| -> String {
        let _ = child.kill();
        let _ = child.wait();
        msg
    };
    if let Err(e) = await_listening(&mut child, port) {
        return Err(fail(&mut child, format!("{e}{}", log_tail(log))));
    }
    if let Err(e) = probe_refuses_foreign_host(port) {
        return Err(fail(&mut child, e));
    }
    Ok(Proc { child, port, groups: Vec::new() })
}

/// A spawn error with the path in it — `No such file or directory` alone names
/// nothing, and the two cases a user can actually fix (it is missing, it is the
/// wrong architecture) look identical without it.
fn e_with_path(bin: &Path, e: &std::io::Error) -> String {
    format!("{}: {e}", bin.display())
}

/// The last few lines of the viewer's log, for an error message. A child that
/// exited before it listened put its reason here and nowhere else.
fn log_tail(log: &Path) -> String {
    let Ok(s) = std::fs::read_to_string(log) else { return String::new() };
    let tail: Vec<&str> = s.lines().rev().take(3).filter(|l| !l.trim().is_empty()).collect();
    if tail.is_empty() {
        return String::new();
    }
    format!(" — {}", tail.into_iter().rev().collect::<Vec<_>>().join(" / "))
}

/// Register a place's derived tree as a group on the running viewer.
///
/// `-wR <dir>` rather than a file list: one argument instead of up to 2,000
/// (the index's cap), which keeps this off `ARG_MAX` entirely, and the watch is
/// what makes a regenerated tree refresh an already-open browser tab. Verified
/// idempotent — re-running it against the same directory leaves the group's file
/// count unchanged.
fn register(bin: &Path, state_dir: &Path, port: u16, group: &str, tree: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["--no-open", "--port"]);
    cmd.arg(port.to_string());
    cmd.args(["--target", group, "-wR"]);
    cmd.arg(tree);
    cmd.env("XDG_STATE_HOME", state_dir);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());
    // This is a short-lived CLIENT — it talks to the server over loopback and
    // exits. It is not the supervised child, so it gets a deadline of its own
    // rather than a `try_wait` loop.
    let out = crate::run_deadline(cmd, PROBE_TIMEOUT.as_secs().max(1))
        .map_err(|e| format!("registering {group} with the viewer: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let msg = String::from_utf8_lossy(&out.stderr);
    Err(format!("registering {group} with the viewer failed ({}): {}", out.status, msg.trim()))
}

// ── the command's body ───────────────────────────────────────────────────────

/// Everything `open_docs_viewer` does, minus the Tauri plumbing.
///
/// `config_dir` and `resource_dir` come from the app handle; `entries` is the
/// index the caller already walked. Returns the URL for the frontend to hand to
/// `openUrl`.
pub fn open(
    v: &Viewer,
    config_dir: &Path,
    resource_dir: Option<&Path>,
    entries: &[DocEntry],
    req: &Request,
) -> Result<String, String> {
    let bin = binary_path(resource_dir).ok_or_else(|| {
        "the documentation viewer is not installed with this app (Settings → Health reports it); \
         the Docs tab still lists and reads every document in place"
            .to_string()
    })?;
    let vdir = viewer_dir(config_dir);
    let state_dir = vdir.join("state");
    let log = vdir.join("viewer.log");
    std::fs::create_dir_all(&vdir).map_err(|e| format!("{}: {e}", vdir.display()))?;

    let mut slot = v.0.lock().unwrap();
    // Liveness the way `Shells` does it: ask the handle we hold, on every open,
    // and never a timer. A viewer that died takes the whole slot with it —
    // including the group map, because the next process knows nothing about what
    // the last one was serving.
    if let Some(p) = slot.as_mut() {
        if !matches!(p.child.try_wait(), Ok(None)) {
            *slot = None;
        }
    }
    if slot.is_none() {
        *slot = Some(spawn(&bin, &state_dir, &log)?);
        // A fresh viewer serves nothing yet, so this is the moment the old
        // trees stop being anybody's. They are COPIES of the user's documents —
        // the ones §4.3 says carry a client's signed agreement — sitting outside
        // the repo's own gitignore in a directory Spotlight and Time Machine
        // both index, so they do not get to outlive the process that needed
        // them. `cleanup` does the same on a clean exit; this is what covers a
        // crash, which is the case a shutdown hook cannot.
        empty_dir(&vdir.join("tree"));
    }
    let proc = slot.as_mut().expect("spawned or returned");
    let port = proc.port;

    let root = req.root.to_path_buf();
    let group = group_name(req.slug, &root, &proc.groups);
    let tree = vdir.join("tree").join(tree_key(req.slug, &root));

    let derived = write_tree(&tree, entries, &req.stale, port, &group)?;
    if !proc.groups.iter().any(|(r, _)| *r == root) {
        register(&bin, &state_dir, port, &group, &tree)?;
        proc.groups.push((root, group.clone()));
    }

    let Some(want) = req.path else { return Ok(group_url(port, &group)) };
    let hit = derived.iter().find(|(i, _)| entries[*i].path == want);
    match hit {
        Some((_, p)) => Ok(file_url(port, &group, p)),
        None => Err(format!("{want} is not in this place's documentation index")),
    }
}

/// The derived tree's directory name for one place: readable, and unique.
///
/// The slug alone would collide across projects the way the group name would;
/// the hash alone would make the directory unreadable when someone goes looking
/// at what the app wrote. Both, so a stray tree can be traced back to a place by
/// eye.
fn tree_key(slug: &str, root: &Path) -> String {
    let mut s: String = slug
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .take(40)
        .collect();
    if s.is_empty() {
        s = "place".into();
    }
    format!("{s}-{}", id8(root.as_os_str().as_encoded_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rel: &str, title: &str, root: &Path) -> DocEntry {
        DocEntry {
            path: root.join(rel).to_string_lossy().into_owned(),
            rel: rel.to_string(),
            title: title.to_string(),
            group: String::new(),
        }
    }

    fn stale() -> Staleness {
        Staleness {
            place: "viewer-guarded".into(),
            branch: Some("viewer-guarded".into()),
            behind: Some(25),
            base: "origin/main".into(),
            dirty: Some(false),
            dirty_files: Some(0),
            last_commit_subject: Some("feat(app): the viewer".into()),
            last_commit_epoch: Some(1_000_000),
            now_epoch: 1_000_000 + 60,
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-viewer-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // ── the gate ────────────────────────────────────────────────────────────

    /// The whole of §11.3, as one assertion. A viewer that SERVES a request
    /// carrying a foreign Host is a viewer any web page can read, and the shape
    /// this must never take is "200 looked fine, so we carried on".
    #[test]
    fn only_a_refusal_passes_the_host_probe() {
        assert!(probe_verdict("HTTP/1.1 403 Forbidden").is_ok());
        assert!(probe_verdict("HTTP/1.0 403 Forbidden\r\n").is_ok());
        for bad in [
            "HTTP/1.1 200 OK",
            "HTTP/1.1 404 Not Found",
            "HTTP/1.1 405 Method Not Allowed",
            "HTTP/1.1 500 Internal Server Error",
        ] {
            assert!(probe_verdict(bad).is_err(), "{bad} must not pass the gate");
        }
        // Not an answer at all. "I could not tell" and "it refused" must not
        // collapse into the same branch — only one of them is safe.
        //
        // The last two are why the status line is checked for `HTTP/` at all:
        // the port was chosen by us but bound by something else in the window
        // between the probe and the bind (`pick_port`), and a protocol that
        // greets rather than answers can put `403` in the second field by pure
        // coincidence. Without those two cases the version check is dead code
        // and a mutation that deletes it passes — which is how they got here.
        for junk in ["", "403", "HTTP/1.1", "hello", "\0\0\0", "SSH-2.0-OpenSSH 403 x", "RFB 403 x"] {
            assert!(probe_verdict(junk).is_err(), "{junk:?} must not pass the gate");
        }
    }

    /// The message a failed gate produces has to say what happened, because the
    /// only place it is ever read is a toast in the app.
    #[test]
    fn a_served_probe_says_what_it_means() {
        let e = probe_verdict("HTTP/1.1 200 OK").unwrap_err();
        assert!(e.contains("200"), "{e}");
        assert!(e.contains("403"), "{e}");
    }

    // ── URLs ────────────────────────────────────────────────────────────────

    /// The id is `mo`'s, verified against a running server rather than read out
    /// of its source: `sha256(<absolute path>)[..8]`.
    #[test]
    fn a_file_url_carries_the_viewers_own_path_hash() {
        let p = Path::new("/tmp/x/a.md");
        let want = {
            let d = Sha256::digest(p.as_os_str().as_encoded_bytes());
            d.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>()
        };
        assert_eq!(file_url(6275, "g", p), format!("http://127.0.0.1:6275/g?file={want}"));
        assert_eq!(want.len(), 8);
    }

    /// Every URL this module emits has to survive the check `derive` applies on
    /// the way into a `click` directive — otherwise the drill-down silently
    /// emits nothing and every test still passes.
    #[test]
    fn every_url_we_build_is_a_loopback_target() {
        let p = Path::new("/tmp/x/a.md");
        assert!(derive::is_loopback_target(&file_url(6275, "docs", p)));
        assert!(derive::is_loopback_target(&group_url(6275, "docs")));
        assert!(derive::is_loopback_target(&file_url(65535, "a-b-c", p)));
    }

    // ── group names ─────────────────────────────────────────────────────────

    /// Two places may share a slug — the same branch name in two projects — and
    /// `mo` MERGES same-named groups rather than refusing them, so a collision
    /// shows one project's documents under the other's name.
    #[test]
    fn two_places_with_one_slug_do_not_share_a_group() {
        let a = PathBuf::from("/w/one/.worktrees/docs");
        let b = PathBuf::from("/w/two/.worktrees/docs");
        let mut taken = Vec::new();
        let ga = group_name("docs", &a, &taken);
        taken.push((a.clone(), ga.clone()));
        let gb = group_name("docs", &b, &taken);
        assert_eq!(ga, "docs");
        assert_ne!(ga, gb, "a second place must not land in the first one's group");
        assert!(gb.starts_with("docs-"), "{gb}");
        // And asking again for a place already registered gives the same name,
        // or a re-open would keep making new groups.
        taken.push((b.clone(), gb.clone()));
        assert_eq!(group_name("docs", &a, &taken), ga);
        assert_eq!(group_name("docs", &b, &taken), gb);
    }

    /// `mo` reserves `/_/` for its own API and a group is a URL path segment, so
    /// the name may only ever be `[a-z0-9-]` — and may never be empty.
    #[test]
    fn a_group_name_can_only_be_a_url_path_segment() {
        let r = PathBuf::from("/w/p");
        for slug in ["_", "../etc", "a/b", "%2e%2e", "", "   ", "Feature/PLACE.1", "ünïcødé"] {
            let g = group_name(slug, &r, &[]);
            assert!(!g.is_empty(), "{slug:?} gave an empty group");
            assert!(g.len() <= GROUP_MAX, "{slug:?} gave {g}");
            assert!(
                g.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{slug:?} gave {g}"
            );
            assert!(g.starts_with(|c: char| c.is_ascii_alphanumeric()), "{slug:?} gave {g}");
        }
        assert_eq!(group_name("Feature/PLACE.1", &r, &[]), "feature-place-1");
    }

    // ── writing ─────────────────────────────────────────────────────────────

    /// The walk's entries are safe by construction; this is the one place in the
    /// feature that CREATES a file, and the guarantee and the use are far enough
    /// apart that the check belongs at the write.
    #[test]
    fn a_rel_that_could_leave_the_tree_is_never_written() {
        for bad in [
            "",
            "/etc/passwd",
            "../outside.md",
            "docs/../../outside.md",
            "docs//a.md",
            "./a.md",
            "a\\b.md",
            "C:/x.md",
            "a\0b.md",
        ] {
            assert!(safe_rel(bad).is_none(), "{bad:?} must not be written");
        }
        for ok in ["README.md", "docs/adr/0001.md", ".planning/brief.md", "a-b_c.md"] {
            assert_eq!(safe_rel(ok), Some(ok));
        }
    }

    /// The tree is what the viewer serves, so it has to carry the header and the
    /// derived links, and it has to MIRROR the place's layout — `mo` resolves a
    /// prose `[x](./x.md)` against the file's own directory, so flattening the
    /// tree would break every relative link in the user's documents.
    #[test]
    fn the_derived_tree_mirrors_the_place_and_carries_the_header() {
        let src = tmp("mirror");
        std::fs::create_dir_all(src.join("docs/adr")).unwrap();
        std::fs::write(src.join("README.md"), "---\ntitle: Read me\n---\n\nbody\n").unwrap();
        std::fs::write(src.join("docs/adr/0001.md"), "# ADR\n\n[back](../../README.md)\n").unwrap();
        let entries =
            vec![entry("README.md", "Read me", &src), entry("docs/adr/0001.md", "ADR", &src)];
        let out = tmp("mirror-out");

        let derived = write_tree(&out, &entries, &stale(), 6275, "g").unwrap();
        assert_eq!(derived.len(), 2);
        assert!(out.join("README.md").is_file());
        assert!(out.join("docs/adr/0001.md").is_file());

        let readme = std::fs::read_to_string(out.join("README.md")).unwrap();
        assert!(readme.starts_with("> **viewer-guarded**"), "{readme}");
        assert!(readme.contains("25 behind `origin/main`"), "{readme}");
        // Frontmatter gone, the title it carried restored as a heading.
        assert!(!readme.contains("---\ntitle:"), "{readme}");
        assert!(readme.contains("# Read me"), "{readme}");
        // Prose links are untouched — the mirror is what makes them resolve.
        let adr = std::fs::read_to_string(out.join("docs/adr/0001.md")).unwrap();
        assert!(adr.contains("[back](../../README.md)"), "{adr}");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A diagram node that names a page becomes a link to OUR url — the whole
    /// point of generating a tree instead of serving the repo.
    #[test]
    fn a_diagram_node_links_to_the_derived_page() {
        let src = tmp("click");
        std::fs::create_dir_all(src.join("docs")).unwrap();
        std::fs::write(
            src.join("docs/overview.md"),
            "# Overview\n\n```mermaid\nflowchart TD\n  runbook --> x\n  click runbook \"http://evil.example/\"\n```\n",
        )
        .unwrap();
        std::fs::write(src.join("docs/runbook.md"), "# Runbook\n").unwrap();
        let entries = vec![
            entry("docs/overview.md", "Overview", &src),
            entry("docs/runbook.md", "Runbook", &src),
        ];
        let out = tmp("click-out");
        let derived = write_tree(&out, &entries, &stale(), 6275, "g").unwrap();

        let text = std::fs::read_to_string(out.join("docs/overview.md")).unwrap();
        assert!(!text.contains("evil.example"), "an author's click survived: {text}");
        let want = file_url(6275, "g", &derived[1].1);
        assert!(text.contains(&format!("click runbook href \"{want}\"")), "{text}");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// §2.7 — a document that cannot be rendered is a page that SAYS so, next to
    /// the ones that did. Dropping the row instead would make the viewer
    /// disagree with the index the user just clicked in.
    #[test]
    fn an_unreadable_document_becomes_a_page_that_says_why() {
        let src = tmp("stub");
        std::fs::write(src.join("bad.md"), [0xffu8, 0xfe, 0xfd]).unwrap();
        let entries = vec![entry("bad.md", "bad", &src)];
        let out = tmp("stub-out");
        write_tree(&out, &entries, &stale(), 6275, "g").unwrap();
        let text = std::fs::read_to_string(out.join("bad.md")).unwrap();
        assert!(text.contains("not valid UTF-8"), "{text}");
        assert!(text.starts_with("> **viewer-guarded**"), "{text}");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A document removed from the place must leave the tree, or the viewer goes
    /// on serving a page the index no longer lists — a stale document with a
    /// staleness header on it, which is the joke this feature cannot afford.
    #[test]
    fn a_document_that_left_the_index_leaves_the_tree() {
        let src = tmp("prune");
        std::fs::create_dir_all(src.join("docs")).unwrap();
        std::fs::write(src.join("a.md"), "# A\n").unwrap();
        std::fs::write(src.join("docs/b.md"), "# B\n").unwrap();
        let out = tmp("prune-out");

        let both = vec![entry("a.md", "A", &src), entry("docs/b.md", "B", &src)];
        write_tree(&out, &both, &stale(), 6275, "g").unwrap();
        assert!(out.join("docs/b.md").is_file());

        write_tree(&out, &both[..1], &stale(), 6275, "g").unwrap();
        assert!(out.join("a.md").is_file());
        assert!(!out.join("docs/b.md").exists(), "a dropped document stayed in the tree");
        assert!(!out.join("docs").exists(), "the directory it emptied stayed");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// `mo`'s session file is keyed by PORT and restores whatever that port
    /// served last time, so a port the OS hands us twice would resurrect a place
    /// the user has removed.
    #[test]
    fn the_viewers_state_is_emptied_before_it_can_restore_anything() {
        let d = tmp("state");
        std::fs::create_dir_all(d.join("mo/backup")).unwrap();
        std::fs::write(d.join("mo/backup/mo-6275.json"), "{\"groups\":{}}").unwrap();
        std::fs::write(d.join("loose"), "x").unwrap();
        empty_dir(&d);
        assert!(d.is_dir(), "the directory itself must survive");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    // ── the whole thing, against a real viewer ──────────────────────────────

    /// Spawn, gate, register, resolve — end to end, with the actual binary.
    ///
    /// **Runs only when `WORKTREES_VIEWER_BIN` names one.** CI has no viewer
    /// (it is 26 MB of someone else's Go, built in `release.yml` and never
    /// committed), and the alternative — a fake server that speaks just enough
    /// HTTP — would be a second answer to the question this test exists to ask.
    /// Everything above it is pure and always runs; this is the one that proves
    /// the process, the port and the URL are real, and it is run by hand:
    ///
    /// ```sh
    /// WORKTREES_VIEWER_BIN=~/workspace/mo/mo cargo test -p app --lib the_whole_path -- --nocapture
    /// ```
    ///
    /// What it holds that nothing else can: that `--foreground` really keeps the
    /// child ours (kill it, and the port goes), that the §11.3 gate passes
    /// against the binary we ship rather than against a string, and that the URL
    /// we hand `openUrl` addresses the DERIVED document — header and all — and
    /// not the repo's own file.
    #[test]
    fn the_whole_path_works_against_a_real_viewer() {
        if std::env::var(BIN_ENV).is_err() {
            return;
        }
        let src = tmp("e2e");
        std::fs::create_dir_all(src.join("docs")).unwrap();
        std::fs::write(src.join("README.md"), "---\ntitle: Read me\n---\n\nhello\n").unwrap();
        let entries = vec![entry("README.md", "Read me", &src)];
        let cfg = tmp("e2e-cfg");
        let v = Viewer::default();
        let req = Request { root: &src, slug: "e2e-place", path: Some(&entries[0].path), stale: stale() };

        // A tree left behind by a previous run — a crash, or a place the user
        // has since removed. The first spawn of a run must take it with it.
        let stale_tree = viewer_dir(&cfg).join("tree/gone-deadbeef");
        std::fs::create_dir_all(&stale_tree).unwrap();
        std::fs::write(stale_tree.join("secret.md"), "a client's signed agreement").unwrap();

        let url = open(&v, &cfg, None, &entries, &req).expect("the viewer must come up");
        assert!(!stale_tree.exists(), "a previous run's derived documents survived a fresh spawn");
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        assert!(derive::is_loopback_target(&url), "{url}");
        let port = v.0.lock().unwrap().as_ref().unwrap().port;

        // The gate, against the running process — not against `probe_verdict`.
        probe_refuses_foreign_host(port).expect("the shipped viewer must refuse a foreign Host");

        // And the URL addresses OUR document: fetch what the viewer would serve
        // and find the header in it.
        let id = url.rsplit("file=").next().unwrap().to_string();
        let body = http_get(port, &format!("/_/api/groups/e2e-place/files/{id}/content"));
        assert!(body.contains("25 behind"), "the derived header is not in what the viewer serves: {body}");
        assert!(!body.contains("title: Read me"), "the frontmatter survived: {body}");

        // `--foreground` means this child is ours. Without it the handle is a
        // launcher that already exited and the port would outlive the app.
        kill(&v);
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), PROBE_TIMEOUT).is_err(),
            "the viewer survived being killed — it daemonised and we are holding the wrong process"
        );
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&cfg);
    }

    /// The derived tree is copies of the user's documents outside the repo's own
    /// gitignore. A viewer that is dead has no business still having them.
    #[test]
    fn nothing_the_viewer_wrote_outlives_a_clean_exit() {
        let cfg = tmp("cleanup");
        let v = viewer_dir(&cfg);
        std::fs::create_dir_all(v.join("tree/a-1234abcd/docs")).unwrap();
        std::fs::write(v.join("tree/a-1234abcd/docs/x.md"), "a client's signed agreement").unwrap();
        std::fs::create_dir_all(v.join("state/mo/backup")).unwrap();
        std::fs::write(v.join("state/mo/backup/mo-6275.json"), "{}").unwrap();
        // The log is kept on purpose: it is how a failed spawn is explained, and
        // it holds no document content.
        std::fs::write(v.join("viewer.log"), "serving\n").unwrap();

        cleanup(&cfg);
        assert_eq!(std::fs::read_dir(v.join("tree")).unwrap().count(), 0, "derived documents survived");
        assert_eq!(std::fs::read_dir(v.join("state")).unwrap().count(), 0, "the session survived");
        assert!(v.join("viewer.log").is_file(), "the log is not the viewer's state");
        let _ = std::fs::remove_dir_all(&cfg);
    }

    /// A loopback GET, for the test above. Not worth a dependency.
    #[cfg(test)]
    fn http_get(port: u16, path: &str) -> String {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut s = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).unwrap();
        s.set_read_timeout(Some(PROBE_TIMEOUT)).unwrap();
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Detection is a STAT. Executing a candidate to find out whether it is
    /// there would put an exec of an unknown binary on the path rule 2 exists to
    /// keep clear.
    #[test]
    fn a_missing_viewer_is_absent_rather_than_an_error() {
        let d = tmp("bin");
        assert!(binary_path(Some(&d)).is_none());
        assert!(binary_path(None).is_none());
        std::fs::create_dir_all(d.join("viewer")).unwrap();
        // A DIRECTORY where the binary should be is still "not installed".
        std::fs::create_dir_all(d.join(RESOURCE_REL)).unwrap();
        assert!(binary_path(Some(&d)).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}

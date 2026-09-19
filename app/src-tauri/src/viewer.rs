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

/// How long the registration client gets. It is not the startup path — the
/// server is already up and proven — but it does scale with the place: the
/// index's cap is 2,000 documents and the client waits while the server reads a
/// title out of each. Deliberately several times `PROBE_TIMEOUT`, which talks to
/// a process that has nothing to do.
const REGISTER_DEADLINE_SECS: u64 = 15;

/// Shutting down is a single request to a server that is already listening, so
/// it needs none of `REGISTER_DEADLINE_SECS`' room for a place with 2,000
/// documents in it. Short enough that a wedged one does not hold the open path.
const SHUTDOWN_DEADLINE_SECS: u64 = 5;

/// Per-document read cap, matching `read_file`'s. A document over this is not
/// silently dropped — it gets a stub page saying so (§2.7, "fail per file,
/// loudly"), because a row that opens onto nothing is the same silent wrongness
/// the staleness header exists to prevent.
const DOC_MAX_BYTES: u64 = 1_000_000;

/// Longest group name we will build. `mo` serves a group at `/{name}`, so this
/// is a URL path segment; the length is cosmetic, the character set is not.
const GROUP_MAX: usize = 48;

/// What a document may bring with it into the derived tree.
///
/// An ALLOW-LIST, never a denylist: the set of things a browser will execute
/// grows, and a denylist written today is a list of the attacks that were known
/// today. Matched on the extension because that is also what the viewer matches
/// on when it decides what to serve and with which content type — agreeing with
/// it is the point.
///
/// **SVG is deliberately absent, and adding it is not a fix.** An SVG is a live
/// document: it can carry `<script>`, `<foreignObject>` and external references.
/// Inside an `<img>` it is script-inert by spec, which is the reading that makes
/// "it's just an image" sound true — but `mo` serves this tree over HTTP and
/// hands any raw asset back by direct URL
/// (`/_/api/groups/{g}/files/{id}/raw/{path}` → `http.ServeFile`), where it is a
/// top-level document with `image/svg+xml` on it and script runs. We do not
/// control that content type, the port has no authentication, and the documents
/// reached through it carry a client's signed agreement (§4.3). So: rasters
/// only. A place's `logo.svg` stays broken on purpose, and the way to fix that
/// is a viewer that serves assets with a content type we chose — not a seventh
/// entry in this array.
const ASSET_EXTS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp"];

/// Per-image cap. Ten times `DOC_MAX_BYTES`, because a screenshot is not a
/// document and 1 MB is an ordinary PNG; past this it is a video frame or a
/// mistake, and either way the copy is not worth making on a tick.
const ASSET_MAX_BYTES: u64 = 10_000_000;

/// Per-place cap on everything copied. The derived tree is written on every
/// re-derive, which happens whenever any document in the place moves, so this
/// is not a disk limit so much as a bound on what one edit can cost.
const ASSETS_MAX_TOTAL: u64 = 64_000_000;

/// Hard cap on how many distinct references one place may turn into a stat.
///
/// Every candidate is stat'ed on the derive AND folded into the fingerprint, so
/// this bounds the tick as well as the copy — and the list comes out of
/// documents, which means its length is chosen by the repo rather than by us.
/// `MAX_ENTRIES` is the same rule one level up.
const ASSET_MAX_FILES: usize = 500;

/// Where the viewer binary is looked for, relative to the bundle's resource
/// directory. `release.yml` puts it here and signs it.
const RESOURCE_REL: &str = "viewer/mo";

/// Development / test escape hatch: an absolute path to a viewer binary. This is
/// the USER's environment, never a cloned repo's file, so ADR 0001 is untouched
/// — the repo still cannot name a program to run. It exists because `tauri dev`
/// has no bundle and therefore no resource directory.
const BIN_ENV: &str = "WORKTREES_VIEWER_BIN";

// ── the managed child ────────────────────────────────────────────────────────

/// One place registered with the running viewer, and everything the tick needs
/// to re-derive it without the frontend saying anything.
///
/// The map used to be `(root, group)` — enough to keep two places from
/// colliding into one `mo` group. It is now everything `write_tree` takes,
/// because the browser tab has to keep up with the documents on its own: `mo`
/// watches the DERIVED tree (`-wR`), not the repo, so its live-reload was
/// reloading a copy that could only change when the button was pressed again.
struct Group {
    /// Canonical place root — the walk's root, and the identity of this entry.
    root: PathBuf,
    /// The place's slug. Names the tree directory and is the first fact in the
    /// header; kept so an error about this group can name it.
    slug: String,
    /// The `mo` group serving it, for THIS process only.
    name: String,
    /// Where the derived copy lives.
    tree: PathBuf,
    /// The `[docs]` section the index was walked with, as it stood at the last
    /// OPEN.
    ///
    /// Held rather than re-read, so the tick stays stat-only: re-parsing
    /// `.worktrees.toml` per place per tick would put a file read and a TOML
    /// parse back on the path this whole design exists to keep cheap, and
    /// `list_docs` — which the Docs tab polls every 4 s — already shows the new
    /// configuration in the dock immediately. The cost is that a `[docs]` EDIT
    /// does not reach the browser copy until the next open; a document edit,
    /// which is what changes minute to minute, reaches it on the next tick.
    docs: Option<worktrees_core::projcfg::Docs>,
    /// The facts the header states, from the open that registered this place.
    /// `derived_epoch` is re-stamped on every re-derive; nothing else here is
    /// re-measured, because every other field costs a git fan-out (§15.3).
    stale: Staleness,
    /// `docs::fingerprint_with`, with this place's images folded in
    /// (`docs::fold_assets`), as of the last derive. The tick re-derives when
    /// and only when this moves.
    fingerprint: u64,
    /// The images the last derive considered, relative to `root` — copied ones
    /// and referenced-but-missing ones alike.
    ///
    /// Held for the fingerprint, which has no other way to learn that a
    /// screenshot changed: the walk lists markdown by design, so without this
    /// list an edited image moves nothing and the browser keeps serving the
    /// copy made at click time. See `docs::fold_assets` for what that does and
    /// does not cover.
    assets: Vec<String>,
}

/// One running viewer, and the places already registered with it.
struct Proc {
    child: std::process::Child,
    port: u16,
    /// The places this process is serving.
    ///
    /// Registration is idempotent in `mo` (verified: re-running the client
    /// against the same directory leaves the file count unchanged), so this list
    /// is not correctness for the registration — it is the record of which names
    /// are taken, which `group_name` needs in order to keep two places from
    /// colliding into one group and showing each other's documents, and it is
    /// the tick's whole work list.
    groups: Vec<Group>,
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

/// Drop one place's derived documents, because the place itself is gone.
///
/// Called from `remove_place`, and it closes a hole that is small but exactly
/// the wrong shape for this feature: after `worktrees rm` the worktree's
/// documents are deleted from disk, and the derived copy would be the LAST
/// readable copy of them — served, on a port, until the app quit. A staleness
/// viewer serving documents that no longer exist anywhere is the joke this whole
/// proposal is built to avoid.
///
/// Removing the tree is a complete deregistration and needs no second call.
/// `mo` watches the directory (`-wR`), and it drops what leaves it: verified —
/// two seconds after the tree was removed the group listed zero files and the
/// content endpoint answered 404.
///
/// Best-effort and silent: a place with no tree (never opened in the browser) is
/// the common case, not an error.
pub fn forget_place(config_dir: &Path, slug: &str, canonical_root: &Path) {
    let dir = viewer_dir(config_dir).join("tree").join(tree_key(slug, canonical_root));
    let _ = std::fs::remove_dir_all(dir);
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
    /// The `[docs]` section `entries` was walked with, so the tick can walk the
    /// same set of files. Cloned into the group's record — see `Group::docs`.
    pub docs: Option<worktrees_core::projcfg::Docs>,
    /// `docs::fingerprint_with` over this place, taken by the caller **before**
    /// it walked the index.
    ///
    /// Before, not after, and that is the one ordering rule in this feature: a
    /// document written between the two walks is then in the index but not in
    /// the fingerprint, so the next tick sees a difference and re-derives —
    /// one wasted pass. Taken afterwards, the same write would be sealed in as
    /// "already derived" and that place's browser copy would sit one edit behind
    /// until something else changed, which is the failure this is being built to
    /// remove, reintroduced inside its own registration.
    pub fingerprint: u64,
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

/// One derived tree, as it stands after a write.
pub struct Derived {
    /// The derived path of each entry, in index order — what `open` resolves a
    /// requested document against.
    pages: Vec<(usize, PathBuf)>,
    /// Every image this place's documents ask for, relative to the place root:
    /// what was copied, plus what was NOT because it is missing. Both belong in
    /// the fingerprint — a missing image that appears later has to re-derive,
    /// and it is the only way that transition is ever noticed (`fold_assets`).
    assets: Vec<String>,
    /// One line per reference the copy refused, for the app log. Never an error
    /// return: §2.7 fails per file, and one unreadable screenshot may not cost
    /// the place its documents.
    notes: Vec<String>,
}

/// Write the whole derived tree for one place — every document, and the images
/// they reference.
///
/// Written IN PLACE rather than wiped and recreated. `mo` watches this directory
/// (`-wR`), and a delete-then-create cycle is two events per file on a watcher
/// that is also the thing keeping the user's open tab live; overwriting is one.
/// Files that are no longer in the index are pruned afterwards, which is the
/// only removal that has to happen at all.
///
/// `root` is the place itself, canonical, and it is here for the images: a
/// reference is resolved against it and every candidate has to prove it still
/// lies under it (`copy_assets`). It is passed rather than derived from an
/// entry's absolute path minus its `rel`, because that subtraction is a third
/// place that would have to agree with the walk about what a root is.
fn write_tree(
    dir: &Path,
    root: &Path,
    entries: &[DocEntry],
    stale: &Staleness,
    port: u16,
    group: &str,
) -> Result<Derived, String> {
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
    // Deduped and ordered: two documents in the same directory usually share
    // their screenshots, and a set means the file is stat'ed and copied once
    // however many pages point at it. Sorted, so what the cap below keeps is
    // the same set on every derive rather than whichever ones the walk reached
    // first.
    let mut refs: BTreeSet<String> = BTreeSet::new();
    let mut notes: Vec<String> = Vec::new();
    for (i, out) in &derived {
        let entry = &entries[*i];
        let body = render_one(Path::new(&entry.path), entry, stale, &links);
        // Asked of the RENDERED body, not of the file: the transform leaves
        // every image reference exactly as its author wrote it (that is why the
        // copy has to mirror the path at all), so the two texts give the same
        // answer — and reading the document a second time to ask the same
        // question is the second reader this feature keeps refusing. A stub page
        // has no references, which is also correct: nothing of it is shown.
        collect_refs(entry, &body, &mut refs, &mut notes);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(out, body).map_err(|e| format!("{}: {e}", out.display()))?;
        written.insert(out.clone());
    }
    // Before the prune, and into the same `written` set: a copied image is a
    // file in the tree like any other, so an image that stops being referenced
    // has to leave with the document that referenced it. Left out of the set it
    // would be deleted on the very next derive instead — the tree would work
    // once and then serve broken images — and left out of the prune entirely it
    // would sit there being served forever.
    let assets = copy_assets(dir, root, &refs, &mut written, &mut notes);
    prune(dir, &written);
    Ok(Derived { pages: derived, assets, notes })
}

/// The images one document asks for, as paths relative to the place root.
///
/// Layer A is `derive::asset_rel`; this adds the extension allow-list and
/// nothing else. A reference it drops is dropped SILENTLY, with one exception:
/// almost everything refused here is an ordinary `https://` image or an anchor
/// a document happens to define, and a log line per external image per derive
/// would bury the lines that mean something. The exception is SVG, which is
/// refused for a reason the author cannot guess from a broken image.
fn collect_refs(entry: &DocEntry, body: &str, out: &mut BTreeSet<String>, notes: &mut Vec<String>) {
    for r in derive::image_refs(body) {
        let Some(rel) = derive::asset_rel(&entry.rel, &r) else { continue };
        match ext_of(&rel) {
            e if ASSET_EXTS.contains(&e.as_str()) => {
                out.insert(rel);
            }
            e if e == "svg" => notes.push(format!(
                "{rel}: an SVG is not copied into the derived tree — the viewer serves it by URL, \
                 where it is a document that can run script rather than an image"
            )),
            _ => {}
        }
    }
}

/// A path's extension, lowercased, or `""`. A file with no dot has no extension
/// — not the whole name as one, which would make `README` an allow-list
/// question.
fn ext_of(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Copy the images the documents reference into the derived tree, mirroring
/// each one's path relative to the place root. Returns every candidate it
/// considered — copied or missing — which is the list the fingerprint stats.
///
/// **Why the path is mirrored and the document is not rewritten.** The viewer
/// resolves a relative `src` against the directory of the markdown file it is
/// serving, so a copy at the same relative offset makes the author's own
/// reference resolve with nothing edited. A derived document that had its links
/// rewritten would no longer be a faithful copy of the file it claims to show,
/// on the one surface built to say how faithful it is.
///
/// **Layer B, and why Layer A is not enough.** `derive::asset_rel` has already
/// refused absolute paths, `~`, `$`, URL schemes, empty components, `.git` and
/// any `..` that escapes — but it is arithmetic on a string, and a string cannot
/// show a symlink. `docs/assets` may BE a link to `/`, in which case
/// `docs/assets/x.png` is textually innocent and reads somebody else's file. So
/// the candidate is `symlink_metadata`'d (never `metadata` — the same rule
/// `docs.rs` keeps, and for the same `docs/logo.png -> ~/.ssh/id_rsa`) and then
/// canonicalised and required to start with the canonical root, which resolves
/// every link in every parent component. The second check is the one that holds.
///
/// Every refusal is per file and never fatal (§2.7): one bad image may not cost
/// a place its documents.
fn copy_assets(
    dir: &Path,
    root: &Path,
    refs: &BTreeSet<String>,
    written: &mut BTreeSet<PathBuf>,
    notes: &mut Vec<String>,
) -> Vec<String> {
    // Canonical once, here: `starts_with` is a comparison of components, so a
    // root that still contains a symlink (`/tmp` is `/private/tmp` on this
    // platform) would fail to match its own files and quietly copy nothing.
    let Ok(canon_root) = std::fs::canonicalize(root) else {
        notes.push(format!("{}: the place could not be resolved; no images were copied", root.display()));
        return Vec::new();
    };
    let mut kept: Vec<String> = Vec::new();
    let mut total: u64 = 0;
    let mut over_total: usize = 0;
    for rel in refs {
        if kept.len() >= ASSET_MAX_FILES {
            notes.push(format!(
                "stopped after {ASSET_MAX_FILES} images; {} more were referenced and not copied",
                refs.len() - kept.len()
            ));
            break;
        }
        // Counted as a candidate even when the copy below refuses it: the
        // fingerprint stats this list, and an image that is missing TODAY is
        // exactly the one whose arrival tomorrow has to re-derive the place.
        kept.push(rel.clone());
        // The write-side shape check the documents get, for the same reason they
        // get it: this is where a path becomes a file we create.
        let Some(safe) = safe_rel(rel) else { continue };
        let src = canon_root.join(safe);
        // Missing is silent. A document that points at a file the repo does not
        // have is a broken image in the repo too, and saying so once per derive
        // per reference would be a log of somebody else's typos.
        let Ok(md) = std::fs::symlink_metadata(&src) else { continue };
        if !md.is_file() {
            notes.push(format!("{rel}: not a regular file — a symlinked image is never followed"));
            continue;
        }
        if md.len() > ASSET_MAX_BYTES {
            notes.push(format!(
                "{rel}: {} bytes is over the {ASSET_MAX_BYTES}-byte per-image cap and was not copied",
                md.len()
            ));
            continue;
        }
        if total.saturating_add(md.len()) > ASSETS_MAX_TOTAL {
            over_total += 1;
            continue;
        }
        // The check that actually holds — every parent component resolved.
        let Ok(canon) = std::fs::canonicalize(&src) else { continue };
        if !canon.starts_with(&canon_root) {
            notes.push(format!("{rel}: resolves to {}, which is outside the place", canon.display()));
            continue;
        }
        let dest = dir.join(safe);
        // Already current? Then leave it alone. `mo` watches this tree, so an
        // identical rewrite is a reload event in somebody's open tab for no
        // change at all — and a re-derive happens whenever ANY document in the
        // place moves, which on a place an agent is writing in is constantly.
        // The test is the fingerprint's own (length, and a copy no older than
        // its source); it can only be wrong in the direction the fingerprint is
        // already wrong in, and that window is documented there.
        //
        // Note what `modified()` means on each side here. On macOS
        // `std::fs::copy` goes through `copyfile`/`clonefile`, which carries the
        // source's mtime across, so a fresh copy compares EQUAL rather than
        // newer — which is why the test is `>=`. (It is also why a copy cannot
        // be detected by watching the destination's mtime; the test that guards
        // this skip had to mark the copy's bytes instead, and says so.)
        let fresh = std::fs::symlink_metadata(&dest)
            .ok()
            .filter(|d| d.is_file() && d.len() == md.len())
            .and_then(|d| Some((d.modified().ok()?, md.modified().ok()?)))
            .map(|(dst, srct)| dst >= srct)
            .unwrap_or(false);
        if !fresh {
            if let Some(parent) = dest.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    notes.push(format!("{rel}: {} could not be created: {e}", parent.display()));
                    continue;
                }
            }
            if let Err(e) = std::fs::copy(&src, &dest) {
                notes.push(format!("{rel}: could not be copied into the derived tree: {e}"));
                continue;
            }
        }
        total += md.len();
        written.insert(dest);
    }
    if over_total > 0 {
        notes.push(format!(
            "stopped at {total} bytes of images (cap {ASSETS_MAX_TOTAL}); {over_total} more were not copied"
        ));
    }
    kept
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
    resolve_binary(std::env::var(BIN_ENV).ok().as_deref(), resource_dir)
}

/// The resolution itself, with the environment as an ARGUMENT.
///
/// Split out so its test can state the precedence instead of depending on the
/// ambient environment — which it did, and which made the suite fail the moment
/// it was run the way the end-to-end test asks to be run
/// (`WORKTREES_VIEWER_BIN=… cargo test`). A test that passes only when nobody is
/// exercising the feature is not a test.
fn resolve_binary(from_env: Option<&str>, resource_dir: Option<&Path>) -> Option<PathBuf> {
    let is_file = |p: PathBuf| std::fs::metadata(&p).ok().filter(|m| m.is_file()).map(|_| p);
    // The override WINS, and a broken override does not fall through to the
    // bundle: someone who set it is testing that binary, and quietly running a
    // different one is how you spend an afternoon.
    if let Some(p) = from_env {
        return is_file(PathBuf::from(p));
    }
    is_file(resource_dir?.join(RESOURCE_REL))
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

    // The PATH in the message, not just the errno: "No such file or directory"
    // alone names nothing, and the two cases a user can act on — it is missing,
    // it is the wrong architecture — read identically without it.
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", bin.display()))?;
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
    // `run_deadline` sets stdout and stderr itself and spawns a thread to drain
    // each — setting them here would be overwritten, and the draining is the
    // point: a >64KB burst on an undrained pipe deadlocks `try_wait`, and this
    // client prints a line per file registered.
    //
    // This is a short-lived CLIENT: it talks to the server over loopback and
    // exits. It is not the supervised child, so it gets a deadline of its own
    // rather than a `try_wait` loop.
    let out = crate::run_deadline(cmd, REGISTER_DEADLINE_SECS)
        .map_err(|e| format!("registering {group} with the viewer: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let msg = String::from_utf8_lossy(&out.stderr);
    Err(format!("registering {group} with the viewer failed ({}): {}", out.status, msg.trim()))
}

/// Ask whatever is listening on `port` to shut down, and do not care much
/// whether it answers.
///
/// Only ever called about a server we did NOT spawn: `register` runs the
/// viewer binary a second time as a *client*, and `mo`'s client falls through
/// to starting its own daemonised server when it finds the port empty
/// (`cmd/root.go` `startBackground`, `setsid`). That process is not our child,
/// so `RunEvent::Exit` has never heard of it and `kill` cannot reach it — the
/// one way this app can leave an unauthenticated viewer listening after it
/// quits, which is precisely the failure `--foreground` exists to prevent,
/// arriving through the back door.
///
/// Best-effort by construction: if the shutdown fails there is nothing further
/// this process can do about a server it does not own, and the caller has
/// already failed the open for its own reasons.
fn shutdown_port(bin: &Path, state_dir: &Path, port: u16) {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["--shutdown", "--port"]);
    cmd.arg(port.to_string());
    cmd.env("XDG_STATE_HOME", state_dir);
    cmd.stdin(std::process::Stdio::null());
    let _ = crate::run_deadline(cmd, SHUTDOWN_DEADLINE_SECS);
}

// ── the command's body ───────────────────────────────────────────────────────

/// Everything `open_docs_viewer` does, minus the Tauri plumbing.
///
/// `config_dir` and `resource_dir` come from the app handle; `entries` is the
/// index the caller already walked. Returns the URL for the frontend to hand to
/// `openUrl`.
/// What one successful `open` produced: the URL for `openUrl`, and anything the
/// derive refused along the way.
///
/// The notes are not errors — the open succeeded — but they are the only record
/// that an image the reader is about to not-see was skipped on purpose, so the
/// caller logs them. Swallowing them would leave a missing screenshot with no
/// explanation anywhere in the app.
pub struct Opened {
    pub url: String,
    pub notes: Vec<String>,
}

pub fn open(
    v: &Viewer,
    config_dir: &Path,
    resource_dir: Option<&Path>,
    entries: &[DocEntry],
    req: &Request,
) -> Result<Opened, String> {
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
    // `group_name` stays a pure function over `(root, name)` pairs — the tie it
    // breaks has nothing to do with the rest of a `Group`, and its tests say so
    // in the shape they build.
    let taken: Vec<(PathBuf, String)> =
        proc.groups.iter().map(|g| (g.root.clone(), g.name.clone())).collect();
    let group = group_name(req.slug, &root, &taken);
    let tree = vdir.join("tree").join(tree_key(req.slug, &root));

    let derived = write_tree(&tree, &root, entries, &req.stale, port, &group)?;
    // The images this place brought with it are part of what the tick has to
    // watch, so the digest recorded below is the caller's (taken BEFORE the
    // walk, which is the one ordering rule here) with their stats folded in.
    let fingerprint = worktrees_core::docs::fold_assets(req.fingerprint, &root, &derived.assets);
    let pages = &derived.pages;
    // A group of no documents is not worth registering, and registering one
    // would put a watch pattern on an empty directory that nothing will ever
    // fill. The footer button is disabled on an empty index, so the way here is
    // the race: the index was walked 30 seconds ago and the place has been
    // emptied since. Say that, rather than opening a blank group.
    if pages.is_empty() {
        return Err(format!("{} has no documents to show", req.slug));
    }
    if !proc.groups.iter().any(|g| g.root == root) {
        // Re-checked HERE, not only at the top: `write_tree` above can take a
        // while on a large place, and `register` runs the binary again as a
        // CLIENT. A client that finds the port empty does not fail — it
        // daemonises a server of its own (`shutdown_port`). Asking the handle
        // first shrinks that window to the width of one `try_wait`.
        if !matches!(proc.child.try_wait(), Ok(None)) {
            *slot = None;
            return Err("the viewer exited before its documents could be registered.".to_string());
        }
        let outcome = register(&bin, &state_dir, port, &group, &tree);
        // …and again AFTER, because the window cannot be closed, only narrowed:
        // the child can die during the registration itself. If it did, the
        // client we just ran was talking to a server it started rather than to
        // ours, and that server is nobody's child. Shut the port down instead
        // of leaving it listening, then fail the open — a success here would
        // hand the user a URL into a process this app can never kill.
        if !matches!(proc.child.try_wait(), Ok(None)) {
            shutdown_port(&bin, &state_dir, port);
            *slot = None;
            return Err("the viewer exited while registering documents.".to_string());
        }
        outcome?;
        proc.groups.push(Group {
            root: root.clone(),
            slug: req.slug.to_string(),
            name: group.clone(),
            tree: tree.clone(),
            docs: req.docs.clone(),
            stale: req.stale.clone(),
            fingerprint,
            assets: derived.assets.clone(),
        });
    }
    // Every open re-derives, registered or not (`write_tree` above), so the
    // record has to be brought forward on the re-open path too — otherwise a
    // place opened twice keeps the FIRST open's fingerprint and facts, and the
    // tick compares today's documents against a digest from yesterday. It would
    // re-derive once and then settle, which is the worst version of this bug:
    // correct-looking, and wrong by exactly one stale header.
    if let Some(g) = proc.groups.iter_mut().find(|g| g.root == root) {
        g.docs = req.docs.clone();
        g.stale = req.stale.clone();
        g.fingerprint = fingerprint;
        g.assets = derived.assets.clone();
    }

    let notes = derived.notes;
    let Some(want) = req.path else { return Ok(Opened { url: group_url(port, &group), notes }) };
    let hit = pages.iter().find(|(i, _)| entries[*i].path == want);
    match hit {
        Some((_, p)) => Ok(Opened { url: file_url(port, &group, p), notes }),
        None => Err(format!("{want} is not in this place's documentation index")),
    }
}

/// Re-derive every registered place whose documents have moved on disk.
///
/// **Why this exists.** `mo` watches the DERIVED tree (`-wR`), not the user's
/// repo, so its live-reload was perfect over a copy that could only change when
/// the button was pressed again: an edit to a real file, and a file added since
/// the open, both reached nothing, and refreshing the browser did not help
/// because the derived document genuinely had not changed. The dock, meanwhile,
/// re-indexes every 4 s and says "1 document new" beside a tab showing the copy
/// made at click time.
///
/// **The cost is the whole design.** `index_with` sniffs 8 KiB of every file for
/// its title — measured at 10.1 ms and ~3.2 MB of reads for a 400-document place
/// — and doing that per registered place every few seconds is not acceptable.
/// `docs::fingerprint_with` walks the same paths with `symlink_metadata` alone
/// (1.8 ms, zero bytes read, on the same tree) and the full walk runs only when
/// it has moved.
///
/// Four rules, all of them `Shells`' rules, because this is the same kind of
/// child:
///
/// 1. **Nothing registered, nothing done.** No lock contention, no walk, and in
///    particular no spawn — a timer may not decide the user wants a viewer.
/// 2. **Never resurrect a dead one.** `try_wait` says whether the child is
///    alive; if it is not, the slot is dropped and the next OPEN spawns. A timer
///    that respawned would put an unauthenticated port back up minutes after the
///    user stopped using it, with nothing on screen to say so.
/// 3. **A failed derive keeps its old fingerprint**, so it is retried on the
///    next tick rather than recorded as done. The caller must dedupe the log
///    (lib.rs does): a place that fails will fail every 3 s.
/// 4. **Only `derived_epoch` moves.** Every other fact in the header costs a git
///    fan-out, which §15.3 refused for a click and which is worse on a timer —
///    so the page says when it was copied and, separately, when its status was
///    measured. See `Staleness::derived_epoch`.
///
/// Returns one message per place that could not be re-derived, plus one per
/// image a derive refused to copy — the same lines `Opened::notes` carries on
/// the click path, for the same reason: a screenshot that is skipped on purpose
/// must be findable somewhere other than in this source file. Never panics on
/// a poisoned lock's account — this runs on the app's tick thread, and taking
/// the process down over a documentation copy is not a trade worth making.
pub fn refresh(v: &Viewer, now_epoch: i64) -> Vec<String> {
    let mut errs: Vec<String> = Vec::new();
    let Ok(mut slot) = v.0.lock() else {
        return vec!["the viewer's lock is poisoned; documents will not refresh".to_string()];
    };
    // Rule 1: a viewer nobody has opened is the common case, and it must cost
    // nothing at all.
    let Some(proc) = slot.as_mut() else { return errs };
    // Rule 2: ask the handle, the way every read of `Shells` does.
    if !matches!(proc.child.try_wait(), Ok(None)) {
        *slot = None;
        return errs;
    }
    let port = proc.port;
    for g in proc.groups.iter_mut() {
        let docs_fp = worktrees_core::docs::fingerprint_with(&g.root, g.docs.as_ref());
        // The images the last derive knew about, stat'ed alongside the
        // documents. Without them an edited screenshot moves nothing — the walk
        // lists markdown by design — and the tab goes on showing the version
        // that was current when the button was pressed, which is this feature's
        // own failure wearing a different hat.
        let fp = worktrees_core::docs::fold_assets(docs_fp, &g.root, &g.assets);
        if fp == g.fingerprint {
            continue;
        }
        let idx = worktrees_core::docs::index_with(&g.root, g.docs.as_ref());
        let mut stale = g.stale.clone();
        stale.derived_epoch = now_epoch;
        match write_tree(&g.tree, &g.root, &idx.entries, &stale, port, &g.name) {
            Ok(d) => {
                // Folded over the NEW list rather than storing `fp`: this derive
                // may have added or dropped an image, and recording a digest
                // taken over the old list would differ from the next tick's for
                // no change at all — one re-derive per asset change, forever.
                g.fingerprint = worktrees_core::docs::fold_assets(docs_fp, &g.root, &d.assets);
                g.assets = d.assets;
                g.stale = stale;
                for n in d.notes {
                    errs.push(format!("{}: {n}", g.slug));
                }
            }
            // Rule 3: the fingerprint is NOT advanced, so this is tried again.
            Err(e) => errs.push(format!("{}: {e}", g.slug)),
        }
    }
    errs
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
            mtime_ms: 0,
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
            // The open path's case: the facts were read and the copy was made
            // in the same breath. `refresh` is what moves them apart, and the
            // test that asserts it says so where it sets them.
            derived_epoch: 1_000_000 + 60,
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

        let derived = write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap().pages;
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
        let derived = write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap().pages;

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
        write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap();
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
        write_tree(&out, &src, &both, &stale(), 6275, "g").unwrap();
        assert!(out.join("docs/b.md").is_file());

        write_tree(&out, &src, &both[..1], &stale(), 6275, "g").unwrap();
        assert!(out.join("a.md").is_file());
        assert!(!out.join("docs/b.md").exists(), "a dropped document stayed in the tree");
        assert!(!out.join("docs").exists(), "the directory it emptied stayed");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A removed place's derived documents are the LAST readable copy of them —
    /// `cmd_rm` deleted the originals — and they would be on a port. Keyed by
    /// the canonical root, so the same slug in another project keeps its own.
    #[test]
    fn removing_a_place_takes_its_derived_documents_with_it() {
        let cfg = tmp("forget");
        let gone = PathBuf::from("/w/one/.worktrees/docs");
        let other = PathBuf::from("/w/two/.worktrees/docs");
        let trees = viewer_dir(&cfg).join("tree");
        for root in [&gone, &other] {
            let d = trees.join(tree_key("docs", root));
            std::fs::create_dir_all(d.join("sub")).unwrap();
            std::fs::write(d.join("sub/x.md"), "a client's signed agreement").unwrap();
        }
        assert_eq!(std::fs::read_dir(&trees).unwrap().count(), 2);

        forget_place(&cfg, "docs", &gone);
        assert!(!trees.join(tree_key("docs", &gone)).exists(), "the removed place's documents stayed");
        assert!(trees.join(tree_key("docs", &other)).is_file() || trees.join(tree_key("docs", &other)).is_dir(),
            "the same slug in another project lost its tree");
        // And a place that was never opened in the browser has no tree at all,
        // which is the common case and not an error.
        forget_place(&cfg, "never-opened", &gone);
        let _ = std::fs::remove_dir_all(&cfg);
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

    // ── images ──────────────────────────────────────────────────────────────

    /// A 1×1 PNG. Real bytes rather than a placeholder, because the end-to-end
    /// test below hands them to a server that decides a content type.
    const PNG: [u8; 67] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
        0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
        0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
        0x42, 0x60, 0x82,
    ];

    fn png_at(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, PNG).unwrap();
    }

    fn doc_at(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// **The bug, as one assertion.** `write_tree` wrote markdown and nothing
    /// else, and the walk lists markdown by design — so a document's images were
    /// never in the entry list and nothing copied them. Every illustrated page
    /// rendered with broken images in the browser while the dock, reading the
    /// repo itself, showed them perfectly: the viewer was strictly worse than the
    /// surface it exists to improve on.
    ///
    /// The copy mirrors the reference's own relative path, and the document is
    /// NOT rewritten — asserted here, because a rewrite is the other way to make
    /// this pass and it would make the derived text stop being a copy.
    #[test]
    fn a_referenced_image_is_copied_beside_its_document() {
        let src = tmp("img");
        doc_at(
            &src,
            "docs/guides/p.md",
            "# P\n\n![up](../assets/y.png)\n![down](pics/z.png)\n![same](w.png)\n\
             ![remote](https://example.com/r.png)\n![inline](data:image/png;base64,iVBORw0KGgo=)\n",
        );
        png_at(&src, "docs/assets/y.png");
        png_at(&src, "docs/guides/pics/z.png");
        png_at(&src, "docs/guides/w.png");
        let entries = vec![entry("docs/guides/p.md", "P", &src)];
        let out = tmp("img-out");

        let d = write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap();

        for rel in ["docs/assets/y.png", "docs/guides/pics/z.png", "docs/guides/w.png"] {
            assert!(out.join(rel).is_file(), "{rel} was not copied into the derived tree");
            assert_eq!(std::fs::read(out.join(rel)).unwrap(), PNG, "{rel} arrived with the wrong bytes");
        }
        // The list the fingerprint stats: the local images, and only those. A
        // URL and a `data:` URI are not files and must never become one.
        assert_eq!(
            d.assets,
            vec!["docs/assets/y.png", "docs/guides/pics/z.png", "docs/guides/w.png"]
        );
        assert!(d.notes.is_empty(), "{:?}", d.notes);
        // The document is untouched — this is a copy, not a rewrite.
        let text = std::fs::read_to_string(out.join("docs/guides/p.md")).unwrap();
        assert!(text.contains("![up](../assets/y.png)"), "the reference was rewritten: {text}");
        assert!(text.contains("![down](pics/z.png)"), "{text}");
        assert!(text.contains("![remote](https://example.com/r.png)"), "{text}");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// `symlink_metadata`, never `metadata` — the rule `docs.rs` keeps about
    /// documents, applied to images for the identical reason. A committed
    /// `docs/logo.png -> ~/.ssh/id_rsa` must copy NOTHING: the derived tree is
    /// served on an unauthenticated loopback port, so a copy here is a
    /// publication.
    #[test]
    fn a_symlinked_image_copies_nothing() {
        let src = tmp("img-link");
        let secret = tmp("img-link-secret").join("id_rsa");
        std::fs::write(&secret, "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
        doc_at(&src, "docs/p.md", "# P\n\n![logo](logo.png)\n");
        std::os::unix::fs::symlink(&secret, src.join("docs/logo.png")).unwrap();
        let entries = vec![entry("docs/p.md", "P", &src)];
        let out = tmp("img-link-out");

        let d = write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap();

        assert!(!out.join("docs/logo.png").exists(), "a symlinked image was copied");
        assert!(
            d.notes.iter().any(|n| n.contains("docs/logo.png") && n.contains("regular file")),
            "the refusal was silent: {:?}",
            d.notes
        );
        // It is still a candidate, so replacing the link with a real file
        // re-derives the place rather than leaving the image broken forever.
        assert_eq!(d.assets, vec!["docs/logo.png"]);

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&secret.parent().unwrap());
        let _ = std::fs::remove_dir_all(&out);
    }

    /// **Layer B, which is the check that actually holds.** Layer A is
    /// arithmetic on a string and a string cannot show a symlink: with
    /// `docs/assets` a link to a directory elsewhere, `../assets/x.png` is
    /// textually innocent, its final component really is a regular file, and
    /// only canonicalising the whole path says where it is.
    #[test]
    fn an_image_that_resolves_outside_the_place_is_refused() {
        let src = tmp("img-escape");
        let elsewhere = tmp("img-escape-elsewhere");
        std::fs::write(elsewhere.join("x.png"), PNG).unwrap();
        std::fs::create_dir_all(src.join("docs/guides")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, src.join("docs/assets")).unwrap();
        doc_at(
            &src,
            "docs/guides/p.md",
            "# P\n\n![through a link](../assets/x.png)\n![up and out](../../../../etc/passwd.png)\n\
             ![absolute](/etc/passwd.png)\n![home](~/.ssh/id_rsa.png)\n",
        );
        let entries = vec![entry("docs/guides/p.md", "P", &src)];
        let out = tmp("img-escape-out");

        let d = write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap();

        assert!(!out.join("docs/assets/x.png").exists(), "a file outside the place was copied in");
        assert!(
            d.notes.iter().any(|n| n.contains("outside the place")),
            "the escape was refused silently: {:?}",
            d.notes
        );
        // Layer A refused the other three before any of them reached a stat, so
        // the only candidate is the one that needed Layer B.
        assert_eq!(d.assets, vec!["docs/assets/x.png"]);
        // And the tree holds the document and nothing else.
        let mut found: Vec<String> = Vec::new();
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), base, out);
                } else {
                    out.push(e.path().strip_prefix(base).unwrap().to_string_lossy().into_owned());
                }
            }
        }
        walk(&out, &out, &mut found);
        assert_eq!(found, vec!["docs/guides/p.md"], "the derived tree gained a file it should not have");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&elsewhere);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// An image over the per-file cap is skipped and SAID — §2.7 is "fail per
    /// file, loudly", and a screenshot that silently does not arrive is a reader
    /// wondering whether the document is broken or the tool is.
    #[test]
    fn an_over_cap_image_is_skipped_and_said_out_loud() {
        let src = tmp("img-cap");
        doc_at(&src, "p.md", "# P\n\n![huge](huge.png)\n![fine](fine.png)\n");
        png_at(&src, "fine.png");
        // Sparse: the cap is about bytes the copy would move, and the test is
        // not about waiting for a disk.
        std::fs::File::create(src.join("huge.png")).unwrap().set_len(ASSET_MAX_BYTES + 1).unwrap();
        let entries = vec![entry("p.md", "P", &src)];
        let out = tmp("img-cap-out");

        let d = write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap();

        assert!(!out.join("huge.png").exists(), "an over-cap image was copied");
        assert!(out.join("fine.png").is_file(), "one refusal took the other image with it");
        assert!(
            d.notes.iter().any(|n| n.contains("huge.png") && n.contains("cap")),
            "the excess was dropped silently: {:?}",
            d.notes
        );

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// SVG is not an image here, it is a document that can run script — and the
    /// viewer hands assets back by URL, where an `<img>`'s inertness does not
    /// apply. The allow-list carries the long version; this is the assertion
    /// that stops someone "fixing" a broken logo by adding three letters.
    #[test]
    fn an_svg_is_never_copied_however_ordinary_it_looks() {
        let src = tmp("img-svg");
        doc_at(&src, "p.md", "# P\n\n![logo](logo.svg)\n![shot](shot.png)\n");
        std::fs::write(src.join("logo.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>fetch('/_/api/groups')</script></svg>").unwrap();
        png_at(&src, "shot.png");
        let entries = vec![entry("p.md", "P", &src)];
        let out = tmp("img-svg-out");

        let d = write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap();

        assert!(!out.join("logo.svg").exists(), "an SVG reached the derived tree");
        assert!(out.join("shot.png").is_file());
        assert!(!d.assets.iter().any(|a| a.ends_with(".svg")), "{:?}", d.assets);
        assert!(
            d.notes.iter().any(|n| n.contains("logo.svg") && n.contains("script")),
            "a reader with a broken logo has no way to find out why: {:?}",
            d.notes
        );

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// An image must leave the tree when the document that referenced it does —
    /// the derived copy is served on a port, so a deleted screenshot that stays
    /// behind is the feature serving something the place no longer has. The
    /// prune only knows what the write told it, so this is really an assertion
    /// about the copy joining the `written` set.
    #[test]
    fn an_image_that_is_no_longer_referenced_leaves_the_tree() {
        let src = tmp("img-prune");
        doc_at(&src, "docs/p.md", "# P\n\n![shot](shots/a.png)\n");
        png_at(&src, "docs/shots/a.png");
        let entries = vec![entry("docs/p.md", "P", &src)];
        let out = tmp("img-prune-out");

        write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap();
        assert!(out.join("docs/shots/a.png").is_file(), "the first derive did not copy it");

        // The author drops the image from the document.
        doc_at(&src, "docs/p.md", "# P\n\nno picture any more\n");
        let d = write_tree(&out, &src, &entries, &stale(), 6275, "g").unwrap();

        assert!(out.join("docs/p.md").is_file(), "the prune took the document too");
        assert!(!out.join("docs/shots/a.png").exists(), "a dropped image is still being served");
        assert!(!out.join("docs/shots").exists(), "the directory it emptied stayed");
        assert!(d.assets.is_empty(), "{:?}", d.assets);

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    // ── the tick ────────────────────────────────────────────────────────────

    /// A viewer slot holding a child that is alive and is not a viewer.
    ///
    /// `refresh` never talks to the process: it writes files into the tree and
    /// lets `mo`'s own watcher (`-wR`) notice, which is the whole reason the
    /// derived tree is written in place rather than recreated. So what the child
    /// IS does not matter here — what matters is that `try_wait` reports it
    /// alive, which is the liveness rule this shares with `Shells`. A real `mo`
    /// is `the_whole_path_works_against_a_real_viewer`'s business.
    fn registered(place: &Path, tree: &Path, fingerprint: u64, assets: Vec<String>, stale: Staleness) -> Viewer {
        let child = std::process::Command::new("/bin/sleep")
            .arg("120")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("a child to stand in for the viewer");
        let v = Viewer::default();
        *v.0.lock().unwrap() = Some(Proc {
            child,
            port: 6275,
            groups: vec![Group {
                root: place.to_path_buf(),
                slug: "tick-place".into(),
                name: "tick-place".into(),
                tree: tree.to_path_buf(),
                docs: None,
                stale,
                fingerprint,
                assets,
            }],
        });
        v
    }

    /// Derive a place the way `open` does, and hand back the state the tick
    /// carries forward: the digest with this place's images folded into it, and
    /// the list of images that fold was over.
    fn first_derive(place: &Path, tree: &Path, st: &Staleness) -> (u64, Vec<String>) {
        let fp = worktrees_core::docs::fingerprint(place);
        let idx = worktrees_core::docs::index(place);
        let d = write_tree(tree, place, &idx.entries, st, 6275, "tick-place").unwrap();
        (worktrees_core::docs::fold_assets(fp, place, &d.assets), d.assets)
    }

    /// **The bug, as one assertion.** `mo` watches the DERIVED tree, not the
    /// repo, so before this the live-reload was perfect over a copy that could
    /// only change when the button was pressed again: editing a document reached
    /// nothing, adding one reached nothing, and refreshing the browser did not
    /// help because the derived file genuinely had not changed.
    #[test]
    fn an_edit_after_the_open_reaches_the_derived_copy() {
        let place = tmp("tick-edit");
        let tree = tmp("tick-edit-tree");
        std::fs::write(place.join("README.md"), "# Read me\n\nbefore\n").unwrap();
        let st = stale();
        let (fp, assets) = first_derive(&place, &tree, &st);
        assert!(std::fs::read_to_string(tree.join("README.md")).unwrap().contains("before"));

        // The user edits the document, and adds one — the two cases that both
        // reached nothing.
        for _ in 0..200 {
            std::fs::write(place.join("README.md"), "# Read me\n\nAFTER\n").unwrap();
            if worktrees_core::docs::fingerprint(&place) != fp {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        std::fs::create_dir_all(place.join("docs")).unwrap();
        std::fs::write(place.join("docs/new.md"), "# Brand new\n").unwrap();

        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        assert!(refresh(&v, 1_000_000 + 600).is_empty(), "the re-derive reported an error");

        let readme = std::fs::read_to_string(tree.join("README.md")).unwrap();
        assert!(readme.contains("AFTER"), "the edit never reached the derived copy: {readme}");
        assert!(!readme.contains("before"), "the old text is still being served: {readme}");
        assert!(tree.join("docs/new.md").is_file(), "a document added after the open was not derived");
        // …and the tick recorded the new state, so the next one is free.
        let after = v.0.lock().unwrap().as_ref().unwrap().groups[0].fingerprint;
        assert_ne!(after, fp, "the fingerprint was not carried forward; every tick would re-derive");

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// A place nobody has touched must cost a stat per document and NOTHING
    /// else. The derived file's own mtime is the witness: if it moved, the full
    /// walk ran — 8 KiB read per document, per place, every three seconds,
    /// forever — and the fingerprint bought nothing.
    #[test]
    fn a_quiet_place_is_not_re_derived() {
        let place = tmp("tick-quiet");
        let tree = tmp("tick-quiet-tree");
        std::fs::write(place.join("README.md"), "# Read me\n").unwrap();
        let st = stale();
        let (fp, assets) = first_derive(&place, &tree, &st);
        let before = std::fs::metadata(tree.join("README.md")).unwrap().modified().unwrap();

        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        // Long enough that a rewrite would land in a later millisecond.
        std::thread::sleep(Duration::from_millis(20));
        assert!(refresh(&v, 1_000_000 + 600).is_empty());
        assert!(refresh(&v, 1_000_000 + 900).is_empty());

        let after = std::fs::metadata(tree.join("README.md")).unwrap().modified().unwrap();
        assert_eq!(before, after, "an untouched place was re-derived anyway");

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// **The fingerprint's blind spot, closed.** `docs::fingerprint_with` is
    /// stat-only over MARKDOWN — that is what makes it cheap enough to run on a
    /// timer — so an edited screenshot moved nothing and the tab went on serving
    /// the copy made at click time, while the dock beside it showed the new one.
    /// The repair is not to make the digest read documents (that is the cost it
    /// exists to avoid) but to stat the images the LAST derive already copied,
    /// whose paths are known.
    #[test]
    fn an_edited_image_reaches_the_derived_copy() {
        let place = tmp("tick-img");
        let tree = tmp("tick-img-tree");
        std::fs::write(place.join("README.md"), "# Read me\n\n![shot](shots/a.png)\n").unwrap();
        png_at(&place, "shots/a.png");
        let st = stale();
        let (fp, assets) = first_derive(&place, &tree, &st);
        assert_eq!(std::fs::read(tree.join("shots/a.png")).unwrap(), PNG, "the first derive did not copy it");
        assert_eq!(assets, vec!["shots/a.png"]);

        // The user replaces the screenshot. No document changes, nothing is
        // clicked: before this, that reached nothing at all.
        let newer = [PNG.to_vec(), vec![0u8; 16]].concat();
        for _ in 0..200 {
            std::fs::write(place.join("shots/a.png"), &newer).unwrap();
            if worktrees_core::docs::fold_assets(
                worktrees_core::docs::fingerprint(&place),
                &place,
                &assets,
            ) != fp
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }

        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        assert!(refresh(&v, 1_000_000 + 600).is_empty(), "the re-derive reported an error");

        assert_eq!(
            std::fs::read(tree.join("shots/a.png")).unwrap(),
            newer,
            "the edited image never reached the derived copy"
        );
        let after = v.0.lock().unwrap().as_ref().unwrap().groups[0].fingerprint;
        assert_ne!(after, fp, "the fingerprint was not carried forward; every tick would re-derive");

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// An image a document already points at but the repo does not yet have is
    /// carried in the candidate list as a stat that fails — so the moment it
    /// ARRIVES the digest moves and the place re-derives. Without that, the one
    /// broken image nobody can explain is the one that was fixed by adding the
    /// missing file.
    #[test]
    fn an_image_that_arrives_later_re_derives_the_place() {
        let place = tmp("tick-img-new");
        let tree = tmp("tick-img-new-tree");
        std::fs::write(place.join("README.md"), "# Read me\n\n![shot](shots/a.png)\n").unwrap();
        let st = stale();
        let (fp, assets) = first_derive(&place, &tree, &st);
        assert!(!tree.join("shots/a.png").exists(), "there was nothing to copy yet");
        assert_eq!(assets, vec!["shots/a.png"], "a missing image must still be a candidate");

        png_at(&place, "shots/a.png");

        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        assert!(refresh(&v, 1_000_000 + 600).is_empty());
        assert_eq!(
            std::fs::read(tree.join("shots/a.png")).unwrap(),
            PNG,
            "the image that arrived after the open never reached the tree"
        );

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// …and the other half of the same rule: a place whose images are untouched
    /// re-copies none of them, even on a tick that DOES re-derive because a
    /// document moved. Every write into this tree is an event for `mo`'s
    /// watcher, so a re-copied screenshot is a reload in somebody's open tab
    /// for no change at all.
    ///
    /// **The witness is the copy's BYTES, not its mtime**, and that is a trap
    /// worth naming: on macOS `std::fs::copy` goes through
    /// `copyfile(COPYFILE_ALL)`, which carries the source's modification time
    /// over with the data — so a re-copied file has exactly the mtime it had
    /// before, and an mtime assertion here passes whatever the code does. (It
    /// did: this test was green with the skip deleted.) Marking the copy with a
    /// byte of its own, at the same length so the freshness test still reads it
    /// as current, makes a re-copy visible as the marker being overwritten.
    #[test]
    fn a_quiet_place_does_not_re_copy_its_images() {
        let place = tmp("tick-img-quiet");
        let tree = tmp("tick-img-quiet-tree");
        std::fs::write(place.join("README.md"), "# Read me\n\n![shot](shots/a.png)\n").unwrap();
        png_at(&place, "shots/a.png");
        let st = stale();
        let (fp, assets) = first_derive(&place, &tree, &st);
        let mut marked = PNG.to_vec();
        marked[PNG.len() - 1] ^= 0xff;
        std::fs::write(tree.join("shots/a.png"), &marked).unwrap();

        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        std::thread::sleep(Duration::from_millis(20));
        assert!(refresh(&v, 1_000_000 + 600).is_empty());
        assert!(refresh(&v, 1_000_000 + 900).is_empty());
        assert_eq!(
            std::fs::read(tree.join("shots/a.png")).unwrap(),
            marked,
            "an untouched image was copied again"
        );

        // And on a tick that really does re-derive: the document changed, the
        // image did not.
        std::fs::write(place.join("README.md"), "# Read me\n\nedited\n\n![shot](shots/a.png)\n").unwrap();
        assert!(refresh(&v, 1_000_000 + 1200).is_empty());
        assert!(
            std::fs::read_to_string(tree.join("README.md")).unwrap().contains("edited"),
            "the document edit did not re-derive, so this proves nothing"
        );
        assert_eq!(
            std::fs::read(tree.join("shots/a.png")).unwrap(),
            marked,
            "an unchanged image was re-copied because a document moved"
        );

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// The header has to date the COPY, and it may not claim the git facts were
    /// measured then. Every one of those costs a fan-out to recompute — §15.3
    /// refused that for a click, and a timer is worse — so after a background
    /// re-derive the page carries two instants, and the older one belongs to the
    /// status line.
    #[test]
    fn a_re_derived_page_is_stamped_now_and_keeps_the_facts_it_had() {
        let place = tmp("tick-stamp");
        let tree = tmp("tick-stamp-tree");
        std::fs::write(place.join("README.md"), "# Read me\n\nbefore\n").unwrap();
        let mut st = stale();
        st.now_epoch = 1_789_776_000;
        st.derived_epoch = 1_789_776_000;
        let (fp, assets) = first_derive(&place, &tree, &st);
        let first = std::fs::read_to_string(tree.join("README.md")).unwrap();
        assert!(first.contains("derived 2026-09-19 00:00:00 UTC"), "{first}");
        assert!(!first.contains("status as of"), "one instant, one stamp: {first}");

        for _ in 0..200 {
            std::fs::write(place.join("README.md"), "# Read me\n\nAFTER\n").unwrap();
            if worktrees_core::docs::fingerprint(&place) != fp {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        assert!(refresh(&v, 1_789_779_661).is_empty());

        let out = std::fs::read_to_string(tree.join("README.md")).unwrap();
        assert!(out.contains("derived 2026-09-19 01:01:01 UTC"), "the copy is not dated now: {out}");
        assert!(out.contains("status as of 2026-09-19 00:00:00 UTC"), "the facts claim to be fresh: {out}");
        // The facts themselves are untouched — the tick re-reads documents, not
        // git.
        assert!(out.contains("25 behind `origin/main`"), "{out}");

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// Two refusals that are one rule: a timer may not decide the user wants a
    /// viewer.
    ///
    /// Nothing registered must cost nothing at all, and a viewer that has died
    /// must stay dead until somebody presses the button — respawning from a
    /// timer would put an unauthenticated loopback port back up minutes after
    /// the user stopped using it, with nothing on screen to say so. `Shells`
    /// makes the same promise for the same reason.
    #[test]
    fn the_tick_never_spawns_and_never_resurrects() {
        let v = Viewer::default();
        assert!(refresh(&v, 1_000_000).is_empty(), "an idle viewer reported something");
        assert!(v.0.lock().unwrap().is_none(), "the tick put a viewer in an empty slot");

        let place = tmp("tick-dead");
        let tree = tmp("tick-dead-tree");
        std::fs::write(place.join("README.md"), "# Read me\n\nbefore\n").unwrap();
        let st = stale();
        let (fp, assets) = first_derive(&place, &tree, &st);
        let before = std::fs::metadata(tree.join("README.md")).unwrap().modified().unwrap();
        std::fs::write(place.join("README.md"), "# Read me\n\nAFTER\n").unwrap();

        let v = registered(&place, &tree, fp, assets, st);
        {
            let mut slot = v.0.lock().unwrap();
            let p = slot.as_mut().unwrap();
            p.child.kill().unwrap();
            p.child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(20));
        assert!(refresh(&v, 1_000_000 + 600).is_empty());
        assert!(v.0.lock().unwrap().is_none(), "a dead viewer stayed in the slot");
        assert_eq!(
            before,
            std::fs::metadata(tree.join("README.md")).unwrap().modified().unwrap(),
            "the tick wrote documents for a viewer that is not running",
        );

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    // ── the gate's WIRING, against a viewer that fails it ───────────────────

    /// A viewer binary that answers `200` to everything, including a forged
    /// `Host`. Returns `None` when there is no `python3` to build it out of, in
    /// which case the test skips rather than fails — the fake is a prop, and a
    /// missing prop is not a defect in the thing being tested.
    fn fake_viewer_that_answers_200(dir: &Path) -> Option<PathBuf> {
        if !std::process::Command::new("python3")
            .arg("-c")
            .arg("pass")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|st| st.success())
            .unwrap_or(false)
        {
            return None;
        }
        let path = dir.join("fake-viewer");
        // Parses only `--port`, ignores the rest of our argv, then serves 200
        // to any request on that port until it is killed.
        let script = r#"#!/bin/sh
while [ $# -gt 0 ]; do case "$1" in --port) p="$2"; shift 2 ;; *) shift ;; esac; done
exec python3 -c '
import socket, sys
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", int(sys.argv[1])))
s.listen(8)
while True:
    c, _ = s.accept()
    try:
        c.recv(4096)
        c.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    except Exception:
        pass
    finally:
        c.close()
' "$p"
"#;
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Some(path)
    }

    /// `spawn` must consult the gate, not merely contain a correct one.
    ///
    /// The end-to-end test below deliberately has no fake, on the grounds that
    /// one would be "a second answer to the question this test exists to ask".
    /// That is right about ITS question — does the gate pass against the binary
    /// we ship — and wrong about this one, which is whether the gate is CALLED.
    /// Delete the `probe_refuses_foreign_host` line from `spawn` and every
    /// other test in this file still passes, the end-to-end one included,
    /// because it reaches the probe directly rather than through `spawn`. Two
    /// independent reviews found that hole before this test existed; it is the
    /// only assertion here that goes red for it.
    ///
    /// It also pins the consequence, which is the part that matters: a viewer
    /// that answers a forged `Host` must never reach the user's browser, so
    /// `spawn` fails AND leaves nothing listening.
    #[test]
    fn spawn_refuses_a_viewer_that_serves_a_forged_host() {
        let d = tmp("gatewire");
        std::fs::create_dir_all(&d).unwrap();
        let Some(fake) = fake_viewer_that_answers_200(&d) else {
            eprintln!("skipped: no python3 to build the fake viewer out of");
            let _ = std::fs::remove_dir_all(&d);
            return;
        };
        let state = d.join("state");
        std::fs::create_dir_all(&state).unwrap();

        // `Proc` is not `Debug` (it holds a `Child`), so match rather than
        // `expect_err` — and a spawn that SUCCEEDED must be cleaned up here or
        // the fake outlives the test.
        let err = match spawn(&fake, &state, &d.join("viewer.log")) {
            Err(e) => e,
            Ok(mut p) => {
                let _ = p.child.kill();
                let _ = p.child.wait();
                panic!("a viewer that serves a forged Host was accepted and handed to the browser");
            }
        };
        assert!(
            err.contains("foreign Host header"),
            "the refusal must name what was wrong, got: {err}"
        );
        // `spawn`'s failure path kills the child. If it did not, an
        // unauthenticated server would outlive the open that refused it.
        assert!(
            std::process::Command::new("pgrep")
                .args(["-f", "fake-viewer"])
                .output()
                .map(|o| o.stdout.is_empty())
                .unwrap_or(true),
            "the refused viewer is still running",
        );
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
        let _reap = Reaper(&v);
        let req = Request {
            root: &src,
            slug: "e2e-place",
            path: Some(&entries[0].path),
            stale: stale(),
            docs: None,
            fingerprint: worktrees_core::docs::fingerprint(&src),
        };

        // A tree left behind by a previous run — a crash, or a place the user
        // has since removed. The first spawn of a run must take it with it.
        let stale_tree = viewer_dir(&cfg).join("tree/gone-deadbeef");
        std::fs::create_dir_all(&stale_tree).unwrap();
        std::fs::write(stale_tree.join("secret.md"), "a client's signed agreement").unwrap();

        let url = open(&v, &cfg, None, &entries, &req).expect("the viewer must come up").url;
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

    /// The tick, against the real thing — the one assertion no stub can make.
    ///
    /// `an_edit_after_the_open_reaches_the_derived_copy` proves the bytes on
    /// disk change. What it cannot prove is the half this feature actually
    /// depends on: that `mo`, watching the derived tree with `-wR`, SERVES the
    /// rewritten file to a tab that is already open. That is a claim about
    /// somebody else's watcher, and the only honest way to make it is to ask a
    /// running one.
    ///
    /// **Runs only when `WORKTREES_VIEWER_BIN` names a viewer**, like the test
    /// above and for the same reason — CI has no `mo`, and a fake that answered
    /// this question would be answering it with our own assumption:
    ///
    /// ```sh
    /// WORKTREES_VIEWER_BIN=~/workspace/mo/mo cargo test -p app --lib the_tick_reaches -- --nocapture
    /// ```
    #[test]
    fn the_tick_reaches_a_real_viewers_open_tab() {
        if std::env::var(BIN_ENV).is_err() {
            return;
        }
        let src = tmp("e2e-tick");
        let cfg = tmp("e2e-tick-cfg");
        std::fs::write(src.join("README.md"), "# Read me\n\nBEFORE-THE-EDIT\n").unwrap();
        // The REAL walk, so the entries and the fingerprint describe the same
        // place — which is what `open`'s caller does.
        let fp = worktrees_core::docs::fingerprint(&src);
        let entries = worktrees_core::docs::index(&src).entries;
        let v = Viewer::default();
        let _reap = Reaper(&v);
        let req = Request {
            root: &src,
            slug: "e2e-tick",
            path: Some(&entries[0].path),
            stale: stale(),
            docs: None,
            fingerprint: fp,
        };
        let url = open(&v, &cfg, None, &entries, &req).expect("the viewer must come up").url;
        let port = v.0.lock().unwrap().as_ref().unwrap().port;
        let id = url.rsplit("file=").next().unwrap().to_string();
        let content = format!("/_/api/groups/e2e-tick/files/{id}/content");
        assert!(http_get(port, &content).contains("BEFORE-THE-EDIT"), "the first derive did not reach the viewer");

        // The user edits the document. Nothing is clicked.
        for _ in 0..200 {
            std::fs::write(src.join("README.md"), "# Read me\n\nAFTER-THE-EDIT\n").unwrap();
            if worktrees_core::docs::fingerprint(&src) != fp {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(refresh(&v, 1_789_779_661).is_empty(), "the tick reported an error");

        // Polled, not slept: the watcher is another process and the only thing
        // being asserted is that it gets there, not how fast. A bounded wait
        // fails loudly; a fixed sleep either flakes or hides a regression.
        let mut body = String::new();
        for _ in 0..40 {
            body = http_get(port, &content);
            if body.contains("AFTER-THE-EDIT") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(body.contains("AFTER-THE-EDIT"), "the viewer is still serving the copy made at click time: {body}");
        assert!(!body.contains("BEFORE-THE-EDIT"), "the old text is still being served: {body}");
        // …and it carries the new stamp, so a reader can tell the copy apart
        // from the status above it.
        assert!(body.contains("derived 2026-09-19 01:01:01 UTC"), "the re-derived page is not dated: {body}");
        assert!(body.contains("status as of"), "the git facts are passed off as current: {body}");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&cfg);
    }

    /// **An image, loaded through the server.** The one assertion that a file on
    /// disk cannot make: `mo` resolves a relative `src` against the directory of
    /// the markdown file it is serving and hands the bytes back from
    /// `/_/api/groups/{group}/files/{id}/raw/{path}`, so the copy is only
    /// correct if the viewer can actually reach it at the path the author wrote.
    ///
    /// **Runs only when `WORKTREES_VIEWER_BIN` names a viewer:**
    ///
    /// ```sh
    /// WORKTREES_VIEWER_BIN=~/workspace/mo/mo cargo test -p app --lib an_image_loads -- --nocapture
    /// ```
    ///
    /// The `../` case is deliberately not asserted over HTTP, and that is a
    /// finding rather than an omission: `resolveImageSrc` appends the author's
    /// `src` to that raw prefix verbatim, so `../assets/y.png` builds a URL
    /// carrying a dot segment — which both the browser and Go's own mux
    /// normalise away, eating the `/raw/` segment and landing on a route that
    /// does not exist (probed: 307 to `…/files/{id}/assets/y.png`, which serves
    /// the SPA shell). The copy below is correct for that reference too — the
    /// file lands exactly where the reference points — and it is asserted on
    /// disk; making it *reachable* is a fix in the viewer, not here.
    #[test]
    fn an_image_loads_through_a_real_viewer() {
        if std::env::var(BIN_ENV).is_err() {
            return;
        }
        let src = tmp("e2e-img");
        let cfg = tmp("e2e-img-cfg");
        std::fs::write(
            src.join("README.md"),
            "# Read me\n\n![beside](shots/a.png)\n![above](../outside.png)\n",
        )
        .unwrap();
        png_at(&src, "shots/a.png");
        doc_at(&src, "docs/guides/p.md", "# P\n\n![up](../assets/y.png)\n");
        png_at(&src, "docs/assets/y.png");
        let entries = worktrees_core::docs::index(&src).entries;
        let v = Viewer::default();
        let _reap = Reaper(&v);
        let req = Request {
            root: &src,
            slug: "e2e-image",
            path: Some(&entries[0].path),
            stale: stale(),
            docs: None,
            fingerprint: worktrees_core::docs::fingerprint(&src),
        };

        let opened = open(&v, &cfg, None, &entries, &req).expect("the viewer must come up");
        let port = v.0.lock().unwrap().as_ref().unwrap().port;
        let tree = v.0.lock().unwrap().as_ref().unwrap().groups[0].tree.clone();
        let id = opened.url.rsplit("file=").next().unwrap().to_string();

        // The bytes, through the server, at the path the document asked for.
        let res = http_get(port, &format!("/_/api/groups/e2e-image/files/{id}/raw/shots/a.png"));
        assert!(res.starts_with("HTTP/1.1 200"), "the image did not load: {}", res.lines().next().unwrap_or(""));
        assert!(res.contains("Content-Type: image/png"), "{res:?}");
        assert!(res.contains(&format!("Content-Length: {}", PNG.len())), "{res:?}");

        // The parent-directory reference: copied to exactly where it points,
        // which is all this side of the seam can do (see the note above).
        assert!(tree.join("docs/assets/y.png").is_file(), "the parent-directory image was not copied");
        assert_eq!(std::fs::read(tree.join("docs/assets/y.png")).unwrap(), PNG);
        // …and a reference pointing OUT of the place copied nothing at all.
        assert!(!tree.join("outside.png").exists());
        assert!(!src.parent().unwrap().join("outside.png").exists());

        kill(&v);
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

    /// Kills the viewer however the test ends.
    ///
    /// A `#[test]` that panics before reaching its own `kill` leaves a REAL
    /// server holding a REAL port, on the machine of whoever ran it. That is not
    /// hypothetical: proving the tree-wipe assertion red left `mo` listening on
    /// 53381 until it was found by hand — a test suite reproducing, in miniature,
    /// the exact failure rule 1 exists to prevent. `kill` takes the slot, so the
    /// test's own explicit call still means what it says and this is a no-op
    /// after it.
    struct Reaper<'a>(&'a Viewer);
    impl Drop for Reaper<'_> {
        fn drop(&mut self) {
            kill(self.0);
        }
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
        assert!(resolve_binary(None, Some(&d)).is_none(), "an empty resource dir is not an install");
        assert!(resolve_binary(None, None).is_none(), "no resource dir at all is not an install");

        // A DIRECTORY where the binary should be is still "not installed".
        std::fs::create_dir_all(d.join(RESOURCE_REL)).unwrap();
        assert!(resolve_binary(None, Some(&d)).is_none(), "a directory passed as the binary");
        let _ = std::fs::remove_dir_all(d.join(RESOURCE_REL));

        let real = d.join("viewer/mo");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "#!/bin/sh\n").unwrap();
        assert_eq!(resolve_binary(None, Some(&d)).as_deref(), Some(real.as_path()));

        // The override wins, and a BROKEN override does not fall through to the
        // bundled one — running a different binary than the one you named is how
        // an afternoon goes missing.
        let other = d.join("other-mo");
        std::fs::write(&other, "#!/bin/sh\n").unwrap();
        assert_eq!(resolve_binary(Some(other.to_str().unwrap()), Some(&d)).as_deref(), Some(other.as_path()));
        assert!(resolve_binary(Some("/does/not/exist/mo"), Some(&d)).is_none());

        let _ = std::fs::remove_dir_all(&d);
    }
}

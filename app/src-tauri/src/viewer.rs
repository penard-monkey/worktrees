//! The tool-owned documentation viewer — the derived tree, and the places the
//! server may serve from it.
//!
//! `docs.rs` decides which documents a place has; `derive.rs` decides what each
//! one looks like once a viewer gets hold of it. This is the third part: where
//! the derived tree lands on disk, which places are registered against it, and
//! what URL the frontend hands to `openUrl`. `docserver.rs` is the fourth —
//! the loopback server that answers for them, and the one part that holds a
//! port.
//!
//! **What used to be here.** Until this branch the viewer was `mo`, a
//! third-party Go binary the app spawned, supervised and probed. It is gone,
//! and the reason it is gone is the reason the server exists: `mo` does not
//! validate the `Host` header (proposal §13.2), which makes a loopback server
//! readable by any web page through DNS rebinding, and §5.1's
//! one-process-for-the-whole-app made that blast radius every document in
//! every place at once. A patch was written and reported upstream; waiting on
//! somebody else's release to be allowed to ship is not a position this feature
//! should be in, and the server we needed instead is routing and policy —
//! which is where `mo`'s bug actually was.
//!
//! **What survived, and why.** The derived tree is the durable asset here. It
//! is generated the same way it always was, by the same transform, with the
//! same asset copying and the same caps; only the thing that served it changed.
//! Four of its rules came from real defects and are kept verbatim:
//!
//! 1. **Nothing happens at launch.** The first `open_docs_viewer` starts the
//!    server; there is deliberately no probe at startup deciding whether to
//!    show the Docs tab, and the blast radius of the whole subsystem is one
//!    button. Phases 1–2 work with nothing serving at all.
//! 2. **Liveness at the point of use, never a timer.** `Handle::alive` on
//!    every open, the way `Shells` asks `try_wait`. A timer that respawned
//!    would put a port back up minutes after the user stopped using it, with
//!    nothing on screen to say so.
//! 3. **The derived trees never outlive the process.** They are copies of the
//!    user's documents — the ones §4.3 says carry a client's signed agreement —
//!    in a directory Spotlight and Time Machine both index. `cleanup` empties
//!    them on a clean exit; the sweep on the first start of a run is what
//!    covers a crash, which a shutdown hook cannot.
//! 4. **A removed place's derived documents go with it.** `cmd_rm` deletes the
//!    originals, so without `forget_place` the derived copy would be the last
//!    readable one — and it would be on a port.
//!
//! The tick (`refresh`) is the fifth, and it is what makes the browser copy
//! keep up with the place: `docs::fingerprint_with` is stat-only (1.8 ms on a
//! 400-document place against `index_with`'s 10.1 ms and 3.2 MB of reads), so
//! the full walk runs only when the digest has moved.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use worktrees_core::derive::{self, Links, Staleness};
use worktrees_core::docs::DocEntry;

/// Per-document read cap, matching `read_file`'s. A document over this is not
/// silently dropped — it gets a stub page saying so (§2.7, "fail per file,
/// loudly"), because a row that opens onto nothing is the same silent wrongness
/// the staleness header exists to prevent.
pub(crate) const DOC_MAX_BYTES: u64 = 1_000_000;

/// Longest place key we will build. It is a URL path segment
/// (`/<token>/p/<place>/`), so the length is cosmetic and the character set is
/// not.
pub(crate) const PLACE_KEY_MAX: usize = 48;

/// What a document may bring with it into the derived tree.
///
/// An ALLOW-LIST, never a denylist: the set of things a browser will execute
/// grows, and a denylist written today is a list of the attacks that were known
/// today. Matched on the extension because that is also what the server matches
/// on when it picks a content type — agreeing with it is the point
/// (`docserver::ASSET_TYPES` carries the other half, and a debug assertion ties
/// them together).
///
/// **SVG is here again, and only because the RESPONSE changed.** It was
/// excluded under `mo`, which handed any raw asset back by direct URL as
/// `image/svg+xml` — where an SVG is not an image but a top-level document,
/// and its `<script>` runs. Inside an `<img>` it is script-inert by spec,
/// which is the reading that makes "it's just an image" sound true; served by
/// URL with a content type we did not choose, it is not. Owning the server is
/// exactly what findings §6.4 said would turn that permanent amputation into a
/// header, and it has: every asset goes out with
/// `Content-Security-Policy: default-src 'none'` and
/// `X-Content-Type-Options: nosniff`, under which a navigated SVG can neither
/// run inline script nor fetch anything.
///
/// **So this entry and that header are ONE decision.** Delete the header and
/// every `logo.svg` in every place is live again, with nothing in this file
/// saying so — which is why `an_svg_is_copied_only_because_the_response_makes_it_inert`
/// asserts both halves and goes red if either leaves.
pub(crate) const ASSET_EXTS: [&str; 8] =
    ["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "svg"];

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

/// Where the browser bundle is looked for, relative to the bundle's resource
/// directory. `release.yml` puts it here.
const BUNDLE_REL: &str = "viewer/viewer.js";

/// Development escape hatch: an absolute path to a browser bundle. This is the
/// USER's environment, never a cloned repo's file, so ADR 0001 is untouched —
/// the repo still cannot name a program to run, and this names no program at
/// all. It exists because `tauri dev` has no bundle and therefore no resource
/// directory, and because the page is built in a different worktree.
const BUNDLE_ENV: &str = "WORKTREES_VIEWER_JS";

// ── what the server is allowed to serve ──────────────────────────────────────

/// One place, as both the tick and the server see it.
///
/// **One list, shared.** `docserver` reads this same `Vec` under the same
/// mutex rather than keeping a projection of its own: two lists that must agree
/// about which places exist is a drift bug, and the fields the server reads
/// (`key`, `tree`, `root`, `etag`, `meta`, `index`) are exactly the fields the
/// derive writes.
pub(crate) struct Group {
    /// Canonical place root — the walk's root, and the identity of this entry.
    pub(crate) root: PathBuf,
    /// The place's slug. Names the tree directory and is the first fact in the
    /// header; kept so an error about this group can name it.
    slug: String,
    /// The URL path segment serving it, for THIS launch only.
    pub(crate) key: String,
    /// Where the derived copy lives. The server's root for this place, and the
    /// only directory a request about it may reach.
    pub(crate) tree: PathBuf,
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
    /// The `ETag` stem for everything this place serves.
    ///
    /// The fingerprint AND both epochs, because a re-open re-derives and
    /// re-stamps the header without the documents having moved: a tag built
    /// from the digest alone would still match, and the reader would keep a
    /// status line from the last open while the app showed a new one.
    pub(crate) etag: String,
    /// The `meta` object of the contract, pre-serialized. Built by the derive,
    /// spliced into every `doc` response.
    pub(crate) meta: Arc<str>,
    /// The whole `index` response body, pre-serialized. Up to 2,000 entries,
    /// built once per derive rather than once per poll.
    pub(crate) index: Arc<str>,
}

/// The app's single server slot and the places it serves.
///
/// `None` = nothing has been started, or the last one died and has not been
/// replaced yet. Both states are normal.
#[derive(Default)]
pub struct Viewer {
    srv: Mutex<Option<crate::docserver::Handle>>,
    places: Arc<Mutex<Vec<Group>>>,
}

/// Stop the server, if there is one. Called from `RunEvent::Exit` next to the
/// shells: what it is holding is an unauthenticated port onto the user's
/// documents, and it dies with the app deliberately.
pub fn kill(v: &Viewer) {
    if let Ok(mut slot) = v.srv.lock() {
        if let Some(h) = slot.take() {
            h.stop();
        }
    }
    if let Ok(mut places) = v.places.lock() {
        places.clear();
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
/// The registration goes with the tree, so the route answers `410 Gone`
/// immediately rather than after the next tick — a page open on that place says
/// so instead of polling a hole. (`with_place` also refuses a place whose tree
/// has left the disk, which is what covers the removal this never hears about.)
///
/// Best-effort and silent: a place with no tree (never opened in the browser) is
/// the common case, not an error.
pub fn forget_place(v: &Viewer, config_dir: &Path, slug: &str, canonical_root: &Path) {
    let key = tree_key(slug, canonical_root);
    if let Ok(mut places) = v.places.lock() {
        places.retain(|g| tree_key(&g.slug, &g.root) != key);
    }
    let dir = viewer_dir(config_dir).join("tree").join(key);
    let _ = std::fs::remove_dir_all(dir);
}

/// Drop what the viewer left on disk. Called from `RunEvent::Exit` after `kill`.
///
/// The derived trees are copies of the user's documents in a directory that is
/// neither the repo nor covered by its gitignore. They are not worth keeping for
/// a server that is, by design, dead. Best-effort: a file that will not go is
/// not worth failing a shutdown over, and the first start of the next run empties
/// the directory again anyway.
pub fn cleanup(config_dir: &Path) {
    empty_dir(&viewer_dir(config_dir).join("tree"));
}

// ── what the command is asked for ────────────────────────────────────────────

/// One `open_docs_viewer` call, as the frontend states it.
pub struct Request<'a> {
    /// The place directory. Already through `guard_under_projects` by the time
    /// it arrives here.
    pub root: &'a Path,
    /// The place's slug. Names the route segment, and is the first fact in the
    /// header.
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

/// A place's route segment: the slug, reduced to what can stand in a URL path
/// segment, disambiguated against the segments already taken.
///
/// Two places may legitimately share a slug — the same branch name in two
/// projects — and a collision would silently show one project's documents under
/// the other's name. That is §1.1's failure with the axes swapped, so the tie is
/// broken by a hash of the canonical root, which is the one thing that cannot
/// collide.
///
/// The character set is not cosmetic: this becomes a path segment in a URL the
/// tool generates and then validates on the way back in
/// (`docserver::place_key_ok`), so anything with a `/`, a `?` or a `%` in it
/// would not be the segment we emitted.
pub fn place_key(slug: &str, root: &Path, taken: &[(PathBuf, String)]) -> String {
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
        if base.len() >= PLACE_KEY_MAX {
            break;
        }
    }
    // A slug of nothing but punctuation lands here.
    if base.is_empty() {
        base = "place".to_string();
    }
    if !taken.iter().any(|(_, n)| *n == base) {
        return base;
    }
    let suffix = &id8(root.as_os_str().as_encoded_bytes());
    let keep = PLACE_KEY_MAX.saturating_sub(suffix.len() + 1);
    format!("{}-{}", &base[..base.len().min(keep)], suffix)
}

/// The first 8 hex characters of a sha256. Used to break a tie between two
/// places that share a slug, in the route segment and in the tree's directory
/// name — never as a secret, and never as anything a request may supply.
fn id8(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// The URL of one place's shell, and optionally of one document inside it.
///
/// `127.0.0.1` rather than `localhost`: it needs no resolver, it cannot be
/// pointed anywhere by a hosts file, and it is the literal the server's own
/// `Host` check will see.
///
/// **The document is a FRAGMENT, `#/<rel>`, and this was a query once.** The
/// shell is a single page that routes on `location.hash`; a `?path=` deep link
/// landed on the place's INDEX, so the Docs tab's per-row "open in the browser"
/// action opened the right place at the wrong document and nothing anywhere
/// said so. Measured in Chrome against the real page, which is the only way
/// this was ever going to be found — both halves were green on their own.
///
/// Two things follow from it being a fragment, and both are worth having:
/// the document's path is never sent to the server at all (it is not in the
/// request line, so it cannot reach a log or a `Referer`), and there is exactly
/// one place that knows which document is open — the page's own route — rather
/// than a query string that would have to be kept in step with it on every
/// in-page navigation, or else lie.
///
/// `route_escape` is per-segment `encodeURIComponent`, which is what the page's
/// `decodeURIComponent` undoes.
pub fn place_url(port: u16, token: &str, key: &str, rel: Option<&str>) -> String {
    match rel {
        None => format!("http://127.0.0.1:{port}/{token}/p/{key}/"),
        Some(r) => format!(
            "http://127.0.0.1:{port}/{token}/p/{key}/#/{}",
            crate::docserver::route_escape(r)
        ),
    }
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
fn stub(entry: &DocEntry, why: &str) -> String {
    let mut s = format!("# {}\n\n", entry.title.replace(['\n', '\r'], " "));
    s.push_str(&format!("*This document was not rendered: {why}.*\n\n"));
    s.push_str(&format!("`{}`\n", entry.rel.replace('`', "'")));
    s
}

// ── generating the tree ──────────────────────────────────────────────────────

/// Read one document and transform it, or produce the stub that says why not.
fn render_one(path: &Path, entry: &DocEntry, links: &Links) -> String {
    let Ok(file) = std::fs::File::open(path) else {
        return stub(entry, "it could not be opened");
    };
    let mut bytes = Vec::new();
    if file.take(DOC_MAX_BYTES + 1).read_to_end(&mut bytes).is_err() {
        return stub(entry, "it could not be read");
    }
    if bytes.len() as u64 > DOC_MAX_BYTES {
        return stub(entry, "it is larger than 1 MB");
    }
    match String::from_utf8(bytes) {
        // Not lossy. A document with a broken byte in it is a document whose
        // author would want to know, and a silently mangled page is indis-
        // tinguishable from the real thing — which is exactly what `mo` does on
        // its own (it skips an invalid-UTF-8 file with no log line at all).
        Err(_) => stub(entry, "it is not valid UTF-8"),
        Ok(text) => derive::body(entry, &text, links),
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
/// The staleness facts are NOT passed: the derived text no longer carries the
/// header, because the page renders it from `doc`'s `meta` and a second copy
/// in the prose could only be a stale one.
///
/// `root` is the place itself, canonical, and it is here for the images: a
/// reference is resolved against it and every candidate has to prove it still
/// lies under it (`copy_assets`). It is passed rather than derived from an
/// entry's absolute path minus its `rel`, because that subtraction is a third
/// place that would have to agree with the walk about what a root is.
#[allow(clippy::too_many_arguments)]
fn write_tree(
    dir: &Path,
    root: &Path,
    entries: &[DocEntry],
    port: u16,
    token: &str,
    key: &str,
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
        let (_i, _p) = derived.iter().find(|(i, _)| entries[*i].path == e.path)?;
        Some(place_url(port, token, key, Some(&entries[*_i].rel)))
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
        let body = render_one(Path::new(&entry.path), entry, &links);
        // Asked of the RENDERED body, not of the file: the transform leaves
        // every image reference exactly as its author wrote it (that is why the
        // copy has to mirror the path at all), so the two texts give the same
        // answer — and reading the document a second time to ask the same
        // question is the second reader this feature keeps refusing. A stub page
        // has no references, which is also correct: nothing of it is shown.
        collect_refs(entry, &body, &mut refs);
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
/// nothing else. A reference it drops is dropped SILENTLY: almost everything
/// refused here is an ordinary `https://` image or an anchor a document happens
/// to define, and a log line per external image per derive would bury the lines
/// that mean something. The refusals worth SAYING are the ones a local file
/// reached and failed — `copy_assets` writes those.
fn collect_refs(entry: &DocEntry, body: &str, out: &mut BTreeSet<String>) {
    for r in derive::image_refs(body) {
        let Some(rel) = derive::asset_rel(&entry.rel, &r) else { continue };
        match ext_of(&rel) {
            e if ASSET_EXTS.contains(&e.as_str()) => {
                out.insert(rel);
            }
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

/// Where the viewer's own state lives, and where the derived trees go.
fn viewer_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("viewer")
}

/// Empty a directory without removing it, ignoring what will not go. Used on
/// the derived-tree root at the first start of a run: a crash cannot honour a
/// shutdown hook, so this is what stops one run's copies of the user's
/// documents outliving it into the next.
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

// ── the contract's JSON, built by the derive rather than by the poll ─────────

/// The `meta` object every response carries: the staleness facts, as data.
///
/// The dock renders these from the `Place` it is holding and the browser page
/// renders them from here, so the two surfaces state the same facts — which is
/// §1.1's whole hazard and the reason the header exists at all. They are DATA
/// and not the markdown header `derive::header` writes, because the page
/// refreshes them live: a blockquote baked into the document would be a second,
/// frozen copy of numbers the reader can see changing above it.
fn meta_json(slug: &str, s: &Staleness) -> String {
    // `dirty` is the COUNT, per the contract. `dirty: Some(false)` with no
    // count is a clean place and says `0`; `None` on both is "not computed",
    // which is not zero and must not render as it.
    let dirty = match (s.dirty, s.dirty_files) {
        (_, Some(n)) => serde_json::json!(n),
        (Some(false), None) => serde_json::json!(0),
        _ => serde_json::Value::Null,
    };
    serde_json::json!({
        "place": slug,
        "branch": s.branch,
        "behind": s.behind,
        "base": s.base,
        "dirty": dirty,
        "subject": s.last_commit_subject,
        // Not in the contract's table, and sent anyway: `subject` without an
        // age is half of the dock's third line, and the page has no other way
        // to say "2h ago". An extra field costs a reader nothing.
        "last_commit_epoch": s.last_commit_epoch,
        "derived_epoch": s.derived_epoch,
        "status_epoch": s.now_epoch,
    })
    .to_string()
}

/// The whole `index` response body, built once per derive.
///
/// `path` is the PLACE-RELATIVE path (`DocEntry::rel`), not the absolute one
/// the dock uses — the page addresses a document by the same string it passes
/// back to `doc?path=`, and an absolute path would hand the browser the user's
/// home directory for nothing.
fn index_json(meta: &str, idx: &worktrees_core::docs::DocsIndex) -> String {
    let entries: Vec<serde_json::Value> = idx
        .entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "path": e.rel,
                "title": e.title,
                "group": e.group,
                // Extras, for the same reason as `last_commit_epoch`: the
                // dock's recency mark is `mtime_ms` against the place's seen
                // epoch, and a browser tab that cannot show it is the surface
                // §7.4 says needs the staleness signal MORE, not less.
                "mtime_ms": e.mtime_ms,
            })
        })
        .collect();
    format!(
        "{{\"meta\":{},\"entries\":{},\"truncated\":{}}}",
        meta,
        serde_json::to_string(&entries).unwrap_or_else(|_| "[]".into()),
        idx.truncated
    )
}

/// A registry row, for the server's own tests. Nothing in production builds
/// one: `open` is the only thing that registers a place, and it is the only
/// thing that may.
#[cfg(test)]
pub(crate) fn test_group(key: &str, root: &Path, tree: &Path, meta: &str, index: &str) -> Group {
    Group {
        root: root.to_path_buf(),
        slug: key.to_string(),
        key: key.to_string(),
        tree: tree.to_path_buf(),
        docs: None,
        stale: Staleness::default(),
        fingerprint: 0,
        assets: Vec::new(),
        etag: "fp0".into(),
        meta: Arc::from(meta),
        index: Arc::from(index),
    }
}

/// The `ETag` stem for a place: its digest and both epochs.
fn etag_stem(fingerprint: u64, s: &Staleness) -> String {
    format!("{fingerprint:016x}{:x}{:x}", s.derived_epoch, s.now_epoch)
}

// ── the command's body ───────────────────────────────────────────────────────

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

/// Everything `open_docs_viewer` does, minus the Tauri plumbing.
///
/// `config_dir` and `resource_dir` come from the app handle; `entries` is the
/// index the caller already walked. Returns the URL for the frontend to hand to
/// `openUrl`.
pub fn open(
    v: &Viewer,
    config_dir: &Path,
    resource_dir: Option<&Path>,
    idx: &worktrees_core::docs::DocsIndex,
    req: &Request,
) -> Result<Opened, String> {
    let vdir = viewer_dir(config_dir);
    std::fs::create_dir_all(&vdir).map_err(|e| format!("{}: {e}", vdir.display()))?;

    let mut slot = v.srv.lock().map_err(|_| "the documentation server's lock is poisoned")?;
    // Liveness the way `Shells` does it: ask the handle we hold, on every open,
    // and never a timer. A server whose accept loop is gone takes the whole
    // slot with it — including the registry, because a fresh server has a fresh
    // token and nothing registered against the old one is reachable.
    if slot.as_ref().map(|h| !h.alive()).unwrap_or(false) {
        *slot = None;
    }
    if slot.is_none() {
        if let Ok(mut places) = v.places.lock() {
            places.clear();
        }
        *slot = Some(crate::docserver::start(v.places.clone(), bundle_path(resource_dir))?);
        // A fresh server serves nothing yet, so this is the moment the old
        // trees stop being anybody's. They are COPIES of the user's documents —
        // the ones §4.3 says carry a client's signed agreement — sitting outside
        // the repo's own gitignore in a directory Spotlight and Time Machine
        // both index, so they do not get to outlive the process that needed
        // them. `cleanup` does the same on a clean exit; this is what covers a
        // crash, which a shutdown hook cannot.
        empty_dir(&vdir.join("tree"));
    }
    let srv = slot.as_ref().expect("started or returned");
    let (port, token) = (srv.port, srv.token.clone());

    let root = req.root.to_path_buf();
    let mut places = v.places.lock().map_err(|_| "the place registry is poisoned")?;
    // `place_key` stays a pure function over `(root, key)` pairs — the tie it
    // breaks has nothing to do with the rest of a `Group`, and its tests say so
    // in the shape they build.
    let taken: Vec<(PathBuf, String)> =
        places.iter().map(|g| (g.root.clone(), g.key.clone())).collect();
    let key = place_key(req.slug, &root, &taken);
    let tree = vdir.join("tree").join(tree_key(req.slug, &root));

    let derived = write_tree(&tree, &root, &idx.entries, port, &token, &key)?;
    // The images this place brought with it are part of what the tick has to
    // watch, so the digest recorded below is the caller's (taken BEFORE the
    // walk, which is the one ordering rule here) with their stats folded in.
    let fingerprint = worktrees_core::docs::fold_assets(req.fingerprint, &root, &derived.assets);
    // A place of no documents is not worth registering, and a route onto it
    // would be a shell over an empty index. The footer button is disabled on an
    // empty index, so the way here is the race: the index was walked 30 seconds
    // ago and the place has been emptied since. Say that, rather than opening a
    // blank page.
    if derived.pages.is_empty() {
        return Err(format!("{} has no documents to show", req.slug));
    }
    let meta = meta_json(req.slug, &req.stale);
    let group = Group {
        root: root.clone(),
        slug: req.slug.to_string(),
        key: key.clone(),
        tree,
        docs: req.docs.clone(),
        stale: req.stale.clone(),
        fingerprint,
        assets: derived.assets.clone(),
        etag: etag_stem(fingerprint, &req.stale),
        index: Arc::from(index_json(&meta, idx).as_str()),
        meta: Arc::from(meta.as_str()),
    };
    // Every open re-derives, registered or not (`write_tree` above), so the
    // record is REPLACED on the re-open path too — otherwise a place opened
    // twice keeps the FIRST open's fingerprint and facts, and the tick compares
    // today's documents against a digest from yesterday. It would re-derive
    // once and then settle, which is the worst version of this bug:
    // correct-looking, and wrong by exactly one stale header.
    match places.iter_mut().find(|g| g.root == root) {
        Some(g) => *g = group,
        None => places.push(group),
    }
    drop(places);

    let notes = derived.notes;
    let Some(want) = req.path else {
        return Ok(Opened { url: place_url(port, &token, &key, None), notes });
    };
    match idx.entries.iter().find(|e| e.path == want) {
        Some(e) => Ok(Opened { url: place_url(port, &token, &key, Some(&e.rel)), notes }),
        None => Err(format!("{want} is not in this place's documentation index")),
    }
}

/// The browser bundle on disk, by STAT — never by running anything.
pub fn bundle_path(resource_dir: Option<&Path>) -> Option<PathBuf> {
    resolve_bundle(std::env::var(BUNDLE_ENV).ok().as_deref(), resource_dir)
}

/// The resolution itself, with the environment as an ARGUMENT.
///
/// Split out so its test can state the precedence instead of depending on the
/// ambient environment — which the `mo` version of this did, and which made the
/// suite fail the moment it was run the way its own end-to-end test asked to be
/// run. A test that passes only when nobody is exercising the feature is not a
/// test.
fn resolve_bundle(from_env: Option<&str>, resource_dir: Option<&Path>) -> Option<PathBuf> {
    let is_file = |p: PathBuf| std::fs::metadata(&p).ok().filter(|m| m.is_file()).map(|_| p);
    // The override WINS, and a broken override does not fall through to the
    // bundle: someone who set it is testing that file, and quietly serving a
    // different one is how you spend an afternoon.
    if let Some(p) = from_env {
        return is_file(PathBuf::from(p));
    }
    is_file(resource_dir?.join(BUNDLE_REL))
}

/// Re-derive every registered place whose documents have moved on disk.
///
/// **Why this exists.** The server reads the DERIVED tree, not the user's repo,
/// so without this an edit to a real file reaches nothing: the page would poll
/// a copy that could only change when the button was pressed again, while the
/// dock re-indexes every 4 s and says "1 document new" beside it.
///
/// **The cost is the whole design.** `index_with` sniffs 8 KiB of every file
/// for its title — measured at 10.1 ms and ~3.2 MB of reads for a 400-document
/// place — and doing that per registered place every few seconds is not
/// acceptable. `docs::fingerprint_with` walks the same paths with
/// `symlink_metadata` alone (1.8 ms, zero bytes read, on the same tree) and the
/// full walk runs only when it has moved.
///
/// Four rules, all of them `Shells`' rules, because this is the same kind of
/// thing:
///
/// 1. **Nothing registered, nothing done.** No lock contention, no walk, and in
///    particular no start — a timer may not decide the user wants a server.
/// 2. **Never resurrect a dead one.** `Handle::alive` says whether the accept
///    loop is running; if it is not, the slot is dropped and the next OPEN
///    starts one. A timer that restarted would put a port back up minutes after
///    the user stopped using it, with nothing on screen to say so.
/// 3. **A failed derive keeps its old fingerprint**, so it is retried on the
///    next tick rather than recorded as done. The caller must dedupe the log
///    (lib.rs does): a place that fails will fail every 3 s.
/// 4. **Only `derived_epoch` moves.** Every other fact in the header costs a
///    git fan-out, which §15.3 refused for a click and which is worse on a
///    timer — so the page says when it was copied and, separately, when its
///    status was measured.
///
/// Returns one message per place that could not be re-derived, plus one per
/// image a derive refused to copy. Never panics on a poisoned lock's account —
/// this runs on the app's tick thread, and taking the process down over a
/// documentation copy is not a trade worth making.
pub fn refresh(v: &Viewer, now_epoch: i64) -> Vec<String> {
    let mut errs: Vec<String> = Vec::new();
    let Ok(slot) = v.srv.lock() else {
        return vec!["the documentation server's lock is poisoned; documents will not refresh".into()];
    };
    // Rule 1: a server nobody has opened is the common case, and it must cost
    // nothing at all.
    let Some(srv) = slot.as_ref() else { return errs };
    // Rule 2: ask the handle, the way every read of `Shells` does.
    if !srv.alive() {
        drop(slot);
        if let Ok(mut s) = v.srv.lock() {
            *s = None;
        }
        if let Ok(mut p) = v.places.lock() {
            p.clear();
        }
        return errs;
    }
    let (port, token) = (srv.port, srv.token.clone());
    drop(slot);
    let Ok(mut places) = v.places.lock() else {
        return vec!["the place registry is poisoned; documents will not refresh".into()];
    };
    for g in places.iter_mut() {
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
        match write_tree(&g.tree, &g.root, &idx.entries, port, &token, &g.key) {
            Ok(d) => {
                // Folded over the NEW list rather than storing `fp`: this derive
                // may have added or dropped an image, and recording a digest
                // taken over the old list would differ from the next tick's for
                // no change at all — one re-derive per asset change, forever.
                let fingerprint =
                    worktrees_core::docs::fold_assets(docs_fp, &g.root, &d.assets);
                let meta = meta_json(&g.slug, &stale);
                g.index = Arc::from(index_json(&meta, &idx).as_str());
                g.meta = Arc::from(meta.as_str());
                g.etag = etag_stem(fingerprint, &stale);
                g.fingerprint = fingerprint;
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The token and route segment every `write_tree` in these tests derives
    /// against. Fixed, so an assertion about a URL can spell it out.
    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const KEY: &str = "tick-place";
    const PORT: u16 = 6275;

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

    /// `write_tree` with this module's fixed URL parts.
    fn derive_tree(
        out: &Path,
        src: &Path,
        entries: &[DocEntry],
        _st: &Staleness,
    ) -> Result<Derived, String> {
        write_tree(out, src, entries, PORT, TOKEN, KEY)
    }

    // ── URLs ────────────────────────────────────────────────────────────────

    /// Every URL this module emits has to survive the check `derive` applies on
    /// the way into a `click` directive — otherwise the drill-down silently
    /// emits nothing and every test still passes.
    #[test]
    fn every_url_we_build_is_a_loopback_target() {
        assert!(derive::is_loopback_target(&place_url(PORT, TOKEN, "docs", None)));
        assert!(derive::is_loopback_target(&place_url(PORT, TOKEN, "docs", Some("docs/a.md"))));
        assert!(derive::is_loopback_target(&place_url(65535, TOKEN, "a-b-c", Some("x.md"))));
    }

    /// **A deep link names the document in the FRAGMENT**, because that is what
    /// the page routes on. It was a `?path=` query first, and the page — which
    /// reads `location.hash` and nothing else — answered it with the place's
    /// INDEX: the Docs tab's per-row browser action opened the right place at
    /// the wrong document, silently, and both halves were green apart.
    ///
    /// The document's own name may not end the fragment or start the page's
    /// `?h=` anchor separator. A file called `q?x.md` or `h#h.md` is legal on
    /// disk, and a URL is the one place in this feature where a filename
    /// becomes syntax.
    #[test]
    fn a_deep_link_is_a_fragment_and_survives_the_name_it_carries() {
        let u = place_url(PORT, TOKEN, "p", Some("docs/a?h=x#frag .md"));
        assert_eq!(
            u,
            format!("http://127.0.0.1:{PORT}/{TOKEN}/p/p/#/docs/a%3Fh%3Dx%23frag%20.md")
        );
        // Exactly one `#`, or the browser cuts the fragment in the wrong place.
        assert_eq!(u.matches('#').count(), 1, "{u}");
        // No `?` at all, or the page reads the tail as an anchor.
        assert!(!u.contains('?'), "{u}");
        assert!(derive::is_loopback_target(&u), "{u}");
        // And it comes back as exactly the string that went in — the page's
        // `decodeURIComponent` undoes this escaping.
        let frag = u.split_once("/#/").unwrap().1;
        assert_eq!(
            crate::docserver::pct_decode_once(frag).as_deref(),
            Some("docs/a?h=x#frag .md")
        );
    }

    // ── place keys ──────────────────────────────────────────────────────────

    /// Two places may share a slug — the same branch name in two projects — and
    /// one route segment for both would show one project's documents under the
    /// other's name. §1.1's failure with the axes swapped.
    #[test]
    fn two_places_with_one_slug_do_not_share_a_route() {
        let a = PathBuf::from("/w/one/.worktrees/docs");
        let b = PathBuf::from("/w/two/.worktrees/docs");
        let mut taken = Vec::new();
        let ga = place_key("docs", &a, &taken);
        taken.push((a.clone(), ga.clone()));
        let gb = place_key("docs", &b, &taken);
        assert_eq!(ga, "docs");
        assert_ne!(ga, gb, "a second place must not land on the first one's route");
        assert!(gb.starts_with("docs-"), "{gb}");
        // And asking again for a place already registered gives the same key,
        // or a re-open would keep making new routes.
        taken.push((b.clone(), gb.clone()));
        assert_eq!(place_key("docs", &a, &taken), ga);
        assert_eq!(place_key("docs", &b, &taken), gb);
    }

    /// A place key is a URL path segment the server validates on the way back
    /// in, so it may only ever be `[a-z0-9-]` — and may never be empty.
    #[test]
    fn a_place_key_can_only_be_a_url_path_segment() {
        let r = PathBuf::from("/w/p");
        for slug in ["_", "../etc", "a/b", "%2e%2e", "", "   ", "Feature/PLACE.1", "ünïcødé"] {
            let g = place_key(slug, &r, &[]);
            assert!(!g.is_empty(), "{slug:?} gave an empty key");
            assert!(g.len() <= PLACE_KEY_MAX, "{slug:?} gave {g}");
            assert!(crate::docserver::place_key_ok(&g), "{slug:?} gave {g}, which the server refuses");
            assert!(g.starts_with(|c: char| c.is_ascii_alphanumeric()), "{slug:?} gave {g}");
        }
        assert_eq!(place_key("Feature/PLACE.1", &r, &[]), "feature-place-1");
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

    /// The tree is what the server reads, so it has to MIRROR the place's
    /// layout — a prose `[x](./x.md)` resolves against the file's own
    /// directory, and flattening the tree would break every relative link in
    /// the user's documents.
    ///
    /// And it carries **no staleness header**. That moved to `doc`'s `meta`
    /// when the page took over rendering it: a blockquote in the text would be
    /// a second, frozen copy of numbers the reader watches change above it, and
    /// two disagreeing statements of how stale a place is are §1.1 exactly.
    #[test]
    fn the_derived_tree_mirrors_the_place_and_leaves_the_header_to_meta() {
        let src = tmp("mirror");
        std::fs::create_dir_all(src.join("docs/adr")).unwrap();
        std::fs::write(src.join("README.md"), "---\ntitle: Read me\n---\n\nbody\n").unwrap();
        std::fs::write(src.join("docs/adr/0001.md"), "# ADR\n\n[back](../../README.md)\n").unwrap();
        let entries =
            vec![entry("README.md", "Read me", &src), entry("docs/adr/0001.md", "ADR", &src)];
        let out = tmp("mirror-out");

        let derived = derive_tree(&out, &src, &entries, &stale()).unwrap().pages;
        assert_eq!(derived.len(), 2);
        assert!(out.join("README.md").is_file());
        assert!(out.join("docs/adr/0001.md").is_file());

        let readme = std::fs::read_to_string(out.join("README.md")).unwrap();
        assert!(!readme.contains("25 behind"), "the header is prose again: {readme}");
        assert!(!readme.contains("origin/main"), "{readme}");
        assert!(!readme.starts_with('>'), "{readme}");
        // Frontmatter gone, the title it carried restored as a heading, and
        // that heading is the first thing on the page.
        assert!(readme.starts_with("# Read me"), "{readme}");
        assert!(!readme.contains("---\ntitle:"), "{readme}");
        // …and the facts are in `meta` instead, which is the other half of the
        // same assertion: they did not simply disappear.
        let meta = meta_json("viewer-guarded", &stale());
        let v: serde_json::Value = serde_json::from_str(&meta).unwrap();
        assert_eq!(v["place"], "viewer-guarded");
        assert_eq!(v["behind"], 25);
        assert_eq!(v["base"], "origin/main");
        assert_eq!(v["dirty"], 0);
        assert_eq!(v["status_epoch"], 1_000_060);

        // Prose links are untouched — the mirror is what makes them resolve.
        let adr = std::fs::read_to_string(out.join("docs/adr/0001.md")).unwrap();
        assert!(adr.contains("[back](../../README.md)"), "{adr}");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// `dirty` is a COUNT in the contract, and "clean" and "not computed" are
    /// different answers — a place whose status could not be measured must not
    /// render as a place with nothing to commit.
    #[test]
    fn an_unmeasured_dirty_count_is_not_a_clean_place() {
        let mut s = stale();
        s.dirty = None;
        s.dirty_files = None;
        let v: serde_json::Value = serde_json::from_str(&meta_json("p", &s)).unwrap();
        assert!(v["dirty"].is_null(), "{v}");
        s.dirty = Some(false);
        let v: serde_json::Value = serde_json::from_str(&meta_json("p", &s)).unwrap();
        assert_eq!(v["dirty"], 0);
        s.dirty = Some(true);
        s.dirty_files = Some(3);
        let v: serde_json::Value = serde_json::from_str(&meta_json("p", &s)).unwrap();
        assert_eq!(v["dirty"], 3);
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
        derive_tree(&out, &src, &entries, &stale()).unwrap();

        let text = std::fs::read_to_string(out.join("docs/overview.md")).unwrap();
        assert!(!text.contains("evil.example"), "an author's click survived: {text}");
        let want = place_url(PORT, TOKEN, KEY, Some("docs/runbook.md"));
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
        derive_tree(&out, &src, &entries, &stale()).unwrap();
        let text = std::fs::read_to_string(out.join("bad.md")).unwrap();
        assert!(text.contains("not valid UTF-8"), "{text}");
        assert!(text.starts_with("# bad"), "{text}");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A document removed from the place must leave the tree, or the server goes
    /// on serving a page the index no longer lists — a stale document, which is
    /// the joke this feature cannot afford.
    #[test]
    fn a_document_that_left_the_index_leaves_the_tree() {
        let src = tmp("prune");
        std::fs::create_dir_all(src.join("docs")).unwrap();
        std::fs::write(src.join("a.md"), "# A\n").unwrap();
        std::fs::write(src.join("docs/b.md"), "# B\n").unwrap();
        let out = tmp("prune-out");

        let both = vec![entry("a.md", "A", &src), entry("docs/b.md", "B", &src)];
        derive_tree(&out, &src, &both, &stale()).unwrap();
        assert!(out.join("docs/b.md").is_file());

        derive_tree(&out, &src, &both[..1], &stale()).unwrap();
        assert!(out.join("a.md").is_file());
        assert!(!out.join("docs/b.md").exists(), "a dropped document stayed in the tree");
        assert!(!out.join("docs").exists(), "the directory it emptied stayed");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A removed place's derived documents are the LAST readable copy of them —
    /// `cmd_rm` deleted the originals — and they would be on a port. Keyed by
    /// the canonical root, so the same slug in another project keeps its own.
    ///
    /// The REGISTRATION goes too, so the route answers `410 Gone` at once
    /// rather than after the next tick: a page open on that place says the place
    /// is gone instead of polling a hole.
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

        let v = Viewer::default();
        for root in [&gone, &other] {
            v.places.lock().unwrap().push(registry_entry("docs", root, &trees.join(tree_key("docs", root))));
        }

        forget_place(&v, &cfg, "docs", &gone);
        assert!(!trees.join(tree_key("docs", &gone)).exists(), "the removed place's documents stayed");
        assert!(trees.join(tree_key("docs", &other)).is_dir(), "the same slug in another project lost its tree");
        let left: Vec<PathBuf> = v.places.lock().unwrap().iter().map(|g| g.root.clone()).collect();
        assert_eq!(left, vec![other.clone()], "the route onto the removed place is still registered");

        // And a place that was never opened in the browser has no tree at all,
        // which is the common case and not an error.
        forget_place(&v, &cfg, "never-opened", &gone);
        let _ = std::fs::remove_dir_all(&cfg);
    }

    /// The derived trees are copies of documents §4.3 says carry a client's
    /// signed agreement, in a directory Spotlight and Time Machine both index.
    /// They do not get to outlive the process that needed them.
    #[test]
    fn nothing_the_derive_wrote_outlives_a_clean_exit() {
        let cfg = tmp("cleanup");
        let trees = viewer_dir(&cfg).join("tree");
        std::fs::create_dir_all(trees.join("p-0000/sub")).unwrap();
        std::fs::write(trees.join("p-0000/sub/x.md"), "a client's signed agreement").unwrap();
        std::fs::write(trees.join("loose"), "x").unwrap();

        cleanup(&cfg);

        assert!(trees.is_dir(), "the directory itself must survive");
        assert_eq!(std::fs::read_dir(&trees).unwrap().count(), 0, "derived documents survived");
        let _ = std::fs::remove_dir_all(&cfg);
    }

    // ── images ──────────────────────────────────────────────────────────────

    /// A 1×1 PNG. Real bytes rather than a placeholder, because the end-to-end
    /// tests hand them to a server that states a content type.
    pub(crate) const PNG: [u8; 67] = [
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

        let d = derive_tree(&out, &src, &entries, &stale()).unwrap();

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
    /// served on a loopback port, so a copy here is a publication.
    #[test]
    fn a_symlinked_image_copies_nothing() {
        let src = tmp("img-link");
        let secret = tmp("img-link-secret").join("id_rsa");
        std::fs::write(&secret, "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
        doc_at(&src, "docs/p.md", "# P\n\n![logo](logo.png)\n");
        std::os::unix::fs::symlink(&secret, src.join("docs/logo.png")).unwrap();
        let entries = vec![entry("docs/p.md", "P", &src)];
        let out = tmp("img-link-out");

        let d = derive_tree(&out, &src, &entries, &stale()).unwrap();

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
        let _ = std::fs::remove_dir_all(secret.parent().unwrap());
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

        let d = derive_tree(&out, &src, &entries, &stale()).unwrap();

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

        let d = derive_tree(&out, &src, &entries, &stale()).unwrap();

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

    /// **An SVG is copied again, and only because the response now says so.**
    ///
    /// It was excluded under `mo`, which handed any asset back by direct URL
    /// with a content type we did not control — where an SVG is a top-level
    /// document and its `<script>` runs. Owning the server replaces that
    /// permanent amputation with two headers (findings §6.4), so the
    /// allow-list entry and the headers are ONE decision and this test asserts
    /// both halves: remove the CSP from the asset response and this goes red,
    /// which is the only thing standing between a copied SVG and a live one.
    #[test]
    fn an_svg_is_copied_only_because_the_response_makes_it_inert() {
        let src = tmp("img-svg");
        doc_at(&src, "p.md", "# P\n\n![logo](logo.svg)\n![shot](shot.png)\n");
        std::fs::write(
            src.join("logo.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>fetch('/steal')</script></svg>",
        )
        .unwrap();
        png_at(&src, "shot.png");
        let entries = vec![entry("p.md", "P", &src)];
        let out = tmp("img-svg-out");

        let d = derive_tree(&out, &src, &entries, &stale()).unwrap();

        assert!(out.join("logo.svg").is_file(), "the SVG was not copied");
        assert!(out.join("shot.png").is_file());
        assert!(d.assets.iter().any(|a| a.ends_with(".svg")), "{:?}", d.assets);
        // The half that makes it safe. `docserver` states the policy; this is
        // the assertion that ties it to the allow-list, so that deleting the
        // header cannot quietly re-arm every `logo.svg` in every place.
        assert!(
            crate::docserver::ASSET_CSP.contains("default-src 'none'"),
            "an SVG is served with no policy on it"
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

        derive_tree(&out, &src, &entries, &stale()).unwrap();
        assert!(out.join("docs/shots/a.png").is_file(), "the first derive did not copy it");

        // The author drops the image from the document.
        doc_at(&src, "docs/p.md", "# P\n\nno picture any more\n");
        let d = derive_tree(&out, &src, &entries, &stale()).unwrap();

        assert!(out.join("docs/p.md").is_file(), "the prune took the document too");
        assert!(!out.join("docs/shots/a.png").exists(), "a dropped image is still being served");
        assert!(!out.join("docs/shots").exists(), "the directory it emptied stayed");
        assert!(d.assets.is_empty(), "{:?}", d.assets);

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&out);
    }

    // ── the tick ────────────────────────────────────────────────────────────

    /// One runtime for the whole test binary. `docserver::start` spawns onto
    /// the ambient runtime, and a runtime dropped at the end of a test takes
    /// its server with it — which would make every liveness assertion below
    /// about the test harness rather than about the code.
    pub(crate) fn rt() -> &'static tokio::runtime::Runtime {
        static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
        })
    }

    /// A registry row with nothing in it but an identity — enough for the tests
    /// that only ask which places are registered.
    fn registry_entry(slug: &str, root: &Path, tree: &Path) -> Group {
        Group {
            root: root.to_path_buf(),
            slug: slug.to_string(),
            key: slug.to_string(),
            tree: tree.to_path_buf(),
            docs: None,
            stale: stale(),
            fingerprint: 0,
            assets: Vec::new(),
            etag: "0".into(),
            meta: Arc::from("{}"),
            index: Arc::from("{}"),
        }
    }

    /// A viewer with a REAL server running and one place registered.
    ///
    /// The server is real because the tick's second rule is about liveness, and
    /// a stand-in that is merely alive cannot answer whether `Handle::alive`
    /// means what `refresh` reads it as.
    fn registered(
        place: &Path,
        tree: &Path,
        fingerprint: u64,
        assets: Vec<String>,
        st: Staleness,
    ) -> Viewer {
        let v = Viewer::default();
        let _g = rt().enter();
        *v.srv.lock().unwrap() = Some(crate::docserver::start(v.places.clone(), None).unwrap());
        v.places.lock().unwrap().push(Group {
            root: place.to_path_buf(),
            slug: KEY.into(),
            key: KEY.into(),
            tree: tree.to_path_buf(),
            docs: None,
            stale: st,
            fingerprint,
            assets,
            etag: "0".into(),
            meta: Arc::from("{}"),
            index: Arc::from("{}"),
        });
        v
    }

    /// The fingerprint the tick is carrying for the one registered place.
    fn recorded_fp(v: &Viewer) -> u64 {
        v.places.lock().unwrap()[0].fingerprint
    }

    /// Derive a place the way `open` does, and hand back the state the tick
    /// carries forward: the digest with this place's images folded into it, and
    /// the list of images that fold was over.
    fn first_derive(place: &Path, tree: &Path, st: &Staleness) -> (u64, Vec<String>) {
        let fp = worktrees_core::docs::fingerprint(place);
        let idx = worktrees_core::docs::index(place);
        let d = derive_tree(tree, place, &idx.entries, st).unwrap();
        (worktrees_core::docs::fold_assets(fp, place, &d.assets), d.assets)
    }

    /// Stops the server however the test ends.
    ///
    /// A `#[test]` that panics before reaching its own `kill` leaves a REAL
    /// server holding a REAL port, on the machine of whoever ran it. That is not
    /// hypothetical: proving an earlier tree-wipe assertion red left a viewer
    /// listening on 53381 until it was found by hand — a test suite reproducing,
    /// in miniature, the exact failure the lifecycle rules exist to prevent.
    struct Reaper<'a>(&'a Viewer);
    impl Drop for Reaper<'_> {
        fn drop(&mut self) {
            kill(self.0);
        }
    }

    /// **The bug, as one assertion.** The server reads the DERIVED tree, not the
    /// repo, so before the tick existed a live page was perfect over a copy that
    /// could only change when the button was pressed again: editing a document
    /// reached nothing, adding one reached nothing, and reloading did not help
    /// because the derived file genuinely had not changed.
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
            std::thread::sleep(std::time::Duration::from_millis(2));
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
        assert_ne!(recorded_fp(&v), fp, "the fingerprint was not carried forward; every tick would re-derive");
        // …including the `ETag` stem, or a page polling with `If-None-Match`
        // gets a `304` over text that just changed — the update arrives on
        // disk and never reaches the reader.
        assert_ne!(v.places.lock().unwrap()[0].etag, "0", "the ETag did not move with the document");

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
        std::thread::sleep(std::time::Duration::from_millis(20));
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
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        assert!(refresh(&v, 1_000_000 + 600).is_empty(), "the re-derive reported an error");

        assert_eq!(
            std::fs::read(tree.join("shots/a.png")).unwrap(),
            newer,
            "the edited image never reached the derived copy"
        );
        assert_ne!(recorded_fp(&v), fp, "the fingerprint was not carried forward; every tick would re-derive");

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
    /// document moved.
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
        std::thread::sleep(std::time::Duration::from_millis(20));
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

    /// `meta` has to date the COPY, and it may not claim the git facts were
    /// measured then. Every one of those costs a fan-out to recompute — §15.3
    /// refused that for a click, and a timer is worse — so after a background
    /// re-derive the page carries two instants and the older one belongs to the
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

        for _ in 0..200 {
            std::fs::write(place.join("README.md"), "# Read me\n\nAFTER\n").unwrap();
            if worktrees_core::docs::fingerprint(&place) != fp {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        assert!(refresh(&v, 1_789_779_661).is_empty());

        let meta: serde_json::Value =
            serde_json::from_str(&v.places.lock().unwrap()[0].meta).unwrap();
        assert_eq!(meta["derived_epoch"], 1_789_779_661i64, "the copy is not dated now: {meta}");
        assert_eq!(meta["status_epoch"], 1_789_776_000i64, "the facts claim to be fresh: {meta}");
        // The facts themselves are untouched — the tick re-reads documents, not
        // git.
        assert_eq!(meta["behind"], 25);
        assert_eq!(meta["base"], "origin/main");

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// Two refusals that are one rule: a timer may not decide the user wants a
    /// server.
    ///
    /// Nothing registered must cost nothing at all, and a server that has died
    /// must stay dead until somebody presses the button — restarting from a
    /// timer would put a loopback port back up minutes after the user stopped
    /// using it, with nothing on screen to say so. `Shells` makes the same
    /// promise for the same reason.
    #[test]
    fn the_tick_never_starts_and_never_resurrects() {
        let v = Viewer::default();
        assert!(refresh(&v, 1_000_000).is_empty(), "an idle server reported something");
        assert!(v.srv.lock().unwrap().is_none(), "the tick put a server in an empty slot");

        let place = tmp("tick-dead");
        let tree = tmp("tick-dead-tree");
        std::fs::write(place.join("README.md"), "# Read me\n\nbefore\n").unwrap();
        let st = stale();
        let (fp, assets) = first_derive(&place, &tree, &st);
        let before = std::fs::metadata(tree.join("README.md")).unwrap().modified().unwrap();
        std::fs::write(place.join("README.md"), "# Read me\n\nAFTER\n").unwrap();

        let v = registered(&place, &tree, fp, assets, st);
        let _reap = Reaper(&v);
        v.srv.lock().unwrap().as_ref().unwrap().abort_for_test();
        for _ in 0..200 {
            if !v.srv.lock().unwrap().as_ref().unwrap().alive() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(refresh(&v, 1_000_000 + 600).is_empty());
        assert!(v.srv.lock().unwrap().is_none(), "a dead server stayed in the slot");
        assert!(v.places.lock().unwrap().is_empty(), "the routes of a dead server stayed registered");
        assert_eq!(
            before,
            std::fs::metadata(tree.join("README.md")).unwrap().modified().unwrap(),
            "the tick wrote documents for a server that is not running",
        );

        let _ = std::fs::remove_dir_all(&place);
        let _ = std::fs::remove_dir_all(&tree);
    }

    /// Detection is a STAT. Nothing about the browser bundle may be executed,
    /// and nothing about it may run at launch.
    #[test]
    fn a_missing_bundle_is_absent_rather_than_an_error() {
        let d = tmp("bundle");
        assert!(resolve_bundle(None, Some(&d)).is_none(), "an empty resource dir is not an install");
        assert!(resolve_bundle(None, None).is_none(), "no resource dir at all is not an install");

        // A DIRECTORY where the bundle should be is still "not installed".
        std::fs::create_dir_all(d.join(BUNDLE_REL)).unwrap();
        assert!(resolve_bundle(None, Some(&d)).is_none(), "a directory passed as the bundle");
        let _ = std::fs::remove_dir_all(d.join(BUNDLE_REL));

        let real = d.join(BUNDLE_REL);
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "export {}\n").unwrap();
        assert_eq!(resolve_bundle(None, Some(&d)).as_deref(), Some(real.as_path()));

        // The override wins, and a BROKEN override does not fall through to the
        // bundled one — serving a different file than the one you named is how
        // an afternoon goes missing.
        let other = d.join("other.js");
        std::fs::write(&other, "export {}\n").unwrap();
        assert_eq!(resolve_bundle(Some(other.to_str().unwrap()), Some(&d)).as_deref(), Some(other.as_path()));
        assert!(resolve_bundle(Some("/does/not/exist/viewer.js"), Some(&d)).is_none());

        let _ = std::fs::remove_dir_all(&d);
    }
}

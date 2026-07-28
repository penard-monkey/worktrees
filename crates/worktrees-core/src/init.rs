//! `worktrees init` — the suggestion flow (proposal §9).
//!
//! The tool tells a project what it qualifies for. Detection needs no config, so
//! this module is the one place that looks at a repo and guesses; everything it
//! guesses is printed for a human to accept or reject, and **nothing is written
//! without confirmation**.
//!
//! Same probe/plan discipline as `materialize.rs`, for the same reason:
//!
//! ```text
//! probe    the ONLY filesystem read  — impure, answers "what is in this repo?"
//! detect   PURE                      — every signal in §9's table lives here
//! render   PURE                      — the emitted `.worktrees.toml`, as text
//! ```
//!
//! `detect` and `render` are pure functions over a `Facts` struct a unit test can
//! build from literals, so every row of §9's table is testable without a
//! filesystem — and `render`'s output is asserted to round-trip through
//! `projcfg::parse` with ZERO findings, which is the one test that keeps the
//! emitter and the parser from drifting apart.
//!
//! ⚠ The file is hand-templated as a string, never serialized (§8). No serde
//! serializer emits comments, and this file's whole value is its comments: a
//! `[[file]]` entry that does not say *why it is a copy* teaches nothing, and the
//! credential entries have to say out loud that they fail silently.

use std::fs;
use std::path::{Path, PathBuf};

use crate::git;
use crate::projcfg::{self, Ports};

/// The legacy per-repo prefix file `[project] prefix` subsumes (§5).
pub const PREFIX_FILE: &str = ".worktree-prefix";

/// The compose file whose mere presence means "this project already namespaces a
/// stack per worktree" (§9).
const WORKTREE_COMPOSE: &[&str] = &["docker-compose.worktree.yml", "docker-compose.worktree.yaml"];

// ── walk bounds ──────────────────────────────────────────────────────────────
// A monorepo is large and `init` must not take 30 seconds — and the passive nudge
// (§9) runs this on `new`, which is a hot path. So the walk is bounded four ways
// and reports when a bound was hit, rather than silently returning half a repo.

/// Directory depth below the repo root. Generous on purpose: credentials live
/// deep (`apps/mobile/android/app/src/google-services.json` is 6), and a bound
/// this walk trips on every repo would make the "I stopped early" note noise.
const MAX_DEPTH: usize = 8;
/// Directories descended into.
const MAX_DIRS: usize = 2_000;
/// `read_dir` entries looked at, across the whole walk.
const MAX_ENTRIES: usize = 20_000;
/// Candidate files collected. Well past any real repo; a hit is one `[[file]]`
/// entry and §4 caps the config at 256 of those anyway.
const MAX_HITS: usize = 64;
/// A compose file bigger than this is not read — the port scan is a suggestion,
/// not a parser, and nothing good is at the end of a 1 MiB compose file.
const MAX_COMPOSE_BYTES: u64 = 512 * 1024;
/// Compose files whose contents are read.
const MAX_COMPOSE_FILES: usize = 8;

/// Heavy directories that never hold a credential worth linking. Hidden
/// directories are skipped wholesale by the walk (one rule covers `.git`,
/// `.worktrees`, `.next`, `.venv`, `.turbo`, `.gradle`, …) — hidden *files* like
/// `.env` are of course still visited, since they are the point.
const SKIP_DIRS: &[&str] =
    &["node_modules", "target", "vendor", "build", "dist", "out", "coverage", "Pods", "DerivedData"];

// ── facts ────────────────────────────────────────────────────────────────────

/// Which §9 row a candidate came from. The distinction is not cosmetic: the
/// credential class is the one that **fails silently** (§1.2), so it is flagged
/// louder in the emitted file and it is the only class the passive nudge counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// `.env*` — gitignored, present, tracked nowhere.
    Env,
    /// `google-services.json`, `GoogleService-Info.plist`, `*.keystore`, `*.jks`,
    /// `*-service-account*.json`.
    Credential,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Repo-relative path, `/`-separated.
    pub rel: String,
    pub kind: Kind,
}

impl Candidate {
    pub fn new(rel: impl Into<String>, kind: Kind) -> Self {
        Candidate { rel: rel.into(), kind }
    }
}

/// What a heuristic line scan could read out of one compose file. See
/// [`scan_compose_ports`] for exactly how much that is — and is not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposeFacts {
    pub rel: String,
    /// The basename is `docker-compose.worktree.yml`: this project already
    /// namespaces a stack per worktree.
    pub is_override: bool,
    /// `(service, host_port)` read confidently, in file order.
    pub published: Vec<(String, u32)>,
    /// Entries inside a `ports:` block that the scan did NOT understand. Nonzero
    /// means "this file publishes ports I can't name" — enough to suggest
    /// `[ports]`, never enough to invent a number.
    pub unreadable: usize,
}

impl ComposeFacts {
    fn publishes(&self) -> bool {
        !self.published.is_empty() || self.unreadable > 0
    }
}

/// One existing worktree, and how many suggested files it is missing. This is
/// §9's last row — "existing worktrees whose files diverge from main" — made
/// computable *before* a config exists: divergence is measured against what the
/// suggestion would declare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceFacts {
    pub slug: String,
    pub missing: usize,
}

/// Everything `detect` is allowed to know.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Gitignored, present on disk, tracked nowhere — all three checked, none
    /// guessed (`git check-ignore` and `git ls-files`, one call each).
    pub files: Vec<Candidate>,
    pub compose: Vec<ComposeFacts>,
    /// First line of `.worktree-prefix`, if the repo has one.
    pub prefix_file: Option<String>,
    pub places: Vec<PlaceFacts>,
    /// A walk bound was hit, so `files`/`compose` may be incomplete. Reported,
    /// never swallowed.
    pub truncated: bool,
}

// ── the suggestion ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortsSuggestion {
    /// The derived map, in emission order. Empty is legal — then the block is
    /// emitted commented out, because §9's rule is *never silently invent port
    /// numbers*.
    pub base: Vec<(String, u32)>,
    /// Which file the numbers came from.
    pub source: String,
    /// `Some(reason)` ⇒ the block is emitted COMMENTED OUT, with the reason.
    pub commented: Option<String>,
}

/// A candidate the config parser would refuse (§4). Emitted COMMENTED OUT with
/// the parser's own reason — a real repo may legitimately hold `apps$1/.env`,
/// and dropping it silently is the failure §1.2 is about while failing the run
/// over it is worse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub rel: String,
    /// Why Layer A refused it, in the parser's own words.
    pub why: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Suggestion {
    pub files: Vec<Candidate>,
    /// Found on disk, not declarable. See [`Rejected`].
    pub rejected: Vec<Rejected>,
    pub compose: Option<String>,
    pub ports: Option<PortsSuggestion>,
    /// Contents of `.worktree-prefix`. Emitted COMMENTED OUT — §12 defers
    /// `[project] prefix` to v3, and an uncommented one would parse to a
    /// `deferred-key` warning on every command from here on.
    pub prefix: Option<String>,
    /// Places that are already missing at least one suggested file.
    pub stale_places: Vec<String>,
    pub truncated: bool,
}

impl Suggestion {
    /// Nothing to configure. `stale_places` alone does not count — a divergence
    /// from an empty declaration is not a divergence. A REJECTED candidate does
    /// count: there is a real file here that the config cannot name, and "nothing
    /// to configure" would be exactly the silent drop this module refuses.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && self.rejected.is_empty()
            && self.compose.is_none()
            && self.ports.is_none()
            && self.prefix.is_none()
    }

    pub fn credentials(&self) -> usize {
        self.files.iter().filter(|c| c.kind == Kind::Credential).count()
    }
}

// ── detect — PURE ────────────────────────────────────────────────────────────

/// §9's table, as a pure function. Every row is here and nothing here reads the
/// filesystem.
pub fn detect(f: &Facts) -> Suggestion {
    let mut candidates = f.files.clone();
    // Deterministic order: the `.env` group first (it is what a reader expects to
    // see at the top), credentials after, each sorted by path.
    candidates.sort_by(|a, b| (a.kind, &a.rel).cmp(&(b.kind, &b.rel)));
    candidates.dedup_by(|a, b| a.rel == b.rel);
    let (files, rejected) = split_declarable(candidates);

    let compose = f.compose.iter().find(|c| c.is_override).map(|c| c.rel.clone());

    // `[ports]` is suggested by EITHER docker signal: the worktree override (the
    // project already namespaces a stack, so it wants slots), or any compose file
    // publishing host ports with no override present (§9 — "the project has
    // collisions waiting even without docker namespacing").
    let port_src = f
        .compose
        .iter()
        .find(|c| c.is_override && !c.published.is_empty())
        .or_else(|| f.compose.iter().find(|c| !c.published.is_empty()))
        .or_else(|| f.compose.iter().find(|c| c.is_override))
        .or_else(|| f.compose.iter().find(|c| c.publishes()));
    let ports = port_src.map(|c| {
        let base = derive_base(&c.published);
        let commented = if base.is_empty() {
            Some(format!(
                "no host port could be read confidently out of {} — fill this in by hand",
                c.rel
            ))
        } else {
            // The lint is free (§6) and it is the difference between a suggestion
            // and a trap: bases a stride apart make worktree 3's API port worktree
            // 1's Postgres port. A map that fails it is emitted commented out with
            // the reason, so the file still parses clean.
            lint_of(&base).err()
        };
        PortsSuggestion { base, source: c.rel.clone(), commented }
    });

    Suggestion {
        files,
        rejected,
        compose,
        ports,
        prefix: f.prefix_file.clone(),
        stale_places: f.places.iter().filter(|p| p.missing > 0).map(|p| p.slug.clone()).collect(),
        truncated: f.truncated,
    }
}

/// Layer A (§4), applied at DETECT time rather than discovered at write time.
///
/// `probe` reads real filenames, and a perfectly legal repo can hold
/// `apps$1/.env`, a `~backup/` directory, a path over the 1024-byte limit, or
/// `A/.env` beside `a/.env` on a case-sensitive filesystem. Every one of those
/// makes `projcfg::parse` reject the WHOLE config — so emitting them live turns
/// a legal repo into `init` exiting 1 with "please report this". They are
/// emitted COMMENTED OUT with the parser's own reason instead, exactly like a
/// `[ports]` map that fails the lint: never dropped, never a failure.
fn split_declarable(candidates: Vec<Candidate>) -> (Vec<Candidate>, Vec<Rejected>) {
    let (mut files, mut rejected) = (Vec::new(), Vec::new());
    let mut seen: Vec<(String, String)> = Vec::new(); // (case-folded, first path)
    for c in candidates {
        if let Err(e) = projcfg::RelPath::parse(&c.rel) {
            rejected.push(Rejected { rel: c.rel, why: e.message });
            continue;
        }
        // The case-only-duplicate rule is list-level — it needs the other
        // entries, so it cannot live inside `RelPath::parse`. Same fold as
        // `projcfg`'s (`to_lowercase`), and same caveat: it is a lowercase
        // mapping, not Unicode case folding.
        let fold = c.rel.to_lowercase();
        if let Some((_, first)) = seen.iter().find(|(k, _)| *k == fold) {
            rejected.push(Rejected {
                why: format!(
                    "case-only duplicate: `{}` and `{first}` are one file on macOS and two on Linux",
                    c.rel
                ),
                rel: c.rel,
            });
            continue;
        }
        seen.push((fold, c.rel.clone()));
        files.push(c);
    }
    (files, rejected)
}

/// Default `[ports]` stride/max_slots, matching `projcfg`'s own defaults — they
/// are emitted explicitly because this file teaches.
const STRIDE: u32 = 100;
const MAX_SLOTS: u32 = 50;

fn lint_of(base: &[(String, u32)]) -> Result<(), String> {
    let p = Ports {
        stride: STRIDE,
        max_slots: MAX_SLOTS,
        base: base.iter().map(|(k, v)| (k.clone(), *v)).collect(),
    };
    projcfg::lint_ports(&p).map_err(|e| e.message)
}

/// `(service, host_port)` → a shell-safe `[ports] base` map. Nothing is invented:
/// every key is the service's own name and every value came out of the file. A
/// service publishing two ports gets `NAME` and `NAME_<port>` rather than a
/// silently dropped second entry.
fn derive_base(published: &[(String, u32)]) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for (service, port) in published {
        let key = shell_key(service);
        if out.iter().any(|(k, v)| k == &key && v == port) {
            continue; // the same service:port twice — one entry
        }
        let key = if out.iter().any(|(k, _)| k == &key) { format!("{key}_{port}") } else { key };
        if out.iter().any(|(k, _)| k == &key) {
            continue;
        }
        out.push((key, *port));
    }
    out
}

/// A service name as a `[A-Za-z_][A-Za-z0-9_]*` shell variable name — `.worktree.env`
/// is `set -a; . `-sourced, so a base key is a shell name and not free text (§3).
fn shell_key(service: &str) -> String {
    let mut s: String = service
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect();
    if !s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
        s.insert(0, '_');
    }
    s
}

// ── render — PURE ────────────────────────────────────────────────────────────

/// The `.worktrees.toml` this repo would get, as text. Hand-templated (§8).
///
/// INVARIANT: whatever comes out of here parses through `projcfg::parse` with
/// zero findings. That is asserted by `emitted_config_round_trips_with_no_findings`
/// for every section combination, and re-checked at runtime before `init` writes.
pub fn render(s: &Suggestion) -> String {
    let mut o = String::new();
    o.push_str(
        "# .worktrees.toml — per-project setup for the `worktrees` tool. COMMITTED:\n\
         # this is project structure, like docker-compose.yml, not per-machine state.\n\
         #\n\
         # Generated by `worktrees init` from what this repo looks like today. Nothing\n\
         # below was inferred with certainty — read it, delete what does not apply.\n\
         # Every path is repo-relative; absolute paths, `~`, `$` and `.git` are refused.\n",
    );

    if s.files.is_empty() && s.rejected.is_empty() {
        o.push_str(
            "\n# ── files ─────────────────────────────────────────────────────────────────\n\
             # No gitignored credential file was found in this checkout. When one appears,\n\
             # declare it here so every worktree gets it:\n\
             #\n\
             # [[file]]\n\
             # path = \".env\"\n",
        );
    } else {
        o.push_str(
            "\n# ── files ─────────────────────────────────────────────────────────────────\n\
             # One list, per-entry mode: anything gitignored that a build or a script\n\
             # reads. `mode` defaults to \"link\" — ONE source of truth in the main\n\
             # checkout, because a per-worktree copy drifts and you fix the same value\n\
             # five times. Use mode = \"copy\" ONLY for a file something REWRITES at\n\
             # runtime: a link would scribble on the main checkout, and through it on\n\
             # every other worktree. Each copy entry should say why it is a copy.\n",
        );
        let mut in_credentials = false;
        for c in &s.files {
            if c.kind == Kind::Credential && !in_credentials {
                in_credentials = true;
                o.push_str(
                    "\n# ⚠ Credentials. These are the ones worth being careful about: when one\n\
                     # is missing, nothing fails. The build succeeds, the app runs, and the\n\
                     # feature it powers is simply dead — an Android build with no FCM sender\n\
                     # id gets no push token, no exception, no log line, and you find out on a\n\
                     # device days later. That silence is why this file exists.\n",
                );
            }
            o.push_str("\n[[file]]\n");
            o.push_str(&format!("path = {}\n", toml_string(&c.rel)));
        }
    }

    if !s.rejected.is_empty() {
        o.push_str(
            "\n# ⚠ Found in this checkout and NOT declared. Each breaks a rule the parser\n\
             # applies to the WHOLE file (§4), so uncommenting one here would fail every\n\
             # command — rename the file, or link it by hand. They are listed rather than\n\
             # dropped because a credential nobody knows about is the bug this file fixes.\n",
        );
        for r in &s.rejected {
            o.push_str(&format!("#\n# {}\n", comment_line(&r.why)));
            o.push_str("# [[file]]\n");
            o.push_str(&format!("# path = {}\n", toml_string(&r.rel)));
        }
    }

    if let Some(p) = &s.ports {
        o.push_str(
            "\n# ── ports ─────────────────────────────────────────────────────────────────\n\
             # Port isolation. Stands alone — no docker required. Each place takes the\n\
             # lowest free slot k, and every port below becomes base + stride*k. The\n\
             # numbers are written to <worktree>/.worktree.env, which the tool owns and\n\
             # your scripts source. Run `worktrees provision --all` after editing this.\n",
        );
        o.push_str(&format!("# Read out of {}.\n", p.source));
        match &p.commented {
            Some(why) => {
                o.push_str(&format!("# ⚠ Left commented out: {why}.\n"));
                o.push_str("#\n# [ports]\n");
                o.push_str(&format!("# stride = {STRIDE}\n# max_slots = {MAX_SLOTS}\n"));
                if p.base.is_empty() {
                    o.push_str("# base = { API = 3000, PG = 5432 }\n");
                } else {
                    o.push_str(&format!("# base = {}\n", inline_base(&p.base)));
                }
            }
            None => {
                o.push_str("[ports]\n");
                o.push_str(&format!("stride = {STRIDE}\nmax_slots = {MAX_SLOTS}\n"));
                o.push_str(&format!("base = {}\n", inline_base(&p.base)));
            }
        }
    }

    if let Some(file) = &s.compose {
        o.push_str(
            "\n# ── compose ───────────────────────────────────────────────────────────────\n\
             # Only because this repo already has a per-worktree compose file. The tool\n\
             # assembles the argv itself — `docker compose -f <file> -p <project> …` —\n\
             # so this section is data, never a command. {prefix} and {slug} are the\n\
             # only placeholders. The stack is torn down when the worktree is removed.\n",
        );
        o.push_str("[compose]\n");
        o.push_str(&format!("file = {}\n", toml_string(file)));
        o.push_str("project = \"{prefix}-wt-{slug}\"\n");
    }

    if let Some(prefix) = &s.prefix {
        o.push_str(
            "\n# ── prefix ────────────────────────────────────────────────────────────────\n\
             # This repo has a .worktree-prefix file, which `[project] prefix` will\n\
             # subsume. It is COMMENTED OUT on purpose: the key is parsed but not yet\n\
             # honored, because the prefix is the tmux session identity — changing it\n\
             # renames every live session, and `close` would report \"nothing to close\"\n\
             # for a session that is very much alive. Keep using .worktree-prefix.\n",
        );
        o.push_str("#\n# [project]\n");
        o.push_str(&format!("# prefix = {}\n", toml_string(prefix)));
    }

    o
}

/// A TOML basic string. Every path here came from a `read_dir` on this machine,
/// so this escapes what can actually occur in a filename rather than pretending
/// to be a general TOML writer.
fn toml_string(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// One `#` comment line's worth of text. The reason a path was rejected quotes
/// the path itself, and a path can contain a newline — which would end the
/// comment and leave the rest of the reason standing as live TOML.
fn comment_line(s: &str) -> String {
    s.chars().map(|c| if (c as u32) < 0x20 || c == '\u{7f}' { ' ' } else { c }).collect()
}

fn inline_base(base: &[(String, u32)]) -> String {
    let body: Vec<String> = base.iter().map(|(k, v)| format!("{k} = {v}")).collect();
    format!("{{ {} }}", body.join(", "))
}

// ── probe — the ONLY filesystem read ─────────────────────────────────────────

/// Everything `detect` needs, read once. Bounded (see the constants above).
pub fn probe(main_root: &Path, wt_root: &Path) -> Facts {
    let (candidates, compose_rels, truncated) = walk(main_root);
    let files = keep_gitignored_untracked(main_root, candidates);

    let compose = compose_rels
        .into_iter()
        .take(MAX_COMPOSE_FILES)
        .map(|rel| {
            let is_override = WORKTREE_COMPOSE.contains(&basename(&rel));
            let (published, unreadable) = match read_capped(&main_root.join(&rel)) {
                Some(text) => scan_compose_ports(&text),
                None => (Vec::new(), 0),
            };
            ComposeFacts { rel, is_override, published, unreadable }
        })
        .collect();

    let prefix_file = fs::read_to_string(main_root.join(PREFIX_FILE))
        .ok()
        .and_then(|c| c.lines().next().map(|l| l.trim().to_string()))
        .filter(|s| !s.is_empty());

    let places = probe_places(wt_root, &files);

    Facts { files, compose, prefix_file, places, truncated }
}

/// The file half of `probe`, on its own — what the passive nudge on `new` runs.
/// One bounded walk plus two `git` calls, and no file is opened.
pub fn probe_files(main_root: &Path) -> Vec<Candidate> {
    let (candidates, _, _) = walk(main_root);
    keep_gitignored_untracked(main_root, candidates)
}

fn probe_places(wt_root: &Path, files: &[Candidate]) -> Vec<PlaceFacts> {
    let Ok(rd) = fs::read_dir(wt_root) else { return Vec::new() };
    let mut out: Vec<PlaceFacts> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| {
            let dir = e.path();
            let missing = files
                .iter()
                .filter(|c| fs::symlink_metadata(dir.join(&c.rel)).is_err())
                .count();
            PlaceFacts { slug: e.file_name().to_string_lossy().into_owned(), missing }
        })
        .collect();
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// "Gitignored and present but tracked nowhere" is three separate questions, and
/// only the first is answered by looking at the disk. `git check-ignore` and
/// `git ls-files` answer the other two — one subprocess each over the whole
/// candidate list, never one per file.
fn keep_gitignored_untracked(main_root: &Path, candidates: Vec<Candidate>) -> Vec<Candidate> {
    if candidates.is_empty() {
        return candidates;
    }
    let root = main_root.to_string_lossy().into_owned();
    let paths: Vec<String> = candidates.iter().map(|c| c.rel.clone()).collect();
    let ignored = git_ignored(&root, &paths);
    let tracked = git_tracked(&root, &paths);
    candidates.into_iter().filter(|c| ignored.contains(&c.rel) && !tracked.contains(&c.rel)).collect()
}

// ⚠ `-z` on both calls is load-bearing, not tidiness. Under git's default
// `core.quotePath` these commands C-quote any path holding a non-ASCII byte,
// `"`, `\` or a control character — `sécrets/.env` comes back as
// `"s\303\251crets/.env"`, which matches no candidate, so the file drops out of
// the filter with no error and no note. A credential under an accented directory
// going unmentioned is precisely the silent failure §1.2 exists to catch. NUL
// delimiting is also the only form that survives a newline in a filename.
//
// Both are captured directly rather than through `git_out` because both exit 1
// for the perfectly ordinary "nothing matched", and that is an empty set, not a
// failure.

/// Which of `paths` git ignores. `-z` on `check-ignore` "only makes sense with
/// --stdin", so the list goes in on stdin rather than in argv.
fn git_ignored(root: &str, paths: &[String]) -> std::collections::BTreeSet<String> {
    let mut input: Vec<u8> = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    let Ok(out) = git::git_stdin(root, &["check-ignore", "-z", "--stdin"], &input) else {
        return Default::default();
    };
    nul_set(&out.stdout)
}

/// Which of `paths` git tracks.
fn git_tracked(root: &str, paths: &[String]) -> std::collections::BTreeSet<String> {
    let mut argv: Vec<&str> = vec!["ls-files", "-z", "--"];
    argv.extend(paths.iter().map(|s| s.as_str()));
    let Ok(out) = git::git(root, &argv) else { return Default::default() };
    nul_set(&out.stdout)
}

/// NUL-delimited stdout as a set. Lossy on both sides of the comparison — a
/// candidate came out of `to_string_lossy` too, so the same bytes still match.
fn nul_set(stdout: &[u8]) -> std::collections::BTreeSet<String> {
    String::from_utf8_lossy(stdout).split('\0').filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Bounded breadth-first walk of the repo. Returns `(candidates, compose files,
/// truncated)`. Symlinked directories are never followed — a `read_dir` that
/// escapes the repo is how a walk turns into a loop, or into someone's home dir.
fn walk(main_root: &Path) -> (Vec<Candidate>, Vec<String>, bool) {
    let mut hits: Vec<Candidate> = Vec::new();
    let mut compose: Vec<String> = Vec::new();
    let mut truncated = false;
    let (mut dirs, mut entries) = (0usize, 0usize);
    // (path, repo-relative prefix, depth)
    let mut queue: Vec<(PathBuf, String, usize)> = vec![(main_root.to_path_buf(), String::new(), 0)];
    let mut next: Vec<(PathBuf, String, usize)> = Vec::new();

    'walk: while !queue.is_empty() {
        for (dir, prefix, depth) in queue.drain(..) {
            // `break 'walk`, not `break`: leaving only the inner loop would drain
            // this level and then happily walk the NEXT one, which is not what
            // "stop, we have looked at enough" means.
            if dirs >= MAX_DIRS || entries >= MAX_ENTRIES || hits.len() >= MAX_HITS {
                truncated = true;
                break 'walk;
            }
            dirs += 1;
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for e in rd.filter_map(|e| e.ok()) {
                entries += 1;
                if entries >= MAX_ENTRIES {
                    truncated = true;
                    break;
                }
                let name = e.file_name().to_string_lossy().into_owned();
                let Ok(ft) = e.file_type() else { continue };
                let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                if ft.is_dir() {
                    // Hidden dirs (`.git`, `.worktrees`, `.next`, `.venv`, …) and
                    // the heavy build dirs are never descended.
                    if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                        continue;
                    }
                    if depth + 1 > MAX_DEPTH {
                        truncated = true;
                        continue;
                    }
                    next.push((e.path(), rel, depth + 1));
                    continue;
                }
                // A symlink here is not followed and not a candidate: linking a
                // link into a worktree is a question with no good answer, and §4
                // Layer B would refuse it at apply time anyway.
                if !ft.is_file() {
                    continue;
                }
                if is_compose(&name) {
                    compose.push(rel.clone());
                }
                if let Some(kind) = classify(&name) {
                    if hits.len() >= MAX_HITS {
                        truncated = true;
                    } else {
                        hits.push(Candidate::new(rel, kind));
                    }
                }
            }
        }
        std::mem::swap(&mut queue, &mut next);
    }
    hits.sort_by(|a, b| a.rel.cmp(&b.rel));
    // The override, then shallowest, then alphabetical — so `[compose] file` and
    // the port numbers come from the file a human would have picked.
    compose.sort_by_key(|r| (!WORKTREE_COMPOSE.contains(&basename(r)), r.matches('/').count(), r.clone()));
    (hits, compose, truncated)
}

/// §9's two file rows, as a pure name test.
pub fn classify(name: &str) -> Option<Kind> {
    let l = name.to_ascii_lowercase();
    if l == "google-services.json"
        || l == "googleservice-info.plist"
        || l.ends_with(".keystore")
        || l.ends_with(".jks")
        || (l.ends_with(".json") && l.contains("-service-account"))
    {
        return Some(Kind::Credential);
    }
    // `.env*`, which covers `.env`, `.env.local`, `.envrc`. Templates like
    // `.env.example` are normally TRACKED, and the git filter drops them.
    if l.starts_with(".env") {
        return Some(Kind::Env);
    }
    None
}

fn is_compose(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.starts_with("docker-compose") && (l.ends_with(".yml") || l.ends_with(".yaml"))
}

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn read_capped(path: &Path) -> Option<String> {
    let md = fs::metadata(path).ok()?;
    if !md.is_file() || md.len() > MAX_COMPOSE_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

// ── compose port scan — a heuristic, and honest about it ─────────────────────

/// Read a compose file well enough to answer "does this publish host ports, and
/// which?" — with a **line scan and no YAML dependency**. This codebase is
/// deliberately lean and the answer feeds a *suggestion* a human reads before
/// anything is written, so the trade is worth it. What it does:
///
/// - finds a top-level `services:` block, then each service at the next indent
/// - inside a service's `ports:` list, reads short-syntax entries
///   (`"3000:3000"`, `127.0.0.1:5432:5432`, `3000:3000/udp`) and long-syntax
///   `published: <n>`
///
/// What it does NOT catch, all of which yield either nothing or an `unreadable`
/// tick rather than a wrong number:
///
/// - anything interpolated (`${API_PORT}:3000`) or ranged (`3000-3005:3000`)
/// - `extends`, YAML anchors/merge keys, multiple documents, flow-style mappings
/// - `docker-compose.override.yml` layering, profiles, and `expose:` (which
///   publishes nothing anyway)
/// - a service named `ports` or a `ports:` key nested somewhere unexpected
///
/// So: `unreadable > 0` means "there are published ports I could not name" — good
/// enough to suggest `[ports]`, never good enough to invent a number for.
pub fn scan_compose_ports(text: &str) -> (Vec<(String, u32)>, usize) {
    let mut out = Vec::new();
    let mut unreadable = 0usize;
    let mut services: Option<usize> = None;
    let mut service: Option<(String, usize)> = None;
    let mut ports: Option<usize> = None;

    for raw in text.lines() {
        let content = raw.trim_start();
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        let indent = raw.len() - content.len();
        let content = content.trim_end();

        if let Some(pi) = ports {
            if indent > pi {
                match parse_port_line(content) {
                    PortLine::Host(n) => {
                        if let Some((svc, _)) = &service {
                            out.push((svc.clone(), n));
                        }
                    }
                    PortLine::Unreadable => unreadable += 1,
                    PortLine::None => {}
                }
                continue;
            }
            ports = None;
        }

        if content == "services:" {
            services = Some(indent);
            service = None;
            continue;
        }
        let Some(si) = services else { continue };
        if indent <= si {
            // back out to another top-level key (`volumes:`, `networks:`)
            services = None;
            service = None;
            continue;
        }
        match &service {
            None => {
                if let Some(name) = block_key(content) {
                    service = Some((name, indent));
                }
            }
            Some((_, sind)) => {
                if indent == *sind {
                    service = block_key(content).map(|n| (n, indent));
                } else if indent > *sind && content == "ports:" {
                    ports = Some(indent);
                }
            }
        }
    }
    (out, unreadable)
}

/// `name:` with nothing after it — a nested block's key.
fn block_key(content: &str) -> Option<String> {
    let name = content.strip_suffix(':')?;
    if name.is_empty() || name.contains(' ') || name.contains(':') {
        return None;
    }
    Some(name.to_string())
}

enum PortLine {
    Host(u32),
    /// Read fine, publishes no host port (`- 3000` alone, `- target: 3000`).
    None,
    /// Inside a `ports:` block and not confidently readable.
    Unreadable,
}

fn parse_port_line(content: &str) -> PortLine {
    let body = content.strip_prefix('-').map(str::trim_start).unwrap_or(content);
    // Long syntax: `published` is the only key that names a HOST port.
    if let Some(rest) = body.split_once("published:") {
        return match rest.1.trim().trim_matches(['"', '\'']).parse::<u32>() {
            Ok(n) if (1..=65535).contains(&n) => PortLine::Host(n),
            _ => PortLine::Unreadable,
        };
    }
    if body.contains(": ") || body.ends_with(':') {
        return PortLine::None; // `target:`, `protocol:`, `mode:` — not a host port
    }
    parse_short_port(body.trim_matches(['"', '\'']))
}

/// `HOST:CONTAINER`, `IP:HOST:CONTAINER`, either with an optional `/proto`.
fn parse_short_port(s: &str) -> PortLine {
    if s.is_empty() || s.contains('$') {
        return if s.is_empty() { PortLine::None } else { PortLine::Unreadable };
    }
    let s = s.split('/').next().unwrap_or(s);
    let parts: Vec<&str> = s.split(':').collect();
    let host = match parts.len() {
        1 => return PortLine::None, // container-only: docker picks the host port
        2 => parts[0],
        3 => parts[1],
        _ => return PortLine::Unreadable,
    };
    match host.parse::<u32>() {
        Ok(n) if (1..=65535).contains(&n) => PortLine::Host(n),
        _ => PortLine::Unreadable, // a range, a name, something else
    }
}

// ── writing ──────────────────────────────────────────────────────────────────

/// Write `.worktrees.toml`. Atomic (tmp in the same dir, then `rename`), the same
/// pattern `store.rs::write_atomic` uses — a half-written config is exactly the
/// half-provisioned state §1 refuses.
pub fn write_config(main_root: &Path, text: &str) -> std::io::Result<PathBuf> {
    let path = main_root.join(projcfg::CONFIG_FILE);
    let tmp = main_root.join(".worktrees.toml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

// ── the once-only marker for the passive nudge (§9) ──────────────────────────
//
// §9 flags that "once" has no single home and says to decide explicitly. The
// decision, for the CLI:
//
//   $XDG_STATE_HOME/worktrees/init-hints/<hash of canonical repo path>
//
// Three properties, each of which rules out the alternatives:
//
//   - It is PER-MACHINE, so it does not belong in the repo. Writing a dismissal
//     into a checkout would commit one developer's "don't tell me again" to
//     everybody, and into a *gitignored* file in the repo would resurrect it on
//     every fresh clone.
//   - It is not `.worktrees.places.json`: that store is PLACE-keyed declared
//     intent (DESIGN.md:158-176), and this is repo-keyed. A per-repo key has no
//     home there, and the store is safely-losable by design.
//   - The file's CONTENT is a digest of what was detected, not a boolean. A repo
//     that later gains a credential file therefore re-suggests — which is the
//     whole point, since the file that appears after you dismissed the hint is
//     precisely the one nobody links.
//
// ⚠ The app (slice 5) needs its own dismissal for its own banner — `ui-state.json`
// is app-global and invisible to the CLI. Note that the app calling `cmd_new`
// in-process shares THIS marker, because it shares this code; only the app's own
// banner needs the second store.

/// `$XDG_STATE_HOME/worktrees` (or `~/.local/state/worktrees`). State, not config:
/// this is machine-generated and safely deletable, unlike `~/.config/worktrees`.
fn state_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("HOME").ok().filter(|s| !s.is_empty()).map(|h| format!("{h}/.local/state"))
        })?;
    Some(PathBuf::from(base).join("worktrees"))
}

fn marker_path_in(state: &Path, main_root: &Path) -> PathBuf {
    let canon = fs::canonicalize(main_root).unwrap_or_else(|_| main_root.to_path_buf());
    state.join("init-hints").join(format!("{:016x}", fnv1a(&canon.to_string_lossy())))
}

/// FNV-1a, the same hash `materialize.rs` uses. Not a security boundary — it
/// names a file whose only job is "have I said this before".
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// What was detected, as a stable string. Order-independent inputs are sorted by
/// `probe`, so the same repo yields the same digest across runs.
pub fn digest(files: &[Candidate]) -> String {
    let joined: Vec<String> = files
        .iter()
        .map(|c| format!("{}:{}", if c.kind == Kind::Credential { "c" } else { "e" }, c.rel))
        .collect();
    format!("{:016x}", fnv1a(&joined.join("\n")))
}

/// Has this exact detection already been hinted for this repo?
pub fn hint_seen(main_root: &Path, digest: &str) -> bool {
    state_dir().is_some_and(|s| hint_seen_in(&s, main_root, digest))
}

/// Record it. Best-effort: an unwritable state dir means the hint repeats, which
/// is annoying and harmless — it must never fail `new`.
pub fn record_hint(main_root: &Path, digest: &str) {
    if let Some(s) = state_dir() {
        record_hint_in(&s, main_root, digest);
    }
}

// The state root is a parameter here so the tests exercise the real read/write
// without mutating a process-global env var under a parallel test runner.

fn hint_seen_in(state: &Path, main_root: &Path, digest: &str) -> bool {
    let p = marker_path_in(state, main_root);
    fs::read_to_string(p).ok().and_then(|t| t.lines().next().map(str::to_string)).as_deref() == Some(digest)
}

fn record_hint_in(state: &Path, main_root: &Path, digest: &str) {
    let p = marker_path_in(state, main_root);
    let Some(dir) = p.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    // The repo path rides along so a human clearing these can tell what they are.
    let canon = fs::canonicalize(main_root).unwrap_or_else(|_| main_root.to_path_buf());
    let _ = fs::write(p, format!("{digest}\n{}\n", canon.display()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(rel: &str) -> Candidate {
        Candidate::new(rel, Kind::Env)
    }
    fn cred(rel: &str) -> Candidate {
        Candidate::new(rel, Kind::Credential)
    }
    fn compose(rel: &str, published: &[(&str, u32)]) -> ComposeFacts {
        ComposeFacts {
            rel: rel.to_string(),
            is_override: WORKTREE_COMPOSE.contains(&basename(rel)),
            published: published.iter().map(|(s, p)| (s.to_string(), *p)).collect(),
            unreadable: 0,
        }
    }

    // ── classification (§9's two file rows) ──────────────────────────────────

    #[test]
    fn classify_names_the_two_file_rows_and_nothing_else() {
        for n in [".env", ".env.local", ".env.production", ".envrc"] {
            assert_eq!(classify(n), Some(Kind::Env), "{n}");
        }
        for n in [
            "google-services.json",
            "GoogleService-Info.plist",
            "release.keystore",
            "upload.jks",
            "firebase-service-account-prod.json",
            "MY.KEYSTORE",
        ] {
            assert_eq!(classify(n), Some(Kind::Credential), "{n}");
        }
        for n in ["README.md", "package.json", "env.ts", "service-account.txt", "environment.rb"] {
            assert_eq!(classify(n), None, "{n}");
        }
    }

    #[test]
    fn compose_files_are_recognized_by_name() {
        assert!(is_compose("docker-compose.yml"));
        assert!(is_compose("docker-compose.worktree.yml"));
        assert!(is_compose("docker-compose.override.yaml"));
        assert!(!is_compose("compose.yml")); // deliberately narrow — §9 names the docker-compose* form
        assert!(!is_compose("docker-compose.yml.bak"));
    }

    // ── detection over synthetic facts — every §9 row ────────────────────────

    #[test]
    fn nothing_detected_suggests_nothing() {
        let s = detect(&Facts::default());
        assert!(s.is_empty());
        assert!(!render(&s).contains("[[file]]\npath"), "no live entry in an empty suggestion");
        // …and it still parses clean, so `init --print` on a bare repo is valid TOML
        let (cfg, f) = projcfg::parse(&render(&s)).unwrap();
        assert_eq!(cfg, Default::default());
        assert!(f.is_empty());
    }

    #[test]
    fn gitignored_env_files_become_file_entries() {
        let f = Facts { files: vec![env("apps/api/.env"), env(".env")], ..Facts::default() };
        let s = detect(&f);
        assert_eq!(s.files, vec![env(".env"), env("apps/api/.env")], "sorted, env group first");
        assert!(s.ports.is_none() && s.compose.is_none());
        assert!(!s.is_empty());
    }

    #[test]
    fn credentials_sort_after_env_files_and_are_flagged_louder() {
        let f = Facts {
            files: vec![cred("apps/mobile/google-services.json"), env(".env")],
            ..Facts::default()
        };
        let s = detect(&f);
        assert_eq!(s.files[0].kind, Kind::Env);
        assert_eq!(s.files[1].kind, Kind::Credential);
        assert_eq!(s.credentials(), 1);
        let out = render(&s);
        assert!(out.contains("⚠ Credentials"), "{out}");
        assert!(out.contains("no exception, no log line"), "the silence is the point: {out}");
    }

    #[test]
    fn a_worktree_compose_override_suggests_compose_and_ports() {
        let f = Facts {
            compose: vec![compose("docker-compose.worktree.yml", &[("api", 3000), ("pg", 5432)])],
            ..Facts::default()
        };
        let s = detect(&f);
        assert_eq!(s.compose.as_deref(), Some("docker-compose.worktree.yml"));
        let p = s.ports.unwrap();
        assert_eq!(p.base, vec![("API".into(), 3000), ("PG".into(), 5432)]);
        assert_eq!(p.commented, None);
    }

    #[test]
    fn a_plain_compose_publishing_host_ports_suggests_ports_but_not_compose() {
        // §9: the project has collisions waiting even without docker namespacing.
        let f = Facts {
            compose: vec![compose("docker-compose.yml", &[("web", 8080)])],
            ..Facts::default()
        };
        let s = detect(&f);
        assert!(s.compose.is_none(), "no worktree override ⇒ no [compose]");
        assert_eq!(s.ports.unwrap().base, vec![("WEB".into(), 8080)]);
    }

    #[test]
    fn a_compose_that_publishes_nothing_suggests_nothing() {
        let f = Facts { compose: vec![compose("docker-compose.yml", &[])], ..Facts::default() };
        assert!(detect(&f).is_empty());
    }

    #[test]
    fn ports_are_never_invented_when_nothing_could_be_read() {
        // The file publishes ports the scan could not name: suggest [ports], but
        // emit it COMMENTED OUT rather than making numbers up (§9).
        let mut c = compose("docker-compose.worktree.yml", &[]);
        c.unreadable = 2;
        let s = detect(&Facts { compose: vec![c], ..Facts::default() });
        let p = s.ports.clone().unwrap();
        assert!(p.base.is_empty());
        assert!(p.commented.unwrap().contains("no host port could be read confidently"));
        let out = render(&s);
        assert!(out.contains("# [ports]"), "{out}");
        assert!(!out.contains("\n[ports]"), "the live section must not be emitted: {out}");
    }

    #[test]
    fn a_base_map_that_would_collide_across_slots_is_commented_out() {
        // 3000 and 3100 are one stride apart: slot 1's API is slot 0's WEB.
        let f = Facts {
            compose: vec![compose("docker-compose.worktree.yml", &[("api", 3000), ("web", 3100)])],
            ..Facts::default()
        };
        let s = detect(&f);
        let p = s.ports.clone().unwrap();
        assert!(p.commented.as_deref().is_some_and(|m| m.contains("would collide")), "{p:?}");
        let out = render(&s);
        assert!(!out.contains("\n[ports]"), "{out}");
        // and the file still parses clean, with no [ports] at all
        let (cfg, findings) = projcfg::parse(&out).unwrap();
        assert!(cfg.ports.is_none());
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_worktree_prefix_file_is_folded_in_but_stays_commented_out() {
        let s = detect(&Facts { prefix_file: Some("cdv".into()), ..Facts::default() });
        assert_eq!(s.prefix.as_deref(), Some("cdv"));
        assert!(!s.is_empty());
        let out = render(&s);
        assert!(out.contains("# [project]\n# prefix = \"cdv\"\n"), "{out}");
        // §12 defers it: an UNCOMMENTED [project] prefix warns on every command,
        // so the round-trip must produce zero findings.
        let (cfg, findings) = projcfg::parse(&out).unwrap();
        assert_eq!(cfg, Default::default());
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn places_missing_a_suggested_file_are_named_for_relink() {
        let f = Facts {
            files: vec![env(".env")],
            places: vec![
                PlaceFacts { slug: "feat-a".into(), missing: 1 },
                PlaceFacts { slug: "feat-b".into(), missing: 0 },
            ],
            ..Facts::default()
        };
        assert_eq!(detect(&f).stale_places, vec!["feat-a".to_string()]);
    }

    #[test]
    fn duplicate_service_ports_get_distinct_shell_safe_keys() {
        assert_eq!(shell_key("wo-mock"), "WO_MOCK");
        assert_eq!(shell_key("2fast"), "_2FAST");
        let base = derive_base(&[
            ("api".into(), 3000),
            ("api".into(), 3001),
            ("api".into(), 3000), // exact repeat — one entry
        ]);
        assert_eq!(base, vec![("API".into(), 3000), ("API_3001".into(), 3001)]);
    }

    // ── the round-trip that keeps emitter and parser together ────────────────

    #[test]
    fn emitted_config_round_trips_with_no_findings() {
        // Every combination of sections. THIS is the test that stops the emitter
        // and `projcfg::parse` from drifting.
        let files = vec![vec![], vec![env(".env")], vec![env(".env"), cred("apps/mobile/google-services.json")]];
        let composes = vec![vec![], vec![compose("docker-compose.worktree.yml", &[("api", 3000), ("pg", 5432)])]];
        let prefixes = vec![None, Some("cdv".to_string())];
        let mut seen_live_ports = false;
        for fs_ in &files {
            for cs in &composes {
                for pf in &prefixes {
                    let facts = Facts {
                        files: fs_.clone(),
                        compose: cs.clone(),
                        prefix_file: pf.clone(),
                        ..Facts::default()
                    };
                    let s = detect(&facts);
                    let text = render(&s);
                    let (cfg, findings) = projcfg::parse(&text)
                        .unwrap_or_else(|e| panic!("emitted config must parse: {e}\n---\n{text}"));
                    assert!(findings.is_empty(), "no findings, got {findings:?}\n---\n{text}");
                    assert_eq!(cfg.files.len(), s.files.len(), "{text}");
                    for (got, want) in cfg.files.iter().zip(&s.files) {
                        assert_eq!(got.path.as_str(), want.rel);
                        assert_eq!(got.mode, projcfg::Mode::Link, "link is the default (§3)");
                    }
                    assert_eq!(cfg.compose.is_some(), s.compose.is_some(), "{text}");
                    if let Some(c) = &cfg.compose {
                        assert_eq!(c.project, "{prefix}-wt-{slug}");
                    }
                    if let Some(p) = &cfg.ports {
                        seen_live_ports = true;
                        assert_eq!(p.stride, STRIDE);
                        assert_eq!(p.base.get("API"), Some(&3000));
                        assert_eq!(p.base.get("PG"), Some(&5432));
                    }
                }
            }
        }
        assert!(seen_live_ports, "at least one combination must emit a live [ports]");
    }

    // ── what `probe` can actually hand `detect` ──────────────────────────────
    //
    // The round-trip test above feeds `detect` only well-formed synthetic facts,
    // so it cannot see this class at all: these are the shapes a REAL `read_dir`
    // produces and `projcfg::parse` refuses for the whole config. Each must come
    // back commented out, with the run still succeeding and the output still
    // parsing clean.

    #[test]
    fn a_path_the_parser_refuses_is_commented_out_rather_than_failing_the_run() {
        for (rel, needle) in [
            (r"apps$1/.env", "`$` is not expanded"), // a directory named `apps$1`
            ("~backup/.env", "`~` is not expanded"), // a directory named `~backup`
        ] {
            let s = detect(&Facts { files: vec![env(rel)], ..Facts::default() });
            assert!(s.files.is_empty(), "{rel} must not be emitted live");
            assert_eq!(s.rejected.len(), 1, "{rel} must not be dropped either");
            assert!(s.rejected[0].why.contains(needle), "{rel}: {:?}", s.rejected[0]);
            assert!(!s.is_empty(), "{rel}: there IS something to tell the human about");
            let out = render(&s);
            assert!(out.contains(&format!("# path = {}", toml_string(rel))), "{out}");
            assert!(!out.contains("\n[[file]]"), "no live entry: {out}");
            let (cfg, findings) = projcfg::parse(&out)
                .unwrap_or_else(|e| panic!("{rel}: the emitted config must still parse: {e}\n{out}"));
            assert!(cfg.files.is_empty() && findings.is_empty(), "{findings:?}\n{out}");
        }
    }

    #[test]
    fn an_over_length_path_is_commented_out() {
        // Unreachable through macOS's 1024-byte PATH_MAX, reachable on Linux.
        let long = format!("{}/.env", "a".repeat(1100));
        let s = detect(&Facts { files: vec![env(&long)], ..Facts::default() });
        assert!(s.files.is_empty() && s.rejected.len() == 1);
        assert!(s.rejected[0].why.contains("over the 1024-byte limit"), "{:?}", s.rejected[0]);
        let (_, findings) = projcfg::parse(&render(&s)).unwrap();
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_case_only_duplicate_keeps_the_first_and_comments_out_the_second() {
        // Two files on a case-sensitive filesystem, one on macOS — which
        // `check_file_list` refuses for the whole config, so only one can be live.
        let s = detect(&Facts { files: vec![env("A/.env"), env("a/.env")], ..Facts::default() });
        assert_eq!(s.files, vec![env("A/.env")], "the first survives");
        assert_eq!(s.rejected.len(), 1);
        assert!(s.rejected[0].why.contains("case-only duplicate"), "{:?}", s.rejected[0]);
        let out = render(&s);
        let (cfg, findings) = projcfg::parse(&out).unwrap_or_else(|e| panic!("{e}\n{out}"));
        assert_eq!(cfg.files.len(), 1);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_rejected_path_cannot_break_out_of_its_comment() {
        // `-z` git output means a filename with a newline now reaches `detect`,
        // and the reason quotes the path — one unescaped byte would leave the
        // rest of the reason standing as live TOML.
        let s = detect(&Facts { files: vec![env("a\nb$/.env")], ..Facts::default() });
        assert_eq!(s.rejected.len(), 1);
        let out = render(&s);
        assert!(out.lines().filter(|l| !l.is_empty()).all(|l| l.starts_with('#')), "{out}");
        let (cfg, findings) = projcfg::parse(&out).unwrap_or_else(|e| panic!("{e}\n{out}"));
        assert!(cfg.files.is_empty() && findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_rejected_candidate_does_not_suppress_a_good_one() {
        let s = detect(&Facts {
            files: vec![env(".env"), cred("apps$x/google-services.json")],
            ..Facts::default()
        });
        assert_eq!(s.files, vec![env(".env")]);
        assert_eq!(s.rejected.len(), 1);
        let out = render(&s);
        assert!(out.contains("\npath = \".env\"\n"), "{out}");
        assert!(out.contains("# path = \"apps$x/google-services.json\""), "{out}");
        let (cfg, findings) = projcfg::parse(&out).unwrap();
        assert_eq!(cfg.files.len(), 1);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn an_awkward_filename_survives_the_emitter() {
        // `\` is an ordinary filename byte on both shipped targets (projcfg §4),
        // and TOML would read it as an escape — so it must be escaped here.
        let s = detect(&Facts { files: vec![env(r"a\b/.env")], ..Facts::default() });
        let text = render(&s);
        let (cfg, findings) = projcfg::parse(&text).unwrap();
        assert!(findings.is_empty());
        assert_eq!(cfg.files[0].path.as_str(), r"a\b/.env");
    }

    // ── the compose line scan ────────────────────────────────────────────────

    #[test]
    fn the_port_scan_reads_short_syntax_per_service() {
        let text = "\
version: \"3.9\"
services:
  api:
    image: node
    ports:
      - \"3000:3000\"
      - 9229:9229
  db:
    image: postgres
    ports:
      - \"127.0.0.1:5432:5432\"
      - \"6000:6000/udp\"
volumes:
  pgdata:
";
        let (got, bad) = scan_compose_ports(text);
        assert_eq!(bad, 0);
        assert_eq!(
            got,
            vec![
                ("api".to_string(), 3000),
                ("api".to_string(), 9229),
                ("db".to_string(), 5432),
                ("db".to_string(), 6000),
            ]
        );
    }

    #[test]
    fn the_port_scan_reads_long_syntax_published_only() {
        let text = "\
services:
  web:
    ports:
      - target: 80
        published: 8080
        protocol: tcp
";
        assert_eq!(scan_compose_ports(text), (vec![("web".to_string(), 8080)], 0));
    }

    #[test]
    fn what_the_port_scan_cannot_read_is_counted_never_guessed() {
        let text = "\
services:
  api:
    ports:
      - \"${API_PORT}:3000\"
      - \"3000-3005:3000-3005\"
      - 3000
      - \"a:b\"
";
        let (got, bad) = scan_compose_ports(text);
        assert!(got.is_empty(), "{got:?}");
        // interpolated, ranged and non-numeric are unreadable; a bare container
        // port publishes nothing at all and is not counted.
        assert_eq!(bad, 3);
    }

    #[test]
    fn a_compose_with_no_ports_block_reads_as_publishing_nothing() {
        let text = "services:\n  api:\n    image: node\n    expose:\n      - 3000\n";
        assert_eq!(scan_compose_ports(text), (Vec::new(), 0));
    }

    // ── the once-only marker ─────────────────────────────────────────────────

    /// Removes its directory on the way out, so a FAILING assert below does not
    /// leave a tmpdir behind (the same guard `materialize.rs`'s tests use).
    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_hint_marker_is_keyed_by_repo_and_by_what_was_detected() {
        let tmp = Tmp(std::env::temp_dir().join(format!("wtinit-{}", std::process::id())));
        let root = &tmp.0;
        let (repo_a, repo_b, state) = (root.join("a"), root.join("b"), root.join("state"));
        fs::create_dir_all(&repo_a).unwrap();
        fs::create_dir_all(&repo_b).unwrap();

        let d1 = digest(&[env(".env")]);
        let d2 = digest(&[env(".env"), cred("google-services.json")]);
        assert_ne!(d1, d2, "a repo that gains a credential must re-suggest");

        assert!(!hint_seen_in(&state, &repo_a, &d1));
        record_hint_in(&state, &repo_a, &d1);
        assert!(hint_seen_in(&state, &repo_a, &d1), "said once");
        assert!(!hint_seen_in(&state, &repo_a, &d2), "new detection ⇒ hint again");
        assert!(!hint_seen_in(&state, &repo_b, &d1), "keyed by repo, not global");
        record_hint_in(&state, &repo_a, &d2);
        assert!(hint_seen_in(&state, &repo_a, &d2));
    }
}

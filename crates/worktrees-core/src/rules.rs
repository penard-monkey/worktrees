//! The rules pack: the worktrees way of working, written INTO a repository
//! so sessions worktrees did not launch (a hand-started `claude`, a teammate)
//! get it too (ADR 0002).
//!
//! Three pieces, all text this binary ships:
//!
//! - a managed block at the end of the root AGENTS.md, between markers that
//!   carry a version and a hash of the body, so a re-apply replaces it in
//!   place and a hand edit is noticed instead of overwritten;
//! - the `close-out` skill at `.claude/skills/close-out/SKILL.md`, managed the
//!   same way (one marker line under its frontmatter);
//! - `.claude/close-out.md`, the repository's own close-out settings, written
//!   once from a template and never again.
//!
//! It is a SUBSET of the per-launch guidance (`guidance.rs`), which stays the
//! newest word for sessions worktrees starts — the block says so itself.
//!
//! ADR 0001 holds: this is data worktrees WRITES. Reading it back only parses
//! the markers and hashes the body; nothing a repository holds here becomes
//! argv, a launch prompt, or per-launch delivery.
//!
//! The change is committed through `agentfiles`' plumbing on the same
//! `agent-instructions` branch (applying the rules includes the AGENTS.md
//! layout fix), judged against the default branch's ref, never a checkout.

use crate::agentfiles::{self, Change, DirKind, Entry, FixOutcome, Report};
use serde::Serialize;

/// Bumped when what the pack tells agents changes enough to re-ask every
/// project (the app's nudge fingerprints on it). A wording fix does not bump
/// it — an apply still picks the wording up, but nothing asks.
pub const VERSION: u32 = 1;

pub const BLOCK_BODY: &str = include_str!("guidance/rules-block.md");
pub const CLOSE_OUT_SKILL: &str = include_str!("guidance/close-out/SKILL.md");
const CLOSE_OUT_CONFIG: &str = include_str!("guidance/close-out-config.md");

pub const SKILL_PATH: &str = ".claude/skills/close-out/SKILL.md";
pub const CONFIG_PATH: &str = ".claude/close-out.md";

const BEGIN: &str = "<!-- worktrees:rules ";
const END: &str = "<!-- worktrees:rules end -->";
const MANAGED: &str = "<!-- worktrees:managed ";

// ── markers ──────────────────────────────────────────────────────────────────

/// The state of one managed piece.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Not there. Apply adds it.
    Absent,
    /// Exactly what this binary would write.
    Current,
    /// Written by an older worktrees (or older wording), untouched since.
    /// Apply replaces it.
    Outdated,
    /// Its body no longer matches the hash it was written with: someone
    /// edited it. Apply leaves it alone unless forced.
    Edited,
    /// Written by a NEWER worktrees. Apply leaves it alone.
    Newer,
    /// Markers that do not pair up. Apply leaves it alone and says where.
    Malformed,
    /// A file of that name that worktrees did not write (someone's own
    /// close-out skill). Never overwritten, not even forced.
    Foreign,
}

/// One managed piece as found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Found {
    pub state: State,
    /// The version its marker records, when it has one.
    pub version: Option<u32>,
    /// Why, for `malformed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Found {
    fn new(state: State, version: Option<u32>) -> Self {
        Found { state, version, detail: None }
    }
}

fn hash(text: &str) -> String {
    crate::guidance::text_hash(text)
}

/// `v<N> sha=<hex>` out of a marker line's remainder.
fn parse_stamp(rest: &str) -> Option<(u32, String)> {
    let mut it = rest.split_whitespace();
    let v = it.next()?.strip_prefix('v')?.parse().ok()?;
    let sha = it.next()?.strip_prefix("sha=")?;
    sha.chars().all(|c| c.is_ascii_hexdigit()).then(|| (v, sha.to_string()))
}

/// Version FIRST: a block a newer worktrees wrote stays `Newer` even after a
/// hand edit (the edit does not touch the marker's version), so `--force`,
/// which replaces EDITED text, can never downgrade it. Line endings are
/// compared as LF: a repository that commits CRLF has not edited anything.
fn judge(version: u32, recorded: &str, body: &str, shipped: &str) -> State {
    let body = lf(body);
    if version > VERSION {
        State::Newer
    } else if hash(&body) != recorded {
        State::Edited
    } else if version == VERSION && body == shipped {
        State::Current
    } else {
        State::Outdated
    }
}

fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// `text` in the line endings of `like`: CRLF when `like` uses them.
fn eol_like(text: &str, like: &str) -> String {
    if like.contains("\r\n") { lf(text).replace('\n', "\r\n") } else { text.to_string() }
}

/// The block exactly as apply writes it.
pub fn block() -> String {
    format!(
        "{BEGIN}v{VERSION} sha={} — managed by `worktrees rules apply`; edit outside this block, or it stops updating -->\n{BLOCK_BODY}{END}\n",
        hash(BLOCK_BODY)
    )
}

/// Where the block sits in a file: byte range of the whole block (begin line
/// through the end line's newline) and its parsed stamp + body.
struct Span {
    start: usize,
    end: usize,
    version: u32,
    sha: String,
    body: String,
}

fn find_block(text: &str) -> Result<Option<Span>, String> {
    let mut begins = Vec::new();
    let mut ends = Vec::new();
    let mut at = 0;
    for (n, line) in text.split_inclusive('\n').enumerate() {
        let t = line.trim_end_matches(['\n', '\r']);
        if t.trim() == END {
            ends.push((n + 1, at, at + line.len()));
        } else if t.starts_with(BEGIN) {
            begins.push((n + 1, at, at + line.len(), t[BEGIN.len()..].to_string()));
        }
        at += line.len();
    }
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([(bl, bs, be, rest)], [(el, es, ee)]) if el > bl => {
            let Some((version, sha)) = parse_stamp(rest) else {
                return Err(format!("line {bl}: the begin marker has no version and hash"));
            };
            Ok(Some(Span { start: *bs, end: *ee, version, sha, body: text[*be..*es].to_string() }))
        }
        _ => {
            let lines = |v: Vec<usize>| v.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
            Err(format!(
                "the worktrees:rules markers do not pair up (begin on line {}; end on line {})",
                if begins.is_empty() { "none".into() } else { lines(begins.iter().map(|b| b.0).collect()) },
                if ends.is_empty() { "none".into() } else { lines(ends.iter().map(|e| e.0).collect()) },
            ))
        }
    }
}

/// The block's state in an instruction file's text (`None` = no file).
pub fn block_state(text: Option<&str>) -> Found {
    let Some(text) = text else { return Found::new(State::Absent, None) };
    match find_block(text) {
        Ok(None) => Found::new(State::Absent, None),
        Ok(Some(s)) => Found::new(judge(s.version, &s.sha, &s.body, BLOCK_BODY), Some(s.version)),
        Err(e) => Found { state: State::Malformed, version: None, detail: Some(e) },
    }
}

/// `text` with the current block in it: replacing the one there, or appended
/// after the user's own text. Only call it for a state apply may write.
pub fn with_block(text: &str) -> String {
    let ours = eol_like(&block(), text);
    match find_block(text) {
        Ok(Some(s)) => format!("{}{ours}{}", &text[..s.start], &text[s.end..]),
        _ => {
            // Only the trailing NEWLINES: trailing spaces on the last line
            // can be a markdown hard break, which is content.
            let own = text.trim_end_matches(['\n', '\r']);
            let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
            if own.trim().is_empty() { ours } else { format!("{own}{nl}{nl}{ours}") }
        }
    }
}

/// A managed whole file: the marker line goes right under the frontmatter
/// (a skill must START with its frontmatter), and the hash covers the file
/// without it.
pub fn managed_file(shipped: &str) -> String {
    let marker = format!(
        "{MANAGED}v{VERSION} sha={} — written by `worktrees rules apply`; edit it and it stops updating -->\n",
        hash(shipped)
    );
    match frontmatter_end(shipped) {
        Some(i) => format!("{}{marker}{}", &shipped[..i], &shipped[i..]),
        None => format!("{marker}{shipped}"),
    }
}

/// Byte offset just past the closing `---` line of leading frontmatter.
fn frontmatter_end(text: &str) -> Option<usize> {
    let mut at = 0;
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end() != "---" {
        return None;
    }
    at += first.len();
    for l in lines {
        at += l.len();
        if l.trim_end() == "---" {
            return Some(at);
        }
    }
    None
}

pub fn managed_state(text: Option<&str>, shipped: &str) -> Found {
    let Some(text) = text else { return Found::new(State::Absent, None) };
    let mut stamp = None;
    let mut without = String::with_capacity(text.len());
    let mut markers = 0;
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.trim_end_matches(['\n', '\r']).strip_prefix(MANAGED) {
            markers += 1;
            stamp = parse_stamp(rest);
        } else {
            without.push_str(line);
        }
    }
    match (markers, stamp) {
        (0, _) => Found::new(State::Foreign, None),
        (1, Some((v, sha))) => Found::new(judge(v, &sha, &without, shipped), Some(v)),
        _ => Found { state: State::Malformed, version: None, detail: Some("its worktrees:managed marker is damaged or repeated".into()) },
    }
}

// ── status ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    /// The ref inspected (`origin/main`, or the local base).
    pub reference: String,
    /// The pack version this binary writes.
    pub version: u32,
    /// The file the block lives in (or would): `AGENTS.md`, or `CLAUDE.md`
    /// in the one layout where that is the real file.
    pub file: Option<String>,
    pub block: Found,
    pub skill: Found,
    /// `.claude/close-out.md` exists (it is the repository's; never judged).
    pub config: bool,
    /// Why apply cannot write the block at all.
    pub blocked: Option<String>,
    /// Something apply would change (without `--force`).
    pub applicable: bool,
    /// The `agent-instructions` branch already exists: an apply (or the
    /// layout fix) is waiting to be merged.
    pub pending: Option<String>,
    pub pending_on_origin: bool,
}

/// Where the block goes, from the root's layout. `Err` = nowhere safe.
#[derive(Clone)]
enum Target {
    /// Write into this existing file.
    File(String),
    /// The layout fix renames CLAUDE.md → AGENTS.md; the block goes into
    /// AGENTS.md with CLAUDE.md's text above it.
    Renamed,
    /// No instruction file at the root: create AGENTS.md (+ the stub).
    New,
}

/// A link target as a sibling file name: `./AGENTS.md` is `AGENTS.md`.
fn bare(target: &str) -> &str {
    let t = target.trim();
    t.strip_prefix("./").unwrap_or(t)
}

/// The first tracked symlink on `path`'s way down (itself included): the
/// repository's own layout, which apply never writes through.
fn link_on_path<'a>(entries: &'a [Entry], path: &str) -> Option<&'a str> {
    let ends = path.match_indices('/').map(|(i, _)| i).chain(std::iter::once(path.len()));
    ends.map(|i| &path[..i])
        .find_map(|prefix| entries.iter().find(|e| e.path == prefix && e.mode == "120000"))
        .map(|e| e.path.as_str())
}

fn link_target(entries: &[Entry], path: &str, read: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    entries.iter().find(|e| e.path == path && e.mode == "120000").and_then(|e| read(&e.sha))
}

fn target(entries: &[Entry], root_kind: Option<&DirKind>, read: &dyn Fn(&str) -> Option<String>) -> Result<Target, String> {
    match root_kind {
        None => Ok(Target::New),
        Some(DirKind::Stub | DirKind::AgentsOnly) => Ok(Target::File(agentfiles::AGENTS.into())),
        Some(DirKind::ClaudeOnly) => {
            // ClaudeOnly also covers two identical copies, which the layout
            // fix folds into AGENTS.md + a stub.
            if entries.iter().any(|e| e.path == agentfiles::AGENTS) {
                Ok(Target::File(agentfiles::AGENTS.into()))
            } else {
                Ok(Target::Renamed)
            }
        }
        Some(DirKind::Diverged) => Err("CLAUDE.md and AGENTS.md differ at the root — merge them first (Claude reads one, Codex the other)".into()),
        Some(DirKind::Linked) => {
            // The one link layout both agents read through: one real file and
            // the other a link to it.
            let c = link_target(entries, agentfiles::CLAUDE, read);
            let a = link_target(entries, agentfiles::AGENTS, read);
            let has = |p: &str| entries.iter().any(|e| e.path == p && e.mode != "120000");
            match (c.as_deref(), a.as_deref()) {
                (Some(t), None) if bare(t) == agentfiles::AGENTS && has(agentfiles::AGENTS) => Ok(Target::File(agentfiles::AGENTS.into())),
                (None, Some(t)) if bare(t) == agentfiles::CLAUDE && has(agentfiles::CLAUDE) => Ok(Target::File(agentfiles::CLAUDE.into())),
                _ => Err("the root's CLAUDE.md / AGENTS.md are symlinks to somewhere else — point one at the other first".into()),
            }
        }
    }
}

/// A blob's text EXACTLY — `agentfiles::git_env` trims the trailing newline,
/// which would move a managed file's hash and read every one as edited.
fn blob(root: &str, spec: &str) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(root).args(["cat-file", "blob", spec]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Everything status and apply need, read from the ref.
#[derive(Clone)]
struct Inspected {
    report: Report,
    status: Status,
    target: Option<Target>,
    /// The target file's current text (CLAUDE.md's, for `Renamed`).
    text: Option<String>,
    /// Writing the close-out skill also needs `.agents/skills/close-out`:
    /// nothing is there yet, `.agents/skills` is not one link, and the
    /// layout fix will not add it (it only links skills already on the ref).
    needs_link: bool,
    /// A symlink on `.claude/close-out.md`'s path: the config is not written.
    config_link: Option<String>,
}

/// Per (repo, tree id), like `agentfiles::inspect`'s cache: the app asks on
/// every sweep and every sheet open, and a tree id fixes everything here but
/// the pending branch, which is patched in fresh on every read.
static INSPECTED: std::sync::Mutex<Vec<(String, String, Inspected)>> = std::sync::Mutex::new(Vec::new());

fn inspect_with(p: &crate::project::Project) -> Result<Inspected, String> {
    let report = agentfiles::inspect(p)?;
    let tree_id = crate::git::git_out(&p.main_root, &["rev-parse", "--verify", "-q", &format!("{}^{{tree}}", report.reference)]);
    if let Some(t) = &tree_id {
        let g = INSPECTED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, _, i)) = g.iter().find(|(r, id, _)| *r == p.main_root && id == t) {
            let mut i = i.clone();
            i.status.pending = report.pending.clone();
            i.status.pending_on_origin = report.pending_on_origin;
            i.report = report;
            return Ok(i);
        }
    }
    let i = inspect_fresh(p, report)?;
    if let Some(t) = tree_id {
        let mut g = INSPECTED.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|(r, _, _)| *r != p.main_root);
        g.push((p.main_root.clone(), t, i.clone()));
    }
    Ok(i)
}

fn inspect_fresh(p: &crate::project::Project, report: Report) -> Result<Inspected, String> {
    let root = p.main_root.as_str();
    let entries = agentfiles::tree(root, &report.reference)?;
    let read_blob = |sha: &str| blob(root, sha);
    let read_path = |path: &str| -> Option<String> {
        let e = entries.iter().find(|e| e.path == path && e.mode != "120000")?;
        read_blob(&e.sha)
    };
    let root_kind = report.dirs.iter().find(|d| d.dir.is_empty()).map(|d| &d.kind);
    let (target, blocked) = match target(&entries, root_kind, &read_blob) {
        Ok(t) => (Some(t), None),
        Err(e) => (None, Some(e)),
    };
    let (file, text) = match &target {
        Some(Target::File(f)) => (Some(f.clone()), read_path(f)),
        Some(Target::Renamed) => (Some(agentfiles::AGENTS.to_string()), read_path(agentfiles::CLAUDE)),
        Some(Target::New) => (Some(agentfiles::AGENTS.to_string()), None),
        None => (None, None),
    };
    let block = if blocked.is_some() {
        // Judge what IS there, so a status still says whether it was applied.
        block_state(read_path(agentfiles::AGENTS).as_deref())
    } else {
        block_state(text.as_deref())
    };
    let skill_on_ref = entries.iter().find(|e| e.path == SKILL_PATH);
    let skill = match (link_on_path(&entries, SKILL_PATH), skill_on_ref) {
        (Some(l), _) => Found {
            state: State::Foreign,
            version: None,
            detail: Some(format!("`{l}` is a symlink; worktrees does not write through it")),
        },
        (None, Some(e)) => managed_state(read_blob(&e.sha).as_deref(), CLOSE_OUT_SKILL),
        (None, None) => Found::new(State::Absent, None),
    };
    let config = entries.iter().any(|e| e.path == CONFIG_PATH);
    // `.claude` itself a link: the config cannot be written either.
    let config_link = if config { None } else { link_on_path(&entries, CONFIG_PATH).map(str::to_string) };
    let needs_link = skill_on_ref.is_none()
        && !entries.iter().any(|e| {
            e.path == ".agents/skills/close-out"
                || e.path.starts_with(".agents/skills/close-out/")
                || (e.mode == "120000" && (e.path == ".agents/skills" || e.path == ".agents"))
        });
    let writable = |f: &Found| matches!(f.state, State::Absent | State::Outdated);
    let applicable = report.fixable
        || (blocked.is_none() && writable(&block))
        || writable(&skill)
        || (!config && config_link.is_none() && skill.state != State::Foreign);
    let status = Status {
        reference: report.reference.clone(),
        version: VERSION,
        file,
        block,
        skill,
        config,
        blocked,
        applicable,
        pending: report.pending.clone(),
        pending_on_origin: report.pending_on_origin,
    };
    Ok(Inspected { report, status, target, text, needs_link, config_link })
}

pub fn status(p: &crate::project::Project) -> Result<Status, String> {
    inspect_with(p).map(|i| i.status)
}

// ── apply ────────────────────────────────────────────────────────────────────

/// The changes apply makes on top of the layout fix. `force` replaces an
/// EDITED block or skill (never a foreign skill, never a newer or malformed
/// one). Returns the changes and what it left alone, in words.
/// Whether apply writes a piece in this state (`force` adds EDITED).
fn may(f: &Found, force: bool) -> bool {
    matches!(f.state, State::Absent | State::Outdated) || (force && f.state == State::Edited)
}

/// Why apply leaves a piece alone — the same line for `apply` and `diff`.
fn why(what: &str, f: &Found) -> String {
    match f.state {
        State::Edited => format!("{what} was edited by hand — left alone (`worktrees rules diff` shows it; --force replaces it)"),
        State::Newer => format!("{what} was written by a newer worktrees (v{}) — left alone; upgrade worktrees", f.version.unwrap_or(0)),
        State::Malformed => format!("{what}: {} — left alone; fix the markers by hand", f.detail.clone().unwrap_or_default()),
        State::Foreign => match &f.detail {
            Some(d) => format!("{what}: {d} — left alone"),
            None => format!("{what} is not one worktrees wrote — left alone"),
        },
        _ => String::new(),
    }
}

fn rules_changes(i: &Inspected, project: &str, force: bool) -> (Vec<Change>, Vec<String>) {
    let mut v = Vec::new();
    let mut left = Vec::new();
    let may = |f: &Found| may(f, force);
    let s = &i.status;
    match (&i.target, &s.blocked) {
        (_, Some(b)) => left.push(format!("the rules block: {b}")),
        (Some(t), None) if may(&s.block) => {
            let file = s.file.clone().unwrap_or_else(|| agentfiles::AGENTS.into());
            let content = with_block(i.text.as_deref().unwrap_or(""));
            if let Target::New = t {
                v.push(Change::Write { path: agentfiles::CLAUDE.into(), content: agentfiles::STUB.into() });
            }
            v.push(Change::Write { path: file, content });
        }
        _ if s.block.state != State::Current => left.push(why("the rules block", &s.block)),
        _ => {}
    }
    if may(&s.skill) {
        v.push(Change::Write { path: SKILL_PATH.into(), content: managed_file(CLOSE_OUT_SKILL) });
        if i.needs_link {
            // A skill the layout fix has not seen (it is new): link it for
            // Codex the same way the fix links the others.
            v.push(Change::Link { path: ".agents/skills/close-out".into(), target: "../../.claude/skills/close-out".into() });
        }
    } else if s.skill.state != State::Current {
        left.push(why(SKILL_PATH, &s.skill));
    }
    if let Some(l) = &i.config_link {
        left.push(format!("{CONFIG_PATH}: `{l}` is a symlink; worktrees does not write through it — left alone"));
    } else if !s.config && s.skill.state != State::Foreign {
        v.push(Change::Write { path: CONFIG_PATH.into(), content: CLOSE_OUT_CONFIG.replace("{project}", project) });
    }
    (v, left)
}

/// Apply the pack: the layout fix plus the rules, as one commit on
/// `agent-instructions`. Local unless `push`.
pub fn apply(p: &crate::project::Project, push: bool, force: bool) -> Result<FixOutcome, String> {
    apply_with(p, &agentfiles::gh_bin(), push, force)
}

pub fn apply_with(p: &crate::project::Project, gh_bin: &str, push: bool, force: bool) -> Result<FixOutcome, String> {
    agentfiles::prepare(p)?;
    let i = inspect_with(p)?;
    let mut out = FixOutcome { branch: agentfiles::FIX_BRANCH.into(), base: i.report.reference.clone(), ..Default::default() };
    out.left_alone = agentfiles::left_alone(&i.report);
    let project = std::path::Path::new(&p.main_root).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // One commit: the layout fix, then the rules. For a CLAUDE.md-only root
    // the fix's rename is followed by the block written over AGENTS.md, so a
    // short CLAUDE.md can fall under git's 50% similarity and `log --follow`
    // reads delete + add. Accepted: one commit to review and revert beats a
    // rename that survives `--follow`, and the old text is still in history.
    let mut changes = agentfiles::plan(&i.report.dirs, &i.report.skills);
    let (ours, left) = rules_changes(&i, &project, force);
    changes.extend(ours);
    if changes.is_empty() {
        out.notes.push(format!("{} already carries the worktrees rules (v{VERSION}).", i.report.reference));
        out.notes.extend(left);
        return Ok(out);
    }
    let pr = agentfiles::Pr {
        message: message(&i.report, &i.status, &changes),
        title: "Apply the worktrees rules".into(),
        body: body(&i.report, &i.status, &changes, &left),
    };
    agentfiles::publish(p, &i.report, &changes, &pr, push, gh_bin, &mut out)?;
    out.notes.extend(left);
    Ok(out)
}

/// What a write does to a managed piece, in the words a reviewer of the
/// branch reads: an add, an update from an older version, or a replacement.
fn verb(f: &Found) -> String {
    match (&f.state, f.version) {
        (State::Outdated, Some(v)) => format!("update (v{v} → v{VERSION})"),
        (State::Edited, _) => format!("replace the hand-edited (with v{VERSION})"),
        _ => format!("add (v{VERSION})"),
    }
}

fn describe(c: &Change, s: &Status) -> String {
    match c {
        Change::Write { path, .. } if path == SKILL_PATH => format!("- `close-out` skill (`{path}`): {}", verb(&s.skill)),
        Change::Write { path, .. } if path == CONFIG_PATH => format!("- add `{path}`, this repository's close-out settings (yours to edit)"),
        Change::Write { path, content } if content.contains(END) => format!("- worktrees rules block in `{path}`: {}", verb(&s.block)),
        c => agentfiles::describe(c),
    }
}

fn message(r: &Report, s: &Status, changes: &[Change]) -> String {
    let mut m = String::from("chore: apply the worktrees rules\n\n");
    m.push_str("How this repository is worked on with worktrees — places, lanes, the MCP\n");
    m.push_str("tools, planning, finishing and close-out — for every agent that reads\n");
    m.push_str("AGENTS.md, including sessions worktrees did not start.\n\n");
    for c in changes {
        m.push_str(&describe(c, s));
        m.push('\n');
    }
    m.push_str(&format!("\nBuilt against {} by worktrees.\n", r.reference));
    m
}

fn body(r: &Report, s: &Status, changes: &[Change], left: &[String]) -> String {
    let mut b = String::from("Writes the **worktrees rules** into this repository, so every agent — including one worktrees did not launch — knows how places, lanes and close-out work here.\n\n");
    b.push_str("The block in AGENTS.md is managed: `worktrees rules apply` updates it in place and leaves everything outside its markers alone. `.claude/close-out.md` is this repository's to edit.\n\n## Changes\n");
    for c in changes {
        b.push_str(&describe(c, s));
        b.push('\n');
    }
    if !left.is_empty() {
        b.push_str("\n## Left alone\n");
        for l in left {
            b.push_str(&format!("- {l}\n"));
        }
    }
    b.push_str(&format!("\nBuilt against `{}` by worktrees. Revert this PR to undo it.\n", r.reference));
    b
}

// ── diff ─────────────────────────────────────────────────────────────────────

/// One file `rules diff` would show: what is there (under `old_path`, which
/// differs from `new_path` when the layout fix renames CLAUDE.md) and what
/// apply would write.
pub struct Proposal {
    pub old_path: String,
    pub new_path: String,
    pub old: String,
    pub new: String,
}

/// What apply would write vs what is there, per managed piece, for `rules
/// diff`, plus the why-lines for the pieces it would leave alone.
pub fn proposals(p: &crate::project::Project) -> Result<(Vec<Proposal>, Vec<String>), String> {
    let i = inspect_with(p)?;
    let s = &i.status;
    let mut v = Vec::new();
    let mut left = Vec::new();
    // `may(…, true)`: the diff is what `--force` would write too, which is
    // the point of looking at an edited block. Anything else — newer,
    // malformed, foreign — gets the line apply would print, never a patch
    // apply would refuse (with_block on a damaged block APPENDS a second one).
    match (&s.file, &s.blocked) {
        (_, Some(b)) => left.push(format!("the rules block: {b}")),
        (Some(file), None) if may(&s.block, true) => {
            let old_path = match i.target {
                Some(Target::Renamed) => agentfiles::CLAUDE.to_string(),
                _ => file.clone(),
            };
            let cur = i.text.clone().unwrap_or_default();
            let new = with_block(&cur);
            if cur != new {
                v.push(Proposal { old_path, new_path: file.clone(), old: cur, new });
            }
        }
        _ if s.block.state != State::Current => left.push(why("the rules block", &s.block)),
        _ => {}
    }
    if may(&s.skill, true) {
        let cur = blob(&p.main_root, &format!("{}:{SKILL_PATH}", i.report.reference)).unwrap_or_default();
        let new = managed_file(CLOSE_OUT_SKILL);
        if cur != new {
            v.push(Proposal { old_path: SKILL_PATH.into(), new_path: SKILL_PATH.into(), old: cur, new });
        }
    } else if s.skill.state != State::Current {
        left.push(why(SKILL_PATH, &s.skill));
    }
    Ok((v, left))
}

// ── CLI ──────────────────────────────────────────────────────────────────────

fn state_words(f: &Found) -> String {
    match f.state {
        State::Absent => "not applied".into(),
        State::Current => format!("current (v{})", f.version.unwrap_or(VERSION)),
        State::Outdated => format!("outdated (v{} → v{VERSION}) — `worktrees rules apply` updates it", f.version.unwrap_or(0)),
        State::Edited => "edited by hand — `worktrees rules diff`; apply leaves it alone unless --force".into(),
        State::Newer => format!("written by a newer worktrees (v{})", f.version.unwrap_or(0)),
        State::Malformed => format!("damaged: {}", f.detail.clone().unwrap_or_default()),
        State::Foreign => "a close-out skill worktrees did not write — left alone".into(),
    }
}

/// A scratch diff's headers (`a/old`, `b/new`) renamed to the real path.
fn relabel(diff: &str, old_path: &str, new_path: &str) -> String {
    diff.split_inclusive('\n')
        .map(|l| {
            if l.starts_with("diff --git ") || l.starts_with("--- ") || l.starts_with("+++ ") {
                l.replace("a/old", &format!("a/{old_path}")).replace("b/new", &format!("b/{new_path}"))
            } else {
                l.to_string()
            }
        })
        .collect()
}

/// `worktrees rules [status|diff|apply [--push] [--force] [--review]|review [--review]|land|abandon --yes] [--json]`.
pub fn cmd_rules(p: &crate::project::Project, ui: &mut dyn crate::ui::Ui, rest: &[String]) -> i32 {
    let json = rest.iter().any(|a| a == "--json");
    let has = |f: &str| rest.iter().any(|a| a == f);
    let verb = rest.iter().find(|a| !a.starts_with('-')).map(String::as_str).unwrap_or("status");
    match verb {
        "status" => {
            let s = match status(p) {
                Ok(s) => s,
                Err(e) => { ui.error(&e); return 1; }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
                return 0;
            }
            ui.header(&format!("worktrees rules on {} (this worktrees writes v{VERSION})", s.reference));
            match (&s.blocked, &s.file) {
                (Some(b), _) => ui.plain(&format!("  block:  cannot be written — {b}")),
                (None, Some(f)) => ui.plain(&format!("  block:  {} ({f})", state_words(&s.block))),
                (None, None) => {}
            }
            ui.plain(&format!("  skill:  {} ({SKILL_PATH})", state_words(&s.skill)));
            ui.plain(&format!("  config: {} ({CONFIG_PATH})", if s.config { "present — the repository's own" } else { "not there yet" }));
            if let Some(b) = &s.pending {
                if s.pending_on_origin {
                    ui.plain(&format!("  branch '{b}' is waiting to be merged"));
                } else {
                    ui.plain(&format!("  branch '{b}' is committed locally — review it (`worktrees rules review`), then `worktrees rules land` or `worktrees rules abandon --yes`"));
                }
            } else if s.applicable {
                let rules_todo = [&s.block, &s.skill].iter().any(|f| matches!(f.state, State::Absent | State::Outdated)) || !s.config;
                ui.plain(if rules_todo {
                    "  `worktrees rules apply` commits the rest on its own branch"
                } else {
                    "  the rules are current; `worktrees rules apply` would commit only the AGENTS.md layout fix (see `worktrees agent-setup`)"
                });
            }
            0
        }
        "diff" => {
            let (v, why) = match proposals(p) {
                Ok(v) => v,
                Err(e) => { ui.error(&e); return 1; }
            };
            if v.is_empty() && why.is_empty() {
                ui.info("Nothing to change.");
            }
            for w in &why {
                ui.info(w);
            }
            for d in v {
                match crate::guidance::diff_texts_brief(&d.old, &d.new) {
                    Ok(text) => print!("{}", relabel(&text, &d.old_path, &d.new_path)),
                    Err(e) => { ui.error(&format!("{}: {e}", d.new_path)); return 1; }
                }
            }
            0
        }
        "apply" => match apply(p, has("--push"), has("--force")) {
            Ok(o) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
                } else {
                    for n in &o.notes { ui.info(n); }
                }
                // A local commit gets its review lane; `--review` also starts
                // the agent there and then.
                if o.commit.is_some() && !o.pushed {
                    return crate::review::cli_lane(p, ui, has("--review"));
                }
                0
            }
            Err(e) => { ui.error(&e); 1 }
        },
        // The lane for a branch that already exists (`--review`: start the agent).
        "review" => crate::review::cli_lane(p, ui, has("--review")),
        "land" => crate::review::land(p, ui),
        "abandon" => crate::review::abandon(p, ui, has("--yes") || has("-y")),
        other => {
            ui.error(&format!("rules: unknown verb '{other}' (status | diff | apply | review | land | abandon)"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_block_reads_current_and_survives_a_round_trip() {
        let text = with_block("# Our rules\n\nbe kind\n");
        assert!(text.starts_with("# Our rules\n\nbe kind\n\n<!-- worktrees:rules v"), "{text}");
        assert_eq!(block_state(Some(&text)).state, State::Current);
        // Re-applying a current block changes nothing.
        assert_eq!(with_block(&text), text);
        assert_eq!(block_state(Some("# Our rules\n")).state, State::Absent);
        assert_eq!(block_state(None).state, State::Absent);
    }

    #[test]
    fn an_older_block_is_replaced_in_place_and_the_users_text_is_kept() {
        let old_body = "## Working with worktrees\n\nold words\n";
        let old = format!("{BEGIN}v0 sha={} — managed -->\n{old_body}{END}\n", hash(old_body));
        let text = format!("# Top\n\n{old}\n## After\nmine\n");
        assert_eq!(block_state(Some(&text)).state, State::Outdated);
        let new = with_block(&text);
        assert_eq!(block_state(Some(&new)).state, State::Current);
        assert!(new.starts_with("# Top\n\n<!-- worktrees:rules v1"), "{new}");
        assert!(new.ends_with("<!-- worktrees:rules end -->\n\n## After\nmine\n"), "{new}");
        assert!(!new.contains("old words"));
    }

    #[test]
    fn a_hand_edit_inside_the_block_is_noticed() {
        let text = with_block("# R\n").replace("Never `git worktree add`", "Feel free to `git worktree add`");
        assert_eq!(block_state(Some(&text)).state, State::Edited);
    }

    /// The other direction of the hand-edit rule: text the user writes
    /// OUTSIDE the markers is theirs and never makes the block "edited".
    #[test]
    fn an_edit_outside_a_current_block_keeps_it_current() {
        let text = format!("{}\n## Mine\nmore of our own rules\n", with_block("# R\n"));
        assert_eq!(block_state(Some(&text)).state, State::Current);
        assert_eq!(with_block(&text), text, "a re-apply leaves the user's text alone");
    }

    /// A repository that commits AGENTS.md with CRLF: what we wrote reads
    /// current, and a re-apply keeps the file's own line endings.
    #[test]
    fn crlf_files_read_current_and_stay_crlf() {
        let crlf = with_block("# R\n").replace('\n', "\r\n");
        assert_eq!(block_state(Some(&crlf)).state, State::Current);
        let applied = with_block("# R\r\nbe kind\r\n");
        assert!(applied.starts_with("# R\r\nbe kind\r\n\r\n<!-- worktrees:rules"), "{applied:?}");
        assert!(!applied.replace("\r\n", "").contains('\n'), "no bare LF in a CRLF file: {applied:?}");
        assert_eq!(block_state(Some(&applied)).state, State::Current);
        let skill = managed_file(CLOSE_OUT_SKILL).replace('\n', "\r\n");
        assert_eq!(managed_state(Some(&skill), CLOSE_OUT_SKILL).state, State::Current);
    }

    /// Only the trailing NEWLINES go: a markdown hard break (two trailing
    /// spaces) on the user's last line is content.
    #[test]
    fn appending_keeps_the_users_last_line_intact() {
        assert!(with_block("# R\nlast line  \n\n\n").starts_with("# R\nlast line  \n\n<!-- worktrees:rules"));
    }

    #[test]
    fn a_newer_block_and_unpaired_markers_are_not_ours_to_write() {
        let body = "x\n";
        let newer = format!("{BEGIN}v{} sha={} -->\n{body}{END}\n", VERSION + 1, hash(body));
        assert_eq!(block_state(Some(&newer)).state, State::Newer);
        // Edited by hand AFTER a newer worktrees wrote it: still newer, so
        // --force (which replaces EDITED text) can never downgrade it.
        let newer_edited = newer.replace("x\n", "y\n");
        assert_eq!(block_state(Some(&newer_edited)).state, State::Newer);
        let lone = format!("# R\n{BEGIN}v1 sha=00 -->\nx\n");
        let f = block_state(Some(&lone));
        assert_eq!(f.state, State::Malformed);
        assert!(f.detail.unwrap().contains("begin on line 2; end on line none"));
        let twice = format!("{}{}", block(), block());
        assert_eq!(block_state(Some(&twice)).state, State::Malformed);
        let stampless = format!("{BEGIN}hello -->\nx\n{END}\n");
        assert_eq!(block_state(Some(&stampless)).state, State::Malformed);
    }

    #[test]
    fn the_managed_skill_keeps_its_frontmatter_first() {
        let f = managed_file(CLOSE_OUT_SKILL);
        assert!(f.starts_with("---\nname: close-out\n"), "a skill must open with its frontmatter");
        let marker_line = f.lines().position(|l| l.starts_with(MANAGED)).unwrap();
        assert_eq!(f.lines().nth(marker_line - 1), Some("---"));
        assert_eq!(managed_state(Some(&f), CLOSE_OUT_SKILL).state, State::Current);
        assert_eq!(managed_state(Some(&f.replace("Run this top to bottom", "Skip")), CLOSE_OUT_SKILL).state, State::Edited);
        assert_eq!(managed_state(Some("---\nname: close-out\n---\nmine\n"), CLOSE_OUT_SKILL).state, State::Foreign);
        assert_eq!(managed_state(Some(&f), "---\nname: close-out\n---\nnewer default\n").state, State::Outdated);
    }

    #[test]
    fn the_shipped_texts_say_what_agents_need() {
        for needle in ["create_worktree", "--brief", "list_places", "place_status", "report", "wait", "close-out", "(main)"] {
            assert!(BLOCK_BODY.contains(needle), "the block should mention {needle}");
        }
        assert!(BLOCK_BODY.lines().count() <= 45, "the block is a summary, not the skill");
        assert!(!BLOCK_BODY.contains(BEGIN) && !BLOCK_BODY.contains(END) && !BLOCK_BODY.contains(MANAGED));
        assert!(CLOSE_OUT_SKILL.starts_with("---\nname: close-out\ndescription: "));
        assert!(CLOSE_OUT_CONFIG.contains("{project}"));
    }

    fn sh(dir: &std::path::Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args)
            .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
            .output().unwrap();
        assert!(o.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim_end().to_string()
    }

    fn fixture(name: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, std::path::PathBuf) {
        let t = std::fs::canonicalize(std::env::temp_dir()).unwrap()
            .join(format!("wt-rules-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (origin, repo) = (t.join("origin.git"), t.join("myproj"));
        std::fs::create_dir_all(&repo).unwrap();
        sh(&t, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "t"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        for (path, text) in files {
            let f = repo.join(path);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            // "-> target" commits a symlink.
            match text.strip_prefix("-> ") {
                Some(t) => std::os::unix::fs::symlink(t, &f).unwrap(),
                None => std::fs::write(f, text).unwrap(),
            }
        }
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        sh(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        sh(&repo, &["push", "-q", "-u", "origin", "main"]);
        (t, repo)
    }

    /// End to end on real git: a CLAUDE.md-only repo gets the layout fix AND
    /// the rules in one local commit, nothing pushed, the checkout untouched;
    /// merging it makes the status current, and a second apply finds nothing.
    #[test]
    fn apply_commits_locally_and_merging_it_makes_the_status_current() {
        let (t, repo) = fixture("apply", &[("CLAUDE.md", "# Our rules\nbe kind\n")]);
        let p = crate::project::Project::discover(&repo).unwrap();
        let before = status(&p).unwrap();
        assert_eq!(before.block.state, State::Absent);
        assert_eq!(before.skill.state, State::Absent);
        assert!(!before.config && before.applicable);
        let head = sh(&repo, &["rev-parse", "HEAD"]);
        // A DIRTY checkout — an edit and an untracked file — must come out
        // exactly as it went in: apply works on a private index, never here.
        std::fs::write(repo.join("README.md"), "hi, edited\n").unwrap();
        std::fs::write(repo.join("scratch.txt"), "mine\n").unwrap();
        let dirty = sh(&repo, &["status", "--porcelain"]);
        assert!(!dirty.is_empty());

        let out = apply_with(&p, "/nonexistent/gh", false, false).unwrap();
        assert!(out.commit.is_some() && !out.pushed && out.pr_url.is_none(), "{:?}", out.notes);
        assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(sh(&repo, &["status", "--porcelain"]), dirty);
        assert_eq!(std::fs::read_to_string(repo.join("README.md")).unwrap(), "hi, edited\n");
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        std::fs::remove_file(repo.join("scratch.txt")).unwrap();
        assert!(sh(&repo, &["ls-remote", "origin", "agent-instructions"]).is_empty(), "nothing was pushed");
        let b = "agent-instructions";
        let agents = sh(&repo, &["show", &format!("{b}:AGENTS.md")]);
        assert!(agents.starts_with("# Our rules\nbe kind\n\n<!-- worktrees:rules v1"), "{agents}");
        assert_eq!(sh(&repo, &["show", &format!("{b}:CLAUDE.md")]), agentfiles::STUB.trim_end());
        assert!(sh(&repo, &["show", &format!("{b}:{SKILL_PATH}")]).starts_with("---\nname: close-out"));
        assert!(sh(&repo, &["show", &format!("{b}:{CONFIG_PATH}")]).contains("# close-out overrides — myproj"));
        assert_eq!(sh(&repo, &["ls-tree", b, ".agents/skills/close-out"]).split_whitespace().next(), Some("120000"));
        // Pending until merged: a second apply refuses rather than racing it.
        assert_eq!(status(&p).unwrap().pending.as_deref(), Some(b));
        assert!(apply_with(&p, "/nonexistent/gh", false, false).unwrap_err().contains("already exists"));

        sh(&repo, &["merge", "-q", "--ff-only", b]);
        sh(&repo, &["push", "-q", "origin", "main"]);
        sh(&repo, &["branch", "-q", "-D", b]);
        let after = status(&p).unwrap();
        assert_eq!(after.block.state, State::Current);
        assert_eq!(after.skill.state, State::Current);
        assert!(after.config && !after.applicable && after.pending.is_none(), "{after:?}");
        let again = apply_with(&p, "/nonexistent/gh", false, false).unwrap();
        assert!(again.commit.is_none(), "{:?}", again.notes);
        let _ = std::fs::remove_dir_all(&t);
    }

    /// The repository's own files win: an existing close-out skill is never
    /// overwritten (not even forced), an existing close-out.md is never
    /// touched, and an edited block is replaced only with force.
    #[test]
    fn apply_leaves_the_repositorys_own_files_alone() {
        let edited = with_block("# R\n").replace("Never `git worktree add`", "Sometimes `git worktree add`");
        let (t, repo) = fixture("own", &[
            ("AGENTS.md", &edited),
            ("CLAUDE.md", agentfiles::STUB),
            (SKILL_PATH, "---\nname: close-out\ndescription: ours\n---\nmine\n"),
            // Codex already sees it, so the layout fix has nothing to do either.
            (".agents/skills/close-out/SKILL.md", "---\nname: close-out\ndescription: ours\n---\nmine\n"),
            (CONFIG_PATH, "our settings\n"),
        ]);
        let p = crate::project::Project::discover(&repo).unwrap();
        let s = status(&p).unwrap();
        assert_eq!((s.block.state.clone(), s.skill.state.clone(), s.config), (State::Edited, State::Foreign, true));
        assert!(!s.applicable, "{s:?}");
        let out = apply_with(&p, "/nonexistent/gh", false, false).unwrap();
        assert!(out.commit.is_none());
        assert!(out.notes.iter().any(|n| n.contains("edited by hand")), "{:?}", out.notes);
        let forced = apply_with(&p, "/nonexistent/gh", false, true).unwrap();
        assert!(forced.commit.is_some(), "{:?}", forced.notes);
        let b = "agent-instructions";
        // The reviewer of the branch is told it REPLACES their edit.
        assert!(sh(&repo, &["log", "-1", "--format=%B", b]).contains("worktrees rules block in `AGENTS.md`: replace the hand-edited"));
        assert_eq!(block_state(Some(&sh(&repo, &["show", &format!("{b}:AGENTS.md")]))).state, State::Current);
        assert!(sh(&repo, &["show", &format!("{b}:{SKILL_PATH}")]).ends_with("mine"));
        assert_eq!(sh(&repo, &["show", &format!("{b}:{CONFIG_PATH}")]), "our settings");
        let _ = std::fs::remove_dir_all(&t);
    }

    /// A repo with no instruction file at all gets AGENTS.md + the stub; one
    /// whose root pair diverged gets no block (Claude would not read it).
    #[test]
    fn apply_creates_agents_md_and_refuses_a_diverged_root() {
        let (t, repo) = fixture("new", &[]);
        let p = crate::project::Project::discover(&repo).unwrap();
        apply_with(&p, "/nonexistent/gh", false, false).unwrap();
        let b = "agent-instructions";
        assert_eq!(block_state(Some(&sh(&repo, &["show", &format!("{b}:AGENTS.md")]))).state, State::Current);
        assert_eq!(sh(&repo, &["show", &format!("{b}:CLAUDE.md")]), agentfiles::STUB.trim_end());
        let _ = std::fs::remove_dir_all(&t);

        let (t, repo) = fixture("diverged", &[("AGENTS.md", "codex\n"), ("CLAUDE.md", "claude\n")]);
        let p = crate::project::Project::discover(&repo).unwrap();
        let s = status(&p).unwrap();
        assert!(s.blocked.as_deref().unwrap().contains("differ"), "{s:?}");
        let out = apply_with(&p, "/nonexistent/gh", false, false).unwrap();
        assert!(sh(&repo, &["show", &format!("{b}:AGENTS.md")]) == "codex", "the diverged file is untouched");
        assert!(out.notes.iter().any(|n| n.contains("rules block")), "{:?}", out.notes);
        let _ = std::fs::remove_dir_all(&t);
    }

    /// `rules diff` shows only what `apply` would write: a damaged block gets
    /// its reason, never a patch adding a second block; and a CLAUDE.md that
    /// the layout fix renames is labelled as the rename it is.
    #[test]
    fn diff_proposes_only_what_apply_would_write() {
        let damaged = format!("{}{END}\n", with_block("# R\n"));
        let (t, repo) = fixture("diff-bad", &[("AGENTS.md", &damaged), ("CLAUDE.md", agentfiles::STUB)]);
        let p = crate::project::Project::discover(&repo).unwrap();
        let (v, why) = proposals(&p).unwrap();
        assert!(v.iter().all(|d| d.new_path != "AGENTS.md"), "a malformed block must not be re-proposed: {:?}", v.iter().map(|d| &d.new_path).collect::<Vec<_>>());
        assert!(why.iter().any(|w| w.contains("do not pair up")), "{why:?}");
        let _ = std::fs::remove_dir_all(&t);

        let (t, repo) = fixture("diff-rename", &[("CLAUDE.md", "# ours\n")]);
        let p = crate::project::Project::discover(&repo).unwrap();
        let (v, _) = proposals(&p).unwrap();
        let d = v.iter().find(|d| d.new_path == "AGENTS.md").expect("the block is proposed");
        assert_eq!(d.old_path, "CLAUDE.md", "the old text is CLAUDE.md's");
        let _ = std::fs::remove_dir_all(&t);
    }

    /// A tracked symlink anywhere on the skill's path is the repository's own
    /// layout: never written through (git would refuse it as a D/F clash
    /// anyway, with a message naming nothing). The rest still applies.
    #[test]
    fn a_symlinked_skills_dir_is_left_alone_and_the_rest_applies() {
        for (name, link) in [("lnk-skills", ".claude/skills"), ("lnk-skill", ".claude/skills/close-out"), ("lnk-claude", ".claude")] {
            let (t, repo) = fixture(name, &[("AGENTS.md", "# ours\n"), ("CLAUDE.md", agentfiles::STUB), (link, "-> ../elsewhere")]);
            let p = crate::project::Project::discover(&repo).unwrap();
            let s = status(&p).unwrap();
            assert_eq!(s.skill.state, State::Foreign, "{link}: {s:?}");
            assert!(s.skill.detail.as_deref().unwrap_or("").contains("symlink"), "{link}: {s:?}");
            let out = apply_with(&p, "/nonexistent/gh", false, false).unwrap_or_else(|e| panic!("{link}: {e}"));
            assert!(out.commit.is_some(), "{link}: {:?}", out.notes);
            assert_eq!(block_state(Some(&sh(&repo, &["show", "agent-instructions:AGENTS.md"]))).state, State::Current, "{link}");
            let _ = std::fs::remove_dir_all(&t);
        }
    }

    /// `CLAUDE.md -> ./AGENTS.md` is the same layout as `-> AGENTS.md`.
    #[test]
    fn a_dot_slash_link_is_the_same_link() {
        let (t, repo) = fixture("dotslash", &[("AGENTS.md", "# ours\n"), ("CLAUDE.md", "-> ./AGENTS.md")]);
        let p = crate::project::Project::discover(&repo).unwrap();
        let s = status(&p).unwrap();
        assert!(s.blocked.is_none(), "{s:?}");
        assert_eq!(s.file.as_deref(), Some("AGENTS.md"));
        let _ = std::fs::remove_dir_all(&t);
    }
}

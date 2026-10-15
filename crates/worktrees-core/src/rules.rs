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

fn judge(version: u32, recorded: &str, body: &str, shipped: &str) -> State {
    if hash(body) != recorded {
        State::Edited
    } else if version > VERSION {
        State::Newer
    } else if version == VERSION && body == shipped {
        State::Current
    } else {
        State::Outdated
    }
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
    match find_block(text) {
        Ok(Some(s)) => format!("{}{}{}", &text[..s.start], block(), &text[s.end..]),
        _ => {
            let own = text.trim_end();
            if own.is_empty() { block() } else { format!("{own}\n\n{}", block()) }
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
enum Target {
    /// Write into this existing file.
    File(String),
    /// The layout fix renames CLAUDE.md → AGENTS.md; the block goes into
    /// AGENTS.md with CLAUDE.md's text above it.
    Renamed,
    /// No instruction file at the root: create AGENTS.md (+ the stub).
    New,
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
                (Some(t), None) if t.trim() == agentfiles::AGENTS && has(agentfiles::AGENTS) => Ok(Target::File(agentfiles::AGENTS.into())),
                (None, Some(t)) if t.trim() == agentfiles::CLAUDE && has(agentfiles::CLAUDE) => Ok(Target::File(agentfiles::CLAUDE.into())),
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
}

fn inspect_with(p: &crate::project::Project) -> Result<Inspected, String> {
    let report = agentfiles::inspect(p)?;
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
    let skill = match skill_on_ref {
        Some(e) if e.mode == "120000" => Found::new(State::Foreign, None),
        Some(e) => managed_state(read_blob(&e.sha).as_deref(), CLOSE_OUT_SKILL),
        None => Found::new(State::Absent, None),
    };
    let config = entries.iter().any(|e| e.path == CONFIG_PATH);
    let needs_link = skill_on_ref.is_none()
        && !entries.iter().any(|e| {
            e.path == ".agents/skills/close-out"
                || e.path.starts_with(".agents/skills/close-out/")
                || (e.mode == "120000" && (e.path == ".agents/skills" || e.path == ".agents"))
        });
    let writable = |f: &Found| matches!(f.state, State::Absent | State::Outdated);
    let applicable =
        report.fixable || (blocked.is_none() && writable(&block)) || writable(&skill) || (!config && skill.state != State::Foreign);
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
    Ok(Inspected { report, status, target, text, needs_link })
}

pub fn status(p: &crate::project::Project) -> Result<Status, String> {
    inspect_with(p).map(|i| i.status)
}

// ── apply ────────────────────────────────────────────────────────────────────

/// The changes apply makes on top of the layout fix. `force` replaces an
/// EDITED block or skill (never a foreign skill, never a newer or malformed
/// one). Returns the changes and what it left alone, in words.
fn rules_changes(i: &Inspected, project: &str, force: bool) -> (Vec<Change>, Vec<String>) {
    let mut v = Vec::new();
    let mut left = Vec::new();
    let may = |f: &Found| matches!(f.state, State::Absent | State::Outdated) || (force && f.state == State::Edited);
    let why = |what: &str, f: &Found| match f.state {
        State::Edited => format!("{what} was edited by hand — left alone (`worktrees rules diff` shows it; --force replaces it)"),
        State::Newer => format!("{what} was written by a newer worktrees (v{}) — left alone; upgrade worktrees", f.version.unwrap_or(0)),
        State::Malformed => format!("{what}: {} — left alone; fix the markers by hand", f.detail.clone().unwrap_or_default()),
        State::Foreign => format!("{what} is not one worktrees wrote — left alone"),
        _ => String::new(),
    };
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
    if !s.config && s.skill.state != State::Foreign {
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
    let mut changes = agentfiles::plan(&i.report.dirs, &i.report.skills);
    let (ours, left) = rules_changes(&i, &project, force);
    changes.extend(ours);
    if changes.is_empty() {
        out.notes.push(format!("{} already carries the worktrees rules (v{VERSION}).", i.report.reference));
        out.notes.extend(left);
        return Ok(out);
    }
    let pr = agentfiles::Pr {
        message: message(&i.report, &changes),
        title: "Apply the worktrees rules".into(),
        body: body(&i.report, &changes, &left),
    };
    agentfiles::publish(p, &i.report, &changes, &pr, push, gh_bin, &mut out)?;
    out.notes.extend(left);
    Ok(out)
}

fn describe(c: &Change) -> String {
    match c {
        Change::Write { path, .. } if path == SKILL_PATH => format!("- write the `close-out` skill (`{path}`)"),
        Change::Write { path, .. } if path == CONFIG_PATH => format!("- add `{path}`, this repository's close-out settings (yours to edit)"),
        Change::Write { path, content } if content.contains(END) => format!("- add the worktrees rules block to `{path}` (v{VERSION})"),
        c => agentfiles::describe(c),
    }
}

fn message(r: &Report, changes: &[Change]) -> String {
    let mut m = String::from("chore: apply the worktrees rules\n\n");
    m.push_str("How this repository is worked on with worktrees — places, lanes, the MCP\n");
    m.push_str("tools, planning, finishing and close-out — for every agent that reads\n");
    m.push_str("AGENTS.md, including sessions worktrees did not start.\n\n");
    for c in changes {
        m.push_str(&describe(c));
        m.push('\n');
    }
    m.push_str(&format!("\nBuilt against {} by worktrees.\n", r.reference));
    m
}

fn body(r: &Report, changes: &[Change], left: &[String]) -> String {
    let mut b = String::from("Writes the **worktrees rules** into this repository, so every agent — including one worktrees did not launch — knows how places, lanes and close-out work here.\n\n");
    b.push_str("The block in AGENTS.md is managed: `worktrees rules apply` updates it in place and leaves everything outside its markers alone. `.claude/close-out.md` is this repository's to edit.\n\n## Changes\n");
    for c in changes {
        b.push_str(&describe(c));
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

/// What apply would write vs what is there, per managed piece, for `rules
/// diff`: `(path, current text or "", proposed text)`.
pub fn proposals(p: &crate::project::Project) -> Result<Vec<(String, String, String)>, String> {
    let i = inspect_with(p)?;
    let mut v = Vec::new();
    if let (Some(file), None) = (&i.status.file, &i.status.blocked) {
        let cur = i.text.clone().unwrap_or_default();
        let new = with_block(&cur);
        if cur != new {
            v.push((file.clone(), cur, new));
        }
    }
    if i.status.skill.state != State::Foreign {
        let root = p.main_root.as_str();
        let cur = blob(root, &format!("{}:{SKILL_PATH}", i.report.reference)).unwrap_or_default();
        let new = managed_file(CLOSE_OUT_SKILL);
        if cur != new {
            v.push((SKILL_PATH.to_string(), cur, new));
        }
    }
    Ok(v)
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
fn relabel(diff: &str, path: &str) -> String {
    diff.split_inclusive('\n')
        .map(|l| {
            if l.starts_with("diff --git ") || l.starts_with("--- ") || l.starts_with("+++ ") {
                l.replace("a/old", &format!("a/{path}")).replace("b/new", &format!("b/{path}"))
            } else {
                l.to_string()
            }
        })
        .collect()
}

/// `worktrees rules [status|diff|apply [--push] [--force]] [--json]`.
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
                    ui.plain(&format!("  branch '{b}' is committed locally — review it (`worktrees new {b}`), push it and open a PR"));
                }
            } else if s.applicable {
                ui.plain("  `worktrees rules apply` commits the rest on its own branch");
            }
            0
        }
        "diff" => {
            let v = match proposals(p) {
                Ok(v) => v,
                Err(e) => { ui.error(&e); return 1; }
            };
            if v.is_empty() {
                ui.info("Nothing to change.");
            }
            for (path, cur, new) in v {
                match crate::guidance::diff_texts_brief(&cur, &new) {
                    Ok(d) => print!("{}", relabel(&d, &path)),
                    Err(e) => { ui.error(&format!("{path}: {e}")); return 1; }
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
                0
            }
            Err(e) => { ui.error(&e); 1 }
        },
        other => {
            ui.error(&format!("rules: unknown verb '{other}' (status | diff | apply)"));
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

    #[test]
    fn a_newer_block_and_unpaired_markers_are_not_ours_to_write() {
        let body = "x\n";
        let newer = format!("{BEGIN}v{} sha={} -->\n{body}{END}\n", VERSION + 1, hash(body));
        assert_eq!(block_state(Some(&newer)).state, State::Newer);
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
            std::fs::write(f, text).unwrap();
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

        let out = apply_with(&p, "/nonexistent/gh", false, false).unwrap();
        assert!(out.commit.is_some() && !out.pushed && out.pr_url.is_none(), "{:?}", out.notes);
        assert_eq!(sh(&repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(sh(&repo, &["status", "--porcelain"]), "");
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
}

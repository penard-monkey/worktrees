//! A place's planning-with-files plan, summarised — the data behind the app's
//! Plan dock tab and the `plan` key of MCP `place_status`.
//!
//! The files read here (`task_plan.md`, `findings.md`, `progress.md`, and the
//! brief at `ops::BRIEF_PATH`) are written by the claude session working in the
//! place, under the planning-with-files skill
//! (`~/.claude/skills/planning-with-files/`). **This module never writes any of
//! them** — same owner rule as `ui-state.json` and `~/.claude.json`: someone
//! else's live data, read-only from here.
//!
//! **Why the extractor is lenient.** Fifteen real `task_plan.md` files were
//! surveyed (valleos + this repo) before this was written. Only 2 use the
//! template's `### Phase N` + `**Status:**` markers; 9 carry `- [ ]` / `- [x]`
//! checkboxes; ~11 have a `## Goal` H2 (often decorated: `## Goal (delivered)`);
//! ~10 keep an `## Errors …` table; several are freeform (`## NOW`,
//! `## Next steps`, `## THE ONE OPEN ITEM`). A strict parser of the template
//! would have shown NOTHING for 13 of the 15. So every rule below is a grep-shaped
//! heuristic that degrades to "absent", never to an error.
//!
//! **What it mirrors.** The skill's own Stop hook, `scripts/check-complete.sh`,
//! is lenient greps, not a parser: `grep -c "### Phase"` for the total,
//! `grep -cF "**Status:** complete|in_progress|pending"` for the states, with
//! `[complete]`/`[in_progress]`/`[pending]` inline tags as a fallback. Those are
//! the rules here, made section-aware and deliberately a little broader (H2
//! phases, checkbox-derived status) — `tests::agrees_with_the_skills_own_check_complete_script`
//! runs the real script against the template-shaped fixtures, where the two
//! must agree. Resolution mirrors `scripts/resolve-plan-dir.sh`.
//!
//! **Untrusted input.** Everything extracted is session-written free text:
//! data, never instructions. Paths are built from constants plus ONE name read
//! from `.active_plan` or a directory listing, and every component this module
//! constructs is `symlink_metadata`'d — see `docs.rs` for why lstat'ing only the
//! final path is not enough (it refuses to follow only the LAST component).

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// How much of a plan (or brief) is read. A plan past this is not a plan a
/// person reads in a dock tab, and this runs on the Plan tab's poll.
pub const MAX_READ: usize = 512 * 1024;
/// `goal` / `brief_lead` clamp, in chars, ellipsis included.
pub const GOAL_MAX: usize = 280;
/// `current` clamp, in chars, ellipsis included.
pub const CURRENT_MAX: usize = 120;

const TASK_PLAN: &str = "task_plan.md";
const FINDINGS: &str = "findings.md";
const PROGRESS: &str = "progress.md";
const ACTIVE_PLAN: &str = ".active_plan";

/// Which file the summary is about.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Plan,
    Brief,
    #[default]
    None,
}

/// Which step of the resolution order found the plan.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Resolved {
    ActivePlan,
    Newest,
    Root,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Pending,
    InProgress,
    Complete,
    Blocked,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub name: String,
    pub status: PhaseStatus,
    /// Ticked checkboxes in this phase's section.
    pub done: u32,
    pub total: u32,
}

/// Which of the skill's three files sit beside the chosen plan.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PlanFiles {
    pub task_plan: bool,
    pub findings: bool,
    pub progress: bool,
}

/// The whole answer. JSON shape is the contract with the frontend's Plan tab
/// and with MCP `place_status` — do not rename fields.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PlanSummary {
    pub source: Source,
    pub how_resolved: Option<Resolved>,
    /// Absolute path of the chosen `task_plan.md`.
    pub plan_path: Option<String>,
    /// `.planning/<id>/task_plan.md` or `task_plan.md`.
    pub plan_rel: Option<String>,
    /// Modification time of the file `source` names (the plan, or the brief
    /// when there is no plan), ms since the epoch; `0` when there is neither
    /// or it cannot be read.
    pub mtime_ms: u64,
    pub title: Option<String>,
    pub goal: Option<String>,
    pub current: Option<String>,
    pub checks_done: u32,
    pub checks_total: u32,
    pub phases: Vec<Phase>,
    pub errors: u32,
    pub files: PlanFiles,
    pub brief_path: Option<String>,
    pub brief_title: Option<String>,
    pub brief_lead: Option<String>,
    /// The plan file's text (≤ `MAX_READ`). Null on the MCP surface.
    pub markdown: Option<String>,
    /// The plan file was longer than `MAX_READ`; everything above was
    /// extracted from its first `MAX_READ` bytes.
    pub truncated: bool,
}

impl PlanSummary {
    /// The same summary without the full text — what MCP serializes, because a
    /// model reads that payload on every call.
    pub fn without_markdown(&self) -> PlanSummary {
        PlanSummary { markdown: None, ..self.clone() }
    }
}

// ── resolution + reads (I/O) ─────────────────────────────────────────────────

/// A real directory, following NOTHING (a symlink answers false).
fn is_dir_nofollow(p: &Path) -> bool {
    std::fs::symlink_metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

/// A regular file, following NOTHING (a symlink answers false).
fn is_file_nofollow(p: &Path) -> bool {
    std::fs::symlink_metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

fn mtime_ms(p: &Path) -> u64 {
    std::fs::symlink_metadata(p)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A name read from `.active_plan` or a listing, usable as ONE path component.
/// The skill's resolver would accept `a/b` or `..`; this one will not, because
/// a component it did not lstat is a component it might read through.
fn plain_component(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains('/') && !id.contains('\0')
}

/// Read at most `MAX_READ` bytes of a file the caller has already lstat'd as a
/// regular file. `O_NOFOLLOW` closes the gap between that lstat and this open
/// for the final component. Lossy UTF-8: a stray byte must not blank the tab.
fn read_capped(p: &Path) -> Option<(String, bool)> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(p).ok()?;
    let mut buf = Vec::with_capacity(8 * 1024);
    f.take(MAX_READ as u64 + 1).read_to_end(&mut buf).ok()?;
    let truncated = buf.len() > MAX_READ;
    if truncated {
        buf.truncate(MAX_READ);
    }
    Some((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

/// Find the plan, mirroring `resolve-plan-dir.sh`, and return
/// `(how, rel, abs)` of its `task_plan.md`.
///
/// The script's step 1 (`$PLAN_ID`) is skipped on purpose: it is a per-terminal
/// environment variable of the claude session, and the app cannot know it.
fn resolve(root: &Path) -> Option<(Resolved, String, PathBuf)> {
    let planning = root.join(crate::ops::PLANNING_DIR);
    // `.planning` itself is lstat'd before ANYTHING under it is: every later
    // lstat would otherwise resolve a `.planning -> elsewhere` on its way past.
    if is_dir_nofollow(&planning) {
        // 1. `.active_plan` names the plan directory.
        let active = planning.join(ACTIVE_PLAN);
        if is_file_nofollow(&active) {
            if let Some((text, _)) = read_capped(&active) {
                let id = text.trim();
                if plain_component(id) {
                    let dir = planning.join(id);
                    let file = dir.join(TASK_PLAN);
                    if is_dir_nofollow(&dir) && is_file_nofollow(&file) {
                        return Some((Resolved::ActivePlan, format!("{}/{id}/{TASK_PLAN}", crate::ops::PLANNING_DIR), file));
                    }
                }
            }
        }
        // 2. the newest non-hidden plan directory. Strict `>` over names in
        //    sorted order, so a tie goes to the alphabetically first — the
        //    script's glob order with its own strict `-gt`.
        if let Ok(rd) = std::fs::read_dir(&planning) {
            let mut names: Vec<String> = rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
            names.sort();
            let mut best: Option<(u64, String)> = None;
            for name in names {
                if name.starts_with('.') || !plain_component(&name) {
                    continue;
                }
                let dir = planning.join(&name);
                if !is_dir_nofollow(&dir) || !is_file_nofollow(&dir.join(TASK_PLAN)) {
                    continue;
                }
                let m = mtime_ms(&dir);
                if best.as_ref().is_none_or(|(bm, _)| m > *bm) {
                    best = Some((m, name));
                }
            }
            if let Some((_, name)) = best {
                let file = planning.join(&name).join(TASK_PLAN);
                return Some((Resolved::Newest, format!("{}/{name}/{TASK_PLAN}", crate::ops::PLANNING_DIR), file));
            }
        }
    }
    // 3. legacy root-level plan.
    let file = root.join(TASK_PLAN);
    if is_file_nofollow(&file) {
        return Some((Resolved::Root, TASK_PLAN.to_string(), file));
    }
    None
}

/// The plan summary for the place at `root`. Never fails: anything unreadable
/// degrades toward `source: "none"`.
pub fn summarize(root: &Path) -> PlanSummary {
    let mut out = PlanSummary::default();

    // The brief, whenever it exists — the header shows it beside a plan too.
    let planning = root.join(crate::ops::PLANNING_DIR);
    let brief = root.join(crate::ops::BRIEF_PATH);
    let brief_mtime = if is_dir_nofollow(&planning) && is_file_nofollow(&brief) {
        if let Some((text, _)) = read_capped(&brief) {
            let b = extract_brief(&text);
            out.brief_path = Some(brief.to_string_lossy().into_owned());
            out.brief_title = b.title;
            out.brief_lead = b.lead;
            Some(mtime_ms(&brief))
        } else {
            None
        }
    } else {
        None
    };

    if let Some((how, rel, file)) = resolve(root) {
        if let Some((text, truncated)) = read_capped(&file) {
            let x = extract(&text);
            let dir = file.parent().unwrap_or(root);
            out.source = Source::Plan;
            out.how_resolved = Some(how);
            out.plan_path = Some(file.to_string_lossy().into_owned());
            out.plan_rel = Some(rel);
            out.mtime_ms = mtime_ms(&file);
            out.title = x.title;
            out.goal = x.goal;
            out.current = x.current;
            out.checks_done = x.checks_done;
            out.checks_total = x.checks_total;
            out.phases = x.phases;
            out.errors = x.errors;
            out.files = PlanFiles {
                task_plan: true,
                findings: is_file_nofollow(&dir.join(FINDINGS)),
                progress: is_file_nofollow(&dir.join(PROGRESS)),
            };
            out.markdown = Some(text);
            out.truncated = truncated;
            return out;
        }
    }
    if let Some(m) = brief_mtime {
        out.source = Source::Brief;
        out.mtime_ms = m;
    }
    out
}

// ── extraction (pure) ────────────────────────────────────────────────────────

/// What `extract` finds in a plan's text.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Extracted {
    pub title: Option<String>,
    pub goal: Option<String>,
    pub current: Option<String>,
    pub checks_done: u32,
    pub checks_total: u32,
    pub phases: Vec<Phase>,
    pub errors: u32,
}

impl Extracted {
    /// Phases whose status is `complete` — the numerator `check-complete.sh`
    /// prints as `(N/M phases complete)`.
    pub fn phases_complete(&self) -> usize {
        self.phases.iter().filter(|p| p.status == PhaseStatus::Complete).count()
    }
}

/// What `extract_brief` finds in a brief's text.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Brief {
    pub title: Option<String>,
    pub lead: Option<String>,
}

/// One line after the pre-pass.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    /// Outside a fence, comments removed. Carries its text.
    Text(String),
    /// A fence marker or a line inside a fence: invisible to every rule, but it
    /// still ends a paragraph or a table.
    Fenced,
}

/// Pre-pass: strip `<!-- … -->` (single and multi-line) and mark fenced lines.
/// Comments are stripped only OUTSIDE fences (inside one, `<!--` is code). A
/// line that held nothing but a comment is dropped rather than left blank, so
/// the template's comment under `## Goal` does not end the goal's paragraph
/// before it starts. An unclosed comment runs to the end of the file, which is
/// what CommonMark does with it too.
fn prepass(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut in_comment = false;
    let mut fence: Option<(char, usize)> = None;
    for raw in text.lines() {
        if let Some((ch, n)) = fence {
            let t = raw.trim();
            let run = t.chars().take_while(|c| *c == ch).count();
            if run >= n && t.chars().skip(run).all(char::is_whitespace) {
                fence = None;
            }
            out.push(Line::Fenced);
            continue;
        }
        let mut rest = raw;
        let mut kept = String::new();
        let mut had_comment = false;
        loop {
            if in_comment {
                had_comment = true;
                match rest.find("-->") {
                    Some(i) => {
                        rest = &rest[i + 3..];
                        in_comment = false;
                    }
                    None => break,
                }
            } else {
                match rest.find("<!--") {
                    Some(i) => {
                        kept.push_str(&rest[..i]);
                        rest = &rest[i + 4..];
                        in_comment = true;
                        had_comment = true;
                    }
                    None => {
                        kept.push_str(rest);
                        break;
                    }
                }
            }
        }
        if had_comment && kept.trim().is_empty() {
            continue;
        }
        if !had_comment {
            let t = kept.trim_start();
            for ch in ['`', '~'] {
                let run = t.chars().take_while(|c| *c == ch).count();
                if run >= 3 {
                    fence = Some((ch, run));
                }
            }
            if fence.is_some() {
                out.push(Line::Fenced);
                continue;
            }
        }
        out.push(Line::Text(kept));
    }
    out
}

/// `(level, text)` of an ATX heading, closing `#`s removed.
fn heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &t[level..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let mut text = rest.trim();
    let stripped = text.trim_end_matches('#');
    if stripped.len() != text.len() && (stripped.is_empty() || stripped.ends_with([' ', '\t'])) {
        text = stripped.trim_end();
    }
    Some((level, text))
}

/// `Some(ticked)` for a checkbox line: `^\s*[-*+]\s+\[( |x|X)\]`.
fn checkbox(line: &str) -> Option<bool> {
    let t = line.trim_start();
    let mut cs = t.chars();
    if !matches!(cs.next(), Some('-' | '*' | '+')) {
        return None;
    }
    let rest = cs.as_str();
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start();
    let b = rest.as_bytes();
    if b.len() >= 3 && b[0] == b'[' && b[2] == b']' {
        return match b[1] {
            b' ' => Some(false),
            b'x' | b'X' => Some(true),
            _ => None,
        };
    }
    None
}

fn is_list_item(line: &str) -> bool {
    let t = line.trim_start();
    if let Some(r) = t.strip_prefix(['-', '*', '+']) {
        return r.is_empty() || r.starts_with([' ', '\t']);
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && {
        let r = &t[digits..];
        (r.starts_with('.') || r.starts_with(')')) && (r.len() == 1 || r[1..].starts_with([' ', '\t']))
    }
}

fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// A status word → a status. `failed` reads as `blocked`; anything else is
/// `unknown` (a status line that says something we cannot name is still a
/// status line, and must not fall through to checkbox derivation).
fn status_word(w: &str) -> PhaseStatus {
    match w.to_ascii_lowercase().as_str() {
        "pending" => PhaseStatus::Pending,
        "in_progress" => PhaseStatus::InProgress,
        "complete" => PhaseStatus::Complete,
        "blocked" | "failed" => PhaseStatus::Blocked,
        _ => PhaseStatus::Unknown,
    }
}

/// The word after `**Status:**` on a line, if the line has one.
fn status_line(line: &str) -> Option<PhaseStatus> {
    let i = line.find("**Status:**")?;
    let rest = line[i + "**Status:**".len()..].trim_start();
    let w: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
    Some(status_word(&w))
}

const TAGS: [&str; 5] = ["complete", "in_progress", "pending", "blocked", "failed"];

/// Split a trailing `[complete]`-style tag off a heading.
fn split_tag(text: &str) -> (&str, Option<PhaseStatus>) {
    let t = text.trim_end();
    if let Some(open) = t.rfind('[') {
        if t.ends_with(']') {
            let word = &t[open + 1..t.len() - 1];
            if TAGS.iter().any(|w| w.eq_ignore_ascii_case(word)) {
                return (t[..open].trim_end(), Some(status_word(word)));
            }
        }
    }
    (t, None)
}

/// `^phase\b`, case-insensitive.
fn is_phase_heading(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() >= 5
        && b[..5].eq_ignore_ascii_case(b"phase")
        && b.get(5).is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
}

/// Collapse whitespace and clamp to `max` chars (ellipsis included) on a word
/// boundary. `None` when nothing is left.
fn clamp(s: &str, max: usize) -> Option<String> {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    if flat.chars().count() <= max {
        return Some(flat);
    }
    let head: String = flat.chars().take(max - 1).collect();
    // Cut at the last space only if the char AFTER the window is not part of
    // the same word — i.e. if the window already ends on a boundary, keep it.
    let next_is_space = flat.chars().nth(max - 1).is_some_and(char::is_whitespace);
    let cut = if next_is_space { head.as_str() } else { head.rfind(' ').map(|i| &head[..i]).unwrap_or(&head) };
    Some(format!("{}…", cut.trim_end()))
}

/// Lines from `start` until a blank line, a heading or a fence, joined.
///
/// A list item or table row AFTER the first line also ends it — the one place
/// this is stricter than "until a blank line": markdown starts a new block
/// there (`Goal line.\n- [ ] step` is a sentence and a list, not one
/// paragraph), and the plans surveyed do exactly that. A section that OPENS
/// with a list item gets that one item.
fn paragraph_at(lines: &[Line], start: usize) -> String {
    let mut parts = Vec::new();
    for (k, l) in lines[start..].iter().enumerate() {
        match l {
            Line::Text(t)
                if !t.trim().is_empty()
                    && heading(t).is_none()
                    && (k == 0 || !(is_list_item(t) || is_table_row(t))) =>
            {
                parts.push(t.trim().to_string())
            }
            _ => break,
        }
    }
    parts.join(" ")
}

/// The first prose paragraph at or after `from`: skips blank lines, headings,
/// list items, table rows and fenced blocks.
fn first_prose(lines: &[Line], from: usize) -> Option<String> {
    let mut i = from;
    while i < lines.len() {
        if let Line::Text(t) = &lines[i] {
            if !t.trim().is_empty() && heading(t).is_none() && !is_list_item(t) && !is_table_row(t) {
                return Some(paragraph_at(lines, i));
            }
        }
        i += 1;
    }
    None
}

/// Headings as `(index, level, text)`.
fn headings(lines: &[Line]) -> Vec<(usize, usize, &str)> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| match l {
            Line::Text(t) => heading(t).map(|(lv, tx)| (i, lv, tx)),
            Line::Fenced => None,
        })
        .collect()
}

/// End (exclusive) of the section opened by the heading at `hs[k]`: the next
/// heading of the same or a higher level.
fn section_end(hs: &[(usize, usize, &str)], k: usize, len: usize) -> usize {
    let lv = hs[k].1;
    hs[k + 1..].iter().find(|h| h.1 <= lv).map(|h| h.0).unwrap_or(len)
}

fn text_lines(lines: &[Line]) -> impl Iterator<Item = &str> {
    lines.iter().filter_map(|l| match l {
        Line::Text(t) => Some(t.as_str()),
        Line::Fenced => None,
    })
}

/// The pure extractor over a plan file's text. See the module note for why
/// every rule is lenient; the contract is `plan-contract.md`'s extraction rules.
pub fn extract(text: &str) -> Extracted {
    let lines = prepass(text);
    let hs = headings(&lines);
    let mut x = Extracted::default();

    // title: first H1
    let h1 = hs.iter().find(|h| h.1 == 1);
    x.title = h1.and_then(|h| clamp(h.2, GOAL_MAX));

    // goal: first `## Goal…`'s first paragraph, else the first prose paragraph
    // after the H1 (from the top when there is none).
    let goal_h = hs.iter().position(|h| h.1 == 2 && h.2.to_lowercase().starts_with("goal"));
    x.goal = match goal_h {
        Some(k) => {
            let end = section_end(&hs, k, lines.len());
            let body = &lines[hs[k].0 + 1..end];
            let start = body.iter().position(|l| matches!(l, Line::Text(t) if !t.trim().is_empty()));
            start.and_then(|s| clamp(&paragraph_at(body, s), GOAL_MAX))
        }
        None => None,
    }
    .or_else(|| first_prose(&lines, h1.map(|h| h.0 + 1).unwrap_or(0)).and_then(|p| clamp(&p, GOAL_MAX)));

    // whole-file checkboxes
    for t in text_lines(&lines) {
        if let Some(ticked) = checkbox(t) {
            x.checks_total += 1;
            x.checks_done += ticked as u32;
        }
    }

    // phases
    for (k, h) in hs.iter().enumerate() {
        if !(2..=3).contains(&h.1) || !is_phase_heading(h.2) {
            continue;
        }
        let (name, tag) = split_tag(h.2);
        let end = section_end(&hs, k, lines.len());
        let body = &lines[h.0 + 1..end];
        let (mut done, mut total, mut stated) = (0u32, 0u32, None);
        for t in text_lines(body) {
            if let Some(ticked) = checkbox(t) {
                total += 1;
                done += ticked as u32;
            }
            if stated.is_none() {
                stated = status_line(t);
            }
        }
        let status = stated.or(tag).unwrap_or(match (done, total) {
            (_, 0) => PhaseStatus::Unknown,
            (d, t) if d == t => PhaseStatus::Complete,
            (0, _) => PhaseStatus::Pending,
            _ => PhaseStatus::InProgress,
        });
        x.phases.push(Phase { name: name.to_string(), status, done, total });
    }

    // current: first non-empty line under `## Current Phase`, else the first
    // in-progress phase's name.
    let cur_h = hs.iter().position(|h| h.1 == 2 && h.2.to_lowercase().starts_with("current phase"));
    x.current = cur_h
        .and_then(|k| {
            let end = section_end(&hs, k, lines.len());
            text_lines(&lines[hs[k].0 + 1..end]).find(|t| !t.trim().is_empty()).and_then(|t| clamp(t, CURRENT_MAX))
        })
        .or_else(|| {
            x.phases.iter().find(|p| p.status == PhaseStatus::InProgress).and_then(|p| clamp(&p.name, CURRENT_MAX))
        });

    // errors: data rows of the first table under the FIRST heading that
    // mentions "error". Header row and `---` separators are not data; nor is
    // a row whose first cell is empty (the template ships `|   | 1 |   |`).
    if let Some(k) = hs.iter().position(|h| h.2.to_lowercase().contains("error")) {
        let end = section_end(&hs, k, lines.len());
        let body = &lines[hs[k].0 + 1..end];
        if let Some(s) = body.iter().position(|l| matches!(l, Line::Text(t) if is_table_row(t))) {
            for l in body[s..].iter().skip(1) {
                let Line::Text(t) = l else { break };
                if !is_table_row(t) {
                    break;
                }
                let inner = t.trim().trim_start_matches('|');
                let cells: Vec<&str> = inner.split('|').map(str::trim).collect();
                let separator = cells
                    .iter()
                    .filter(|c| !c.is_empty())
                    .all(|c| c.trim_matches(':').chars().all(|ch| ch == '-') && c.contains('-'));
                if separator || cells.first().is_none_or(|c| c.is_empty()) {
                    continue;
                }
                x.errors += 1;
            }
        }
    }
    x
}

/// The pure extractor over a brief: its H1 and the first prose paragraph after
/// it (from the top when there is no H1).
pub fn extract_brief(text: &str) -> Brief {
    let lines = prepass(text);
    let hs = headings(&lines);
    let h1 = hs.iter().find(|h| h.1 == 1);
    Brief {
        title: h1.and_then(|h| clamp(h.2, GOAL_MAX)),
        lead: first_prose(&lines, h1.map(|h| h.0 + 1).unwrap_or(0)).and_then(|p| clamp(&p, GOAL_MAX)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("wtplan-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }
    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    fn set_mtime(p: &Path, secs: u64) {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        fs::File::open(p).unwrap().set_modified(t).unwrap();
    }
    fn phase(name: &str, status: PhaseStatus, done: u32, total: u32) -> Phase {
        Phase { name: name.into(), status, done, total }
    }

    /// The skill's own template, shape for shape, with two phases filled in and
    /// one real error logged beneath the template's empty row.
    pub(super) const TEMPLATE: &str = "# Task Plan: Ship the Plan tab
<!--
  WHAT: This is your roadmap for the entire task.
-->

## Goal
<!--
  WHAT: One clear sentence describing what you're trying to achieve.
  EXAMPLE: \"Create a Python CLI todo app\"
-->
Show each place's plan in the dock
so a reader knows where it is.

## Current Phase
<!-- WHAT: Which phase you're currently working on -->
Phase 3

## Phases

### Phase 1: Requirements & Discovery
<!-- WHAT: Understand what needs to be done. -->
- [x] Understand user intent
- [x] Identify constraints
- **Status:** complete
<!--
  STATUS VALUES:
  - pending: Not started yet
-->

### Phase 2: Planning & Structure
- [x] Define technical approach
- [x] Document decisions
- **Status:** complete

### Phase 3: Implementation
- [x] Execute the plan
- [ ] Test incrementally
- **Status:** in_progress

### Phase 4: Delivery
- [ ] Deliver to user
- **Status:** pending

## Errors Encountered
<!--
  EXAMPLE:
    | FileNotFoundError | 1 | Check if file exists |
-->
| Error | Attempt | Resolution |
|-------|---------|------------|
|       | 1       |            |
| cargo test ran 0 tests | 1 | grep, not tail |

## Notes
- Update phase status as you progress
";

    /// valleos-shaped: decorated Goal H2, a numbered next-steps list, an errors
    /// table, and NO phase headings or Status lines at all.
    const FREEFORM: &str = "# valleos — ledger import rework

## Goal (delivered)
Import bank statements without the double-post bug, and keep the
reconciliation view fast on 50k rows.

## NOW
Waiting on review of the importer PR.

## Next steps (in order)
1. Land the importer PR
2. Backfill March
3. Drop the old table

## Errors encountered
| What | Fix |
|------|-----|
| duplicate key on re-import | upsert on (account, ref) |
| slow reconcile query | index on posted_at |
| timezone drift | store UTC |
";

    #[test]
    fn a_template_shaped_plan_extracts_phases_counts_current_and_errors() {
        let x = extract(TEMPLATE);
        assert_eq!(x.title.as_deref(), Some("Task Plan: Ship the Plan tab"));
        assert_eq!(
            x.goal.as_deref(),
            Some("Show each place's plan in the dock so a reader knows where it is."),
            "the comment under ## Goal must be stripped, not read as the goal or as its end",
        );
        assert_eq!(x.current.as_deref(), Some("Phase 3"));
        assert_eq!(
            x.phases,
            vec![
                phase("Phase 1: Requirements & Discovery", PhaseStatus::Complete, 2, 2),
                phase("Phase 2: Planning & Structure", PhaseStatus::Complete, 2, 2),
                phase("Phase 3: Implementation", PhaseStatus::InProgress, 1, 2),
                phase("Phase 4: Delivery", PhaseStatus::Pending, 0, 1),
            ]
        );
        assert_eq!((x.checks_done, x.checks_total), (5, 7));
        assert_eq!(x.errors, 1, "the template's empty first-cell row is not an error; the comment's example row is not either");
    }

    #[test]
    fn the_untouched_template_shows_no_errors_and_phase_one_in_progress() {
        let raw = fs::read_to_string(
            std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".claude/skills/planning-with-files/templates/task_plan.md"),
        );
        let Ok(raw) = raw else {
            eprintln!("SKIPPED the_untouched_template…: the skill's template is not installed on this machine");
            return;
        };
        let x = extract(&raw);
        assert_eq!(x.errors, 0);
        assert_eq!(x.phases.len(), 5);
        assert_eq!(x.phases[0].status, PhaseStatus::InProgress);
        assert_eq!(x.current.as_deref(), Some("Phase 1"));
        assert_eq!(x.goal.as_deref(), Some("[One sentence describing the end state]"));
    }

    #[test]
    fn a_freeform_plan_still_yields_its_goal_and_errors() {
        let x = extract(FREEFORM);
        assert_eq!(x.title.as_deref(), Some("valleos — ledger import rework"));
        assert_eq!(
            x.goal.as_deref(),
            Some("Import bank statements without the double-post bug, and keep the reconciliation view fast on 50k rows.")
        );
        assert!(x.phases.is_empty());
        assert_eq!((x.checks_done, x.checks_total), (0, 0));
        assert_eq!(x.errors, 3);
        assert_eq!(x.current, None);
    }

    #[test]
    fn h2_phases_without_a_status_line_derive_status_from_their_checkboxes() {
        let x = extract(
            "# Plan
## Phase A — scaffold
- [x] one
- [X] two
### notes
- [x] nested under A still counts for A
## Phase B — wire
- [x] one
- [ ] two
## Phase C — ship
- [ ] one
## Phase D — think
prose only
",
        );
        assert_eq!(
            x.phases,
            vec![
                phase("Phase A — scaffold", PhaseStatus::Complete, 3, 3),
                phase("Phase B — wire", PhaseStatus::InProgress, 1, 2),
                phase("Phase C — ship", PhaseStatus::Pending, 0, 1),
                phase("Phase D — think", PhaseStatus::Unknown, 0, 0),
            ]
        );
        assert_eq!(x.current.as_deref(), Some("Phase B — wire"), "no ## Current Phase: the first in-progress phase");
        assert_eq!((x.checks_done, x.checks_total), (4, 6));
    }

    #[test]
    fn an_inline_tag_is_honoured_and_stripped_and_a_status_line_beats_it() {
        let x = extract(
            "### Phase 1: Setup [complete]
- [ ] never ticked
### Phase 2: Build [in_progress]
### Phase 3: Ship [pending]
- **Status:** blocked
### Phase 4: Old [failed]
## Phases overview
",
        );
        assert_eq!(
            x.phases,
            vec![
                phase("Phase 1: Setup", PhaseStatus::Complete, 0, 1),
                phase("Phase 2: Build", PhaseStatus::InProgress, 0, 0),
                phase("Phase 3: Ship", PhaseStatus::Blocked, 0, 0),
                phase("Phase 4: Old", PhaseStatus::Blocked, 0, 0),
            ],
            "`## Phases overview` is not a phase (`\\b`)",
        );
    }

    #[test]
    fn an_unrecognised_status_word_is_unknown_not_derived() {
        let x = extract("### Phase 1\n- [x] a\n- **Status:** done\n");
        assert_eq!(x.phases[0].status, PhaseStatus::Unknown);
    }

    #[test]
    fn fenced_code_is_invisible_to_every_rule() {
        let x = extract(
            "# Real title
```md
# Not a title
## Goal
fake goal
### Phase 9: fake
- [x] fake box
- **Status:** complete
```
## Goal
The real goal.
### Phase 1: real
~~~
- [x] fenced box
- **Status:** complete
~~~
- [ ] real box
",
        );
        assert_eq!(x.title.as_deref(), Some("Real title"));
        assert_eq!(x.goal.as_deref(), Some("The real goal."));
        assert_eq!(x.phases, vec![phase("Phase 1: real", PhaseStatus::Pending, 0, 1)]);
        assert_eq!((x.checks_done, x.checks_total), (0, 1));
    }

    #[test]
    fn with_no_goal_heading_the_first_prose_after_the_title_is_the_goal() {
        let x = extract("# T\n\n- a list\n| a | table |\n\nThe lead\nparagraph.\n\nSecond.\n");
        assert_eq!(x.goal.as_deref(), Some("The lead paragraph."));
    }

    /// A line that held only a comment is DROPPED, not left blank — a blank
    /// line would end the paragraph the comment sits inside.
    #[test]
    fn a_comment_only_line_does_not_split_a_paragraph() {
        let x = extract("## Goal\nfirst half\n<!-- aside -->\nsecond half <!-- trailing --> end\n");
        assert_eq!(x.goal.as_deref(), Some("first half second half end"));
    }

    #[test]
    fn the_goal_is_clamped_at_280_on_a_word_boundary() {
        let long = "word ".repeat(100); // 500 chars
        let x = extract(&format!("## Goal\n{long}\n"));
        let g = x.goal.unwrap();
        assert!(g.chars().count() <= GOAL_MAX, "{} chars", g.chars().count());
        assert!(g.ends_with('…'));
        assert!(g.trim_end_matches('…').split(' ').all(|w| w == "word"), "a word was cut: {g:?}");
        // a single unbroken token still clamps
        let x = extract(&format!("## Goal\n{}\n", "x".repeat(400)));
        assert_eq!(x.goal.unwrap().chars().count(), GOAL_MAX);
        // exactly at the limit is untouched
        let exact = "y".repeat(GOAL_MAX);
        assert_eq!(extract(&format!("## Goal\n{exact}\n")).goal.as_deref(), Some(exact.as_str()));
    }

    #[test]
    fn a_brief_gives_its_title_and_lead() {
        let b = extract_brief("# Rework the importer\n\nFix the double-post bug.\nKeep it fast.\n\n## Context\nmore\n");
        assert_eq!(b.title.as_deref(), Some("Rework the importer"));
        assert_eq!(b.lead.as_deref(), Some("Fix the double-post bug. Keep it fast."));
        assert_eq!(extract_brief("# Task\n- do x\n"), Brief { title: Some("Task".into()), lead: None });
    }

    // ── resolution ──────────────────────────────────────────────────────────

    #[test]
    fn active_plan_wins_over_a_newer_directory() {
        let t = tmp("active");
        let r = &t.0;
        write(r, ".planning/old/task_plan.md", "# old");
        write(r, ".planning/new/task_plan.md", "# new");
        write(r, ".planning/old/findings.md", "x");
        write(r, ".planning/.active_plan", "old\n");
        set_mtime(&r.join(".planning/old"), 1_000);
        set_mtime(&r.join(".planning/new"), 2_000);
        let s = summarize(r);
        assert_eq!(s.source, Source::Plan);
        assert_eq!(s.how_resolved, Some(Resolved::ActivePlan));
        assert_eq!(s.plan_rel.as_deref(), Some(".planning/old/task_plan.md"));
        assert_eq!(s.plan_path, Some(r.join(".planning/old/task_plan.md").to_string_lossy().into_owned()));
        assert_eq!(s.title.as_deref(), Some("old"));
        assert_eq!(s.files, PlanFiles { task_plan: true, findings: true, progress: false });
        assert_eq!(s.markdown.as_deref(), Some("# old"));
        assert!(s.mtime_ms > 0);
    }

    #[test]
    fn a_stale_active_plan_falls_through_to_the_newest_plan_directory() {
        let t = tmp("stale");
        let r = &t.0;
        write(r, ".planning/a/task_plan.md", "# a");
        write(r, ".planning/b/task_plan.md", "# b");
        write(r, ".planning/c/notes.md", "no plan here"); // newest, but not a plan dir
        write(r, ".planning/.hidden/task_plan.md", "# hidden"); // newest, but hidden
        write(r, ".planning/.active_plan", "gone");
        set_mtime(&r.join(".planning/a"), 3_000);
        set_mtime(&r.join(".planning/b"), 1_000);
        set_mtime(&r.join(".planning/c"), 9_000);
        set_mtime(&r.join(".planning/.hidden"), 9_000);
        let s = summarize(r);
        assert_eq!(s.how_resolved, Some(Resolved::Newest));
        assert_eq!(s.plan_rel.as_deref(), Some(".planning/a/task_plan.md"));
        // and an .active_plan that tries to leave `.planning` is ignored too
        write(r, ".planning/.active_plan", "../.planning/b");
        assert_eq!(summarize(r).how_resolved, Some(Resolved::Newest));
    }

    #[test]
    fn a_root_plan_is_the_fallback_and_the_brief_rides_along() {
        let t = tmp("root");
        let r = &t.0;
        write(r, "task_plan.md", "# legacy\n## Goal\nold style.\n");
        write(r, "progress.md", "x");
        write(r, ".planning/brief.md", "# The brief\n\nDo the thing.\n");
        let s = summarize(r);
        assert_eq!(s.source, Source::Plan);
        assert_eq!(s.how_resolved, Some(Resolved::Root));
        assert_eq!(s.plan_rel.as_deref(), Some("task_plan.md"));
        assert_eq!(s.goal.as_deref(), Some("old style."));
        assert_eq!(s.files, PlanFiles { task_plan: true, findings: false, progress: true });
        assert_eq!(s.brief_title.as_deref(), Some("The brief"));
        assert_eq!(s.brief_lead.as_deref(), Some("Do the thing."));
    }

    #[test]
    fn a_brief_alone_is_source_brief_and_nothing_is_none() {
        let t = tmp("brief");
        let r = &t.0;
        assert_eq!(summarize(r), PlanSummary::default());
        assert_eq!(serde_json::to_value(summarize(r)).unwrap()["source"], serde_json::json!("none"));
        write(r, ".planning/brief.md", "# Brief title\n\nLead line.\n");
        let s = summarize(r);
        assert_eq!(s.source, Source::Brief);
        assert_eq!(s.how_resolved, None);
        assert_eq!(s.plan_path, None);
        assert_eq!(s.brief_path, Some(r.join(".planning/brief.md").to_string_lossy().into_owned()));
        assert_eq!(s.brief_title.as_deref(), Some("Brief title"));
        assert_eq!(s.brief_lead.as_deref(), Some("Lead line."));
        assert!(s.mtime_ms > 0);
        assert_eq!(s.files, PlanFiles::default());
    }

    /// `symlink_metadata` refuses to follow only the LAST component (docs.rs),
    /// so each constructed component is lstat'd: a linked `.planning`, a linked
    /// plan directory and a linked `task_plan.md` all read as absent, and the
    /// resolution falls through exactly as if they were not there.
    #[test]
    fn a_symlinked_planning_dir_plan_dir_or_plan_file_is_refused() {
        use std::os::unix::fs::symlink;
        let t = tmp("links");
        let r = &t.0;
        write(r, "elsewhere/p/task_plan.md", "# outside");
        write(r, "elsewhere/brief.md", "# outside brief");
        write(r, "elsewhere/task_plan.md", "# outside file");
        write(r, "task_plan.md", "# ours");

        // linked .planning: neither the plan dir nor the brief behind it
        symlink(r.join("elsewhere"), r.join(".planning")).unwrap();
        let s = summarize(r);
        assert_eq!(s.how_resolved, Some(Resolved::Root), "read a plan through a linked .planning");
        assert_eq!(s.title.as_deref(), Some("ours"));
        assert_eq!(s.brief_path, None, "read the brief through a linked .planning");
        fs::remove_file(r.join(".planning")).unwrap();

        // linked plan dir, named by .active_plan AND reachable by newest
        fs::create_dir_all(r.join(".planning")).unwrap();
        symlink(r.join("elsewhere/p"), r.join(".planning/p")).unwrap();
        write(r, ".planning/.active_plan", "p");
        assert_eq!(summarize(r).how_resolved, Some(Resolved::Root), "followed a linked plan directory");

        // linked task_plan.md inside a real plan dir, and a linked root plan
        fs::create_dir_all(r.join(".planning/q")).unwrap();
        symlink(r.join("elsewhere/task_plan.md"), r.join(".planning/q/task_plan.md")).unwrap();
        write(r, ".planning/.active_plan", "q");
        fs::remove_file(r.join("task_plan.md")).unwrap();
        symlink(r.join("elsewhere/task_plan.md"), r.join("task_plan.md")).unwrap();
        // and a linked brief
        symlink(r.join("elsewhere/brief.md"), r.join(".planning/brief.md")).unwrap();
        let s = summarize(r);
        assert_eq!(s.source, Source::None, "followed a linked task_plan.md or brief: {s:?}");
    }

    #[test]
    fn a_plan_past_the_read_cap_is_truncated_and_says_so() {
        let t = tmp("big");
        let r = &t.0;
        let mut body = String::from("# Big\n## Goal\nstill found.\n");
        while body.len() <= MAX_READ + 64 * 1024 {
            body.push_str("- [ ] filler line to make the file large\n");
        }
        write(r, "task_plan.md", &body);
        let s = summarize(r);
        assert!(s.truncated);
        assert!(s.markdown.as_ref().unwrap().len() <= MAX_READ);
        assert_eq!(s.goal.as_deref(), Some("still found."));
        write(r, "task_plan.md", "# small");
        assert!(!summarize(r).truncated);
    }

    #[test]
    fn without_markdown_drops_only_the_text_and_serializes_the_contract_shape() {
        let t = tmp("json");
        let r = &t.0;
        write(r, "task_plan.md", TEMPLATE);
        let s = summarize(r);
        let m = s.without_markdown();
        assert_eq!(m.markdown, None);
        assert_eq!(PlanSummary { markdown: s.markdown.clone(), ..m.clone() }, s);
        let v = serde_json::to_value(&m).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "brief_lead", "brief_path", "brief_title", "checks_done", "checks_total", "current", "errors",
                "files", "goal", "how_resolved", "markdown", "mtime_ms", "phases", "plan_path", "plan_rel",
                "source", "title", "truncated",
            ]
        );
        assert_eq!(v["source"], serde_json::json!("plan"));
        assert_eq!(v["how_resolved"], serde_json::json!("root"));
        assert_eq!(v["phases"][2]["status"], serde_json::json!("in_progress"));
        assert_eq!(v["files"], serde_json::json!({ "task_plan": true, "findings": false, "progress": false }));
        assert!(v["markdown"].is_null());
    }

    /// A MACHINE-LOCAL witness, like `docs/ai-profiles-manual-checks.md`: when
    /// the skill is installed, run its real Stop hook over each fixture and
    /// require the same `(complete/total)` it prints. Absent, it says so loudly
    /// and passes — CI has no skill installed.
    ///
    /// Only the TEMPLATE-shaped fixtures are compared. Our rules are broader on
    /// purpose where the script's greps are not section-aware: it counts only
    /// `### Phase` (so H2 phases are 0 to it), counts matches inside comments
    /// and fences, and has no checkbox derivation. On those fixtures a
    /// disagreement is the design, not a bug.
    #[test]
    fn agrees_with_the_skills_own_check_complete_script() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        let script = home.join(".claude/skills/planning-with-files/scripts/check-complete.sh");
        if !script.is_file() {
            eprintln!("SKIPPED agrees_with_the_skills_own_check_complete_script: {} is not installed", script.display());
            return;
        }
        let tagged = "### Phase 1: a [complete]\n### Phase 2: b [complete]\n### Phase 3: c [in_progress]\n";
        let t = tmp("xcheck");
        for (name, body) in [("template", TEMPLATE), ("tagged", tagged)] {
            let f = t.0.join(format!("{name}.md"));
            fs::write(&f, body).unwrap();
            let out = std::process::Command::new("sh").arg(&script).arg(&f).output().unwrap();
            let so = String::from_utf8_lossy(&out.stdout);
            // "(N/M phases complete)" or, when all are done, "(N/M)".
            let i = so.find('(').unwrap_or_else(|| panic!("{name}: no count in {so:?}"));
            let inner: String = so[i + 1..].chars().take_while(|c| c.is_ascii_digit() || *c == '/').collect();
            let (n, m) = inner.split_once('/').unwrap();
            let x = extract(body);
            assert_eq!(
                (n.parse::<usize>().unwrap(), m.parse::<usize>().unwrap()),
                (x.phases_complete(), x.phases.len()),
                "{name}: check-complete.sh said {so:?}",
            );
        }
        let untouched = home.join(".claude/skills/planning-with-files/templates/task_plan.md");
        if let Ok(body) = fs::read_to_string(&untouched) {
            let out = std::process::Command::new("sh").arg(&script).arg(&untouched).output().unwrap();
            let x = extract(&body);
            let want = format!("({}/{} phases complete)", x.phases_complete(), x.phases.len());
            assert!(String::from_utf8_lossy(&out.stdout).contains(&want), "the skill's own template disagrees");
        }
    }
}

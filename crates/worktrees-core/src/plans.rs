//! The plan reader (`docs/proposals/idea-board-phases.md` §2.2, phase 4):
//! the two sections of a plan document the app reads, `## Projects` and
//! `## Work`. Pure: text in, data out, nothing on disk.
//!
//! **It never blocks the document.** A line it cannot read is a [`Problem`]
//! with the line's number (1-based, in the whole file, frontmatter included,
//! so it matches an editor's gutter) and the words that say what was
//! expected; the rest of the section is still read, and the client shows the
//! document whatever this answers. Everything outside the two sections is
//! left alone: a plan is a document first.
//!
//! ```markdown
//! ## Projects
//! - worktrees            (registered)
//! - homelab-ansible      (clone: git@github.com:david/homelab-ansible.git)
//! - jellyfin-config      (new: ~/workspace/jellyfin-config)
//!
//! ## Work
//! ### Ansible role for Jellyfin
//! project: homelab-ansible
//! branch: jellyfin-role
//! brief: |
//!   Goal, deliverable, rules, when to stop.
//! started: /Users/d/workspace/homelab-ansible · jellyfin-role · 2026-11-12 09:30
//! ```
//!
//! - A project line is `- <name> (<kind>)`; the kind is the LAST
//!   parenthesised group, so a name may hold parentheses of its own.
//! - A work item starts at `### <title>` and holds `project:`, `branch:`,
//!   `brief:` (`|` and an indented block, or one line), optionally `ai:` and
//!   `model:` (the agent to start, else the app's default), and `started:`.
//! - `started:` is the app's own line (phase 5, [`with_started`]): `<project>
//!   · <place> · <when>`, where the project is a registered root (or a name,
//!   as a hand might write it). It is provenance: what is THERE is asked of
//!   the snapshot by (root, slug), never stored. A line that is not those
//!   three parts is named, and its text kept.
//! - Blank lines and one-line HTML comments (`<!-- … -->`) are skipped in
//!   both sections. Any other line is read or named.
//! - Headings are found outside fences; a section ends at the next `#` or
//!   `##` heading.

use serde::Serialize;

/// Where a plan's project comes from (§2.4: what phase 5 does with each).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    /// A project already in this host's registry.
    Registered,
    /// To be cloned from `url` (phase 5).
    Clone { url: String },
    /// To be made at `path` (phase 5).
    New { path: String },
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub line: usize,
    pub name: String,
    #[serde(flatten)]
    pub source: Source,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct WorkItem {
    pub line: usize,
    pub title: String,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub brief: Option<String>,
    /// The harness to start, when the item says one.
    pub ai: Option<String>,
    /// The model to start it with, when the item says one.
    pub model: Option<String>,
    /// The provenance line, as written.
    pub started: Option<String>,
    /// The provenance line, read ([`Started`]).
    pub place: Option<Started>,
    /// Who proposed it, as written: `<chat title> · <chat id> · <when>`, the
    /// line a workspace chat's `plan_propose_work` writes ([`with_item`]).
    /// Absent for an item a person wrote.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed: Option<String>,
    /// The proposer's line, read ([`Proposed`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Proposed>,
    /// Which workspace chat made it, as written: `<chat title> · <chat id> ·
    /// <when>`, the line `plan_add_work` writes for an item that is not a
    /// proposal (`workspace-chat-powers.md` §1.1). Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub made: Option<String>,
    /// The maker's line, read ([`Proposed`]'s shape).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maker: Option<Proposed>,
    /// Lines the edits below need: the `project:` and `started:` lines, and
    /// the item's last line that is not blank.
    #[serde(skip)]
    project_line: Option<usize>,
    #[serde(skip)]
    started_line: Option<usize>,
    #[serde(skip)]
    last_line: usize,
}

/// Which place a work item started (or was linked to): `<project> · <slug> ·
/// <at>`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Started {
    /// The project: its registered root when the app wrote it.
    pub project: String,
    pub slug: String,
    /// When, as written (`2026-11-12 09:30`, local time).
    pub at: String,
}

impl Started {
    /// `<project> · <slug> · <at>`, or `None` for any other shape.
    pub fn parse(text: &str) -> Option<Started> {
        let mut it = text.rsplitn(3, '·').map(str::trim);
        let (at, slug, project) = (it.next()?, it.next()?, it.next()?);
        let ok = !at.is_empty() && !project.is_empty() && !slug.is_empty() && !slug.contains(char::is_whitespace);
        ok.then(|| Started { project: project.into(), slug: slug.into(), at: at.into() })
    }

    /// The value the app writes. Refused when it would not read back as
    /// itself (a root that ends in a `·`, a slug with a space).
    pub fn text(&self) -> Result<String, String> {
        let t = format!("{} · {} · {}", self.project, self.slug, self.at);
        if t.chars().any(char::is_control) || Started::parse(&t).as_ref() != Some(self) {
            return Err(format!("not a place to record: {t:?}"));
        }
        Ok(t)
    }
}

/// Which workspace chat proposed a work item: `<by> · <chat> · <at>`, where
/// `by` is the chat's title then and `chat` its id, which is what names it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Proposed {
    pub by: String,
    pub chat: String,
    pub at: String,
}

impl Proposed {
    /// `<by> · <chat> · <at>`, the chat a ULID, or `None` for any other
    /// shape. The title may hold a `·` of its own: it is everything left of
    /// the last two.
    pub fn parse(text: &str) -> Option<Proposed> {
        let mut it = text.rsplitn(3, '·').map(str::trim);
        let (at, chat, by) = (it.next()?, it.next()?, it.next()?);
        let ok = !at.is_empty() && !by.is_empty() && crate::ulid::is_valid(chat);
        ok.then(|| Proposed { by: by.into(), chat: chat.into(), at: at.into() })
    }

    /// The value written. Refused when it would not read back as itself.
    pub fn text(&self) -> Result<String, String> {
        let t = format!("{} · {} · {}", self.by, self.chat, self.at);
        if t.chars().any(|c| c.is_control() || crate::harness::invisible(c)) || Proposed::parse(&t).as_ref() != Some(self) {
            return Err(format!("not a proposed line to record: {t:?}"));
        }
        Ok(t)
    }
}

/// A work item to add to a plan ([`with_item`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewItem {
    pub title: String,
    pub project: String,
    pub branch: String,
    pub brief: String,
    pub ai: Option<String>,
    pub model: Option<String>,
    pub proposed: Option<Proposed>,
    /// The chat that made it, when it is not a proposal: a `made:` line.
    pub made: Option<Proposed>,
}

/// The longest brief [`with_item`] writes.
pub const BRIEF_MAX: usize = 16 * 1024;

/// One line of a value: no line break, control or invisible character.
fn one_line(v: &str) -> bool {
    !v.chars().any(|c| c.is_control() || crate::harness::invisible(c))
}

/// `text` with `item` added as the last item of `## Work`, after the
/// section's last line that is not blank, or in a `## Work` section made at
/// the end. Refused when a value would not read back as itself (a title
/// twice, a line break in a one-line value, a control character anywhere),
/// which is checked by reading the result back: the new item must be the
/// last, equal to `item`, and every item before it unchanged.
pub fn with_item(text: &str, item: &NewItem) -> Result<String, String> {
    let title = item.title.trim();
    if title.is_empty() || title != item.title || !one_line(title) || title.chars().count() > 200 || title.starts_with('#') {
        return Err(format!("not a work item's title: {:?} (one line, at most 200 characters)", item.title));
    }
    if item.project.trim().is_empty() || item.project.trim() != item.project || !one_line(&item.project) {
        return Err(format!("not a project name: {:?}", item.project));
    }
    let b = &item.branch;
    if b.is_empty() || b.starts_with('-') || b.chars().any(char::is_whitespace) || !one_line(b) {
        return Err(format!("not a branch: {b:?} (no spaces, and it does not start with `-`)"));
    }
    for (k, v) in [("ai", &item.ai), ("model", &item.model)] {
        if let Some(v) = v {
            if v.is_empty() || v.chars().any(char::is_whitespace) || !one_line(v) {
                return Err(format!("not a value for `{k}:`: {v:?}"));
            }
        }
    }
    let brief = item.brief.trim_end();
    if brief.trim().is_empty() || brief.len() > BRIEF_MAX || brief.chars().any(|c| (c.is_control() && c != '\n' && c != '\t') || crate::harness::invisible(c)) {
        return Err(format!("not a brief: it needs words, at most {BRIEF_MAX} bytes, and no control character but line breaks and tabs"));
    }
    let proposed = item.proposed.as_ref().map(Proposed::text).transpose().map_err(|e| format!("proposed: {e}"))?;
    let made = item.made.as_ref().map(Proposed::text).transpose().map_err(|e| format!("made: {e}"))?;
    if proposed.is_some() && made.is_some() {
        return Err("an item is proposed or made, not both".into());
    }
    let before = read(text);
    let w0 = before.work.clone().unwrap_or_default();
    if w0.iter().any(|w| w.title == title) {
        return Err(format!("the plan has an item “{title}” already: give this one another title"));
    }
    let sep = eol(text);
    let mut block = format!("### {title}{sep}project: {}{sep}branch: {b}{sep}", item.project);
    for (k, v) in [("ai", &item.ai), ("model", &item.model)] {
        if let Some(v) = v {
            block.push_str(&format!("{k}: {v}{sep}"));
        }
    }
    if let Some(p) = &proposed {
        block.push_str(&format!("proposed: {p}{sep}"));
    }
    if let Some(m) = &made {
        block.push_str(&format!("made: {m}{sep}"));
    }
    block.push_str(&format!("brief: |{sep}"));
    for l in brief.split('\n') {
        let l = l.trim_end_matches('\r');
        if l.trim().is_empty() {
            block.push_str(sep);
        } else {
            block.push_str(&format!("  {l}{sep}"));
        }
    }
    // The text after the new block: a blank line before what follows it.
    let spaced = |rest: &str| if rest.is_empty() || rest.starts_with(sep) { rest.to_string() } else { format!("{sep}{rest}") };
    let out = match section_span(text, "work") {
        Some((_, body, end)) if text[body..end].trim().is_empty() => format!("{}{block}{}", &text[..body], spaced(&text[body..])),
        Some((_, body, end)) => {
            // Right after the section's last character that is not a line
            // ending: that line's ending, a blank line, the item.
            let at = body + text[body..end].trim_end_matches(['\n', '\r']).len();
            match text[at..].strip_prefix(sep) {
                Some(rest) => format!("{}{sep}{sep}{block}{}", &text[..at], spaced(rest)),
                None if at == text.len() => format!("{text}{sep}{sep}{block}"),
                None => return Err("the plan's Work section ends in a way it could not be added to: add the item by hand".into()),
            }
        }
        None => {
            let kept = text.trim_end();
            if kept.is_empty() { format!("## Work{sep}{sep}{block}") } else { format!("{kept}{sep}{sep}## Work{sep}{sep}{block}") }
        }
    };
    let after = read(&out);
    let w1 = after.work.unwrap_or_default();
    let same = |a: &WorkItem, b: &WorkItem| (&a.title, &a.project, &a.branch, &a.brief, &a.ai, &a.model, &a.started, &a.proposed, &a.made) == (&b.title, &b.project, &b.branch, &b.brief, &b.ai, &b.model, &b.started, &b.proposed, &b.made);
    let new_ok = w1.last().is_some_and(|n| {
        n.title == title && n.project.as_deref() == Some(item.project.as_str()) && n.branch.as_deref() == Some(b.as_str()) && n.brief.as_deref() == Some(brief) && n.ai == item.ai && n.model == item.model && n.proposed == proposed && n.made == made && n.started.is_none()
    });
    if !(new_ok && w1.len() == w0.len() + 1 && w0.iter().zip(&w1).all(|(a, b)| same(a, b)) && after.projects == before.projects) {
        return Err(format!("“{title}” could not be added to the plan without changing the rest of it"));
    }
    Ok(out)
}

/// `2026-11-12 09:30`, the host's local time: what a person reads in the
/// plan. Nothing orders by it. UTC when the local time cannot be had.
pub fn when(epoch: i64) -> String {
    let t = epoch as libc::time_t;
    // SAFETY: `localtime_r` writes only into `tm`, and reads only `t`.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&t, &mut tm).is_null() {
            return format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min);
        }
    }
    let s = epoch.rem_euclid(86_400);
    format!("{} {:02}:{:02}", crate::ideas::date_of(epoch), s / 3600, s / 60 % 60)
}

/// A line the reader could not read, and why.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub line: usize,
    /// `projects` or `work`.
    pub section: &'static str,
    /// The line as written (trimmed at the end).
    pub text: String,
    pub words: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Sections {
    /// `None` when the plan has no `## Projects` section.
    pub projects: Option<Vec<Project>>,
    /// `None` when the plan has no `## Work` section.
    pub work: Option<Vec<WorkItem>>,
    pub problems: Vec<Problem>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sec {
    Other,
    Projects,
    Work,
}

/// One line of the text: its 1-based number, its byte span (without the
/// line ending) and the next line's start.
struct Line<'a> {
    n: usize,
    start: usize,
    next: usize,
    text: &'a str,
}

fn lines(text: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut n = 1;
    while at < text.len() {
        let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        out.push(Line { n, start: at, next: (end + 1).min(text.len()), text: text[at..end].trim_end_matches('\r') });
        at = end + 1;
        n += 1;
    }
    out
}

/// How many lines the frontmatter (with its two fences) takes, 0 without one.
fn front_lines(ls: &[Line]) -> usize {
    if ls.first().map(|l| l.text) != Some("---") {
        return 0;
    }
    ls.iter().skip(1).position(|l| l.text == "---" || l.text == "...").map_or(0, |i| i + 2)
}

/// A level-1 or level-2 ATX heading's text, else `None`.
fn top_heading(line: &str) -> Option<&str> {
    let t = line.strip_prefix("## ").or_else(|| line.strip_prefix("# "))?;
    Some(t.trim().trim_end_matches('#').trim())
}

fn section_of(title: &str) -> Sec {
    match title.to_lowercase().as_str() {
        "projects" => Sec::Projects,
        "work" => Sec::Work,
        _ => Sec::Other,
    }
}

fn fence_run(line: &str) -> Option<(u8, usize)> {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return None;
    }
    [b'`', b'~'].into_iter().find_map(|c| {
        let n = t.bytes().take_while(|b| *b == c).count();
        (n >= 3).then_some((c, n))
    })
}

/// Each line of the body with the section it is in, fences and headings
/// accounted for. Heading lines themselves are `Other` with `heading: true`.
fn walk<'a>(ls: &'a [Line<'a>]) -> Vec<(&'a Line<'a>, Sec, bool)> {
    let mut out = Vec::new();
    let mut sec = Sec::Other;
    let mut fence: Option<(u8, usize)> = None;
    for l in &ls[front_lines(ls)..] {
        if let Some((c, n)) = fence {
            if fence_run(l.text).is_some_and(|(c2, n2)| c2 == c && n2 >= n) && l.text.trim().trim_start_matches(c as char).is_empty() {
                fence = None;
            }
            out.push((l, sec, false));
            continue;
        }
        if let Some(h) = top_heading(l.text) {
            sec = section_of(h);
            out.push((l, Sec::Other, true));
            continue;
        }
        // A fence opens only outside a work item's indented brief, which
        // starts with spaces and is judged by the item reader instead.
        if !l.text.starts_with(' ') && !l.text.starts_with('\t') {
            fence = fence_run(l.text);
        }
        out.push((l, sec, false));
    }
    out
}

fn skippable(t: &str) -> bool {
    let t = t.trim();
    t.is_empty() || (t.starts_with("<!--") && t.ends_with("-->"))
}

const PROJECT_FORM: &str = "write `- <name> (registered)`, `- <name> (clone: <url>)` or `- <name> (new: <path>)`";
const WORK_KEYS: &str = "`project:`, `branch:`, `brief:`, `ai:`, `model:`, `proposed:`, `made:` or `started:`";

fn project_line(l: &Line, problems: &mut Vec<Problem>) -> Option<Project> {
    let bad = |problems: &mut Vec<Problem>, words: String| {
        problems.push(Problem { line: l.n, section: "projects", text: l.text.trim_end().to_string(), words });
    };
    let t = l.text.trim();
    let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) else {
        bad(problems, format!("not a project line: {PROJECT_FORM}"));
        return None;
    };
    let rest = rest.trim();
    let open = match rest.strip_suffix(')').and_then(|r| r.rfind('(')) {
        Some(i) if rest.ends_with(')') => i,
        _ => {
            bad(problems, format!("no kind at the end of the line: {PROJECT_FORM}"));
            return None;
        }
    };
    let name = rest[..open].trim();
    let inner = rest[open + 1..rest.len() - 1].trim();
    if name.is_empty() {
        bad(problems, "a project line needs a name before the parentheses".into());
        return None;
    }
    let after = |k: &str| inner.strip_prefix(k).map(str::trim);
    let source = if inner == "registered" {
        Source::Registered
    } else if let Some(url) = after("clone:") {
        if url.is_empty() {
            bad(problems, "`clone:` needs the repository's URL".into());
            return None;
        }
        Source::Clone { url: url.to_string() }
    } else if let Some(path) = after("new:") {
        if path.is_empty() {
            bad(problems, "`new:` needs the folder to make the project in".into());
            return None;
        }
        Source::New { path: path.to_string() }
    } else {
        bad(problems, format!("unknown kind `({inner})`: {PROJECT_FORM}"));
        return None;
    };
    Some(Project { line: l.n, name: name.to_string(), source })
}

/// `key: value` with a key of letters, digits, `_` and `-`.
fn key_value(t: &str) -> Option<(&str, &str)> {
    let i = t.find(':')?;
    let k = &t[..i];
    (!k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then(|| (k, t[i + 1..].trim()))
}

fn dedent(block: &[&str]) -> String {
    let indent = block
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut v: Vec<&str> = block.iter().map(|l| if l.len() >= indent { &l[indent..] } else { l.trim_start() }).collect();
    while v.last().is_some_and(|l| l.trim().is_empty()) {
        v.pop();
    }
    while v.first().is_some_and(|l| l.trim().is_empty()) {
        v.remove(0);
    }
    v.join("\n")
}

/// Read a plan's `## Projects` and `## Work` sections. See the module doc.
pub fn read(text: &str) -> Sections {
    let ls = lines(text);
    let walked = walk(&ls);
    let mut out = Sections::default();
    let mut problems = Vec::new();
    let mut projects: Vec<Project> = Vec::new();
    let mut work: Vec<WorkItem> = Vec::new();
    let mut item: Option<WorkItem> = None;
    // An open `brief: |` block's lines, and the item it belongs to.
    let mut brief: Option<Vec<&str>> = None;
    let close_item = |item: &mut Option<WorkItem>, brief: &mut Option<Vec<&str>>, work: &mut Vec<WorkItem>, problems: &mut Vec<Problem>, heading: &str| {
        if let Some(mut it) = item.take() {
            if let Some(b) = brief.take() {
                it.brief = Some(dedent(&b));
            }
            for (have, key) in [(it.project.is_some(), "project"), (it.branch.is_some(), "branch")] {
                if !have {
                    problems.push(Problem {
                        line: it.line,
                        section: "work",
                        text: heading.to_string(),
                        words: format!("the item “{}” has no `{key}:` line", it.title),
                    });
                }
            }
            work.push(it);
        }
    };
    let mut item_heading = String::new();
    for (l, sec, heading) in walked {
        if heading {
            close_item(&mut item, &mut brief, &mut work, &mut problems, &item_heading);
            match top_heading(l.text).map(section_of) {
                Some(Sec::Projects) => out.projects = Some(Vec::new()),
                Some(Sec::Work) => out.work = Some(Vec::new()),
                _ => {}
            }
            continue;
        }
        match sec {
            Sec::Other => {}
            Sec::Projects => {
                if skippable(l.text) {
                    continue;
                }
                if let Some(p) = project_line(l, &mut problems) {
                    if let Some(first) = projects.iter().find(|x| x.name == p.name) {
                        problems.push(Problem {
                            line: l.n,
                            section: "projects",
                            text: l.text.trim_end().to_string(),
                            words: format!("“{}” is listed twice (first on line {})", p.name, first.line),
                        });
                    } else {
                        projects.push(p);
                    }
                }
            }
            Sec::Work => {
                if !l.text.trim().is_empty() {
                    if let Some(it) = item.as_mut().filter(|_| !l.text.starts_with("### ")) {
                        it.last_line = l.n;
                    }
                }
                if let Some(b) = brief.as_mut() {
                    if l.text.trim().is_empty() || l.text.starts_with(' ') || l.text.starts_with('\t') {
                        b.push(l.text);
                        continue;
                    }
                    let b = brief.take().unwrap_or_default();
                    if let Some(it) = item.as_mut() {
                        it.brief = Some(dedent(&b));
                    }
                }
                let bad = |problems: &mut Vec<Problem>, words: String| {
                    problems.push(Problem { line: l.n, section: "work", text: l.text.trim_end().to_string(), words });
                };
                if let Some(title) = l.text.strip_prefix("### ") {
                    close_item(&mut item, &mut brief, &mut work, &mut problems, &item_heading);
                    let title = title.trim().trim_end_matches('#').trim();
                    if title.is_empty() {
                        bad(&mut problems, "a work item's heading needs a title".into());
                    }
                    item_heading = l.text.to_string();
                    item = Some(WorkItem { line: l.n, last_line: l.n, title: title.to_string(), ..Default::default() });
                    continue;
                }
                if skippable(l.text) {
                    continue;
                }
                let Some(it) = item.as_mut() else {
                    bad(&mut problems, "text before the first work item: each item starts with `### <title>`".into());
                    continue;
                };
                let t = l.text.trim();
                let Some((key, value)) = key_value(t) else {
                    bad(&mut problems, format!("not a work item line: expected {WORK_KEYS}"));
                    continue;
                };
                let slot = match key {
                    "project" => &mut it.project,
                    "branch" => &mut it.branch,
                    "ai" => &mut it.ai,
                    "model" => &mut it.model,
                    "started" => &mut it.started,
                    "proposed" => &mut it.proposed,
                    "made" => &mut it.made,
                    "brief" => &mut it.brief,
                    _ => {
                        bad(&mut problems, format!("unknown key `{key}:`: expected {WORK_KEYS}"));
                        continue;
                    }
                };
                if slot.is_some() {
                    bad(&mut problems, format!("a second `{key}:` in the item “{}”", it.title));
                    continue;
                }
                if key == "brief" && matches!(value, "|" | "|-" | "|+" | ">") {
                    *slot = Some(String::new());
                    brief = Some(Vec::new());
                    continue;
                }
                if value.is_empty() && key != "started" {
                    bad(&mut problems, format!("`{key}:` needs a value"));
                    continue;
                }
                if key == "branch" && (value.starts_with('-') || value.chars().any(char::is_whitespace)) {
                    bad(&mut problems, "a branch has no spaces and does not start with `-`".into());
                    continue;
                }
                *slot = Some(value.to_string());
                match key {
                    "project" => it.project_line = Some(l.n),
                    "started" => {
                        it.started_line = Some(l.n);
                        it.place = Started::parse(value);
                        if it.place.is_none() && !value.is_empty() {
                            bad(&mut problems, "not a started line: the app writes `started: <project> · <place> · <when>`".into());
                        }
                    }
                    "proposed" => {
                        it.proposal = Proposed::parse(value);
                        if it.proposal.is_none() {
                            bad(&mut problems, "not a proposed line: a workspace chat writes `proposed: <chat> · <chat id> · <when>`".into());
                        }
                    }
                    "made" => {
                        it.maker = Proposed::parse(value);
                        if it.maker.is_none() {
                            bad(&mut problems, "not a made line: a workspace chat writes `made: <chat> · <chat id> · <when>`".into());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    close_item(&mut item, &mut brief, &mut work, &mut problems, &item_heading);
    if out.projects.is_some() {
        out.projects = Some(projects);
    }
    if out.work.is_some() {
        out.work = Some(work);
    }
    problems.sort_by_key(|p| p.line);
    out.problems = problems;
    out
}

/// The byte span of the `## <name>` section: where its heading line starts,
/// where its body starts (after the heading's line ending) and where it ends
/// (the next `#`/`##` heading, or the end of the text). Fences are skipped.
pub fn section_span(text: &str, name: &str) -> Option<(usize, usize, usize)> {
    let ls = lines(text);
    let walked = walk(&ls);
    let mut found: Option<(usize, usize)> = None;
    for (l, _, heading) in walked {
        if !heading {
            continue;
        }
        if let Some((h, b)) = found {
            return Some((h, b, l.start));
        }
        if top_heading(l.text).is_some_and(|t| t.eq_ignore_ascii_case(name)) && l.text.starts_with("## ") {
            found = Some((l.start, l.next));
        }
    }
    found.map(|(h, b)| (h, b, text.len()))
}

/// The item at `line` titled `title` (both, so a line number from a text that
/// has since changed names nothing), or the ONE item so titled when the line
/// has moved.
pub fn find_item<'a>(s: &'a Sections, line: usize, title: &str) -> Result<&'a WorkItem, String> {
    let work = s.work.as_deref().unwrap_or_default();
    if let Some(it) = work.iter().find(|w| w.line == line && w.title == title) {
        return Ok(it);
    }
    let mut named = work.iter().filter(|w| w.title == title);
    match (named.next(), named.next()) {
        (Some(it), None) => Ok(it),
        (Some(_), Some(_)) => Err(format!("the plan has more than one item “{title}”, and none on line {line}: reload to see the latest")),
        _ => Err(format!("the plan has no item “{title}”")),
    }
}

fn eol(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// `text` with the content of line `n` replaced, its ending kept.
fn with_line(text: &str, n: usize, new: &str) -> Option<String> {
    let ls = lines(text);
    let l = ls.get(n.checked_sub(1)?)?;
    Some(format!("{}{new}{}", &text[..l.start], &text[l.start + l.text.len()..]))
}

/// `text` with the item's `started:` line set to `s`: the line it has (an
/// empty one, or one whose place is gone), else a new one after the item's
/// last line. Nothing else in the document moves, which is checked by reading
/// the result back.
pub fn with_started(text: &str, line: usize, title: &str, s: &Started) -> Result<String, String> {
    let before = read(text);
    let it = find_item(&before, line, title)?.clone();
    let new = format!("started: {}", s.text()?);
    let out = match it.started_line {
        Some(n) => with_line(text, n, &new),
        None => lines(text).get(it.last_line.wrapping_sub(1)).map(|l| {
            let sep = eol(text);
            let lead = if text[..l.next].ends_with('\n') { "" } else { sep };
            format!("{}{lead}{new}{sep}{}", &text[..l.next], &text[l.next..])
        }),
    }
    .ok_or("the item's lines could not be found")?;
    let after = read(&out);
    let same = |a: &WorkItem, b: &WorkItem| (&a.title, &a.project, &a.branch, &a.brief, &a.ai, &a.model) == (&b.title, &b.project, &b.branch, &b.brief, &b.ai, &b.model);
    let (w0, w1) = (before.work.unwrap_or_default(), after.work.unwrap_or_default());
    let ok = w0.len() == w1.len()
        && w0.iter().zip(&w1).all(|(a, b)| same(a, b) && if a.line == it.line { b.place.as_ref() == Some(s) } else { a.started == b.started });
    if !ok {
        return Err(format!("“{title}” could not take a `started:` line without changing the rest of the plan: add it by hand: `{new}`"));
    }
    Ok(out)
}

/// `text` with the project line `name` made `- <to> (registered)`, and the
/// `project:` of every work item that named it following when the name
/// changed. A line that says so already is left.
pub fn with_registered(text: &str, name: &str, to: &str) -> Result<String, String> {
    if to.trim() != to || to.is_empty() || to.chars().any(char::is_control) || to.starts_with('(') {
        return Err(format!("not a project name: {to:?}"));
    }
    let s = read(text);
    let projects = s.projects.as_deref().unwrap_or_default();
    let p = projects.iter().find(|p| p.name == name).ok_or_else(|| format!("the plan lists no project “{name}”"))?;
    if p.source == Source::Registered && name == to {
        return Ok(text.to_string());
    }
    if name != to && projects.iter().any(|x| x.name == to) {
        return Err(format!("the plan lists “{to}” already"));
    }
    let ls = lines(text);
    let at = ls[p.line - 1].text;
    let bullet: String = at.chars().take_while(|c| c.is_whitespace() || *c == '-' || *c == '*').collect();
    let mut out = with_line(text, p.line, &format!("{bullet}{to} (registered)")).ok_or("the project's line could not be found")?;
    if name != to {
        for w in s.work.iter().flatten().filter(|w| w.project.as_deref() == Some(name)) {
            if let Some(n) = w.project_line {
                out = with_line(&out, n, &format!("project: {to}")).ok_or("an item's line could not be found")?;
            }
        }
    }
    let back = read(&out);
    if !back.projects.iter().flatten().any(|x| x.line == p.line && x.name == to && x.source == Source::Registered) {
        return Err(format!("“{to}” does not read back as a project's name"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "---\nid: 01JC90A1000000000000000000\nplan_of: [01JC8Z4K000000000000000000]\ncreated: 2026-11-10\n---\n\n\
# Plan: Jellyfin on the home server\n\n## Goal\nOne paragraph.\n\n## Projects\n\
- worktrees            (registered)\n\
- homelab-ansible      (clone: git@github.com:david/homelab-ansible.git)\n\
- jellyfin-config      (new: ~/workspace/jellyfin-config)\n\n\
## Work\n### Ansible role for Jellyfin\nproject: homelab-ansible\nbranch: jellyfin-role\nbrief: |\n  Goal, deliverable, rules.\n\n  When to stop.\n\
started: homelab-ansible · jellyfin-role · 2026-11-12\n\n### Config repo\nproject: jellyfin-config\nbranch: main-config\nbrief: one line\n\n## Notes\n- anything (at all)\n";

    #[test]
    fn the_example_plan_reads_whole() {
        let s = read(PLAN);
        assert_eq!(s.problems, vec![], "{:?}", s.problems);
        let p = s.projects.unwrap();
        assert_eq!(p.iter().map(|x| (x.line, x.name.as_str())).collect::<Vec<_>>(), vec![(13, "worktrees"), (14, "homelab-ansible"), (15, "jellyfin-config")]);
        assert_eq!(p[0].source, Source::Registered);
        assert_eq!(p[1].source, Source::Clone { url: "git@github.com:david/homelab-ansible.git".into() });
        assert_eq!(p[2].source, Source::New { path: "~/workspace/jellyfin-config".into() });
        let w = s.work.unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].line, w[0].title.as_str()), (18, "Ansible role for Jellyfin"));
        assert_eq!(w[0].brief.as_deref(), Some("Goal, deliverable, rules.\n\nWhen to stop."));
        assert_eq!(w[0].started.as_deref(), Some("homelab-ansible · jellyfin-role · 2026-11-12"), "kept, not judged");
        assert_eq!(w[0].place, Some(Started { project: "homelab-ansible".into(), slug: "jellyfin-role".into(), at: "2026-11-12".into() }));
        assert_eq!((w[1].project.as_deref(), w[1].branch.as_deref(), w[1].brief.as_deref()), (Some("jellyfin-config"), Some("main-config"), Some("one line")));
    }

    #[test]
    fn a_typo_in_work_names_its_line_and_the_rest_still_reads() {
        let typo = PLAN.replace("project: homelab-ansible", "projet: homelab-ansible");
        let s = read(&typo);
        let lines: Vec<(usize, &str)> = s.problems.iter().map(|p| (p.line, p.words.as_str())).collect();
        assert_eq!(s.problems[0].line, 18, "the item has no project: {lines:?}");
        assert!(s.problems[0].words.contains("no `project:` line"), "{lines:?}");
        assert_eq!((s.problems[1].line, s.problems[1].text.as_str()), (19, "projet: homelab-ansible"));
        assert!(s.problems[1].words.contains("unknown key `projet:`"), "{lines:?}");
        assert_eq!(s.work.unwrap().len(), 2, "both items still read");
        assert_eq!(s.projects.unwrap().len(), 3);
    }

    #[test]
    fn bad_project_lines_are_named_and_good_ones_kept() {
        let t = "# Plan: x\n\n## Projects\nsome prose\n- a (registerd)\n- b\n- (registered)\n- c (clone: )\n- d (registered)\n- d (registered)\n<!-- a note -->\n- e (thing) (new: /tmp/e)\n";
        let s = read(t);
        let got: Vec<usize> = s.problems.iter().map(|p| p.line).collect();
        assert_eq!(got, vec![4, 5, 6, 7, 8, 10], "{:?}", s.problems);
        assert!(s.problems[1].words.contains("unknown kind `(registerd)`"));
        assert!(s.problems[5].words.contains("listed twice (first on line 9)"));
        let p = s.projects.unwrap();
        assert_eq!(p.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["d", "e (thing)"]);
        assert!(s.work.is_none(), "no Work section is None, not empty");
    }

    #[test]
    fn fences_hide_headings_and_text_before_an_item_is_named() {
        let t = "## Work\nstray words\n### A\nproject: p\nbranch: b\nbrief: |\n  ```\n  ## not a heading\n  ```\n### B\nproject: q\n```\n## Projects\n```\nbranch: c\n";
        let s = read(t);
        assert!(s.projects.is_none(), "the fenced heading is not a section");
        let w = s.work.unwrap();
        assert_eq!(w[0].brief.as_deref(), Some("```\n## not a heading\n```"));
        assert_eq!(s.problems[0].line, 2);
        assert!(s.problems.iter().any(|p| p.line == 12), "a fence in an item is not a key line: {:?}", s.problems);
    }

    /// The `.md`/`.json` pairs under tests/fixtures/plans (their README):
    /// the reader's answer, serialized, is the recorded one. The same JSON
    /// is what app/scripts/plans-check.mjs words.
    #[test]
    fn the_golden_fixtures_read_as_recorded() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plans");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().and_then(|x| x.to_str()) != Some("md") || p.file_name().and_then(|x| x.to_str()) == Some("README.md") {
                continue;
            }
            let got = serde_json::to_value(read(&std::fs::read_to_string(&p).unwrap())).unwrap();
            let want_p = p.with_extension("json");
            if std::env::var_os("WT_PLANS_RECORD").is_some() {
                std::fs::write(&want_p, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
            }
            let want: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&want_p).unwrap()).unwrap();
            assert_eq!(got, want, "{}", p.display());
            n += 1;
        }
        assert!(n >= 1, "no fixture was read");
    }

    fn st(project: &str, slug: &str) -> Started {
        Started { project: project.into(), slug: slug.into(), at: "2025-11-23 12:14".into() }
    }

    /// The round trip: written by `with_started`, read back by `read`, every
    /// other byte the document had still there and in place.
    #[test]
    fn a_started_line_is_written_and_read_back() {
        let s = st("/Users/d/a · b/proj", "jelly.fin-role");
        let w = when(1_763_900_040);
        assert!(w.len() == 16 && w.starts_with("2025-11-2") && w.ends_with(":14"), "{w}");
        assert!(st("p", "s").text().is_ok());
        for (text, title, line) in [
            ("## Work\n### A\nproject: p\nbranch: b\nbrief: |\n  Goal.\n\n  Stop.\n\n\n### B\nproject: q\nbranch: c\n\n## Notes\nx\n", "A", 2),
            ("## Work\n### A\nproject: p\nbranch: b\nbrief: |\n  Goal.\n\n  Stop.\n\n\n### B\nproject: q\nbranch: c\n\n## Notes\nx\n", "B", 11),
            ("## Work\r\n### A\r\nproject: p\r\nbranch: b\r\n\r\n## Notes\r\n", "A", 2),
            ("## Work\n### A\nproject: p\nbranch: b\nbrief: one line", "A", 2),
            ("## Work\n### A\nproject: p\nstarted:\nbranch: b\n", "A", 2),
            ("## Work\n### A\nproject: p\nbranch: b\nstarted: old · gone · 2026-01-01\n### B\nproject: p\nbranch: c\n", "A", 2),
        ] {
            let out = with_started(text, line, title, &s).unwrap();
            let w = read(&out).work.unwrap();
            let it = w.iter().find(|w| w.title == title).unwrap();
            assert_eq!(it.place.as_ref(), Some(&s), "{out:?}");
            let line_out = out.lines().find(|l| l.starts_with("started:") && l.contains("jelly")).unwrap();
            assert_eq!(line_out, "started: /Users/d/a · b/proj · jelly.fin-role · 2025-11-23 12:14");
            let keep = |t: &str| t.lines().filter(|l| !l.starts_with("started:")).map(str::to_string).collect::<Vec<_>>();
            assert_eq!(keep(&out), keep(text), "nothing else moved");
            assert_eq!(out.contains("\r\n"), text.contains("\r\n"));
            assert!(!out.contains("\r\n") || !out.replace("\r\n", "").contains('\n'), "one line ending");
            assert_eq!(read(&out).problems.len(), read(text).problems.len());
        }
        let t = "## Work\n### A\nproject: p\nbranch: b\n### A\nproject: p\nbranch: c\n";
        assert_eq!(read(&with_started(t, 5, "A", &s).unwrap()).work.unwrap()[1].place.as_ref(), Some(&s), "the line picks between two of one title");
        assert!(with_started(t, 9, "A", &s).unwrap_err().contains("more than one item"));
        assert!(with_started(t, 2, "Z", &s).unwrap_err().contains("no item “Z”"));
        assert!(with_started(t, 2, "A", &st("/p", "a b")).unwrap_err().contains("not a place"));
        assert!(with_started(t, 2, "A", &st("/p\nproject: x", "a")).is_err(), "never a second line");
        // Prose the reader names stays where it is, above the new line.
        let odd = "## Work\n### A\nproject: p\nbranch: b\n```\nnot a key\n```\n";
        let out = with_started(odd, 2, "A", &s).unwrap();
        assert!(out.ends_with("```\nstarted: /Users/d/a · b/proj · jelly.fin-role · 2025-11-23 12:14\n"), "{out}");
    }

    #[test]
    fn a_started_line_that_is_not_three_parts_is_named_and_kept() {
        let t = "## Work\n### A\nproject: p\nbranch: b\nstarted: yesterday\nai: codex\nmodel: gpt-5\n";
        let s = read(t);
        assert_eq!(s.problems.iter().map(|p| p.line).collect::<Vec<_>>(), vec![5], "{:?}", s.problems);
        let w = &s.work.unwrap()[0];
        assert_eq!((w.started.as_deref(), &w.place), (Some("yesterday"), &None));
        assert_eq!((w.ai.as_deref(), w.model.as_deref()), (Some("codex"), Some("gpt-5")));
    }

    #[test]
    fn a_project_line_becomes_registered_and_its_items_follow_a_new_name() {
        let out = with_registered(PLAN, "homelab-ansible", "homelab-ansible").unwrap();
        assert_eq!(out, PLAN.replace("- homelab-ansible      (clone: git@github.com:david/homelab-ansible.git)", "- homelab-ansible (registered)"));
        assert_eq!(with_registered(&out, "homelab-ansible", "homelab-ansible").unwrap(), out, "already so: nothing written");
        let out = with_registered(PLAN, "jellyfin-config", "/w/jelly (x)").unwrap();
        let s = read(&out);
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert_eq!(s.projects.unwrap()[2].name, "/w/jelly (x)");
        let w = s.work.unwrap();
        assert_eq!((w[0].project.as_deref(), w[1].project.as_deref()), (Some("homelab-ansible"), Some("/w/jelly (x)")));
        assert!(with_registered(PLAN, "nope", "x").unwrap_err().contains("no project “nope”"));
        assert!(with_registered(PLAN, "jellyfin-config", "worktrees").unwrap_err().contains("already"));
        assert!(with_registered(PLAN, "jellyfin-config", "a\nb").is_err());
    }

    fn prop() -> Proposed {
        Proposed { by: "Release · order".into(), chat: "01JC90A1000000000000000000".into(), at: "2026-11-25 10:00".into() }
    }

    fn new_item(title: &str) -> NewItem {
        NewItem { title: title.into(), project: "worktrees".into(), branch: "feat/x".into(), brief: "Goal.\n\n  ## not a heading\n```\nfenced\n```\nStop.".into(), ai: Some("claude".into()), model: None, proposed: Some(prop()), made: None }
    }

    /// A proposed item lands as the last of `## Work`, reads back as itself
    /// with who proposed it, and every other byte stays where it was.
    #[test]
    fn a_proposed_item_is_added_at_the_end_of_work_and_reads_back() {
        let out = with_item(PLAN, &new_item("Proposed thing")).unwrap();
        let s = read(&out);
        assert_eq!(s.problems, vec![], "{out}");
        let w = s.work.unwrap();
        assert_eq!(w.len(), 3);
        let it = &w[2];
        assert_eq!((it.title.as_str(), it.project.as_deref(), it.branch.as_deref(), it.ai.as_deref()), ("Proposed thing", Some("worktrees"), Some("feat/x"), Some("claude")));
        assert_eq!(it.brief.as_deref(), Some("Goal.\n\n  ## not a heading\n```\nfenced\n```\nStop."));
        assert_eq!(it.proposal.as_ref(), Some(&prop()));
        assert_eq!(it.proposed.as_deref(), Some("Release · order · 01JC90A1000000000000000000 · 2026-11-25 10:00"));
        assert!(it.place.is_none(), "proposed is not started");
        assert!(out.starts_with(&PLAN[..PLAN.find("## Notes").unwrap()].trim_end()), "everything above stays");
        assert!(out.ends_with("## Notes\n- anything (at all)\n"), "the next section stays below: {out}");
        assert_eq!(read(&out).projects, read(PLAN).projects);
        // A plan with no Work section gets one, at the end.
        let bare = "---\nid: x\n---\n# Plan: y\n\nProse.\n";
        let out = with_item(bare, &new_item("A")).unwrap();
        assert!(out.starts_with(bare), "{out}");
        assert_eq!(read(&out).work.unwrap()[0].title, "A");
        // CRLF stays CRLF.
        let crlf = "## Work\r\n### A\r\nproject: p\r\nbranch: b\r\n";
        let out = with_item(crlf, &new_item("B")).unwrap();
        assert!(!out.replace("\r\n", "").contains('\n'), "{out:?}");
        assert_eq!(read(&out).work.unwrap().len(), 2);
    }

    #[test]
    fn a_new_item_that_would_not_read_back_as_itself_is_refused() {
        let bad = |f: &dyn Fn(&mut NewItem)| {
            let mut it = new_item("T");
            f(&mut it);
            with_item(PLAN, &it).unwrap_err()
        };
        assert!(bad(&|i| i.title = "Config repo".into()).contains("already"), "a title twice");
        assert!(bad(&|i| i.title = "two\nlines".into()).contains("title"));
        assert!(bad(&|i| i.title = " ".into()).contains("title"));
        assert!(bad(&|i| i.branch = "-x".into()).contains("branch"));
        assert!(bad(&|i| i.branch = "a b".into()).contains("branch"));
        assert!(bad(&|i| i.project = "a\nbranch: z".into()).contains("project"));
        assert!(bad(&|i| i.brief = "bell\u{7}".into()).contains("brief"));
        assert!(bad(&|i| i.brief = "  ".into()).contains("brief"));
        assert!(bad(&|i| i.ai = Some("a b".into())).contains("ai"));
        assert!(bad(&|i| i.proposed = Some(Proposed { chat: "x y".into(), ..prop() })).contains("proposed"));
    }

    #[test]
    fn a_proposed_line_is_read_or_named() {
        let t = "## Work\n### A\nproject: p\nbranch: b\nproposed: Chat 2 · 01JC90A1000000000000000000 · 2026-11-25 10:00\n### B\nproject: p\nbranch: c\nproposed: someone\n";
        let s = read(t);
        assert_eq!(s.problems.iter().map(|p| p.line).collect::<Vec<_>>(), vec![9], "{:?}", s.problems);
        let w = s.work.unwrap();
        assert_eq!(w[0].proposal.as_ref().map(|p| p.by.as_str()), Some("Chat 2"));
        assert_eq!((w[1].proposed.as_deref(), &w[1].proposal), (Some("someone"), &None));
    }

    #[test]
    fn a_section_span_ends_at_the_next_heading() {
        let (h, b, e) = section_span(PLAN, "projects").unwrap();
        assert_eq!(&PLAN[h..b], "## Projects\n");
        assert!(PLAN[b..e].ends_with("(new: ~/workspace/jellyfin-config)\n\n"));
        assert!(PLAN[e..].starts_with("## Work\n"));
        let (_, _, e) = section_span(PLAN, "work").unwrap();
        assert!(PLAN[e..].starts_with("## Notes"));
        assert!(section_span("# x\n", "projects").is_none());
    }

    /// A chat's item that is not a proposal carries `made:`, read back as
    /// its maker; an item cannot be both, and a bad `made:` is a problem.
    #[test]
    fn a_made_item_round_trips_and_is_never_also_proposed() {
        let item = NewItem { proposed: None, made: Some(prop()), ..new_item("Made") };
        let out = with_item(PLAN, &item).unwrap();
        let it = read(&out).work.unwrap().into_iter().find(|w| w.title == "Made").unwrap();
        assert_eq!((it.maker.as_ref(), it.proposal.as_ref()), (Some(&prop()), None));
        assert!(out.contains(&format!("made: {}", prop().text().unwrap())), "{out}");
        let both = NewItem { made: Some(prop()), ..new_item("Both") };
        assert!(with_item(PLAN, &both).unwrap_err().contains("not both"));
        let bad = out.replace(&format!("made: {}", prop().text().unwrap()), "made: nobody");
        assert!(read(&bad).problems.iter().any(|p| p.words.contains("not a made line")), "{:?}", read(&bad).problems);
    }
}

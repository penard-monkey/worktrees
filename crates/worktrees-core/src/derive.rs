//! The derived document — what a viewer renders, not what the repo wrote.
//!
//! `docs.rs` decides WHICH documents a place has. This decides what each one
//! looks like once a viewer gets hold of it, and it is a pure transform over a
//! string: content in, markdown out, no filesystem. That is not tidiness. The
//! viewer is unsettled (proposal §5.2, §13.4 — the candidate failed its own
//! security gate and the patched build is not landed), so the one thing this
//! module must survive is the viewer changing underneath it. Everything here
//! holds for any renderer; the one part that does not — what a drill-down URL
//! looks like — is a function the caller passes in (`Links::url`).
//!
//! **The tool never edits the repo.** A viewer that writes into the tree it is
//! viewing is not a viewer, so none of this is written back: it is generated at
//! launch into a derived tree (§5.2's answer, "A′"), which is also why a URL
//! that bakes in a port can be in it at all.
//!
//! Three transforms, in the order they apply:
//!
//! 1. **Strip the YAML frontmatter.** Every viewer renders it as noise — `mo`
//!    makes an expanded `<details>` "Metadata" block on every page, a static
//!    renderer shows raw `---`. The block is found by `docs::split_frontmatter`,
//!    the same call `docs::title_from` uses, because a second reader of that
//!    block would eventually disagree with the first about where a document
//!    starts. If the block carried the document's only title, the title comes
//!    back as an H1 — `DocEntry::title` already holds it, extracted once by the
//!    walk, and re-parsing it here would be that same second reader.
//!
//! 2. **Inject the staleness header**, first thing, on every page. §1.1's whole
//!    hazard is a document that does not say which place it is from: seven of
//!    valleos's eleven places carry the pre-restructure `docs/` tree and four
//!    carry the new one, and an unlabelled 53-commits-old ADR reads as current.
//!    A browser tab is FURTHER from the place than the dock is (§7.4), so it
//!    needs the header more, not less. The facts are `DocsPane.tsx`'s — and so
//!    is the ref `behind` is measured against: the project's BASE ref, never
//!    `Place::upstream`. §11.4 has the evidence and the draft had it wrong.
//!
//! 3. **Strip every author-supplied mermaid `click` directive**, then emit our
//!    own. §5.3 rule 1, and it is security rather than tidiness: strict mode
//!    blocks `click … call fn()` but renders `click NODE "<href>"` as a real
//!    anchor, and that href was written by a repo this tool will happily open
//!    five seconds after cloning it. Only tool-emitted targets survive, and by
//!    §5.3 rule 2 a target that is not loopback is not a target.
//!
//! And one thing that is not a transform at all: **which local files a page
//! needs beside it** (`image_refs`, `asset_rel`). A document referencing
//! `images/x.png` renders as a broken image in the browser until that file is
//! copied into the derived tree, which is `viewer::copy_assets`' job — but
//! *which* file, and whether a reference may become a filesystem read at all,
//! is a decision about a string out of a freshly cloned repo, so it is made
//! here, next to the rest of the string handling, and tested without a disk.
//! The document itself is not touched: the reference stays exactly as the
//! author wrote it and the copy is what makes it resolve.
//!
//! **What is deliberately NOT here:** which port, which group, which path hash
//! — `mo`'s working link form is `http://localhost:<port>/<group>?file=<sha256
//! of the absolute path>[:8]` and a static renderer would use a relative link.
//! That is the only viewer-shaped decision in the feature, so it is one
//! injected function and one call site, and swapping viewers does not reopen
//! this file.

use std::collections::{BTreeMap, BTreeSet};

use crate::docs::{self, DocEntry};

/// The facts the staleness header states, as the app already holds them.
///
/// Field-for-field a subset of `Place` (`model.rs`) plus the two things `Place`
/// cannot carry: `base`, which `list_docs` gets from `Project::base_ref()`, and
/// `now`, because a pure transform may not read the clock — hand it the same
/// `Date.now()` the dock's `ago()` uses and the output is reproducible.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Staleness {
    /// The place's slug. The dock does not show it (the nav does, two inches
    /// away); a browser tab has no nav, and §7.4 is the reason this line exists
    /// at all.
    pub place: String,
    /// `Place::branch`. `None`/empty is a detached HEAD and says so — a blank
    /// where the identity goes is worse than naming the state.
    pub branch: Option<String>,
    /// `Place::behind`, measured against `base`. `None` = not computed, which
    /// is NOT the same as zero and does not render.
    pub behind: Option<i64>,
    /// The ref `behind` counts against — `origin/main`, not `origin/<branch>`.
    /// `""` when the project could not be discovered, in which case the header
    /// says "25 behind" and names nothing, which is honest rather than wrong.
    pub base: String,
    pub dirty: Option<bool>,
    pub dirty_files: Option<u32>,
    pub last_commit_subject: Option<String>,
    pub last_commit_epoch: Option<i64>,
    /// Unix seconds "now", passed in rather than read — and specifically **when
    /// the git facts above were captured**, which is when the user last pressed
    /// the button. `age` is measured against it so the commit's age belongs to
    /// the same reading as the rest of the line.
    pub now_epoch: i64,
    /// Unix seconds when THIS COPY was written, which is not the same instant.
    ///
    /// The app re-derives a registered place on its own tick whenever the
    /// documents move, so the body of the page can be seconds old while the
    /// numbers above it are from the last open — and every git fact here costs a
    /// fan-out to recompute, which §15.3 refused for a click and is worse on a
    /// timer. Two ages, so the header says two ages: the reader is told when the
    /// text was copied and, separately, when the status was measured.
    ///
    /// A copy that cannot say how old it is, in a feature whose whole purpose is
    /// to stop people reading stale documents, is §1.1's failure one level in.
    ///
    /// `0` means unknown and prints nothing — the same convention `age` uses for
    /// a missing commit epoch, and the reason `Default` is safe here.
    pub derived_epoch: i64,
}

/// The drill-down seam: the index to resolve a diagram node against, and the
/// function that turns a resolved page into a URL.
///
/// `url` is the whole viewer-specific surface of this module. It returns `None`
/// for a page the viewer is not serving, and whatever it returns is still put
/// through `is_loopback_target` before anything is emitted — the rule is
/// enforced here, on the way out, not trusted to the caller that built the
/// string.
pub struct Links<'a> {
    /// The place's index, as `docs::index_with` produced it.
    pub entries: &'a [DocEntry],
    /// One page → its URL in the viewer, or `None`.
    pub url: &'a dyn Fn(&DocEntry) -> Option<String>,
}

impl Links<'_> {
    /// No viewer: resolve nothing, emit nothing. Author-supplied `click`
    /// directives are still stripped — rule 1 does not depend on rule 2 having
    /// anything to say.
    pub const NONE: Links<'static> = Links { entries: &[], url: &|_| None };
}

/// The diagram types whose grammar has a `click` statement. Emitting one into
/// anything else is a parse error and a diagram that renders as an error box —
/// a document the tool BROKE, which is worse than one it failed to enhance.
/// Flowcharts are where the nodes are anyway: the consumer's six merged
/// diagrams carry ~95 of them (§5.3).
const CLICKABLE: [&str; 2] = ["graph", "flowchart"];

/// Statement heads that are not node declarations. Without this, `class A big`
/// links `big` and `style X fill:#f9f` links `fill`.
const KEYWORDS: [&str; 12] = [
    "graph",
    "flowchart",
    "subgraph",
    "end",
    "direction",
    "style",
    "classdef",
    "class",
    "linkstyle",
    "click",
    "acctitle",
    "accdescr",
];

/// The characters a mermaid link operator is spelled from (`-->`, `==>`,
/// `-.->`, `<-->`, `~~~`, and `&` for a multi-node edge). Two readers: `is_click`
/// uses it to tell `click --> B` — an edge whose node is called `click` — from
/// `click B href "…"`, the directive; `arrows_off` uses a RUN of them to find
/// where one node id ends and the next begins.
const LINK_CHARS: [char; 7] = ['-', '=', '.', '<', '>', '~', '&'];

/// One document, transformed: the staleness header, then the body.
///
/// `entry` is the row `docs::index_with` produced for it, `text` is the file's
/// content, and nothing else is read.
///
/// **The header is prose here and DATA in the server** (`doc`'s `meta`), which
/// is why the split below exists rather than being tidiness: a surface that
/// renders the facts itself must not also receive them as a blockquote, or the
/// reader is told twice and the second copy cannot update. `body` is that
/// surface's half; this composition is the one that writes a self-contained
/// markdown file, and its output is unchanged.
pub fn document(entry: &DocEntry, text: &str, stale: &Staleness, links: &Links) -> String {
    let mut out = header(stale);
    out.push_str(&body(entry, text, links));
    out
}

/// `document` without the staleness header — frontmatter stripped, the title
/// restored if stripping took it, diagram `click` directives rewritten.
///
/// This is what the browser page is served as blocks, because the page renders
/// the same facts from `doc`'s `meta` and renders them LIVE: the header baked
/// into the text would be a second, frozen copy of numbers the reader is
/// looking at two inches away. §1.1's hazard with the axes swapped.
pub fn body(entry: &DocEntry, text: &str, links: &Links) -> String {
    let (had_front, body) = match docs::split_frontmatter(text) {
        Some((_, rest)) => (true, rest),
        None => (false, text),
    };
    let mut out = String::new();
    // Only when stripping took the title away. A document with no heading and
    // no frontmatter never had one, and inventing an H1 for it (the walk's
    // fallback is the filename stem) is the tool writing content.
    let title = oneline(&entry.title);
    if had_front && !title.is_empty() && docs::title_from(body).is_none() {
        out.push_str(&format!("# {title}\n\n"));
    }
    out.push_str(&diagrams(body, links));
    out
}

/// The staleness header, as markdown.
///
/// The dock splits these facts over THREE lines because its floor is 240px and
/// the run that ellipsised first was `origin/main` — the one fact in the header
/// that appears nowhere else in the app (`DocsPane.tsx`, the note above
/// `staleness()`). A browser window has no such floor, so the identity facts
/// share a line and the commit keeps its own paragraph. The WORDS are the
/// dock's on purpose, so the two surfaces read the same; what must never drift
/// is smaller and harder than the wording — the fact set, and which ref
/// `behind` is counted against.
pub fn header(s: &Staleness) -> String {
    let mut facts: Vec<String> = Vec::new();
    if !s.place.trim().is_empty() {
        facts.push(format!("**{}**", oneline(&s.place)));
    }
    let branch = oneline(s.branch.as_deref().unwrap_or(""));
    facts.push(if branch.is_empty() { "detached".to_string() } else { format!("`{branch}`") });
    let n = s.dirty_files.unwrap_or(0);
    facts.push(match (s.dirty.unwrap_or(false), n) {
        (false, _) => "clean".to_string(),
        (true, 1) => "1 dirty".to_string(),
        (true, n) => format!("{n} dirty"),
    });
    // `behind: 0` is said out loud. "Up to date" is the state a reader most
    // wants confirmed, and a line that is simply absent reads identically to
    // one that could not be computed — which is what `None` means here.
    if let Some(b) = s.behind {
        let base = oneline(&s.base);
        facts.push(match (b, base.is_empty()) {
            (0, true) => "up to date".to_string(),
            (0, false) => format!("up to date with `{base}`"),
            (n, true) => format!("{n} behind"),
            (n, false) => format!("{n} behind `{base}`"),
        });
    }
    let mut out = format!("> {}\n", facts.join(" · "));

    let subject = oneline(s.last_commit_subject.as_deref().unwrap_or(""));
    let age = age(s.last_commit_epoch, s.now_epoch);
    if !subject.is_empty() || !age.is_empty() {
        let mut last: Vec<String> = Vec::new();
        if !subject.is_empty() {
            last.push(format!("`{subject}`"));
        }
        if !age.is_empty() {
            last.push(age);
        }
        out.push_str(">\n");
        out.push_str(&format!("> {}\n", last.join(" · ")));
    }
    // WHEN THIS COPY WAS MADE. The facts above are about the place; this one is
    // about the page the reader is holding, and it is the only line that can
    // tell them the difference between a document the tool copied a moment ago
    // and one it copied before lunch. A browser tab left open overnight looks
    // exactly like a fresh one.
    //
    // The second stamp appears only when the two instants differ — i.e. after a
    // background re-derive, where the text is current and the git line is not.
    // Printing one timestamp for both would say the status was measured when it
    // was not, which is the whole class of error this header exists to remove.
    if s.derived_epoch > 0 {
        let made = utc_stamp(s.derived_epoch);
        out.push_str(">\n");
        if s.now_epoch > 0 && s.now_epoch != s.derived_epoch {
            out.push_str(&format!("> *derived {made} · status as of {}*\n", utc_stamp(s.now_epoch)));
        } else {
            out.push_str(&format!("> *derived {made}*\n"));
        }
    }
    // A rule under it, so the reader can see where the tool stops talking and
    // the document starts.
    out.push_str("\n---\n\n");
    out
}

/// Unix seconds → `YYYY-MM-DD HH:MM:SS UTC`.
///
/// Pure arithmetic, because this module is a pure transform and `sysclock`
/// shells out to `date` — a spawn per generated page, times the index's 2,000
/// cap, on a path that already runs on a timer. UTC rather than local for the
/// same reason it cannot ask `date`, and it is the app log's own timezone
/// (CLAUDE.md warns about exactly that cross-reference), so the two read
/// together.
///
/// Howard Hinnant's `civil_from_days`, which is exact for every date this can
/// be handed and needs no table. Days are floored, so a pre-1970 epoch — a
/// clock that has not been set yet, say — still produces a real date instead of
/// a negative month.
fn utc_stamp(epoch: i64) -> String {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);
    // Shift the era so that a year starts on 1 March: February, and therefore
    // the leap day, lands at the END of it and the month-length pattern becomes
    // one formula.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Compact age, the dock's `ago()` vocabulary exactly (`DocsPane.tsx:79`).
/// A missing or zero epoch is "" rather than "now" — the dock's `if (!epoch)`.
fn age(epoch: Option<i64>, now: i64) -> String {
    let Some(e) = epoch.filter(|e| *e != 0) else { return String::new() };
    let s = now - e;
    if s < 60 {
        "now".to_string()
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// One line of plain text, safe to put inside a code span in a document we are
/// generating.
///
/// Every string in the header is author-controlled: a commit subject is
/// arbitrary bytes, and `git check-ref-format` lets a BRANCH name carry a
/// backtick (it forbids space, `~`, `^`, `:`, `?`, `*`, `[` and `\`, not that).
/// A backtick ends the code span, and the rest of the subject becomes markdown
/// — including a newline, which ends the blockquote, which puts attacker text
/// in the document's own voice at the top of every page. Collapse the
/// whitespace, drop the controls, and neuter the one character that can open a
/// span.
fn oneline(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in s.trim().chars() {
        if c.is_whitespace() || c.is_control() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(if c == '`' { '\'' } else { c });
    }
    out
}

// ── mermaid ──────────────────────────────────────────────────────────────────

/// Rewrite every mermaid block in `text`: author `click` directives out, ours
/// in. Everything outside a mermaid fence is copied through unchanged — a line
/// of prose beginning "click here" is prose.
///
/// One exception to "unchanged", stated because the claim is otherwise a lie:
/// line endings come out as `\n`. `str::lines` strips a trailing `\r` and there
/// is no way to put it back that does not amount to a second line splitter, so
/// a CRLF document is normalised. It is a derived copy, nothing round-trips
/// through it, and the alternative is two disagreeing notions of a line.
pub fn diagrams(text: &str, links: &Links) -> String {
    let ends_nl = text.ends_with('\n');
    let mut out: Vec<String> = Vec::new();
    // (fence char, run length, indent) while inside a MERMAID fence; other
    // fences are copied through and only tracked well enough not to end early.
    let mut open: Option<(char, usize, String)> = None;
    let mut other: Option<(char, usize)> = None;
    let mut block: Vec<String> = Vec::new();

    for line in text.lines() {
        if let Some((c, n, indent)) = &open {
            if closes(line, *c, *n) {
                out.extend(rewrite(&block, indent, links));
                out.push(line.to_string());
                open = None;
                block.clear();
            } else {
                block.push(line.to_string());
            }
            continue;
        }
        if let Some((c, n)) = other {
            out.push(line.to_string());
            if closes(line, c, n) {
                other = None;
            }
            continue;
        }
        match opens(line) {
            Some((c, n, info)) if is_mermaid(&info) => {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                open = Some((c, n, indent));
                out.push(line.to_string());
            }
            Some((c, n, _)) => {
                other = Some((c, n));
                out.push(line.to_string());
            }
            None => out.push(line.to_string()),
        }
    }
    // A fence that never closes swallows the rest of the file, and it is still
    // a diagram to the renderer — so it gets the same treatment, not a pass.
    if let Some((_, _, indent)) = &open {
        out.extend(rewrite(&block, indent, links));
    }

    let mut s = out.join("\n");
    // `str::lines` cannot tell "a\n" from "a"; the flag can.
    if ends_nl {
        s.push('\n');
    }
    s
}

/// An opening fence: three or more backticks or tildes, plus its info string.
fn opens(line: &str) -> Option<(char, usize, String)> {
    let t = line.trim_start();
    let c = t.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = t.chars().take_while(|x| *x == c).count();
    if n < 3 {
        return None;
    }
    Some((c, n, t[n..].trim().to_string()))
}

/// A closing fence: the same character, at least as long, and nothing after it.
/// The length matters — a ```` ```` block exists precisely to hold ``` inside
/// it, and closing on the inner one would hand the rest of the diagram to the
/// prose.
fn closes(line: &str, c: char, n: usize) -> bool {
    let t = line.trim_start();
    let run = t.chars().take_while(|x| *x == c).count();
    run >= n && t[run..].trim().is_empty()
}

/// Is this fence a mermaid diagram? The info string's first word, allowing the
/// attribute spellings (`{.mermaid}`) some generators emit.
fn is_mermaid(info: &str) -> bool {
    info.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| c == '{' || c == '}' || c == '.' || c == ',')
        .eq_ignore_ascii_case("mermaid")
}

/// One mermaid block, rewritten: author directives removed, ours appended.
fn rewrite(block: &[String], indent: &str, links: &Links) -> Vec<String> {
    let text = block.join("\n");
    // A mermaid diagram may carry its OWN `---` frontmatter (title, config).
    // That is the diagram's, not the document's: it is preserved verbatim and
    // never scanned, because a `click:` key in it is YAML and stripping it
    // would corrupt the config it belongs to.
    let (prefix, body) = match docs::split_frontmatter(&text) {
        Some((_, rest)) => (&text[..text.len() - rest.len()], rest),
        None => ("", text.as_str()),
    };

    let mut out: Vec<String> = prefix.lines().map(str::to_string).collect();
    let kept: Vec<String> = body.lines().filter_map(declick).collect();
    out.extend(kept.iter().cloned());
    if CLICKABLE.contains(&kind(&kept).as_str()) {
        for (id, url) in targets(&kept, links) {
            out.push(format!("{indent}click {id} href \"{url}\""));
        }
    }
    out
}

/// Drop every `click` statement from one line; `None` when nothing is left.
///
/// Statements, not lines, because mermaid's flowchart grammar separates them
/// with `;` as well as with a newline — so `A-->B; click A "http://evil"` hides
/// a directive on a line that does not begin with one. A `%%` comment is left
/// alone: it is not a statement, and rewriting it would edit the author's prose
/// for no gain.
fn declick(line: &str) -> Option<String> {
    if line.trim_start().starts_with("%%") {
        return Some(line.to_string());
    }
    let parts = statements(line);
    let kept: Vec<&str> = parts.iter().copied().filter(|p| !is_click(p)).collect();
    if kept.len() == parts.len() {
        return Some(line.to_string());
    }
    let joined = kept.join(";");
    if joined.trim().is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// Split a line on top-level `;`. A `;` inside a quoted label (`A["a; b"]`) is
/// part of the label, and splitting there would cut a node in half.
fn statements(line: &str) -> Vec<&str> {
    let b = line.as_bytes();
    let (mut out, mut start, mut quoted) = (Vec::new(), 0usize, false);
    // ASCII-only tests, so no index here can land inside a multi-byte char.
    for i in 0..b.len() {
        match b[i] {
            b'"' => quoted = !quoted,
            b';' if !quoted => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out
}

/// Is this statement a `click` directive?
///
/// Case-insensitively, because being wrong in that direction costs a stripped
/// line and being wrong in the other costs an author-controlled href. The one
/// thing that collides is an EDGE whose node is called `click` — `Click --> B`
/// — and a link operator can never be a click directive's second token, which
/// is the whole test. `click-->B` does not even reach it: the first token runs
/// to the `-`.
fn is_click(stmt: &str) -> bool {
    let t = stmt.trim_start();
    let head: String = t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    if !head.eq_ignore_ascii_case("click") {
        return false;
    }
    let rest = t[head.len()..].trim_start();
    match rest.chars().next() {
        None => false,
        Some(c) => !LINK_CHARS.contains(&c),
    }
}

/// The diagram's type: the first statement's first word, lowercased.
fn kind(lines: &[String]) -> String {
    for line in lines {
        for stmt in statements(line) {
            let t = stmt.trim();
            if t.is_empty() || t.starts_with("%%") {
                continue;
            }
            return t.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
        }
    }
    String::new()
}

/// The drill-down targets for one diagram: `(node id, url)`, in the order the
/// nodes appear.
///
/// The mapping is §5.3's first source and only its first source — a node id
/// that names a page in this place's index. The other three (a sidecar block,
/// page frontmatter, derived package/directory names) are not built, and this
/// is the seam they would arrive at.
fn targets(lines: &[String], links: &Links) -> Vec<(String, String)> {
    let mut by_slug: BTreeMap<String, Vec<&DocEntry>> = BTreeMap::new();
    for e in links.entries {
        by_slug.entry(slug(&e.rel)).or_default().push(e);
    }
    let mut out = Vec::new();
    for id in node_ids(lines) {
        // Exactly one page, or none: two `overview.md`s in one place is a
        // question the diagram did not answer, and picking one would be this
        // module guessing which document a reader meant.
        let Some(hits) = by_slug.get(&id.to_ascii_lowercase()) else { continue };
        let [entry] = hits[..] else { continue };
        let Some(url) = (links.url)(entry) else { continue };
        if is_loopback_target(&url) {
            out.push((id, url));
        }
    }
    out
}

/// A page's slug: its filename without the extension, lowercased.
fn slug(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name).to_ascii_lowercase()
}

/// The node ids a flowchart declares, first appearance first.
///
/// This scan can MISS a node; it cannot invent one. Every id it returns is a
/// token that stood in node position in the source, and a target is only
/// emitted when that token also names exactly one page — so the failure mode is
/// a node that did not become a link, never a directive pointing at a node that
/// is not there.
fn node_ids(lines: &[String]) -> Vec<String> {
    let (mut out, mut seen) = (Vec::new(), BTreeSet::new());
    let mut first = true;
    for line in lines {
        for stmt in statements(line) {
            let t = stmt.trim();
            if t.is_empty() || t.starts_with("%%") {
                continue;
            }
            if first {
                // The type declaration (`flowchart TD`). KEYWORDS catches it
                // too; this also catches the spellings KEYWORDS does not know,
                // like `flowchart-elk`.
                first = false;
                continue;
            }
            let head: String = t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
            if KEYWORDS.contains(&head.to_ascii_lowercase().as_str()) {
                continue;
            }
            for tok in arrows_off(&labels_off(t)).split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
                if tok.is_empty() || !tok.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                    continue;
                }
                if KEYWORDS.contains(&tok.to_ascii_lowercase().as_str()) {
                    continue;
                }
                if seen.insert(tok.to_ascii_lowercase()) {
                    out.push(tok.to_string());
                }
            }
        }
    }
    out
}

/// Everything outside a label: `A[Read the docs] --> B` becomes `A --> B`.
/// Label text is prose — it holds spaces, punctuation and whole sentences, and
/// tokenising it would turn every word in a diagram into a candidate node id.
///
/// A removed span leaves a SPACE behind. Without it `A[x]B` closes up to `AB`,
/// and a token that appeared in neither the source nor the diagram is exactly
/// the "invent" half of the promise `node_ids` makes.
fn labels_off(s: &str) -> String {
    let (mut out, mut depth, mut quoted, mut piped) = (String::new(), 0i32, false, false);
    for c in s.chars() {
        if quoted {
            if c == '"' {
                quoted = false;
                out.push(' ');
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '|' => {
                piped = !piped;
                out.push(' ');
            }
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => {
                depth = (depth - 1).max(0);
                if depth == 0 {
                    out.push(' ');
                }
            }
            _ if depth == 0 && !piped => out.push(c),
            _ => {}
        }
    }
    out
}

/// A run of two or more link characters is an ARROW, not part of a node id.
///
/// `-` cannot simply be a separator — mermaid ids carry it (`my-node`) — but
/// `A-->B`, the spelling with no spaces, is the common one, and tokenising on
/// the id charset alone reads it as `A--` and `B`. `A--` matches no page, so
/// the whole diagram silently fails to link while every test still passes:
/// exactly the shape this repo keeps finding (a value that is correct at the
/// moment it is written).
fn arrows_off(s: &str) -> String {
    // A single link character is left exactly as it was: `-` because it is part
    // of the id, and `.` because replacing it with anything from the id charset
    // would GLUE `a.b` into one token instead of two.
    fn flush(out: &mut String, run: &mut String) {
        if run.chars().count() >= 2 {
            out.push(' ');
        } else {
            out.push_str(run);
        }
        run.clear();
    }
    let (mut out, mut run) = (String::new(), String::new());
    for c in s.chars() {
        if LINK_CHARS.contains(&c) {
            run.push(c);
            continue;
        }
        flush(&mut out, &mut run);
        out.push(c);
    }
    flush(&mut out, &mut run);
    out
}

/// §5.3 rule 2: a target that is not loopback is not a target.
///
/// The threat is not another local process — anything running as this user can
/// read `docs/` without our help (§11.3). It is a WEB PAGE: script in any tab
/// the user has open can reach `127.0.0.1` where it cannot reach the
/// filesystem, and these documents carry a client's signed agreement (§4.3).
/// An href we emit is the one string in a generated page that can point
/// anywhere, so it is checked here, on the way out, rather than trusted to the
/// code that built it.
///
/// Four shapes this refuses that a `starts_with("http://127.0.0.1")` would not:
///
/// - `http://127.0.0.1.evil.com/` — a host that merely BEGINS with the literal;
/// - `http://127.0.0.1@evil.com/` — the loopback literal is USERINFO here and
///   the browser connects to `evil.com`. Any `@` in the authority is refused
///   outright rather than parsed past: a URL this tool generates for its own
///   viewer has no credentials in it, so the only thing userinfo can do in a
///   drill-down target is make the host read as something it is not;
/// - `javascript:x("http://127.0.0.1")` — a scheme that never opens a socket;
/// - `http:/\evil.com` and friends — a browser normalises `\` to `/`, so a URL
///   carrying one does not mean what it reads as.
///
/// And the emitted form is `click ID href "<url>"`, so a URL containing a quote
/// or a newline would not be a bad link — it would be a second directive.
///
/// The PORT is deliberately not checked. This module does not know which port
/// the viewer got; the caller that built the URL does, and narrowing loopback
/// to one port is its business, not the transform's.
pub fn is_loopback_target(url: &str) -> bool {
    if url.is_empty() || url.len() > 2048 {
        return false;
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || "\"'\\<>`".contains(c)) {
        return false;
    }
    let Some((scheme, rest)) = url.split_once("://") else { return false };
    if !scheme.eq_ignore_ascii_case("http") {
        return false;
    }
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host_port.contains('@') {
        return false;
    }
    let (host, port) = if let Some(rest) = host_port.strip_prefix('[') {
        // IPv6 literal: the colons inside the brackets are not the port's — and
        // what follows the `]` is a port or NOTHING. `http://[::1].evil.example/`
        // is the bracketed spelling of the same trick the dotted suffix plays,
        // and treating the tail as decoration accepts it. (Which this did, until
        // its own test said so.)
        match rest.split_once(']') {
            Some((h, "")) => (h.to_string(), String::new()),
            Some((h, tail)) => match tail.strip_prefix(':') {
                Some(p) => (h.to_string(), p.to_string()),
                None => return false,
            },
            None => return false,
        }
    } else {
        match host_port.split_once(':') {
            Some((h, p)) => (h.to_string(), p.to_string()),
            None => (host_port.to_string(), String::new()),
        }
    };
    if !port.is_empty() && (!port.chars().all(|c| c.is_ascii_digit()) || port.parse::<u32>().map(|p| p == 0 || p > 65535).unwrap_or(true)) {
        return false;
    }
    is_loopback_host(&host)
}

/// `localhost`, any `127.0.0.0/8` literal, or `::1`. The same three the `mo`
/// patch serves (§14) — a name the browser can be pointed at by the user's own
/// hosts file is still the user's decision, and refusing `localhost` would
/// reject the form that project's own README documents.
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if host == "::1" || host == "0:0:0:0:0:0:0:1" {
        return true;
    }
    let mut parts = host.split('.');
    let Some(first) = parts.next() else { return false };
    if first != "127" {
        return false;
    }
    let rest: Vec<&str> = parts.collect();
    rest.len() == 3 && rest.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u32>().map(|n| n <= 255).unwrap_or(false))
}

// ── assets ──────────────────────────────────────────────────────────────────

/// Longest reference we will look at. A `src` past this is not a path anybody
/// typed, and every step after this one is arithmetic on it.
const REF_MAX: usize = 1024;

/// Every image reference in `text`, as the document wrote it — a REPORT, not a
/// transform. Nothing here edits the document, and that is the point: the
/// derived copy keeps the author's `images/x.png` byte for byte, and the viewer
/// resolves it because the tool copied that file to the same relative place
/// beside it (`viewer::copy_assets`). Rewriting the reference instead would
/// make the derived text disagree with the source it claims to be a copy of, on
/// the one surface whose whole job is to say how faithful it is.
///
/// What it finds, in the order a document is read:
///
/// - inline images, `![alt](dest)`, including `![alt](dest "title")` and the
///   angle form `![alt](<dest with spaces>)`;
/// - link reference definitions, `[label]: dest "title"`, which is how
///   `![alt][label]` gets its path. The definition is taken without asking
///   whether a `!`-usage exists: destinations are filtered by extension
///   downstream, so the worst case is copying a raster that a plain link points
///   at, and joining usages to definitions here would be a second markdown
///   parser living next to the one the viewer already has.
/// - **`<img src="…">` in raw HTML**, which is not an exotic case: it is how a
///   README centres or sizes a screenshot, and this repo's own does it three
///   times. §4.3 lets a viewer either show raw HTML as inert text or sanitise it
///   through an allow-list, and `mo` does the second — `rehype-raw` +
///   `rehype-sanitize`, whose default schema keeps `img` — so the tag renders
///   and its `src` needs the same file beside it as any other image. Only a
///   quoted `src` is read; an unquoted one is left alone rather than guessed at,
///   and HTML entities are not decoded, for the reason `asset_rel` gives about
///   percent escapes.
///
/// **Fences are skipped.** A `![x](y.png)` inside a ```` ``` ```` block is an
/// EXAMPLE of a reference, not one — nothing renders it, so copying the file it
/// names would spend the caps in `viewer` on an image no reader can see.
///
/// Not found, and said here rather than discovered later: a reference-style
/// usage whose definition lives in another file, an unquoted or entity-escaped
/// HTML `src`, `srcset` (several candidates and their descriptors, which is a
/// parser of its own), and CSS `url()`. An image this misses stays broken
/// exactly as it is today; it cannot become a copy of the wrong file.
pub fn image_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        if let Some((c, n)) = fence {
            if closes(line, c, n) {
                fence = None;
            }
            continue;
        }
        if let Some((c, n, _)) = opens(line) {
            fence = Some((c, n));
            continue;
        }
        inline_images(line, &mut out);
        html_images(line, &mut out);
        if let Some(d) = link_definition(line) {
            out.push(d);
        }
    }
    out
}

/// Every `![alt](dest …)` on one line, appended to `out`.
fn inline_images(line: &str, out: &mut Vec<String>) {
    let b = line.as_bytes();
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] != b'!' || b[i + 1] != b'[' {
            i += 1;
            continue;
        }
        // The alt text may carry brackets of its own (`![a [b] c](x.png)`), so
        // the closing one is found by depth rather than by the first `]`.
        let mut depth = 0i32;
        let mut j = i + 1;
        let close = loop {
            if j >= b.len() {
                break None;
            }
            match b[j] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        break Some(j);
                    }
                }
                _ => {}
            }
            j += 1;
        };
        let Some(close) = close else { return };
        if b.get(close + 1) != Some(&b'(') {
            i = close + 1;
            continue;
        }
        match destination(&line[close + 2..]) {
            Some((dest, used)) => {
                if !dest.is_empty() {
                    out.push(dest.to_string());
                }
                i = close + 2 + used;
            }
            None => i = close + 2,
        }
    }
}

/// Every quoted `src` of an `<img>` tag on one line, appended to `out`.
///
/// Deliberately not an HTML parser: it finds `<img`, then the first quoted
/// `src=` before that tag closes, and takes what is between the quotes. A tag
/// spread over several lines, an unquoted value and a `src` that is really
/// `data-src` all come out as nothing, which is the direction to be wrong in —
/// a missed image is the broken image we already have, while a mis-parsed one
/// is a path we then open.
fn html_images(line: &str, out: &mut Vec<String>) {
    let lower = line.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(at) = lower[from..].find("<img").map(|i| from + i) {
        // `<imgx` is not an `<img`; the tag name ends at whitespace or `/>`.
        let after = at + 4;
        if !lower[after..].starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            from = after;
            continue;
        }
        let end = lower[after..].find('>').map(|i| after + i).unwrap_or(lower.len());
        let tag = &lower[after..end];
        from = end.max(after + 1);
        // `src`, as an attribute of its own: preceded by whitespace so that
        // `data-src` and `srcset` are not read as one.
        let mut at_src = None;
        let mut scan = 0usize;
        while let Some(i) = tag[scan..].find("src").map(|i| scan + i) {
            let before_ok = i == 0 || tag[..i].ends_with(|c: char| c.is_whitespace());
            let rest = tag[i + 3..].trim_start();
            if before_ok && rest.starts_with('=') {
                at_src = Some(i + 3 + (tag[i + 3..].len() - rest.len()) + 1);
                break;
            }
            scan = i + 3;
        }
        let Some(v) = at_src else { continue };
        let value = tag[v..].trim_start();
        let quote = match value.chars().next() {
            Some(q @ ('"' | '\'')) => q,
            _ => continue, // unquoted: not guessed at
        };
        let rel_start = after + (tag.len() - value.len()) + 1;
        let Some(len) = value[1..].find(quote) else { continue };
        // Sliced out of the ORIGINAL line, not the lowercased copy. The
        // lowercasing exists only to FIND the tag and the attribute; the value
        // is a path, and the copy has to land under the name the viewer will
        // ask for. macOS would forgive that and a case-sensitive volume — or
        // anyone reading the tree by hand — would not. ASCII-lowercasing keeps
        // the byte length, so the offsets line up in both strings.
        let dest = &line[rel_start..rel_start + len];
        if !dest.is_empty() {
            out.push(dest.to_string());
        }
    }
}

/// The destination at the head of `s`, which begins just after the `(`, and how
/// many bytes of `s` it consumed.
///
/// CommonMark's two spellings: bare, ending at the first whitespace (a title
/// follows) or at the closing `)`; and `<…>`, which is how a path with a space
/// in it is written. The title is dropped HERE rather than downstream — it is
/// prose, and prose arriving at the path layer would be one more string that
/// layer has to be right about refusing.
fn destination(s: &str) -> Option<(&str, usize)> {
    let start = s.len() - s.trim_start().len();
    let rest = &s[start..];
    if let Some(inner) = rest.strip_prefix('<') {
        let end = inner.find('>')?;
        return Some((&inner[..end], start + 1 + end + 1));
    }
    let end = rest.find(|c: char| c.is_whitespace() || c == ')').unwrap_or(rest.len());
    Some((&rest[..end], start + end))
}

/// A link reference definition's destination — `[label]: dest "title"` — or
/// `None` for any other line.
fn link_definition(line: &str) -> Option<String> {
    let indent = line.len() - line.trim_start().len();
    // Four spaces in is an indented code block, which is an example rather than
    // a definition — the same reading `image_refs` gives a fence.
    if indent > 3 {
        return None;
    }
    let t = line.trim_start();
    if !t.starts_with('[') {
        return None;
    }
    let close = t.find("]:")?;
    let (dest, _) = destination(&t[close + 2..])?;
    if dest.is_empty() { None } else { Some(dest.to_string()) }
}

/// **Layer A.** One reference, written in the document at `doc_rel`, resolved
/// to a path relative to the PLACE ROOT — or `None` for everything we refuse to
/// turn into a filesystem read.
///
/// This is a string out of a document in a repository the user may have cloned
/// seconds ago, and the caller is about to open what it names and copy it where
/// a loopback server hands it out. So it gets the walk's treatment (§4.2), one
/// layer here and the other at the filesystem, and the two are not
/// interchangeable: this one cannot see through a symlink, which is why
/// `viewer::copy_assets` canonicalises afterwards and requires the result to be
/// under the root. Neither layer alone is the boundary.
///
/// Refused, each for a failure rather than for tidiness:
///
/// - **a URL scheme** — `http:`, `https:`, `data:`, `file:`, `mailto:`, or any
///   other `scheme:` shape, and the protocol-relative `//host/x`. These are not
///   local files and are left entirely alone: the viewer fetches them, or does
///   not, exactly as it would from the repo. (A Windows drive letter, `C:/x`,
///   is refused by the same test, which is the right answer for a different
///   reason.)
/// - **an absolute path** — `/etc/passwd` names a file that has nothing to do
///   with this place, and `Path::join` would take it whole, so the root would
///   not even appear in the result.
/// - **`~` and `$`** — this tool expands neither, so a reference carrying one
///   is either an author's mistake or an attempt on a shell that is not there.
///   `~/.ssh/id_rsa` as a literal directory name is not worth the ambiguity.
/// - **a `..` that escapes the place**, an **empty component** (`a//b`, a
///   trailing `/`), and a **`.git` component** — the rules `RelPath` already
///   enforces on a configured path, applied to a reference for the same reasons.
/// - **a control character**, which has no business in a path and can end a
///   line in anything that later logs it.
///
/// `#fragment` and `?query` are stripped — a browser does not send them as part
/// of a filename, so neither may we. Percent escapes are deliberately NOT
/// decoded: decoding would let `%2e%2e` become `..` AFTER this function's own
/// check on it, and the cost of leaving them alone is that `a%20b.png` resolves
/// to nothing and stays broken — a broken image rather than a read outside the
/// place.
pub fn asset_rel(doc_rel: &str, reference: &str) -> Option<String> {
    let r = reference.trim();
    if r.is_empty() || r.len() > REF_MAX {
        return None;
    }
    if r.chars().any(|c| c.is_control()) {
        return None;
    }
    if has_scheme(r) || r.starts_with("//") {
        return None;
    }
    if r.starts_with('/') || r.starts_with('~') || r.contains('$') || r.contains('\\') {
        return None;
    }
    // After the scheme test, never before it: `data:image/png;base64,…#x` is a
    // URL whose tail happens to look like a fragment, and cutting it off first
    // would hand the scheme test a shorter string to be wrong about.
    let r = r.split(['#', '?']).next().unwrap_or("");
    if r.is_empty() {
        return None;
    }

    // The document's own directory is where a relative reference starts. `rel`
    // comes out of the walk clean, but it arrives here as a field of a struct
    // rather than from the walk, so it is checked rather than assumed.
    let mut parts: Vec<&str> = doc_rel.split('/').collect();
    parts.pop();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || *p == ".git") {
        return None;
    }
    for comp in r.split('/') {
        match comp {
            "" => return None,
            "." => continue,
            // Popping past the document's own directory is popping past the
            // place root, which is the traversal this function exists for.
            ".." => {
                parts.pop()?;
            }
            ".git" => return None,
            _ => parts.push(comp),
        }
    }
    if parts.is_empty() {
        return None;
    }
    let rel = parts.join("/");
    if rel.len() > REF_MAX { None } else { Some(rel) }
}

/// Does this reference begin with a URL scheme? `scheme:` per RFC 3986 — a
/// letter, then letters, digits, `+`, `-` and `.`, then a colon.
///
/// Matched by SHAPE rather than against a list of schemes, because the list is
/// the part that goes out of date: `blob:`, `filesystem:` and whatever a
/// browser ships next are all things this tool must not open as a file, and a
/// path with a colon before its first slash is not one a browser would read as
/// a path either.
fn has_scheme(r: &str) -> bool {
    let Some(colon) = r.find(':') else { return false };
    if colon == 0 || r[..colon].contains('/') {
        return false;
    }
    let mut cs = r[..colon].chars();
    cs.next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

// ── blocks ───────────────────────────────────────────────────────────────────

/// One top-level block of a document, as the browser page consumes it.
///
/// **Why a document is ever cut up at all.** The page polls, and when the text
/// moves it replaces only the blocks whose source changed, leaving every other
/// DOM node identical. That is not an optimisation: measured on a real page in
/// both browsers, block-level replacement holds `scroll 400→400, sel 55→55`
/// across an edit, and replacing the whole document measures `sel 70→0` — as
/// destructive as a reload, which is the thing this transport exists to be
/// better than (`docs-transport` findings §3.3, §1.2). A wrong split does not
/// fail loudly; it quietly ruins reading.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Stable across re-derives: the same source text is always the same id,
    /// wherever it has moved to in the document.
    ///
    /// **Derived from the content, deliberately not from the position.** A
    /// positional `b0, b1, b2…` is stable only while nothing is inserted: add a
    /// paragraph at the top and every id below it shifts, so every block reads
    /// as changed and the page replaces the whole document — exactly the
    /// `sel 70→0` failure the split exists to prevent, arriving on the most
    /// ordinary edit there is. Keyed by content instead, an insert is one new
    /// id among unchanged neighbours, a delete is one id gone, and a moved
    /// block keeps its own.
    ///
    /// The cost is that an EDIT changes the id rather than the body under it.
    /// That is the same single-node replacement either way: the block whose
    /// text changed is the one block whose DOM cannot be preserved.
    pub id: String,
    /// The block's source, byte for byte, including the blank lines that
    /// follow it. Concatenating every `md` in order reproduces the input
    /// exactly — `blocks_partition_every_byte` is that assertion, and it is
    /// what makes "the joined output is the document" a fact rather than a
    /// hope.
    pub md: String,
}

/// FNV-1a's offset basis, the seed `docs::fingerprint_with` uses. Same
/// reasoning as there: this digest compares the tool's own output against the
/// tool's own output a second later, so there is nobody to choose colliding
/// inputs, and a cryptographic hash here would be a supply-chain entry to
/// notice that a paragraph moved.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Split a derived document into top-level blocks.
///
/// **The bias is to OVER-group, always.** Splitting one construct into two is a
/// rendering change — a loose list cut at a blank line renders as two `<ul>`s,
/// a fence cut at a blank line renders as prose and code and a stray ``` — and
/// it is invisible to any test that only checks the join. Grouping two adjacent
/// constructs into one block renders identically and costs one extra block
/// redrawn on an edit. So every ambiguous case below continues the block.
///
/// What is recognised, because each of these legally contains a blank line and
/// each would be destroyed by a naive blank-line split: fenced code, indented
/// code, a loose list (and its nested lists and lazy continuations), an HTML
/// comment or `<script>`/`<pre>`/`<style>`/`<textarea>` block. Blockquotes,
/// tables, setext headings and paragraphs are blank-line-terminated runs.
pub fn blocks(text: &str) -> Vec<Block> {
    let sp = line_spans(text);
    let n = sp.len();
    let mut out: Vec<Block> = Vec::new();
    let mut seen: BTreeMap<u64, u32> = BTreeMap::new();
    if n == 0 {
        return out;
    }
    let line = |k: usize| content(text, sp[k]);
    // Leading blank lines belong to the first block: every byte has to land in
    // exactly one block or the join is not the document.
    let mut i = 0;
    while i < n && line(i).trim().is_empty() {
        i += 1;
    }
    if i == n {
        // A document of nothing but blank lines is still bytes we were handed.
        out.push(block(text, &mut seen));
        return out;
    }
    let mut start = 0usize;
    loop {
        let mut k = block_end(text, &sp, i);
        // Trailing blank lines join the block above them, which is what makes
        // the partition contiguous without inventing a separator the join has
        // to guess at.
        while k < n && line(k).trim().is_empty() {
            k += 1;
        }
        out.push(block(&text[start..sp[k - 1].1], &mut seen));
        if k >= n {
            break;
        }
        start = sp[k].0;
        i = k;
    }
    out
}

/// One block, with its id. Two blocks with identical source get `-1`, `-2`…, so
/// a document with three `---` rules still hands the page three distinct keys.
fn block(md: &str, seen: &mut BTreeMap<u64, u32>) -> Block {
    let h = crate::docs::fnv1a(FNV_OFFSET, md.as_bytes());
    let dup = seen.entry(h).or_insert(0);
    let id = if *dup == 0 { format!("b{h:016x}") } else { format!("b{h:016x}-{dup}") };
    *dup += 1;
    Block { id, md: md.to_string() }
}

/// Every line's byte range, the terminator included. `str::lines` cannot be
/// used here: it drops `\r` and cannot tell `"a\n"` from `"a"`, and this
/// partition has to be exact.
fn line_spans(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut s = 0;
    for (i, c) in b.iter().enumerate() {
        if *c == b'\n' {
            out.push((s, i + 1));
            s = i + 1;
        }
    }
    if s < b.len() {
        out.push((s, b.len()));
    }
    out
}

/// One line without its terminator.
fn content(text: &str, sp: (usize, usize)) -> &str {
    let mut s = &text[sp.0..sp.1];
    if let Some(t) = s.strip_suffix('\n') {
        s = t;
    }
    if let Some(t) = s.strip_suffix('\r') {
        s = t;
    }
    s
}

/// Leading indentation in columns, a tab counting as four. Only ever compared
/// against 3 (the most a leaf block may be indented by) and against a list
/// marker's own column, so the approximation cannot be off by enough to matter.
fn indent_of(s: &str) -> usize {
    let mut n = 0;
    for c in s.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4,
            _ => break,
        }
    }
    n
}

/// An opening code fence at a block start: three or more backticks or tildes,
/// indented by at most three.
fn fence_at(s: &str) -> Option<(char, usize)> {
    if indent_of(s) > 3 {
        return None;
    }
    let t = s.trim_start();
    let c = t.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = t.chars().take_while(|x| *x == c).count();
    if n < 3 {
        return None;
    }
    Some((c, n))
}

/// The matching closing fence: the same character, at least as long, and
/// nothing after it.
fn fence_closes(s: &str, c: char, n: usize) -> bool {
    if indent_of(s) > 3 {
        return false;
    }
    let t = s.trim_start();
    let run = t.chars().take_while(|x| *x == c).count();
    run >= n && t[run..].trim().is_empty()
}

/// An ATX heading — the one construct that always interrupts a paragraph.
fn is_atx(s: &str) -> bool {
    if indent_of(s) > 3 {
        return false;
    }
    let t = s.trim_start();
    let h = t.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&h) && t[h..].chars().next().map(|c| c == ' ' || c == '\t').unwrap_or(true)
}

/// A thematic break. Only asked at a BLOCK START, where it cannot be a setext
/// underline — `---` under a paragraph is an H2 and asking there would cut a
/// heading off its own text.
fn is_thematic(s: &str) -> bool {
    if indent_of(s) > 3 {
        return false;
    }
    let t = s.trim_start();
    let Some(c) = t.chars().next() else { return false };
    if c != '-' && c != '*' && c != '_' {
        return false;
    }
    t.chars().filter(|x| *x == c).count() >= 3 && t.chars().all(|x| x == c || x == ' ' || x == '\t')
}

/// A blockquote marker.
fn is_quote(s: &str) -> bool {
    indent_of(s) <= 3 && s.trim_start().starts_with('>')
}

/// A list item's marker column, or `None`. Bullet or ordered; the marker must
/// be followed by whitespace or end the line, or `1.5` and `*emphasis*` become
/// lists.
fn list_at(s: &str) -> Option<usize> {
    let ind = indent_of(s);
    if ind > 3 {
        return None;
    }
    let t = s.trim_start();
    let rest = match t.chars().next()? {
        '-' | '+' | '*' => &t[1..],
        c if c.is_ascii_digit() => {
            let d = t.chars().take_while(|x| x.is_ascii_digit()).count();
            if d > 9 {
                return None;
            }
            let after = &t[d..];
            match after.chars().next()? {
                '.' | ')' => &after[1..],
                _ => return None,
            }
        }
        _ => return None,
    };
    match rest.chars().next() {
        None => Some(ind),
        Some(' ') | Some('\t') => Some(ind),
        _ => None,
    }
}

/// An HTML block that may legally contain a blank line, and the string that
/// ends it. These are the ones a blank-line split would cut in half.
fn html_spanning(s: &str) -> Option<&'static str> {
    if indent_of(s) > 3 {
        return None;
    }
    let t = s.trim_start().to_ascii_lowercase();
    for (open, close) in [
        ("<!--", "-->"),
        ("<script", "</script>"),
        ("<pre", "</pre>"),
        ("<style", "</style>"),
        ("<textarea", "</textarea>"),
    ] {
        if t.starts_with(open) {
            return Some(close);
        }
    }
    None
}

/// Any other HTML block: ends at a blank line, like a paragraph.
fn is_html(s: &str) -> bool {
    indent_of(s) <= 3 && s.trim_start().starts_with('<')
}

/// Where the block beginning at line `i` ends — exclusive, and past its last
/// CONTENT line, never past the blank lines after it (the caller absorbs
/// those). Always greater than `i`, so the caller's loop always advances.
fn block_end(text: &str, sp: &[(usize, usize)], i: usize) -> usize {
    let n = sp.len();
    let line = |k: usize| content(text, sp[k]);
    let blank = |k: usize| line(k).trim().is_empty();
    let cur = line(i);

    // A fence owns everything up to its close, blank lines included. One that
    // never closes owns the rest of the document — which is what a renderer
    // does with it too, so the block and the rendering agree.
    if let Some((c, len)) = fence_at(cur) {
        let mut k = i + 1;
        while k < n {
            if fence_closes(line(k), c, len) {
                return k + 1;
            }
            k += 1;
        }
        return n;
    }

    // Before every other test: a four-space-indented line is code, and code is
    // allowed to look like anything below.
    if indent_of(cur) >= 4 {
        let mut last = i + 1;
        let mut k = i + 1;
        while k < n {
            if blank(k) {
                k += 1;
                continue;
            }
            if indent_of(line(k)) >= 4 {
                k += 1;
                last = k;
                continue;
            }
            break;
        }
        return last;
    }

    if let Some(close) = html_spanning(cur) {
        // From the opening line itself: `<!-- x -->` is one line and one block.
        let open_len = cur.trim_start().len().min(cur.len());
        let from = cur.len() - open_len + 1;
        if cur.len() > from && cur[from..].to_ascii_lowercase().contains(close) {
            return i + 1;
        }
        let mut k = i + 1;
        while k < n {
            if line(k).to_ascii_lowercase().contains(close) {
                return k + 1;
            }
            k += 1;
        }
        return n;
    }

    if is_atx(cur) || is_thematic(cur) {
        return i + 1;
    }

    if let Some(marker) = list_at(cur) {
        let mut last = i + 1;
        let mut k = i + 1;
        // A blank line inside a list does not end it — that is what makes the
        // list LOOSE, and cutting there is the split that renders as two lists.
        let mut after_blank = false;
        while k < n {
            if blank(k) {
                after_blank = true;
                k += 1;
                continue;
            }
            let l = line(k);
            // Indented past the marker: this item's own continuation, or a
            // nested list. A sibling marker: the next item. Neither ends it.
            if indent_of(l) > marker || list_at(l).is_some() {
                k += 1;
                last = k;
                after_blank = false;
                continue;
            }
            // Flush left after a blank line is a new block; flush left with no
            // blank line is a lazy continuation of the item's paragraph.
            if after_blank {
                break;
            }
            k += 1;
            last = k;
        }
        return last;
    }

    if is_quote(cur) || is_html(cur) {
        let mut k = i + 1;
        while k < n && !blank(k) {
            k += 1;
        }
        return k;
    }

    // A paragraph, a table, or a setext heading: a run of non-blank lines.
    // It stops early only for the constructs that genuinely interrupt one —
    // NOT for a thematic break (under a paragraph `---` is a setext underline,
    // and cutting there decapitates the heading) and not for a list (grouping
    // a paragraph with the list under it renders identically; splitting a list
    // that CommonMark would not have started here does not).
    let mut k = i + 1;
    while k < n {
        let l = line(k);
        if l.trim().is_empty() || fence_at(l).is_some() || is_atx(l) || is_quote(l) {
            break;
        }
        k += 1;
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rel: &str, title: &str) -> DocEntry {
        DocEntry {
            path: format!("/place/{rel}"),
            rel: rel.to_string(),
            title: title.to_string(),
            group: String::new(),
            mtime_ms: 0,
        }
    }

    fn stale() -> Staleness {
        Staleness {
            place: "live-docs".into(),
            branch: Some("live-docs-derive".into()),
            behind: Some(25),
            base: "origin/main".into(),
            dirty: Some(true),
            dirty_files: Some(3),
            last_commit_subject: Some("docs(runbook): reconcile §4".into()),
            last_commit_epoch: Some(1_000_000),
            now_epoch: 1_000_000 + 7_200,
            // The open path: the facts were captured at the moment the copy was
            // made, so the header carries ONE stamp. The tick's case (a copy
            // newer than its facts) is `a_re_derived_page_dates_its_facts_apart_
            // from_itself`, which sets them apart deliberately.
            derived_epoch: 1_000_000 + 7_200,
        }
    }

    /// A URL form that is plausible and viewer-shaped, so the tests exercise the
    /// seam rather than a bare "http://127.0.0.1/".
    fn viewer(e: &DocEntry) -> Option<String> {
        Some(format!("http://127.0.0.1:6275/live-docs?file={}", e.rel))
    }

    // ── 1. frontmatter ───────────────────────────────────────────────────────

    #[test]
    fn frontmatter_goes_away_and_the_title_it_carried_comes_back_as_a_heading() {
        let e = entry("docs/a.md", "From Frontmatter");
        let src = "---\ntitle: From Frontmatter\nlayout: page\n---\n\nprose here\n";
        let out = document(&e, src, &stale(), &Links::NONE);
        assert!(!out.contains("layout: page"), "the metadata block survived: {out}");
        assert!(out.contains("# From Frontmatter"), "the only title the document had is gone: {out}");
        assert!(out.contains("prose here"));
        // …and it is NOT re-added when the body already has its own H1.
        let src = "---\ntitle: From Frontmatter\n---\n# An H1\n";
        let out = document(&e, src, &stale(), &Links::NONE);
        assert_eq!(out.matches("# ").count(), 1, "two titles: {out}");
        // …nor invented for a document that never had one.
        let out = document(&entry("docs/b.md", "b"), "just prose\n", &stale(), &Links::NONE);
        assert!(!out.contains("# b"), "an H1 the author never wrote: {out}");
    }

    /// An opening `---` with no closing one is a thematic break, not a block
    /// that runs to EOF. `title_from` could afford that reading (it loses a
    /// title); the derive cannot (it loses the document).
    #[test]
    fn an_unterminated_rule_is_not_frontmatter_and_the_document_survives() {
        let src = "---\n\n# The Real Title\n\nthe whole document\n";
        let out = document(&entry("docs/a.md", "a"), src, &stale(), &Links::NONE);
        assert!(out.contains("the whole document"), "the document was deleted: {out}");
        assert_eq!(docs::title_from(src).as_deref(), Some("The Real Title"));
    }

    // ── 2. the staleness header ──────────────────────────────────────────────

    #[test]
    fn the_header_is_the_first_thing_in_the_document_and_names_the_base_ref() {
        let out = document(&entry("docs/a.md", "a"), "# A\n", &stale(), &Links::NONE);
        assert!(out.starts_with("> "), "the header is not first: {out}");
        let head = out.split("\n---\n").next().unwrap();
        assert!(head.contains("**live-docs**"), "{head}");
        assert!(head.contains("`live-docs-derive`"), "{head}");
        assert!(head.contains("3 dirty"), "{head}");
        // …against the BASE ref (§11.4). There is no assertion that it is not
        // `upstream`, because `Staleness` has no `upstream` field to get it
        // wrong with — the struct is the guard, and it is a stronger one than a
        // string test would be.
        assert!(head.contains("25 behind `origin/main`"), "{head}");
        assert!(head.contains("2h"), "{head}");
        assert!(out.ends_with("# A\n"), "{out}");
    }

    #[test]
    fn the_headers_words_are_the_docks_words() {
        let mut s = stale();
        // `DocsPane.tsx::staleness` / `ago`, fact for fact.
        s.dirty_files = Some(1);
        assert!(header(&s).contains("1 dirty"));
        s.dirty = Some(false);
        assert!(header(&s).contains("clean"));
        s.behind = Some(0);
        assert!(header(&s).contains("up to date with `origin/main`"), "{}", header(&s));
        s.base = String::new();
        assert!(header(&s).contains("up to date"));
        s.behind = Some(4);
        assert!(header(&s).contains("4 behind"), "no ref is honest; the wrong ref is not: {}", header(&s));
        assert!(!header(&s).contains("behind `"));
        // `behind: None` is "not computed", which is not zero and does not render.
        s.behind = None;
        assert!(!header(&s).contains("behind"), "{}", header(&s));
        assert!(!header(&s).contains("up to date"), "{}", header(&s));
        s.branch = None;
        assert!(header(&s).contains("detached"), "{}", header(&s));
        // the age buckets
        let at = |secs: i64| {
            let mut s = stale();
            s.last_commit_epoch = Some(1_000_000);
            s.now_epoch = 1_000_000 + secs;
            header(&s)
        };
        assert!(at(30).contains("· now\n"));
        assert!(at(600).contains("· 10m\n"));
        assert!(at(7_200).contains("· 2h\n"));
        assert!(at(200_000).contains("· 2d\n"));
        let mut s = stale();
        s.last_commit_epoch = None;
        assert!(!header(&s).contains(" · \n"), "{}", header(&s));
    }

    /// The copy has to date ITSELF, not only the place it came from.
    ///
    /// Everything else in this header ages in the reader's hands without saying
    /// so: a browser tab left open overnight, on a page the tool copied before
    /// lunch, looks exactly like one generated a second ago. That is §1.1's
    /// hazard — a document that does not say which world it is from — reproduced
    /// inside the feature built to prevent it, one level in.
    #[test]
    fn a_derived_page_says_when_it_was_copied() {
        let mut s = stale();
        s.now_epoch = 1_789_776_000;
        s.derived_epoch = 1_789_776_000;
        let h = header(&s);
        assert!(h.contains("derived 2026-09-19 00:00:00 UTC"), "no derivation stamp: {h}");
        // One instant, one stamp: the open path captured the facts and wrote the
        // copy in the same breath, and saying it twice would invite the reader
        // to look for a difference that is not there.
        assert_eq!(h.matches("UTC").count(), 1, "{h}");
        // Inside the blockquote, like every other fact here.
        assert!(h.lines().any(|l| l.starts_with("> *derived ")), "{h}");
    }

    /// The tick's case: the text is current, the git line is not.
    ///
    /// The app re-derives a registered place whenever its documents move, and it
    /// does NOT re-run the git fan-out those numbers came from (§15.3 refused
    /// that for a click; a timer is worse). So one stamp would be a lie about
    /// whichever fact it was not measuring — and the lie it tells is "this
    /// status is current", which is the exact error the header exists to remove.
    #[test]
    fn a_re_derived_page_dates_its_facts_apart_from_itself() {
        let mut s = stale();
        s.now_epoch = 1_789_776_000; // the button press
        s.derived_epoch = 1_789_779_661; // an hour and a minute of Claude writing
        let h = header(&s);
        assert!(h.contains("derived 2026-09-19 01:01:01 UTC"), "{h}");
        assert!(h.contains("status as of 2026-09-19 00:00:00 UTC"), "{h}");
        let line = h.lines().find(|l| l.contains("derived ")).unwrap();
        assert!(
            line.find("derived ").unwrap() < line.find("status as of ").unwrap(),
            "the copy's own age comes first — it is the one fact only this line carries: {line}"
        );
    }

    /// `0` is "not known", and a header that answers it with `1970-01-01` would
    /// be stating a fact nobody supplied. Same convention as a missing commit
    /// epoch, which renders no age rather than "now".
    #[test]
    fn an_unknown_derivation_time_says_nothing_rather_than_1970() {
        let mut s = stale();
        s.derived_epoch = 0;
        let h = header(&s);
        assert!(!h.contains("derived"), "{h}");
        assert!(!h.contains("1970"), "{h}");
        // …and the rest of the header is untouched by its absence.
        assert!(h.contains("25 behind `origin/main`"), "{h}");
    }

    /// The stamp is arithmetic rather than `date`, because this module is a pure
    /// transform and `sysclock` spawns a process per call. Arithmetic that is
    /// wrong is worse than a spawn, so the era/leap-day handling is asserted
    /// against dates chosen to break it: a leap day, the day after one, a
    /// century that is NOT a leap year, one that is, and the epoch itself.
    #[test]
    fn the_derivation_stamp_is_the_real_calendar_date() {
        let mut at = |epoch: i64| {
            let mut s = stale();
            s.now_epoch = epoch;
            s.derived_epoch = epoch;
            let h = header(&s);
            let line = h.lines().find(|l| l.starts_with("> *derived ")).unwrap_or("").to_string();
            line.trim_start_matches("> *derived ").trim_end_matches('*').to_string()
        };
        assert_eq!(at(1), "1970-01-01 00:00:01 UTC");
        assert_eq!(at(1_709_164_800), "2024-02-29 00:00:00 UTC"); // a leap day
        assert_eq!(at(1_709_251_199), "2024-02-29 23:59:59 UTC"); // …its last second
        assert_eq!(at(1_709_251_200), "2024-03-01 00:00:00 UTC"); // …and the day after
        assert_eq!(at(951_782_400), "2000-02-29 00:00:00 UTC"); // 2000 IS a leap year
        assert_eq!(at(4_107_542_400), "2100-03-01 00:00:00 UTC"); // 2100 is NOT
        assert_eq!(at(1_789_776_000), "2026-09-19 00:00:00 UTC");
    }

    /// A commit subject is arbitrary bytes and a branch name may carry a
    /// backtick. Both land inside a code span in a blockquote WE write, at the
    /// top of every page — so a backtick plus a newline would end the span, end
    /// the quote, and continue in the document's own voice.
    #[test]
    fn an_author_controlled_fact_cannot_break_out_of_the_header() {
        let mut s = stale();
        s.last_commit_subject = Some("ok`\n\n# Trusted heading\n\nfine print".into());
        s.branch = Some("feat/`x`".into());
        let h = header(&s);
        let quote: Vec<&str> = h.lines().take_while(|l| l.starts_with('>')).collect();
        assert_eq!(quote.len(), h.lines().filter(|l| !l.trim().is_empty() && *l != "---").count(), "text escaped the blockquote: {h}");
        // The words survive, as WORDS — inside the code span, on one line. What
        // must not survive is their markdown meaning.
        assert!(!h.lines().any(|l| l.trim_start().starts_with('#')), "a heading in the tool's own voice: {h}");
        assert!(h.contains("Trusted heading"), "the subject was not shown at all: {h}");
        assert_eq!(h.matches('`').count() % 2, 0, "unbalanced code spans: {h}");
    }

    // ── 3. author `click` directives ─────────────────────────────────────────

    #[test]
    fn every_author_supplied_click_is_stripped() {
        let src = "\
```mermaid
flowchart TD
  A[Start] --> B[End]
  click A \"https://evil.example/steal\"
  CLICK B href \"https://evil.example/steal\" \"tip\"
  click A call danger()
  D --> E; click D \"https://evil.example\"
    click E href \"http://127.0.0.1:6275/x\"
```
";
        let out = diagrams(src, &Links::NONE);
        assert!(!out.to_lowercase().contains("click"), "an author directive survived:\n{out}");
        assert!(out.contains("A[Start] --> B[End]"), "the diagram was damaged:\n{out}");
        assert!(out.contains("D --> E"), "a statement sharing a line with a click was lost:\n{out}");
    }

    /// Rule 1 is about diagrams. Prose that happens to begin with the word, and
    /// a shell fence that contains the command, are not directives.
    #[test]
    fn prose_and_other_fences_are_copied_through_untouched() {
        let src = "\
click here to continue

```sh
click A \"https://example.com\"
```

```mermaid
graph TD
  click A \"https://evil.example\"
```
";
        let out = diagrams(src, &Links::NONE);
        assert!(out.contains("click here to continue"), "{out}");
        assert_eq!(out.matches("click A \"https://example.com\"").count(), 1, "the sh fence was rewritten:\n{out}");
        assert!(!out.contains("evil.example"), "{out}");
    }

    #[test]
    fn a_node_named_click_keeps_its_edge() {
        let src = "```mermaid\nflowchart LR\n  click --> B\n  Click-->C\n```\n";
        let out = diagrams(src, &Links::NONE);
        assert!(out.contains("click --> B"), "an edge was read as a directive:\n{out}");
        assert!(out.contains("Click-->C"), "{out}");
    }

    #[test]
    fn a_semicolon_inside_a_label_is_not_a_statement_separator() {
        // The label is ordinary English — and quote-blind splitting reads its
        // second half as a `click` statement, drops it, and leaves the node
        // without its closing bracket. The damage is to the DIAGRAM, from the
        // rule that exists to protect it.
        let src = "```mermaid\nflowchart TD\n  A[\"Step 2; click Save\"] --> B\n```\n";
        let out = diagrams(src, &Links::NONE);
        assert!(out.contains("A[\"Step 2; click Save\"] --> B"), "a label was cut in half:\n{out}");
    }

    #[test]
    fn a_longer_fence_is_not_closed_by_a_shorter_one() {
        let src = "````mermaid\nflowchart TD\n```\nclick A \"https://evil.example\"\n````\n";
        let out = diagrams(src, &Links::NONE);
        assert!(!out.contains("evil.example"), "the block ended at the inner fence:\n{out}");
    }

    #[test]
    fn the_diagrams_own_frontmatter_is_preserved_and_never_scanned() {
        let entries = vec![entry("docs/overview.md", "Overview")];
        let links = Links { entries: &entries, url: &viewer };
        let src = "```mermaid\n---\ntitle: Flow\nconfig:\n  theme: forest\n---\nflowchart TD\n  overview --> B\n  click A \"https://evil.example\"\n```\n";
        let out = diagrams(src, &links);
        assert!(out.contains("title: Flow") && out.contains("theme: forest"), "the diagram's config was eaten:\n{out}");
        assert!(!out.contains("evil.example"), "{out}");
        // …and the diagram's TYPE is read past the block, not from it: a
        // frontmatter'd flowchart is still a flowchart.
        assert!(out.contains("click overview href"), "the config block hid the diagram's type:\n{out}");
    }

    // ── 3b. our own directives, and rule 2 ───────────────────────────────────

    #[test]
    fn a_click_is_emitted_for_a_node_that_names_exactly_one_page() {
        let entries = vec![entry("docs/overview.md", "Overview"), entry("docs/billing.md", "Billing")];
        let links = Links { entries: &entries, url: &viewer };
        // `overview-->billing` with no spaces is the common spelling and it is
        // the one an id-charset tokeniser reads as `overview--`.
        let src = "```mermaid\nflowchart TD\n  overview[The overview]-->billing\n  billing --> nothing\n```\n";
        let out = diagrams(src, &links);
        assert!(out.contains("click overview href \"http://127.0.0.1:6275/live-docs?file=docs/overview.md\""), "{out}");
        assert!(out.contains("click billing href \"http://127.0.0.1:6275/live-docs?file=docs/billing.md\""), "{out}");
        assert!(!out.contains("click nothing"), "a node that names no page: {out}");
        assert_eq!(out.matches("click overview").count(), 1, "one directive per node: {out}");
        // the emitted lines sit inside the fence, where mermaid reads them
        let fence_end = out.find("\n```\n").unwrap();
        assert!(out.find("click overview").unwrap() < fence_end, "emitted outside the diagram:\n{out}");
    }

    #[test]
    fn an_ambiguous_or_label_only_match_is_not_a_target() {
        // two `overview.md`s in one place: the diagram did not say which.
        let entries = vec![entry("docs/overview.md", "Overview"), entry("handbook/overview.md", "Overview")];
        let links = Links { entries: &entries, url: &viewer };
        let out = diagrams("```mermaid\nflowchart TD\n  overview --> b\n```\n", &links);
        assert!(!out.contains("click"), "guessed between two pages:\n{out}");
        // a LABEL that names a page is not a node id — labels are prose.
        let entries = vec![entry("docs/billing.md", "Billing")];
        let links = Links { entries: &entries, url: &viewer };
        let out = diagrams("```mermaid\nflowchart TD\n  A[see billing] --> B\n```\n", &links);
        assert!(!out.contains("click"), "a word in a label became a node:\n{out}");
    }

    #[test]
    fn keywords_and_the_type_line_are_never_nodes() {
        let entries = vec![entry("docs/flowchart.md", "F"), entry("docs/big.md", "B"), entry("docs/end.md", "E")];
        let links = Links { entries: &entries, url: &viewer };
        let src = "```mermaid\nflowchart TD\n  subgraph end\n  A --> B\n  end\n  class A big\n```\n";
        let out = diagrams(src, &links);
        assert!(!out.contains("click"), "a keyword was linked, which is a diagram that no longer parses:\n{out}");
    }

    #[test]
    fn nothing_is_emitted_into_a_diagram_whose_grammar_has_no_click() {
        let entries = vec![entry("docs/alice.md", "Alice")];
        let links = Links { entries: &entries, url: &viewer };
        let src = "```mermaid\nsequenceDiagram\n  participant alice\n  alice->>bob: hi\n```\n";
        let out = diagrams(src, &links);
        assert!(!out.contains("click"), "a click in a sequence diagram is a parse error, not a link:\n{out}");
        // …and the author's own is still stripped there.
        let src = "```mermaid\nsequenceDiagram\n  click alice \"https://evil.example\"\n```\n";
        assert!(!diagrams(src, &links).contains("evil.example"));
    }

    /// §5.3 rule 2. The first four are the shapes a prefix test passes.
    #[test]
    fn only_a_loopback_http_target_is_emitted() {
        for bad in [
            "http://127.0.0.1.evil.example/x",
            "http://127.0.0.1@evil.example/x",
            "http://evil.example@127.0.0.1/x",
            "http://evil.example/?u=http://127.0.0.1:6275/",
            "http:/\\/\\127.0.0.1:6275/x",
            "javascript:fetch(\"http://127.0.0.1:6275/\")",
            "data:text/html,<script>x</script>",
            "file:///etc/passwd",
            "https://127.0.0.1:6275/x",
            "http://localhost.evil.example/x",
            "http://[::1].evil.example/x",
            "http://127.0.0.1:6275/x\" \"tip\" _blank\nclick B href \"http://evil.example",
            "http://127.0.0.1:99999/x",
            "//127.0.0.1/x",
            "127.0.0.1:6275/x",
            "",
        ] {
            assert!(!is_loopback_target(bad), "accepted {bad:?}");
        }
        for good in [
            "http://127.0.0.1:6275/live-docs?file=abc",
            "http://127.0.0.1/x",
            "http://127.1.2.3:6275/x",
            "http://localhost:6275/x",
            "HTTP://LOCALHOST:6275/x",
            "http://[::1]:6275/x",
            "http://127.0.0.1:6275/a%20b#frag",
        ] {
            assert!(is_loopback_target(good), "refused {good:?}");
        }
    }

    #[test]
    fn a_non_loopback_url_from_the_seam_is_dropped_not_emitted() {
        let entries = vec![entry("docs/overview.md", "Overview")];
        let evil = |_: &DocEntry| Some("http://evil.example/x".to_string());
        let links = Links { entries: &entries, url: &evil };
        let out = diagrams("```mermaid\nflowchart TD\n  overview --> b\n```\n", &links);
        assert!(!out.contains("click"), "the seam's word was taken for it:\n{out}");
        assert!(!out.contains("evil.example"), "{out}");
    }

    /// The emitted form is `click <id> href "<url>"`. A URL carrying a quote
    /// would not be a bad link — it would be a SECOND directive, with an href
    /// nobody here wrote. Being loopback is not enough on its own.
    #[test]
    fn a_target_that_would_become_two_directives_is_not_one() {
        let entries = vec![entry("docs/overview.md", "Overview")];
        let sneaky = |_: &DocEntry| Some("http://127.0.0.1:6275/x\"_\"tip\"".to_string());
        let links = Links { entries: &entries, url: &sneaky };
        let out = diagrams("```mermaid\nflowchart TD\n  overview --> B\n```\n", &links);
        assert!(!out.contains("click"), "a quote in the target, emitted anyway:\n{out}");
    }

    /// An INVARIANT, not a guard, and the difference is worth stating because
    /// this repo's rule is that a new test must first be shown to fail.
    /// This one cannot be: every id `node_ids` returns comes out of a split on
    /// the identifier charset, so removing `labels_off`, removing `arrows_off`
    /// or widening the split all yield either a clean id or no id at all —
    /// four attempts, four still-green runs. It is kept for the reader who
    /// wonders what stops a label from becoming a second directive; the thing
    /// that stops it is the tokeniser's shape, not this assertion.
    #[test]
    fn an_emitted_id_can_only_be_an_identifier() {
        let entries = vec![entry("docs/overview.md", "Overview")];
        let links = Links { entries: &entries, url: &viewer };
        let src = "```mermaid\nflowchart TD\n  overview\" href \"http://evil.example --> B\n  overview --> C\n```\n";
        let out = diagrams(src, &links);
        let emitted: Vec<&str> = out.lines().filter(|l| l.trim_start().starts_with("click ")).collect();
        assert_eq!(emitted.len(), 1, "{out}");
        for line in &emitted {
            let id = line.trim_start().split_whitespace().nth(1).unwrap();
            assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'), "{id:?} in {out}");
            assert_eq!(line.matches('"').count(), 2, "a directive with a third quote is two directives: {line}");
            assert!(!line.contains("evil.example"), "{line}");
        }
    }

    // ── 4. assets ────────────────────────────────────────────────────────────

    /// The reference forms a document actually uses, and the two pieces of a
    /// reference that are NOT part of the filename: a markdown title, and a
    /// `#fragment` / `?query`. A path with either still glued on resolves to
    /// nothing, which is a broken image the tool went to the filesystem for.
    #[test]
    fn an_image_reference_is_found_in_every_form_a_document_writes_it() {
        let text = concat!(
            "# Doc\n",
            "![plain](pipeline.png)\n",
            "![titled](shots/a.png \"A shot\")\n",
            "![angled](<shots/with space.png>)\n",
            "![frag](b.png#fig-1)\n",
            "![query](c.png?v=2)\n",
            "![alt [with] brackets](d.png) and ![two](e.png) on one line\n",
            "[logo]: brand/logo.png \"Logo\"\n",
        );
        assert_eq!(
            image_refs(text),
            vec![
                "pipeline.png",
                "shots/a.png",
                "shots/with space.png",
                "b.png#fig-1",
                "c.png?v=2",
                "d.png",
                "e.png",
                "brand/logo.png",
            ]
        );
        // The fragment and the query are the resolver's to drop, and it does.
        assert_eq!(asset_rel("docs/x.md", "b.png#fig-1").as_deref(), Some("docs/b.png"));
        assert_eq!(asset_rel("docs/x.md", "c.png?v=2").as_deref(), Some("docs/c.png"));
    }

    /// A README centres its screenshots with raw HTML, and `mo` renders that —
    /// `rehype-raw` into `rehype-sanitize`, whose default schema keeps `img`. So
    /// a `<img src>` is a real reference, and this repo's own README carries
    /// three of them. What is NOT read is anything that would have to be
    /// guessed: an unquoted value, a `data-src`, a `srcset`.
    #[test]
    fn a_raw_html_image_is_a_reference_too() {
        let text = concat!(
            "<p align=\"center\">\n",
            "  <img src=\"docs/media/desktop-flow.gif\" width=\"820\" alt=\"flow\">\n",
            "</p>\n",
            "<img alt='single quoted' src='shots/B.PNG' />\n",
            "<img width=100 src=unquoted.png>\n",
            "<img data-src=\"lazy.png\">\n",
            "<img srcset=\"a.png 1x, b.png 2x\">\n",
            "<imgx src=\"nottag.png\">\n",
            "<img src=\"one.png\"><img src=\"two.png\">\n",
        );
        assert_eq!(
            image_refs(text),
            vec!["docs/media/desktop-flow.gif", "shots/B.PNG", "one.png", "two.png"]
        );
        // Case is preserved: the lowercased copy is only ever used to FIND the
        // attribute, never to slice the path out.
        assert_eq!(asset_rel("README.md", "shots/B.PNG").as_deref(), Some("shots/B.PNG"));
    }

    /// A reference is resolved against the DOCUMENT's directory, not the place
    /// root — that is what makes `../assets/x.png` from `docs/guides/` mean
    /// `docs/assets/x.png`, which is the path the copy has to mirror for the
    /// reference to resolve without being rewritten.
    #[test]
    fn a_reference_resolves_against_the_document_that_wrote_it() {
        assert_eq!(asset_rel("docs/guides/p.md", "../assets/x.png").as_deref(), Some("docs/assets/x.png"));
        assert_eq!(asset_rel("docs/guides/p.md", "shots/x.png").as_deref(), Some("docs/guides/shots/x.png"));
        assert_eq!(asset_rel("docs/guides/p.md", "./x.png").as_deref(), Some("docs/guides/x.png"));
        assert_eq!(asset_rel("README.md", "docs/x.png").as_deref(), Some("docs/x.png"));
        assert_eq!(asset_rel("README.md", "x.png").as_deref(), Some("x.png"));
        // Exactly back to the root, which is inside the place and therefore
        // allowed — one component further is not.
        assert_eq!(asset_rel("docs/guides/p.md", "../../x.png").as_deref(), Some("x.png"));
        assert_eq!(asset_rel("docs/guides/p.md", "../../../x.png"), None);
    }

    /// **Layer A, as one assertion.** Every one of these is a string from a
    /// document in a repo the user may have cloned seconds ago, and the caller
    /// turns what comes back into an open() and a copy.
    #[test]
    fn a_reference_that_could_leave_the_place_is_never_resolved() {
        for bad in [
            // escapes the place
            "../../../../etc/passwd.png",
            "../../../../../../../../Users/x/.ssh/id_rsa",
            // absolute, which `join` would take whole
            "/etc/passwd",
            "/Users/x/.ssh/id_rsa.png",
            // a shell this tool does not have
            "~/.ssh/id_rsa",
            "~root/x.png",
            "$HOME/x.png",
            "a/$(whoami).png",
            // the repo's own git directory
            ".git/config",
            "../.git/config",
            "docs/.git/objects/x.png",
            // empty components, and the separator that is not ours
            "",
            "   ",
            "a//b.png",
            "docs/",
            "a\\b.png",
            "..\\..\\x.png",
            // control characters, including one that would end a log line
            "a\nb.png",
            "a\u{0}b.png",
        ] {
            assert!(asset_rel("docs/guides/p.md", bad).is_none(), "{bad:?} must not resolve");
        }
        // A reference longer than the cap is not a path anyone typed.
        let long = format!("{}.png", "a".repeat(2000));
        assert!(asset_rel("docs/p.md", &long).is_none());
        // And a document whose own rel is not walk-shaped resolves nothing.
        assert!(asset_rel("../outside/p.md", "x.png").is_none());
    }

    /// A URL is not a local file, and the difference is not cosmetic: a
    /// `data:` or `file:` reference that reached the path layer would be a
    /// string with slashes in it being joined onto the place root.
    #[test]
    fn a_url_is_left_alone_rather_than_opened() {
        for url in [
            "http://example.com/x.png",
            "https://example.com/x.png",
            "HTTPS://EXAMPLE.COM/x.png",
            "data:image/png;base64,iVBORw0KGgo=",
            "data:image/png;base64,iVBORw0KGgo=#x",
            "file:///etc/passwd",
            "mailto:a@b.c",
            "javascript:alert(1)",
            "blob:http://example.com/1234",
            "//example.com/x.png",
            "C:/Windows/x.png",
        ] {
            assert!(asset_rel("docs/p.md", url).is_none(), "{url:?} is not a local file");
        }
        // …while a colon INSIDE a path segment is just a filename.
        assert_eq!(asset_rel("docs/p.md", "shots/a:b.png").as_deref(), Some("docs/shots/a:b.png"));
    }

    /// A reference inside a fence is an example of one. Copying what it names
    /// spends the viewer's byte caps on an image nothing renders — and the
    /// documents this tool reads are full of fenced markdown samples.
    #[test]
    fn an_image_inside_a_fence_is_not_a_reference() {
        let text = concat!(
            "![real](a.png)\n",
            "```md\n",
            "![an example](../../../etc/passwd.png)\n",
            "[x]: ../../../etc/shadow.png\n",
            "```\n",
            "~~~\n",
            "![also fenced](b.png)\n",
            "~~~\n",
            "![after](c.png)\n",
        );
        assert_eq!(image_refs(text), vec!["a.png", "c.png"]);
    }

    #[test]
    fn a_document_with_no_diagram_is_returned_as_it_was() {
        let src = "# Title\n\nsome prose\n\n- a list\n\n```rust\nfn main() {}\n```\n";
        assert_eq!(diagrams(src, &Links::NONE), src);
        // including the final newline, or its absence: `str::lines` cannot tell
        // them apart and a transform that adds one is editing the file.
        assert_eq!(diagrams("# Title\n\nno trailing newline", &Links::NONE), "# Title\n\nno trailing newline");
    }

    // ── blocks ──────────────────────────────────────────────────────────────

    /// The invariant that makes every other claim about the split checkable:
    /// the blocks PARTITION the document. Nothing is dropped, nothing is
    /// duplicated, no separator is invented that the page would have to guess
    /// at — so "the joined output is the document" is a fact, not a hope.
    ///
    /// Run over this repository's own documents, not a fixture: the corpus that
    /// matters is the one the feature serves, and it carries fences inside
    /// lists, tables, HTML comments, CRLF and files with no trailing newline.
    #[test]
    fn blocks_partition_every_byte_of_every_document() {
        let mut checked = 0;
        let mut stack = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if crate::docs::SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
                if md.is_dir() {
                    stack.push(p);
                } else if name.ends_with(".md") {
                    let Ok(text) = std::fs::read_to_string(&p) else { continue };
                    let joined: String = blocks(&text).iter().map(|b| b.md.as_str()).collect();
                    assert_eq!(joined, text, "{} did not survive the split", p.display());
                    checked += 1;
                }
            }
        }
        // A walk that found nothing would pass this test perfectly.
        assert!(checked > 20, "only {checked} documents were checked; the walk found nothing");
    }

    /// Synthetic shapes the repository may not happen to contain, checked for
    /// the same partition property. An empty file, a file of nothing but blank
    /// lines, CRLF, and a document with no trailing newline are all inputs the
    /// walk can hand us.
    #[test]
    fn the_partition_holds_for_the_shapes_a_repo_may_not_have() {
        for src in [
            "",
            "\n",
            "\n\n\n",
            "   \n\t\n",
            "a",
            "a\r\nb\r\n\r\nc\r\n",
            "# h\n\n\n\npara",
            "```\nunclosed\n\nstill inside\n",
        ] {
            let joined: String = blocks(src).iter().map(|b| b.md.as_str()).collect();
            assert_eq!(joined, src, "partition lost bytes of {src:?}");
        }
    }

    /// A fence is one block however many blank lines are inside it. Cut at a
    /// blank line it renders as prose, then code, then a stray ``` — a
    /// document the tool BROKE, and nothing about the join would notice.
    #[test]
    fn a_fence_with_blank_lines_in_it_is_one_block() {
        let src = "intro\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\nafter\n";
        let bs = blocks(src);
        assert_eq!(bs.len(), 3, "{:?}", bs.iter().map(|b| &b.md).collect::<Vec<_>>());
        assert!(bs[1].md.starts_with("```rust"), "{:?}", bs[1].md);
        assert!(bs[1].md.contains("fn a() {}\n\nfn b() {}"), "the fence was cut: {:?}", bs[1].md);
    }

    /// A LOOSE list — blank lines between its items — is one list to a
    /// renderer and must be one block here. Split, it renders as two `<ul>`s
    /// with a paragraph gap, which looks like a styling bug and is a split bug.
    #[test]
    fn a_loose_list_is_one_block() {
        let src = "# h\n\n- one\n\n- two\n\n  a second paragraph in item two\n\n- three\n\nafter the list\n";
        let bs = blocks(src);
        let list: Vec<&str> = bs.iter().map(|b| b.md.as_str()).filter(|m| m.contains("- one")).collect();
        assert_eq!(list.len(), 1);
        assert!(list[0].contains("- two"), "the list was cut at a blank line: {:?}", list[0]);
        assert!(list[0].contains("- three"), "the list was cut at a blank line: {:?}", list[0]);
        assert!(list[0].contains("a second paragraph"), "an item's own paragraph left the list");
        assert!(!list[0].contains("after the list"), "the list swallowed the paragraph after it");
    }

    /// An indented code block keeps its blank lines for the same reason a
    /// fenced one does, and it has no closing marker to find the end by.
    #[test]
    fn an_indented_code_block_survives_its_blank_line() {
        let src = "text\n\n    line one\n\n    line two\n\nback to prose\n";
        let bs = blocks(src);
        let code: Vec<&str> = bs.iter().map(|b| b.md.as_str()).filter(|m| m.contains("line one")).collect();
        assert_eq!(code.len(), 1);
        assert!(code[0].contains("line two"), "the indented block was cut: {:?}", code[0]);
        assert!(!code[0].contains("back to prose"));
    }

    /// An HTML comment may span blank lines, and this repo's own documents
    /// contain them. Cut, the closing `-->` renders as text.
    #[test]
    fn an_html_comment_is_one_block_across_a_blank_line() {
        let src = "a\n\n<!-- a note\n\nstill the note\n-->\n\nb\n";
        let bs = blocks(src);
        let c: Vec<&str> = bs.iter().map(|b| b.md.as_str()).filter(|m| m.contains("<!--")).collect();
        assert_eq!(c.len(), 1);
        assert!(c[0].contains("-->"), "the comment was cut: {:?}", c[0]);
    }

    /// `---` under a paragraph is a setext H2 underline, not a thematic break.
    /// Cutting there puts the heading's text in one block and its underline in
    /// the next, and each renders as a paragraph and a horizontal rule — a
    /// heading silently demoted to prose.
    #[test]
    fn a_setext_heading_is_not_cut_from_its_underline() {
        let src = "Heading text\n---\n\nbody\n";
        let bs = blocks(src);
        assert!(bs[0].md.starts_with("Heading text\n---"), "{:?}", bs[0].md);
    }

    /// The property the whole design rests on: a block the author did not touch
    /// hands the page the SAME id, so the page leaves its DOM node — and the
    /// reader's selection and scroll — alone.
    ///
    /// Insertion is the case a positional `b0, b1, b2…` gets wrong, and it is
    /// the most ordinary edit there is: everything below the insert shifts, so
    /// every block reads as changed and the page replaces the document.
    #[test]
    fn inserting_a_paragraph_changes_no_other_blocks_id() {
        let before = "# Title\n\nfirst\n\nsecond\n\nthird\n";
        let after = "# Title\n\nINSERTED\n\nfirst\n\nsecond\n\nthird\n";
        // An id must still NAME THE SAME TEXT, not merely still exist: a
        // positional scheme keeps every id alive and slides each one onto its
        // neighbour's content, which is the whole failure and passes a
        // membership check perfectly. (It did, while this test was being
        // written.)
        let a = blocks(before);
        let b: std::collections::BTreeMap<String, String> =
            blocks(after).into_iter().map(|x| (x.id, x.md)).collect();
        for blk in &a {
            assert_eq!(
                b.get(&blk.id),
                Some(&blk.md),
                "{} no longer names {:?} after an insertion above it",
                blk.id,
                blk.md
            );
        }
        assert_eq!(b.len(), a.len() + 1);
    }

    /// Editing one block changes one id. If an edit moved its neighbours' ids
    /// the page would redraw them too, and the measurement this split exists
    /// for (`sel 55→55`) would quietly become `sel 55→0`.
    #[test]
    fn editing_one_block_moves_one_id() {
        let a: Vec<String> = blocks("one\n\ntwo\n\nthree\n").into_iter().map(|b| b.id).collect();
        let b: Vec<String> = blocks("one\n\nTWO\n\nthree\n").into_iter().map(|b| b.id).collect();
        assert_eq!(a.len(), b.len());
        let moved = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert_eq!(moved, 1, "{a:?} vs {b:?}");
    }

    /// Two blocks with identical source are two nodes on the page, so they need
    /// two keys. A content-derived id collides by construction; the occurrence
    /// counter is what stops the page treating them as one.
    #[test]
    fn identical_blocks_still_get_distinct_ids() {
        let bs = blocks("---\n\n---\n\n---\n");
        let ids: std::collections::BTreeSet<&str> = bs.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(bs.len(), 3);
        assert_eq!(ids.len(), 3, "{:?}", bs.iter().map(|b| &b.id).collect::<Vec<_>>());
    }

    /// `document` is `header` + `body`, and the split is what lets the server
    /// hand the page the facts as DATA (`doc`'s `meta`) without the reader
    /// getting a second, frozen copy of them as prose.
    #[test]
    fn a_document_is_its_header_followed_by_its_body() {
        let e = entry("docs/a.md", "A");
        let st = stale();
        let text = "---\ntitle: A\n---\n\nprose\n";
        assert_eq!(document(&e, text, &st, &Links::NONE), format!("{}{}", header(&st), body(&e, text, &Links::NONE)));
        // And the body carries none of the header's facts: the page renders
        // those itself, live, from `meta`.
        let b = body(&e, text, &Links::NONE);
        assert!(!b.contains("origin/main"), "{b:?}");
        assert!(!b.contains("behind"), "{b:?}");
        assert!(b.contains("prose"));
    }

}

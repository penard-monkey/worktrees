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

/// One document, transformed. `entry` is the row `docs::index_with` produced for
/// it, `text` is the file's content, and nothing else is read.
pub fn document(entry: &DocEntry, text: &str, stale: &Staleness, links: &Links) -> String {
    let (had_front, body) = match docs::split_frontmatter(text) {
        Some((_, rest)) => (true, rest),
        None => (false, text),
    };
    let mut out = header(stale);
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

    #[test]
    fn a_document_with_no_diagram_is_returned_as_it_was() {
        let src = "# Title\n\nsome prose\n\n- a list\n\n```rust\nfn main() {}\n```\n";
        assert_eq!(diagrams(src, &Links::NONE), src);
        // including the final newline, or its absence: `str::lines` cannot tell
        // them apart and a transform that adds one is editing the file.
        assert_eq!(diagrams("# Title\n\nno trailing newline", &Links::NONE), "# Title\n\nno trailing newline");
    }
}

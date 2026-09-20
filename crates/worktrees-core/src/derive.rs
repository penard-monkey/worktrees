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
//! 2. **Restore the title** the frontmatter carried, when stripping it took the
//!    document's only one.
//!
//! The staleness facts (§1.1: a document that does not say which place it is
//! from reads as current when it is 53 commits old — seven of valleos's eleven
//! places carry the pre-restructure `docs/` tree and four carry the new one)
//! are NOT a transform. They are `Staleness`, which the app fills and the
//! server hands the page as DATA (`doc`'s `meta`), rendered live beside the
//! text. A markdown copy composed into the body would be a second, frozen
//! answer two inches from the first. What must never drift is the fact set and
//! the ref `behind` is counted against — the project's BASE ref, never
//! `Place::upstream` (§11.4 has the evidence and the draft had it wrong).
//!
//! 3. **Strip every author-supplied mermaid `click` directive**, then emit our
//!    own. §5.3 rule 1, and it is security rather than tidiness: strict mode
//!    blocks `click … call fn()` but renders `click NODE "<href>"` as a real
//!    anchor, and that href was written by a repo this tool will happily open
//!    five seconds after cloning it. Only tool-emitted targets survive, and by
//!    §5.3 rule 2 a target that is not a same-page `#/` fragment is not a
//!    target (`is_fragment_target`).
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

/// The facts the page states about the place a document came from, as the app
/// already holds them — handed to the browser as `doc`'s `meta` and rendered
/// there, never composed into the document's text (see the module note).
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
    /// timer. Two ages, so the page shows two ages: the reader is told when the
    /// text was copied and, separately, when the status was measured.
    ///
    /// A copy that cannot say how old it is, in a feature whose whole purpose is
    /// to stop people reading stale documents, is §1.1's failure one level in.
    ///
    /// `0` means unknown and shows nothing — the same convention the page uses
    /// for a missing commit epoch, and the reason `Default` is safe here.
    pub derived_epoch: i64,
}

/// The drill-down seam: the index to resolve a diagram node against, and the
/// function that turns a resolved page into a URL.
///
/// `url` is the whole viewer-specific surface of this module. It returns `None`
/// for a page the viewer is not serving, and whatever it returns is still put
/// through `is_fragment_target` before anything is emitted — the rule is
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

/// One document, transformed: frontmatter stripped, the title restored if
/// stripping took it, diagram `click` directives rewritten.
///
/// This is what the browser page is served as blocks, and it is the WHOLE
/// transform: there is no second entry point that prepends the staleness facts
/// as prose. The page renders them itself, LIVE, from `doc`'s `meta`
/// (`viewer.rs`), and a copy baked into the text would be a second, frozen copy
/// of numbers the reader is looking at two inches away — §1.1's hazard with the
/// axes swapped. A markdown header composed here existed for a self-contained
/// `.md` file that nothing ever asked for; it was dead code carrying a
/// security-shaped promise about author-controlled strings, which is the worst
/// kind to keep warm.
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

/// One line of plain text, safe to write into a document we are generating.
///
/// `body`'s one call site is the restored title, and `DocEntry::title` is
/// author-controlled: it comes from a `title:` key or an H1 in a repository the
/// user may have cloned seconds ago. It is written as `# {title}`, so a newline
/// in it ends the heading and everything after it continues in the document's
/// own voice, and a backtick opens a code span the rest of the page then lives
/// inside. Collapse the whitespace, drop the controls, and neuter the one
/// character that can open a span.
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
///
/// Indented by at most three, because CommonMark makes the fourth space an
/// indented CODE BLOCK whose content is literal. `blocks::fence_at` has always
/// said so; this said any indentation would do, and the two readings of one
/// line disagreed in both directions — an indented EXAMPLE of a diagram was
/// rewritten as if it were one, and an indented fence that never closed
/// swallowed the rest of the document into a diagram nobody wrote.
fn opens(line: &str) -> Option<(char, usize, String)> {
    if indent_of(line) > 3 {
        return None;
    }
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

/// Split a line on top-level `;`. A `;` inside a label is part of the label,
/// and splitting there cuts a node in half.
///
/// Quotes are not enough, because the quotes are optional. Mermaid 12 lexes
/// label text as `/^(?:[^\[\]\(\)\{\}\|\"]+)/` — every character but the
/// delimiters themselves, `;` included — so `A[Step 2; click Save]` is ONE
/// vertex with an ordinary English label. Split on that `;` and `declick`
/// deletes the half beginning `click`, leaving `A[Step 2`, which mermaid
/// refuses outright (`Expecting 'SQE'`). The rule that exists to protect the
/// diagram is then the only thing that broke it.
///
/// So bracket depth, the way `labels_off` already tracks it. What is
/// deliberately NOT tracked is the pipe form (`A -->|text| B`): `|` is a
/// toggle rather than a pair, so an odd one anywhere on a line would suppress
/// splitting for the whole rest of it — and this split is what finds a `click`
/// hidden behind a `;`. A `;` inside a pipe label is still split; the cost is a
/// diagram, the alternative risks an author-controlled href.
fn statements(line: &str) -> Vec<&str> {
    let b = line.as_bytes();
    let (mut out, mut start, mut quoted, mut depth) = (Vec::new(), 0usize, false, 0i32);
    // ASCII-only tests, so no index here can land inside a multi-byte char.
    for i in 0..b.len() {
        match b[i] {
            b'"' => quoted = !quoted,
            _ if quoted => {}
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => depth = (depth - 1).max(0),
            b';' if depth == 0 => {
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
/// line and being wrong in the other costs an author-controlled href.
///
/// Two things collide with the keyword, and mermaid's own lexer separates both
/// with one rule: its CLICK token is `/^(?:click[\s]+)/`, the word only when
/// WHITESPACE follows. So `click[User clicks Save] --> save` is an ordinary
/// vertex whose id happens to be the word — accepting `[` as the second token
/// deleted the node and its edge from a diagram this module only meant to read
/// — and `click --> B` is an edge whose node is called `click`, which the
/// LINK_CHARS test still catches after the space. `click-->B` does not even
/// reach either check: the head runs to the `-`.
fn is_click(stmt: &str) -> bool {
    let t = stmt.trim_start();
    let head: String = t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    if !head.eq_ignore_ascii_case("click") {
        return false;
    }
    let rest = &t[head.len()..];
    if !rest.starts_with(char::is_whitespace) {
        return false;
    }
    match rest.trim_start().chars().next() {
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
        // Rule 2, enforced HERE rather than trusted to the caller that built
        // the string: whatever the seam hands back, only a same-page fragment
        // is ever written into a diagram.
        if is_fragment_target(&url) {
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
/// **This scan can miss a node, and it can also invent one** — the second half
/// used to be denied here, and the denial was wrong. `labels_off` only removes
/// a label that is bracketed, quoted or piped; mermaid's OTHER edge-label
/// spelling has none of those (`A -- text --> B`), so `text` survives into the
/// tokeniser and stands where a node id would. Hand that diagram a place with a
/// `text.md` in it and a `click text` is emitted for a node that does not
/// exist.
///
/// It is left alone rather than fixed, because what the emitted directive costs
/// is bounded and small: mermaid ignores a `click` naming an unknown id, so the
/// visible result is nothing at all, and the id still had to name exactly one
/// page in this place's own index before a URL was even asked for. Narrowing it
/// means teaching this function the unquoted edge-label grammar, which is the
/// mermaid parser this module exists not to be. Recorded so the next reader
/// does not take "cannot invent one" as a promise something else may lean on.
///
/// One more mismatch worth stating, for the same reason: `seen` dedupes on the
/// LOWERCASED id while mermaid ids are case-sensitive. `Overview` and
/// `OVERVIEW` are two nodes to mermaid and one to this, so only the first is
/// offered a directive. That is the safe direction (a missing link, not a wrong
/// one) and it matches `slug`, which is lowercased too — but it is a real
/// divergence from the grammar, not a coincidence.
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

/// §5.3 rule 2: a target that is not a SAME-PAGE FRAGMENT is not a target.
///
/// The drill-down used to bake `http://127.0.0.1:<port>/<token>/…` into the
/// derived text, which made every emitted href a string that could point
/// anywhere — so the rule was a loopback check, and it had to enumerate the
/// shapes a prefix test passes (`127.0.0.1.evil.example`, a userinfo `@`, a
/// `javascript:` scheme, a `\` a browser normalises). A fragment has no
/// authority to be wrong about: `#/<rel>` is a route inside the page that is
/// already open, it names no host, opens no socket, and survives the port
/// changing on the next launch.
///
/// What is left to check is therefore small and exact, and it is still checked
/// on the way OUT rather than trusted to the caller:
///
/// - it must BEGIN `#/` and name something after it. Not `#x` (an in-page
///   anchor, which is the author's business and not a drill-down), not a bare
///   `#`, not `#/` alone — a route to no page is a link that silently does
///   nothing — and not anything with a scheme: a `javascript:` or `http://`
///   target is refused by the same test that accepts the only form this module
///   emits.
/// - it may not carry a quote, a backslash, an angle bracket, a backtick,
///   whitespace or a control character. The emitted form is
///   `click ID href "<url>"`, so a quote or a newline in the target would not
///   be a bad link — it would be a SECOND directive, with an href nobody here
///   wrote.
pub fn is_fragment_target(url: &str) -> bool {
    if !url.starts_with("#/") || url.len() <= 2 || url.len() > 2048 {
        return false;
    }
    !url.chars().any(|c| c.is_whitespace() || c.is_control() || "\"'\\<>`".contains(c))
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
        // An unclosed `![` is not a reference, but the rest of the line may
        // still hold one — `![ oops and ![real](a.png)` used to yield NOTHING,
        // so one stray bracket rendered every image after it broken. Step past
        // the marker (never past `i` alone, or this does not terminate) and
        // keep scanning.
        let Some(close) = close else {
            i += 2;
            continue;
        };
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
///   The `.git` test is case-INSENSITIVE: on a case-insensitive volume — this
///   machine's APFS — `.GIT` names that same directory, so the spelling the
///   filesystem answers to is the one to refuse. The extension allow-list and
///   the canonical containment check downstream make the practical impact ~0;
///   the point is that both layers state the same rule.
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
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || p.eq_ignore_ascii_case(".git")) {
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
            _ if comp.eq_ignore_ascii_case(".git") => return None,
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

    /// The form `viewer::url_for` produces: a SAME-PAGE fragment, so a
    /// drill-down is a route change in the page already open rather than a
    /// navigation to a baked-in port.
    fn viewer(e: &DocEntry) -> Option<String> {
        Some(format!("#/{}", e.rel))
    }

    // ── 1. frontmatter ───────────────────────────────────────────────────────

    #[test]
    fn frontmatter_goes_away_and_the_title_it_carried_comes_back_as_a_heading() {
        let e = entry("docs/a.md", "From Frontmatter");
        let src = "---\ntitle: From Frontmatter\nlayout: page\n---\n\nprose here\n";
        let out = body(&e, src, &Links::NONE);
        assert!(!out.contains("layout: page"), "the metadata block survived: {out}");
        assert!(out.starts_with("# From Frontmatter"), "the only title the document had is gone: {out}");
        assert!(out.contains("prose here"));
        // …and it is NOT re-added when the body already has its own H1.
        let src = "---\ntitle: From Frontmatter\n---\n# An H1\n";
        let out = body(&e, src, &Links::NONE);
        assert_eq!(out.matches("# ").count(), 1, "two titles: {out}");
        // …nor invented for a document that never had one.
        let out = body(&entry("docs/b.md", "b"), "just prose\n", &Links::NONE);
        assert!(!out.contains("# b"), "an H1 the author never wrote: {out}");
    }

    /// An opening `---` with no closing one is a thematic break, not a block
    /// that runs to EOF. `title_from` could afford that reading (it loses a
    /// title); the derive cannot (it loses the document).
    #[test]
    fn an_unterminated_rule_is_not_frontmatter_and_the_document_survives() {
        let src = "---\n\n# The Real Title\n\nthe whole document\n";
        let out = body(&entry("docs/a.md", "a"), src, &Links::NONE);
        assert!(out.contains("the whole document"), "the document was deleted: {out}");
        assert_eq!(docs::title_from(src).as_deref(), Some("The Real Title"));
    }

    /// `DocEntry::title` is author-controlled — a `title:` key or an H1 out of
    /// a repository cloned seconds ago — and the restore writes it as
    /// `# {title}`. A newline in it ends the heading, and everything after it
    /// continues in the DOCUMENT's own voice rather than the author's; a
    /// backtick opens a code span the rest of the page then lives inside.
    #[test]
    fn an_author_controlled_title_cannot_break_out_of_the_heading_it_is_restored_into() {
        let e = entry("docs/a.md", "ok`\n\n## Trusted subheading\n\nfine print");
        let out = body(&e, "---\ntitle: x\n---\n\nthe real document\n", &Links::NONE);
        let first = out.lines().next().unwrap_or("");
        assert!(first.starts_with("# "), "the restore is a heading: {out:?}");
        // Exactly ONE heading, and the words survive as WORDS inside it.
        assert_eq!(out.lines().filter(|l| l.trim_start().starts_with('#')).count(), 1, "a heading in the tool's voice: {out:?}");
        assert!(first.contains("Trusted subheading"), "the title was not shown at all: {out:?}");
        assert_eq!(first.matches('`').count() % 2, 0, "unbalanced code spans: {first:?}");
        assert!(out.contains("the real document"));
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

    /// The UNQUOTED spelling, which is the one an author writes. Mermaid 12's
    /// lexer reads label text with `/^(?:[^\[\]\(\)\{\}\|\"]+)/` — `;` is
    /// ordinary label text — so a quote-only split cuts the node in half, and
    /// `declick` then deletes the second half as a directive. What survives is
    /// `  A[Step 2`, which mermaid refuses with `Expecting 'SQE'`: the rule that
    /// exists to protect the diagram is the thing that broke it.
    #[test]
    fn a_semicolon_inside_an_unquoted_label_is_not_a_statement_separator() {
        let src = "```mermaid\nflowchart TD\n  A[Step 2; click Save] --> B\n```\n";
        let out = diagrams(src, &Links::NONE);
        assert!(out.contains("A[Step 2; click Save] --> B"), "a label was cut in half:\n{out}");
        // …and the same for the other two bracket shapes the lexer excludes.
        for (open, close) in [("(", ")"), ("{", "}")] {
            let src = format!("```mermaid\nflowchart TD\n  A{open}Step 2; click Save{close} --> B\n```\n");
            let out = diagrams(&src, &Links::NONE);
            assert!(out.contains(&format!("A{open}Step 2; click Save{close} --> B")), "cut in half:\n{out}");
        }
        // The protection itself is unchanged: a real second statement still goes.
        let src = "```mermaid\nflowchart TD\n  A[Step 2] --> B; click A \"https://evil.example\"\n```\n";
        let out = diagrams(src, &Links::NONE);
        assert!(!out.contains("evil.example"), "a directive sharing a line survived:\n{out}");
        assert!(out.contains("A[Step 2] --> B"), "{out}");
    }

    /// Mermaid's CLICK token is `/^(?:click[\s]+)/` — the keyword only when
    /// whitespace follows. `click[User clicks Save] --> save` is an ordinary
    /// vertex whose id happens to be the word, and stripping it deletes the node
    /// AND its edge from a diagram this tool only meant to read.
    #[test]
    fn a_node_whose_id_is_click_keeps_its_label_and_its_edge() {
        let src = "```mermaid\nflowchart TD\n  click[User clicks Save] --> save\n  clickable(Also fine) --> B\n```\n";
        let out = diagrams(src, &Links::NONE);
        assert!(out.contains("click[User clicks Save] --> save"), "a vertex was read as a directive:\n{out}");
        assert!(out.contains("clickable(Also fine) --> B"), "{out}");
    }

    /// The wart `node_ids` documents, pinned so the docstring is a fact rather
    /// than a recollection. An unquoted edge label is NOT bracketed, quoted or
    /// piped, so `labels_off` cannot remove it and the word stands where a node
    /// id would — this scan can invent a node after all. It is harmless and
    /// bounded (mermaid ignores a `click` naming an unknown id) and the fix is
    /// a mermaid parser, so the behaviour stays and the claim is corrected.
    #[test]
    fn an_unquoted_edge_label_can_become_a_node_that_was_never_declared() {
        let entries = vec![entry("docs/text.md", "Text")];
        let links = Links { entries: &entries, url: &viewer };
        let out = diagrams("```mermaid\nflowchart TD\n  A -- text --> B\n```\n", &links);
        assert!(out.contains("click text href \"#/docs/text.md\""), "the docstring's wart is gone; update it:\n{out}");
        // The bracketed and piped spellings are removed, which is why this one
        // is a gap rather than the rule.
        for src in [
            "```mermaid\nflowchart TD\n  A[see text] --> B\n```\n",
            "```mermaid\nflowchart TD\n  A -->|text| B\n```\n",
            "```mermaid\nflowchart TD\n  A -- \"text\" --> B\n```\n",
        ] {
            assert!(!diagrams(src, &links).contains("click text"), "a label became a node: {src}");
        }
    }

    /// `seen` lowercases; mermaid ids do not. Two nodes to the renderer, one
    /// here — so the second is never offered a directive. A missing link rather
    /// than a wrong one, which is the direction to be wrong in.
    #[test]
    fn ids_differing_only_in_case_are_one_node_to_this_scan() {
        let entries = vec![entry("docs/overview.md", "Overview")];
        let links = Links { entries: &entries, url: &viewer };
        let out = diagrams("```mermaid\nflowchart TD\n  overview --> A\n  OVERVIEW --> B\n```\n", &links);
        assert_eq!(out.matches("href").count(), 1, "both cases were linked: {out}");
        assert!(out.contains("click overview href"), "the FIRST spelling is the one offered: {out}");
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
        assert!(out.contains("click overview href \"#/docs/overview.md\""), "{out}");
        assert!(out.contains("click billing href \"#/docs/billing.md\""), "{out}");
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

    /// §5.3 rule 2, as the fragment drill-down restates it. The emitter refuses
    /// everything that is not a same-page `#/` route — including the loopback
    /// URLs it used to be the only thing that accepted, because a derived page
    /// that names a port is a page that stops working at the next launch.
    #[test]
    fn only_a_same_page_fragment_target_is_emitted() {
        for bad in [
            "http://127.0.0.1:6275/live-docs?file=docs/a.md",
            "http://localhost:6275/x",
            "https://evil.example/x",
            "javascript:fetch(\"#/docs/a.md\")",
            "data:text/html,<script>x</script>",
            "file:///etc/passwd",
            "//127.0.0.1/x",
            "#docs/a.md",
            "#",
            "#/",
            "/docs/a.md",
            "docs/a.md",
            "#/docs/a.md\" \"tip\" _blank\nclick B href \"#/evil",
            "#/docs/a b.md",
            "#/docs/a\\b.md",
            "#/<script>",
            "",
        ] {
            assert!(!is_fragment_target(bad), "accepted {bad:?}");
        }
        for good in ["#/docs/a.md", "#/README.md", "#/docs/a%20b.md", "#/.planning/brief.md"] {
            assert!(is_fragment_target(good), "refused {good:?}");
        }
        // `#/` alone is a route to nothing, and 2 KiB is the cap.
        assert!(!is_fragment_target(&format!("#/{}", "a".repeat(3000))));
    }

    #[test]
    fn a_url_from_the_seam_that_is_not_a_fragment_is_dropped_not_emitted() {
        let entries = vec![entry("docs/overview.md", "Overview")];
        let evil = |_: &DocEntry| Some("http://evil.example/x".to_string());
        let links = Links { entries: &entries, url: &evil };
        let out = diagrams("```mermaid\nflowchart TD\n  overview --> b\n```\n", &links);
        assert!(!out.contains("click"), "the seam's word was taken for it:\n{out}");
        assert!(!out.contains("evil.example"), "{out}");
    }

    /// The emitted form is `click <id> href "<url>"`. A URL carrying a quote
    /// would not be a bad link — it would be a SECOND directive, with an href
    /// nobody here wrote. Being a fragment is not enough on its own.
    #[test]
    fn a_target_that_would_become_two_directives_is_not_one() {
        let entries = vec![entry("docs/overview.md", "Overview")];
        let sneaky = |_: &DocEntry| Some("#/x\"_\"tip\"".to_string());
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
            // the repo's own git directory — and on this machine's APFS,
            // `.GIT` IS that directory, so the component test is spelled the
            // way the filesystem reads it rather than the way it is written.
            ".git/config",
            "../.git/config",
            "docs/.git/objects/x.png",
            "../.GIT/config",
            "../.Git/hooks/x.png",
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
    /// An unclosed `![` is not a reference — but it used to take the REST of
    /// the line with it, so one stray bracket hid every real image after it and
    /// the page rendered them all broken.
    #[test]
    fn an_unclosed_image_marker_does_not_hide_the_rest_of_the_line() {
        let mut out = Vec::new();
        inline_images("![ oops and ![real](a.png)", &mut out);
        assert_eq!(out, vec!["a.png"], "the scan gave up at the unclosed marker");
        let mut out = Vec::new();
        inline_images("![a](one.png) then ![b and ![c](two.png)", &mut out);
        assert_eq!(out, vec!["one.png", "two.png"]);
        // …and a line that is nothing but unclosed markers still terminates.
        let mut out = Vec::new();
        inline_images("![![![", &mut out);
        assert!(out.is_empty());
    }

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

    /// CommonMark stops an opening fence at three spaces of indentation: four
    /// makes it an indented CODE BLOCK, whose content is literal. `blocks`'
    /// `fence_at` already knows that and `opens` did not, so the two disagreed
    /// about the same line — an indented EXAMPLE of a diagram was edited as if
    /// it were one, and an indented unterminated fence swallowed the rest of
    /// the file.
    #[test]
    fn a_fence_indented_four_spaces_is_a_code_block_not_a_diagram() {
        // An example of a mermaid diagram, shown as literal text.
        let src = "Write it like this:\n\n    ```mermaid\n    flowchart TD\n    click A \"https://example.com\"\n    ```\n\nand it renders.\n";
        assert_eq!(diagrams(src, &Links::NONE), src, "an indented code block was rewritten");
        // …and one that is never closed does not eat the document either.
        let src = "    ```mermaid\n    flowchart TD\n\nreal prose after it\n";
        assert_eq!(diagrams(src, &Links::NONE), src, "an indented unterminated fence swallowed the file");
        // Three spaces is still a fence, which is the boundary this is about.
        let src = "   ```mermaid\n   flowchart TD\n   click A \"https://evil.example\"\n   ```\n";
        assert!(!diagrams(src, &Links::NONE).contains("evil.example"), "three spaces is a real fence");
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
}

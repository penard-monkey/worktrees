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
//! started: homelab-ansible · jellyfin-role · 2026-11-12
//! ```
//!
//! - A project line is `- <name> (<kind>)`; the kind is the LAST
//!   parenthesised group, so a name may hold parentheses of its own.
//! - A work item starts at `### <title>` and holds `project:`, `branch:`,
//!   `brief:` (`|` and an indented block, or one line) and `started:`.
//!   `started:` is phase 5's (the app writes it when a place starts); this
//!   reader keeps its text and judges nothing in it.
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
    /// Phase 5's provenance line, as written.
    pub started: Option<String>,
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
const WORK_KEYS: &str = "`project:`, `branch:`, `brief:` or `started:`";

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
                    item = Some(WorkItem { line: l.n, title: title.to_string(), ..Default::default() });
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
                    "started" => &mut it.started,
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
}

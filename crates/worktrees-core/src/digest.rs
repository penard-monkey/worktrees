//! "Where you left off", by rule (layer 0 of
//! `docs/proposals/where-you-left-off-plan.md`, stage 1): on asking, a short
//! ranked list of the places that want the owner, across every registered
//! project, built only from what is already on disk and in tmux.
//!
//! Nothing here starts a harness, runs git over the network or writes a file.
//! Every fact on a [`Card`] comes from a reader the snapshot already uses:
//! `Project::ls` (branch, dirty files, divergence, last commit), the declared
//! store (`store::Declared`: title, lifecycle, worked and seen stamps), the
//! one activity derivation (`harness::place_activities` → `activity::
//! place_answer`, the same answer as the nav dots and MCP `place_status`), the
//! health verdict (`health::assess`), the plan summary, and — handed in by the
//! caller, because both live in serve's process — the pull-request cache and
//! the unsent drafts ([`Extras`]). The last prompt is read through `turns`
//! only, cut to [`PROMPT_CHARS`], and only for the rows an answer carries
//! whose place was active in the last [`RECENT_SECS`].
//!
//! Privacy: a private project, and a project nested in one, gives no card and
//! is counted ([`LeftOut::private`]). The rule is `workspacechats::
//! project_reads`', applied to a workspace holding every registered project
//! ([`split`]), so there is one copy of it.
//!
//! The clock is a parameter everywhere ([`gather_in`], [`answer_in`]): tests
//! script time.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::activity::{self, State};
use crate::registry::{Entry, Registry};
use crate::{agent, github, harness, health, store, tmux, Project};

/// Rows an answer carries when the caller names no limit.
pub const LIMIT_DEFAULT: usize = 5;
/// Characters kept of the last prompt.
pub const PROMPT_CHARS: usize = 80;
/// How recent a place's activity must be to count as `recent`, and for its
/// last prompt to be read: the nav's row-dim horizon (`health::STALE_SECS`).
pub const RECENT_SECS: i64 = health::STALE_SECS;
/// Projects whose snapshots run at once. Each `ls` runs up to 16 git calls of
/// its own (`place_json_par`), so this bounds the product. Measured on 17
/// projects, 90 places, at load 7: 6 at once 956 ms, 9 790 ms, 17 820 ms.
const PROJECTS_IN_FLIGHT: usize = 9;

/// Why a place is on the list. The order is the ranking's first key.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    /// An agent there is stopped on the owner.
    Waiting,
    /// An agent finished work there after the owner last looked.
    Finished,
    /// Its pull request has failing checks, changes requested or conflicts.
    Pr,
    /// Text sits unsent at its agent's prompt.
    Draft,
    /// Untouched for longer than the stale horizon, holding work that exists
    /// only here (`health::assess`'s `at-risk`).
    AtRisk,
    /// Active in the last [`RECENT_SECS`].
    Recent,
}

/// What a place's pull request says, from the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrFact {
    pub number: u64,
    /// `github::Chip::label`: `CI failing`, `changes requested`, …
    pub label: &'static str,
    /// Failing checks, changes requested or conflicts.
    pub needs_you: bool,
    /// GitHub's `updatedAt`, epoch seconds.
    pub updated: Option<i64>,
}

/// One place, as the digest knows it.
#[derive(Clone, Debug, Default)]
pub struct Card {
    /// The registry's name for the project.
    pub project: String,
    /// The project's main root.
    pub repo: String,
    pub slug: String,
    /// The place's directory.
    pub path: String,
    /// The declared title, when one is set.
    pub title: Option<String>,
    /// The activity derivation's answer.
    pub state: Option<State>,
    /// The harness that answered (`claude`, `codex`, …).
    pub provider: Option<&'static str>,
    /// When the agent started waiting (Claude's `statusUpdatedAt`), when known.
    pub waiting_since: Option<i64>,
    /// `last_worked_epoch`: when an agent last finished here.
    pub worked: Option<i64>,
    /// The owner last saw it: `max(last_seen_epoch, last_opened_epoch)`, the
    /// app's `seenEpoch`.
    pub seen: Option<i64>,
    /// The newest of the declared stamps and the last commit: what "recent"
    /// and "newest first" read. Not a transcript's mtime (AGENTS.md: Claude
    /// keeps rewriting its files long after the last turn).
    pub last_active: Option<i64>,
    /// `health::assess`'s verdict: `active`, `parked`, `at-risk`, `cold`.
    pub verdict: String,
    pub dirty_files: u32,
    /// Commits not on the base.
    pub ahead: i64,
    pub base: String,
    pub pr: Option<PrFact>,
    /// Unsent text at the agent's prompt. The digest never shows it.
    pub draft: bool,
    /// The place's Claude history directory, for the last prompt.
    pub claude_dir: Option<String>,
}

impl Card {
    /// "An agent finished here and the owner has not looked since" — the
    /// app's `unseenWork` (`afterglow.ts::isUnread`): a real finish, later
    /// than the last look.
    pub fn unseen_work(&self) -> bool {
        self.worked.is_some_and(|w| w > 0 && w > self.seen.unwrap_or(0))
    }

    /// The card's reason and the time it is dated by, or `None` when the place
    /// wants nothing. The first that holds, in [`Reason`]'s order.
    pub fn reason(&self, now: i64) -> Option<(Reason, i64)> {
        let active = self.last_active.unwrap_or(0);
        if self.state == Some(State::Waiting) {
            return Some((Reason::Waiting, self.waiting_since.unwrap_or(now)));
        }
        // A place that is busy RIGHT NOW is being watched, not left behind
        // (the dot's `unreadOf` says the same).
        if self.unseen_work() && self.state != Some(State::Busy) {
            return Some((Reason::Finished, self.worked.unwrap_or(active)));
        }
        if let Some(p) = self.pr.as_ref().filter(|p| p.needs_you) {
            return Some((Reason::Pr, p.updated.unwrap_or(active)));
        }
        if self.draft {
            return Some((Reason::Draft, active));
        }
        if self.verdict == "at-risk" {
            return Some((Reason::AtRisk, active));
        }
        if self.state == Some(State::Busy) || now - active < RECENT_SECS {
            return Some((Reason::Recent, if self.state == Some(State::Busy) { now.max(active) } else { active }));
        }
        None
    }

    fn recent(&self, now: i64) -> bool {
        self.last_active.is_some_and(|a| now - a < RECENT_SECS)
    }
}

/// One row of the answer: the contract's shape.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Row {
    pub project: String,
    pub repo: String,
    pub slug: String,
    /// The declared title, else the slug.
    pub title: String,
    pub reason: Reason,
    pub line: String,
    pub at: i64,
}

/// Projects that gave no card.
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// Private, or nested inside a private project.
    pub private: usize,
    /// Could not be read (gone, not a repository).
    pub unreadable: usize,
}

/// The `digest` row's answer.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Answer {
    pub generated: i64,
    pub changed: bool,
    /// Cards with a reason, before `offset` and `limit`.
    pub total: usize,
    pub rows: Vec<Row>,
    pub left_out: LeftOut,
}

/// What the caller holds that core cannot read for itself: serve's pull
/// request cache and its draft scan. Both may be empty (the CLI has no cache).
#[derive(Default)]
pub struct Extras {
    /// By the project's main root.
    pub prs: HashMap<String, PrSource>,
    /// Unsent text by place directory.
    pub drafts: HashMap<String, String>,
}

/// One project's cached pull requests, as `project::PrCacheEntry` holds them.
pub struct PrSource {
    pub push_owner: String,
    pub snap: github::Snapshot,
}

// ── privacy ─────────────────────────────────────────────────────────────────

/// The registered projects the digest may read, and how many it may not.
/// `workspacechats::project_reads` decides, over a workspace that holds every
/// registered project: a private project, or one inside a private one, is
/// left out. Its other refusals are about whether a PERMISSION RULE can name
/// a path, which is not a digest's question; those projects are read, and
/// one that cannot be is counted by [`gather_in`].
pub fn split(reg: &Registry) -> (Vec<Entry>, LeftOut) {
    use crate::workspacechats as wc;
    let all = crate::workspaces::Workspace {
        projects: reg.projects.iter().map(|e| crate::workspaces::ProjectRef { root: e.root.clone(), ..Default::default() }).collect(),
        ..Default::default()
    };
    let mut out = (Vec::new(), LeftOut::default());
    for (e, r) in reg.projects.iter().zip(wc::project_reads(&all, reg)) {
        if r.private || r.why == Some(wc::WHY_WITHIN) {
            out.1.private += 1;
        } else {
            out.0.push(e.clone());
        }
    }
    out
}

// ── gathering (I/O, read-only) ──────────────────────────────────────────────

/// Every place of every readable project, as a card. One pane list and one
/// probe scan for all of them; `ls` per project, a few at once.
pub fn gather_in(reg: &Registry, now: i64, panes: Option<&tmux::PaneList>, extras: &Extras) -> (Vec<Card>, LeftOut) {
    let (entries, mut left) = split(reg);
    let probes = agent::live_probes_in(panes);
    let mut cards = Vec::new();
    for chunk in entries.chunks(PROJECTS_IN_FLIGHT) {
        let got: Vec<Option<Vec<Card>>> = std::thread::scope(|s| {
            let hs: Vec<_> = chunk.iter().map(|e| s.spawn(|| project_cards(e, now, panes, &probes, extras))).collect();
            hs.into_iter().map(|h| h.join().ok().flatten()).collect()
        });
        for g in got {
            match g {
                Some(c) => cards.extend(c),
                None => left.unreadable += 1,
            }
        }
    }
    (cards, left)
}

/// One project's cards, or `None` when it cannot be read.
fn project_cards(e: &Entry, now: i64, panes: Option<&tmux::PaneList>, probes: &[agent::ClaudeProbe], extras: &Extras) -> Option<Vec<Card>> {
    if !Path::new(&e.root).is_dir() {
        return None;
    }
    let project = Project::discover(Path::new(&e.root)).ok()?;
    let ls = project.ls();
    let st = store::read_lenient(&project.main_root);
    let base = project.base_ref();
    let scan = harness::Scan { probes, panes };
    let pr_view = extras.prs.get(&project.main_root).or_else(|| extras.prs.get(&e.root)).map(|p| {
        let branches: Vec<github::PlaceBranch> = ls
            .places
            .iter()
            .filter_map(|pl| pl.branch.clone().map(|branch| github::PlaceBranch { slug: pl.slug.clone(), branch }))
            .collect();
        github::view(&p.snap, &p.push_owner, &branches)
    });
    let mut out = Vec::new();
    for pl in ls.places.iter().filter(|p| p.registered) {
        let decl = st.places.get(&pl.slug);
        let life = store::reconcile(decl, pl.tmux_session.up, now);
        let readings = harness::place_activities(&project, &pl.slug, &pl.path, &scan);
        let here = panes.map(|p| project.place_panes(&pl.slug, p));
        let act = activity::place_answer(readings.into_iter().map(|(_, x)| x), here.as_deref(), &project.session_name(&pl.slug));
        let waiting = act.state == State::Waiting;
        // Set aside by the owner: shown again only when an agent there asks.
        if matches!(life.as_str(), "archived" | "abandoned") && !waiting {
            continue;
        }
        let waiting_since = if waiting && act.provider == Some("claude") {
            agent::agents_at(probes, &pl.path)
                .first()
                .and_then(|a| probes.iter().find(|p| p.pid == a.pid))
                .and_then(|p| p.status_updated_at)
                .map(|ms| ms / 1000)
        } else {
            None
        };
        let worked = decl.and_then(|d| d.last_worked_epoch);
        let opened = decl.and_then(|d| d.last_opened_epoch);
        let seen = [decl.and_then(|d| d.last_seen_epoch), opened].into_iter().flatten().max();
        let last_active = [worked, seen, pl.last_commit_epoch].into_iter().flatten().max();
        let facts = health::HealthFacts {
            slug: pl.slug.clone(),
            branch: pl.branch.clone(),
            base: base.clone(),
            created_epoch: pl.created_epoch,
            last_commit_epoch: pl.last_commit_epoch,
            last_commit_subject: pl.last_commit_subject.clone(),
            last_opened_epoch: opened,
            last_worked_epoch: worked,
            claude_last_epoch: pl.claude_session_dir.as_deref().and_then(health::claude_last_epoch),
            dirty_files: pl.dirty_files.unwrap_or(0),
            ahead: pl.ahead.unwrap_or(0),
            behind: pl.behind.unwrap_or(0),
            upstream: pl.upstream.clone(),
            tmux_up: pl.tmux_session.up,
            lifecycle_effective: life,
            ..Default::default()
        };
        let (verdict, _) = health::assess(&facts, now, &|e| e.to_string());
        let pr = pr_view.as_ref().and_then(|v| pr_fact(v, &pl.slug));
        out.push(Card {
            project: e.name.clone(),
            repo: project.main_root.clone(),
            slug: pl.slug.clone(),
            path: pl.path.clone(),
            title: decl.and_then(|d| d.title.clone()).filter(|t| !t.trim().is_empty()),
            state: Some(act.state),
            provider: act.provider,
            waiting_since,
            worked,
            seen,
            last_active,
            verdict,
            dirty_files: facts.dirty_files,
            ahead: facts.ahead,
            base: base.clone(),
            pr,
            draft: extras.drafts.contains_key(&pl.path),
            claude_dir: pl.claude_session_dir.clone(),
        });
    }
    Some(out)
}

/// The pull request `view` maps to `slug`, as a fact.
fn pr_fact(v: &github::View, slug: &str) -> Option<PrFact> {
    let n = *v.places.get(slug)?;
    let row = v.open.iter().chain(v.recent.iter()).find(|r| r.number == n)?;
    let needs_you = matches!(row.chip, github::Chip::Failing | github::Chip::ChangesRequested | github::Chip::Conflicts);
    Some(PrFact { number: n, label: row.label, needs_you, updated: crate::sysclock::parse_iso8601(&row.updated_at) })
}

// ── the answer (pure, given the cards and a prompt reader) ──────────────────

/// Has any place finished work, started waiting or changed pull-request
/// state after `since`? With no `since`, whether there is anything to show.
/// A waiting agent whose start is not known counts: the panel would rather
/// open once too often than hide a question.
pub fn changed(cards: &[Card], since: Option<i64>, any_rows: bool) -> bool {
    let Some(s) = since else { return any_rows };
    cards.iter().any(|c| {
        c.worked.is_some_and(|w| w > s)
            || (c.state == Some(State::Waiting) && c.waiting_since.is_none_or(|w| w > s))
            || c.pr.as_ref().and_then(|p| p.updated).is_some_and(|u| u > s)
    })
}

/// Cards with a reason, ranked: reason first, then newest first, then by
/// project and slug so ties are stable.
pub fn rank(cards: &[Card], now: i64) -> Vec<(&Card, Reason, i64)> {
    let mut r: Vec<(&Card, Reason, i64)> = cards.iter().filter_map(|c| c.reason(now).map(|(why, at)| (c, why, at))).collect();
    r.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)).then_with(|| (&a.0.project, &a.0.slug).cmp(&(&b.0.project, &b.0.slug))));
    r
}

/// What a row says, by a fixed template per reason. `prompt` is the place's
/// last prompt, already cut ([`cut`]); `plan` its plan's current step.
pub fn line(c: &Card, why: Reason, at: i64, now: i64, prompt: Option<&str>, plan: Option<&str>) -> String {
    let age = ago(now - at);
    let quoted = |s: &str| format!("\u{201c}{s}\u{201d}");
    match why {
        Reason::Waiting => format!("{} is waiting on your answer ({age})", who(c.provider)),
        Reason::Finished => match prompt {
            Some(p) => format!("Finished {age}, not seen yet: {}", quoted(p)),
            None => format!("Finished {age}, not seen yet"),
        },
        Reason::Pr => {
            let p = c.pr.as_ref();
            format!("Pull request #{}: {}", p.map_or(0, |p| p.number), p.map_or("needs you", |p| p.label))
        }
        Reason::Draft => "A message sits unsent at the prompt".to_string(),
        Reason::AtRisk => {
            let mut parts = Vec::new();
            if c.ahead > 0 {
                parts.push(format!("{} commit{} not on {}", c.ahead, plural(c.ahead), c.base));
            }
            if c.dirty_files > 0 {
                parts.push(format!("{} uncommitted file{}", c.dirty_files, plural(i64::from(c.dirty_files))));
            }
            format!("{}, untouched for {}", parts.join(" and "), ago(now - at).trim_end_matches(" ago"))
        }
        Reason::Recent => {
            let head = if c.state == Some(State::Busy) { "Working now".to_string() } else { format!("Last active {age}") };
            match (prompt, plan) {
                (Some(p), _) => format!("{head}: {}", quoted(p)),
                (None, Some(s)) => format!("{head}, plan at {}", quoted(s)),
                _ => head,
            }
        }
    }
}

/// The answer from gathered cards. `prompt` reads a card's last prompt and
/// `plan` its plan's current step; each is asked only for the rows the answer
/// carries, and the prompt only for a place active in the last
/// [`RECENT_SECS`].
pub fn answer_in(
    cards: &[Card],
    left_out: LeftOut,
    now: i64,
    since: Option<i64>,
    limit: usize,
    offset: usize,
    prompt: impl Fn(&Card) -> Option<String>,
    plan: impl Fn(&Card) -> Option<String>,
) -> Answer {
    let ranked = rank(cards, now);
    let rows: Vec<Row> = ranked
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(c, why, at)| {
            let p = if c.recent(now) && matches!(why, Reason::Finished | Reason::Recent) { prompt(c).map(|p| cut(&p)).filter(|p| !p.is_empty()) } else { None };
            let s = if *why == Reason::Recent && p.is_none() { plan(c).map(|s| cut(&s)).filter(|s| !s.is_empty()) } else { None };
            Row {
                project: c.project.clone(),
                repo: c.repo.clone(),
                slug: c.slug.clone(),
                title: c.title.clone().unwrap_or_else(|| c.slug.clone()),
                reason: *why,
                line: line(c, *why, *at, now, p.as_deref(), s.as_deref()),
                at: *at,
            }
        })
        .collect();
    Answer { generated: now, changed: changed(cards, since, !ranked.is_empty()), total: ranked.len(), rows, left_out }
}

/// The whole digest for `reg` at `now`.
pub fn digest_in(reg: &Registry, now: i64, panes: Option<&tmux::PaneList>, extras: &Extras, since: Option<i64>, limit: usize, offset: usize) -> Answer {
    let (cards, left) = gather_in(reg, now, panes, extras);
    answer_in(&cards, left, now, since, limit, offset, last_prompt, plan_current)
}

/// The place's plan's current step, or its goal.
pub fn plan_current(c: &Card) -> Option<String> {
    let s = crate::plan::summarize_place(Path::new(&c.path));
    s.current.or(s.goal).filter(|t| !t.trim().is_empty())
}

/// The newest prompt a person wrote in this place, from its newest Claude
/// transcript or its newest Codex rollout, through `turns`. Engine-launched
/// openers and Claude's injected notices are not prompts.
pub fn last_prompt(c: &Card) -> Option<String> {
    let mut best: Option<(i64, String)> = None;
    let mut consider = |path: &Path, harness: &str| {
        let Ok(lines) = crate::turns::window(path, harness) else { return };
        let openers = [crate::turns::Opener { engine: crate::turns::Engine::Start, text: crate::ops::BRIEF_OPENER.to_string() }];
        let (turns, _) = crate::turns::latest(harness, &lines, crate::turns::MAX_TURNS, &openers);
        if let Some(t) = turns.iter().rev().find(|t| t.notice.is_none() && t.engine.is_none() && t.user.as_deref().is_some_and(|u| !u.trim().is_empty())) {
            let at = t.at.unwrap_or(0);
            if best.as_ref().is_none_or(|(b, _)| at > *b) {
                best = Some((at, t.user.clone().unwrap_or_default()));
            }
        }
    };
    if let Some(f) = c.claude_dir.as_deref().and_then(newest_jsonl) {
        consider(&f, crate::provider::CLAUDE.id);
    }
    if let Some(f) = crate::codex::latest_rollout(&c.path) {
        consider(&f, crate::provider::CODEX.id);
    }
    best.map(|(_, p)| p)
}

/// The newest `*.jsonl` in `dir`, by mtime: which conversation is the
/// newest, never when its last turn was (that comes from inside it).
fn newest_jsonl(dir: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

/// One line of at most [`PROMPT_CHARS`] characters: whitespace collapsed,
/// control and invisible format characters dropped, `…` when cut.
pub fn cut(s: &str) -> String {
    let clean: String = s.chars().filter(|c| !is_format(*c)).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let text = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= PROMPT_CHARS {
        return text;
    }
    let mut out: String = text.chars().take(PROMPT_CHARS).collect::<String>().trim_end().to_string();
    out.push('…');
    out
}

/// Bidi and zero-width format characters (Trojan Source).
fn is_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

fn who(provider: Option<&str>) -> &'static str {
    crate::provider::PROVIDERS.iter().find(|p| Some(p.id) == provider).map_or("An agent", |p| p.label)
}

fn plural(n: i64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `5 min ago`, `3 h ago`, `2 days ago`; `just now` under a minute.
pub fn ago(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => {
            let d = s / 86_400;
            format!("{d} day{} ago", plural(d))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const NOW: i64 = 1_795_811_733;

    /// A repo of the test's own under the temp dir, with one commit on `main`.
    fn scratch_repo(base: &Path, name: &str) -> String {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [&["init", "-q", "-b", "main"][..], &["commit", "-q", "--allow-empty", "-m", "init"][..]] {
            let ok = Command::new("git")
                .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false", "-C"])
                .arg(&dir)
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        }
        std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned()
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("wt-digest-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::canonicalize(&base).unwrap()
    }

    /// `(main)` finished work an hour ago that nobody has seen: a card in any
    /// project the digest may read.
    fn finished_here(root: &str) {
        let decl = serde_json::json!({"version": 1, "places": {"(main)": {"last_worked_epoch": NOW - 3600, "title": "Secret plan"}}});
        std::fs::write(Path::new(root).join(".worktrees.places.json"), decl.to_string()).unwrap();
    }

    fn entry(root: &str, name: &str, private: bool) -> Entry {
        Entry { root: root.to_string(), name: name.to_string(), private, ..Default::default() }
    }

    /// A private project's places, and those of a project nested inside it,
    /// reach no row and are counted; a project that is gone is counted apart.
    #[test]
    fn a_private_project_and_one_nested_in_it_give_no_row() {
        let base = scratch("private");
        let open = scratch_repo(&base, "open");
        let private = scratch_repo(&base, "private");
        let nested = scratch_repo(&Path::new(&private).join("sub"), "nested");
        for r in [&open, &private, &nested] {
            finished_here(r);
        }
        let gone = base.join("gone").to_string_lossy().into_owned();
        let reg = Registry {
            projects: vec![entry(&open, "open", false), entry(&private, "private", true), entry(&nested, "nested", false), entry(&gone, "gone", false)],
            ..Default::default()
        };
        let a = digest_in(&reg, NOW, None, &Extras::default(), None, 20, 0);
        let projects: Vec<&str> = a.rows.iter().map(|r| r.project.as_str()).collect();
        assert_eq!(projects, ["open"], "{a:?}");
        assert_eq!(a.rows[0].reason, Reason::Finished);
        assert_eq!(a.rows[0].title, "Secret plan");
        assert_eq!(a.left_out, LeftOut { private: 2, unreadable: 1 });
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains(&private), "a private root reaches no row: {json}");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn card(slug: &str) -> Card {
        Card { project: "p".into(), repo: "/r".into(), slug: slug.into(), path: format!("/r/{slug}"), verdict: "cold".into(), base: "origin/main".into(), ..Default::default() }
    }

    /// One card per reason, given out of order: the answer is in the
    /// contract's order, and two of a reason are newest first.
    #[test]
    fn cards_rank_by_reason_then_newest() {
        let waiting = Card { state: Some(State::Waiting), provider: Some("claude"), waiting_since: Some(NOW - 120), ..card("waiting") };
        let finished = Card { worked: Some(NOW - 600), seen: Some(NOW - 7200), last_active: Some(NOW - 600), ..card("finished") };
        let pr = Card {
            pr: Some(PrFact { number: 7, label: "CI failing", needs_you: true, updated: Some(NOW - 60) }),
            last_active: Some(NOW - 30 * 86_400),
            ..card("pr")
        };
        let draft = Card { draft: true, last_active: Some(NOW - 40 * 86_400), ..card("draft") };
        let risk = Card { verdict: "at-risk".into(), ahead: 2, dirty_files: 1, last_active: Some(NOW - 20 * 86_400), ..card("risk") };
        let old = Card { last_active: Some(NOW - 5 * 86_400), ..card("recent-old") };
        let new = Card { last_active: Some(NOW - 3600), ..card("recent-new") };
        let cold = Card { last_active: Some(NOW - 90 * 86_400), ..card("cold") };
        let seen = Card { worked: Some(NOW - 90 * 86_400), seen: Some(NOW - 89 * 86_400), last_active: Some(NOW - 89 * 86_400), ..card("seen") };
        let cards = vec![old, cold, risk, new, draft, seen, pr, finished, waiting];
        let a = answer_in(&cards, LeftOut::default(), NOW, None, 20, 0, |_| None, |_| None);
        let got: Vec<(&str, Reason)> = a.rows.iter().map(|r| (r.slug.as_str(), r.reason)).collect();
        assert_eq!(
            got,
            [
                ("waiting", Reason::Waiting),
                ("finished", Reason::Finished),
                ("pr", Reason::Pr),
                ("draft", Reason::Draft),
                ("risk", Reason::AtRisk),
                ("recent-new", Reason::Recent),
                ("recent-old", Reason::Recent),
            ]
        );
        assert_eq!(a.total, 7, "a cold place and a seen finish want nothing");
        assert_eq!(a.rows[0].line, "Claude is waiting on your answer (2 min ago)");
        assert_eq!(a.rows[2].line, "Pull request #7: CI failing");
        assert_eq!(a.rows[4].line, "2 commits not on origin/main and 1 uncommitted file, untouched for 20 days");
        let page = answer_in(&cards, LeftOut::default(), NOW, None, 2, 1, |_| None, |_| None);
        assert_eq!(page.rows.iter().map(|r| r.slug.as_str()).collect::<Vec<_>>(), ["finished", "pr"]);
        assert_eq!(page.total, 7);
        let json = serde_json::to_value(&a.rows[4]).unwrap();
        assert_eq!(json["reason"], "at-risk");
    }

    /// `changed` is true for a finish after `since` and false when only the
    /// clock fields moved (a look, an open, a commit).
    #[test]
    fn changed_means_work_a_question_or_a_pull_request() {
        let since = NOW - 3600;
        let finish = Card { worked: Some(since + 10), last_active: Some(since + 10), ..card("a") };
        assert!(changed(&[finish], Some(since), true));
        let clocks = Card { worked: Some(since - 10), seen: Some(since + 100), last_active: Some(since + 200), ..card("b") };
        assert!(!changed(&[clocks.clone()], Some(since), true), "only clock fields moved");
        let asked = Card { state: Some(State::Waiting), waiting_since: Some(since + 5), ..card("c") };
        assert!(changed(&[clocks.clone(), asked], Some(since), true));
        let asked_before = Card { state: Some(State::Waiting), waiting_since: Some(since - 5), ..card("c") };
        assert!(!changed(&[clocks.clone(), asked_before], Some(since), true));
        let pr = Card { pr: Some(PrFact { number: 1, label: "merged", needs_you: false, updated: Some(since + 1) }), ..card("d") };
        assert!(changed(&[pr], Some(since), false));
        assert!(changed(&[clocks.clone()], None, true) && !changed(&[clocks], None, false), "no since: is there anything");
    }

    /// The prompt is read only for rows on the page whose place is recent,
    /// cut to one line of 80 characters.
    #[test]
    fn the_last_prompt_is_cut_and_read_only_where_it_is_shown() {
        let long = format!("fix\tthe\n{}\u{202e}tail", "word ".repeat(40));
        let near = Card { worked: Some(NOW - 60), last_active: Some(NOW - 60), ..card("near") };
        let far = Card { draft: true, last_active: Some(NOW - 30 * 86_400), ..card("far") };
        let asked = std::cell::RefCell::new(Vec::new());
        let a = answer_in(&[near, far], LeftOut::default(), NOW, None, 5, 0, |c| {
            asked.borrow_mut().push(c.slug.clone());
            Some(long.clone())
        }, |_| None);
        assert_eq!(*asked.borrow(), ["near"], "never a stale place's transcript");
        let l = &a.rows[0].line;
        assert!(l.starts_with("Finished 1 min ago, not seen yet: \u{201c}fix the word"), "{l}");
        let quoted = l.split('\u{201c}').nth(1).unwrap().trim_end_matches('\u{201d}');
        assert!(quoted.chars().count() <= PROMPT_CHARS + 1 && quoted.chars().count() > PROMPT_CHARS - 5, "{quoted}");
        assert!(quoted.ends_with('…') && !quoted.contains('\u{202e}'));
    }
}

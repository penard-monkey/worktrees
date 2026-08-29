//! One worktree's health verdict — the engine behind `worktrees status <name>`
//! and the app's status sheet.
//!
//! The split here is deliberate: GATHERING facts shells out to git and reads the
//! declared store (`ops::cmd_status`), while JUDGING them is this module's
//! `assess` — pure, deterministic, `now` injected. That is what lets the CLI and
//! the app print the same verdict from the same struct: the app runs
//! `cmd_status --json` in-process and deserializes `Report`, so the two can
//! never disagree about what "at risk" means.
//!
//! The one opinion worth stating up front: **`behind` is never sickness.** A
//! worktree behind its base has done nothing wrong — the base moved. It is a
//! fact this report carries and never a reason on its own.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// How long a place may sit untouched before it stops counting as live work.
///
/// ⚠ Mirrors `STALE_DAYS = 14` in `app/src/App.tsx` (the nav's row-dim horizon).
/// Keep the two in sync: the nav dimming a row while `status` still calls it
/// active — or the reverse — is worse than either number being wrong.
pub const STALE_SECS: i64 = 14 * 24 * 3600;

/// Everything measured about one place. Gathered by `ops::cmd_status`; judged by
/// `assess`. Serialize + Deserialize because the app parses this back out of the
/// CLI's `--json` line.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HealthFacts {
    pub slug: String,
    pub branch: Option<String>,
    /// The ref divergence is measured against (`origin/main`, usually) — carried
    /// in the facts because every reason below NAMES it, and a sentence that
    /// says "not on the base" without saying which ref that is sends the reader
    /// back to the CLI to find out.
    pub base: String,
    pub created_epoch: Option<i64>,
    pub last_commit_epoch: Option<i64>,
    pub last_commit_subject: Option<String>,
    /// Declared: the last real Enter (`touch_place`). Honest only because
    /// selecting a place in the app no longer opens it — browsing used to stamp
    /// this, which made every place look recently used.
    pub last_opened_epoch: Option<i64>,
    /// Declared: Claude finished a task here.
    pub last_worked_epoch: Option<i64>,
    /// Newest `*.jsonl` mtime in the place's claude session dir.
    pub claude_last_epoch: Option<i64>,
    pub dirty_files: u32,
    /// Commits not on the BASE ref — existing `ls --json` semantics. NOT the
    /// same as "unpushed": see `true_unpushed`.
    pub ahead: i64,
    /// Fact only, NEVER sickness.
    pub behind: i64,
    pub upstream: Option<String>,
    pub tmux_up: bool,
    pub lifecycle_effective: String,
    pub note: Option<String>,
    pub title: Option<String>,
    /// `git log --oneline <base>..HEAD`, capped at 20 lines.
    pub not_on_base: Vec<String>,
    /// The real count (`not_on_base` may be capped).
    pub not_on_base_total: u32,
    /// `git rev-list --count @{u}..HEAD` when an upstream exists; `None` when
    /// there is no upstream — and then ALL of `not_on_base` is machine-local.
    pub true_unpushed: Option<u32>,
    /// `git cherry <base>` `-` lines: commits whose PATCH already exists on the
    /// base. Patch-id matching, so it is a hint, not a proof.
    pub maybe_merged: u32,
}

/// The verdict plus the receipts for it. `facts` travels with the judgement so a
/// reader can check the reasoning, and so the app never has to gather anything
/// itself.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Report {
    pub schema_version: u32,
    /// `active` | `parked` | `at-risk` | `cold`
    pub verdict: String,
    /// Ordered, most important first. Empty for `active` and `cold`: the verdict
    /// IS the whole story there, and the facts table below it carries the rest.
    pub reasons: Vec<String>,
    pub facts: HealthFacts,
}

/// Newest `*.jsonl` mtime under `dir`, as an epoch — "when did Claude last write
/// anything here". `None` for a missing/unreadable dir or one with no sessions.
///
/// Plain `std::fs` on purpose: this is not a bash-parity path (nothing in the
/// legacy CLI printed it), so there is no reason to pay a `stat` subprocess for
/// it — and this repo counts spawns.
pub fn claude_last_epoch(dir: &str) -> Option<i64> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut newest: Option<i64> = None;
    for e in rd.flatten() {
        if !e.file_name().to_string_lossy().ends_with(".jsonl") {
            continue;
        }
        let Ok(secs) = e
            .metadata()
            .and_then(|m| m.modified())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other))
        else {
            continue;
        };
        let secs = secs.as_secs() as i64;
        if newest.is_none_or(|n| secs > n) {
            newest = Some(secs);
        }
    }
    newest
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Judge the facts. Pure and deterministic: `now` is injected, and dates are
/// rendered through `fmt_date` so a unit test can pass `|e| e.to_string()`
/// instead of shelling out to `date`.
///
/// Returns `(verdict, reasons)`.
pub fn assess(f: &HealthFacts, now: i64, fmt_date: &dyn Fn(i64) -> String) -> (String, Vec<String>) {
    // ── 1. When was anything last true of this place? ────────────────────────
    //
    // `created_epoch` and `last_opened_epoch` are in the max ON PURPOSE. A
    // worktree cut this morning from a month-old base has a month-old HEAD
    // commit and would otherwise read `cold` on day one; a place you Entered
    // three days ago was touched three days ago even if you committed nothing.
    let activity = [
        f.last_commit_epoch,
        f.last_opened_epoch,
        f.last_worked_epoch,
        f.claude_last_epoch,
        f.created_epoch,
    ]
    .into_iter()
    .flatten()
    .max();

    // ── 2. Is there work here that exists nowhere else? ──────────────────────
    //
    // ⚠ `ahead` counts commits not on the BASE, which is NOT the same as
    // unpushed: a branch fully pushed to its own `origin/feat-x` still reads
    // ahead > 0 (project.rs — pinned by test/json.bats). The REASONS below
    // respect that distinction; the verdict deliberately does not, because
    // unmerged work needs a decision either way.
    let has_unique_work = f.dirty_files > 0 || f.ahead > 0;

    // ── 3. Verdict ───────────────────────────────────────────────────────────
    //
    // Total by construction: `active` short-circuits, and the other three
    // partition "stale" by has_unique_work × tmux_up. No activity at all counts
    // as stale (`map_or(false, …)`).
    if activity.is_some_and(|a| now - a < STALE_SECS) {
        return ("active".into(), Vec::new());
    }

    if !has_unique_work {
        if f.tmux_up {
            // Exactly one sentence, and it names `behind` rather than hiding it:
            // the number is real, it is just not this worktree's fault.
            return (
                "parked".into(),
                vec![format!(
                    "clean and nothing ahead of the base — this worktree holds no unique work; behind {} is just the base moving.",
                    f.behind
                )],
            );
        }
        return ("cold".into(), Vec::new());
    }

    // ── at-risk: stale, and holding work that is only here ───────────────────
    let mut reasons = Vec::new();

    // The commits come first: they are the thing a user cannot reconstruct.
    let n = if f.not_on_base_total > 0 { u64::from(f.not_on_base_total) } else { f.ahead.max(0) as u64 };
    if n > 0 {
        let mut s = format!("{n} commit{} not on {}", plural(n), f.base);
        if let Some(e) = f.last_commit_epoch {
            s.push_str(&format!(", newest from {}", fmt_date(e)));
        }
        if let Some(subj) = f.last_commit_subject.as_deref().filter(|s| !s.is_empty()) {
            s.push_str(&format!(": {subj}"));
        }
        reasons.push(s);
    }

    if f.dirty_files > 0 {
        let d = u64::from(f.dirty_files);
        reasons.push(format!("{d} uncommitted file{}", plural(d)));
    }

    // Where those commits live BESIDES here. Gated on `n > 0` because every
    // wording below is about commits: on a dirty-but-not-ahead place there is
    // nothing for an upstream to have received, and "all pushed — safe" would
    // be a claim about work that was never committed.
    if n > 0 {
        match (f.upstream.as_deref(), f.true_unpushed) {
            (None, _) => reasons
                .push("no upstream — this work exists nowhere but this machine".to_string()),
            (Some(up), Some(k)) if k > 0 => {
                reasons.push(format!("{k} of them not pushed to {up}"));
            }
            (Some(up), _) => reasons.push(format!(
                "all pushed to {up} — safe, but not on {}",
                f.base
            )),
        }
    }

    // Last, and hedged: `git cherry` matches by patch-id, which a squash merge
    // destroys. Useful as "you may already have landed this", never as proof.
    if f.maybe_merged > 0 {
        let k = u64::from(f.maybe_merged);
        reasons.push(format!(
            "{k} commit{} match patches already on {} (patch-id — unreliable across squash merges)",
            plural(k),
            f.base
        ));
    }

    ("at-risk".into(), reasons)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;
    /// Trivial renderer so assertions read as epochs, not as this host's locale.
    fn fd(e: i64) -> String {
        e.to_string()
    }

    /// A healthy, boring place: clean, in sync, touched an hour ago.
    fn base_facts() -> HealthFacts {
        HealthFacts {
            slug: "feat-x".into(),
            branch: Some("feat/x".into()),
            base: "origin/main".into(),
            created_epoch: Some(NOW - 3600),
            last_commit_epoch: Some(NOW - 3600),
            last_commit_subject: Some("wip".into()),
            lifecycle_effective: "closed".into(),
            ..Default::default()
        }
    }

    /// Everything about this place happened `age` seconds ago — every rung of
    /// the activity max at once, so a test that means "stale" cannot be
    /// accidentally rescued by a field it forgot to move.
    fn aged(age: i64) -> HealthFacts {
        HealthFacts {
            created_epoch: Some(NOW - age),
            last_commit_epoch: Some(NOW - age),
            last_opened_epoch: Some(NOW - age),
            last_worked_epoch: Some(NOW - age),
            claude_last_epoch: Some(NOW - age),
            ..base_facts()
        }
    }

    #[test]
    fn recent_activity_is_active() {
        let (v, r) = assess(&base_facts(), NOW, &fd);
        assert_eq!(v, "active");
        assert!(r.is_empty(), "active states its case by being active: {r:?}");
    }

    #[test]
    fn stale_with_unpushed_commits_is_at_risk() {
        let f = HealthFacts {
            ahead: 2,
            not_on_base_total: 2,
            not_on_base: vec!["abc123 one".into(), "def456 two".into()],
            ..aged(30 * 86400)
        };
        let (v, r) = assess(&f, NOW, &fd);
        assert_eq!(v, "at-risk");
        assert!(r[0].starts_with("2 commits not on origin/main"), "got {r:?}");
    }

    #[test]
    fn stale_clean_with_a_session_is_parked() {
        let f = HealthFacts { tmux_up: true, ..aged(30 * 86400) };
        let (v, r) = assess(&f, NOW, &fd);
        assert_eq!(v, "parked");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn stale_clean_with_no_session_is_cold() {
        let f = HealthFacts { tmux_up: false, ..aged(30 * 86400) };
        let (v, r) = assess(&f, NOW, &fd);
        assert_eq!(v, "cold");
        assert!(r.is_empty(), "got {r:?}");
    }

    /// THE opinion of this module. A worktree 99 commits behind its base has
    /// done nothing wrong — the base moved. `behind` must never push a verdict
    /// toward sickness, and the one reason parked emits is the sentence that
    /// says so (it CONTAINS the word "behind", which is why this asserts the
    /// array's length and content rather than grepping for the word's absence).
    #[test]
    fn behind_is_never_sickness() {
        let f = HealthFacts { behind: 99, ahead: 0, dirty_files: 0, tmux_up: true, ..aged(30 * 86400) };
        let (v, r) = assess(&f, NOW, &fd);
        assert_eq!(v, "parked");
        assert_eq!(r.len(), 1, "behind must not add a reason of its own: {r:?}");
        assert_eq!(
            r[0],
            "clean and nothing ahead of the base — this worktree holds no unique work; behind 99 is just the base moving."
        );
    }

    /// The boundary is `< STALE_SECS`, so exactly at the horizon is already
    /// stale. Written down because "14 days" is the number two files agree on.
    #[test]
    fn activity_exactly_at_the_horizon_is_stale() {
        let f = HealthFacts { tmux_up: true, ..aged(STALE_SECS) };
        assert_eq!(assess(&f, NOW, &fd).0, "parked");
        let f = HealthFacts { tmux_up: true, ..aged(STALE_SECS - 1) };
        assert_eq!(assess(&f, NOW, &fd).0, "active");
    }

    /// No timestamps at all (an unregistered-looking place, a repo with no
    /// commits) is stale, not active — the absence of evidence is not evidence
    /// of work.
    #[test]
    fn no_activity_at_all_is_stale() {
        let f = HealthFacts {
            created_epoch: None,
            last_commit_epoch: None,
            last_opened_epoch: None,
            last_worked_epoch: None,
            claude_last_epoch: None,
            ..base_facts()
        };
        assert_eq!(assess(&f, NOW, &fd).0, "cold");
    }

    /// A worktree cut this morning off a month-old base. HEAD's commit date is
    /// ancient and nothing has been done in it yet — without `created_epoch` in
    /// the activity max this reads `cold` on day one, which is the single most
    /// insulting thing the check could say to a brand-new worktree.
    #[test]
    fn fresh_worktree_off_an_old_base_is_active() {
        let f = HealthFacts {
            created_epoch: Some(NOW - 3600),
            last_commit_epoch: Some(NOW - 40 * 86400),
            last_opened_epoch: None,
            last_worked_epoch: None,
            claude_last_epoch: None,
            ..base_facts()
        };
        assert_eq!(assess(&f, NOW, &fd).0, "active");
    }

    /// Pushed to its own remote, still not merged. The work is SAFE — it exists
    /// on a server — so the reason must say so rather than crying
    /// machine-local. Verdict stays at-risk: unmerged work still needs a
    /// decision.
    #[test]
    fn pushed_but_unmerged_says_safe_not_machine_local() {
        let f = HealthFacts {
            ahead: 3,
            not_on_base_total: 3,
            upstream: Some("origin/feat-x".into()),
            true_unpushed: Some(0),
            ..aged(30 * 86400)
        };
        let (v, r) = assess(&f, NOW, &fd);
        assert_eq!(v, "at-risk");
        assert!(
            r.iter().any(|x| x == "all pushed to origin/feat-x — safe, but not on origin/main"),
            "got {r:?}"
        );
        assert!(!r.iter().any(|x| x.contains("nowhere but this machine")), "got {r:?}");
    }

    #[test]
    fn some_pushed_some_not_counts_the_stragglers() {
        let f = HealthFacts {
            ahead: 5,
            not_on_base_total: 5,
            upstream: Some("origin/feat-x".into()),
            true_unpushed: Some(2),
            ..aged(30 * 86400)
        };
        let (_, r) = assess(&f, NOW, &fd);
        assert!(r.iter().any(|x| x == "2 of them not pushed to origin/feat-x"), "got {r:?}");
    }

    /// Ordering is part of the contract: the commits come first because they are
    /// the thing a person cannot reconstruct from memory.
    #[test]
    fn at_risk_reasons_lead_with_the_commits() {
        let f = HealthFacts {
            ahead: 1,
            not_on_base_total: 1,
            dirty_files: 4,
            maybe_merged: 1,
            last_commit_epoch: Some(NOW - 30 * 86400),
            last_commit_subject: Some("extract invoice service".into()),
            ..aged(30 * 86400)
        };
        let (v, r) = assess(&f, NOW, &fd);
        assert_eq!(v, "at-risk");
        assert_eq!(
            r[0],
            format!("1 commit not on origin/main, newest from {}: extract invoice service", NOW - 30 * 86400)
        );
        assert_eq!(r[1], "4 uncommitted files");
        assert_eq!(r[2], "no upstream — this work exists nowhere but this machine");
        assert!(r[3].starts_with("1 commit match"), "got {r:?}");
        assert!(r[3].contains("patch-id"), "the caveat must travel with the claim: {r:?}");
    }

    /// Dirty but not ahead: the only reason is the file count. No upstream line,
    /// because there are no commits for an upstream to be missing.
    #[test]
    fn dirty_only_reports_just_the_files() {
        let f = HealthFacts { dirty_files: 1, ahead: 0, ..aged(30 * 86400) };
        let (v, r) = assess(&f, NOW, &fd);
        assert_eq!(v, "at-risk");
        assert_eq!(r, vec!["1 uncommitted file".to_string()]);
    }

    /// The struct the app deserializes must survive a round trip through the
    /// exact wire the CLI writes.
    #[test]
    fn report_round_trips_as_json() {
        let f = base_facts();
        let (verdict, reasons) = assess(&f, NOW, &fd);
        let r = Report { schema_version: SCHEMA_VERSION, verdict, reasons, facts: f };
        let s = serde_json::to_string(&r).unwrap();
        let back: Report = serde_json::from_str(&s).unwrap();
        assert_eq!(back.schema_version, SCHEMA_VERSION);
        assert_eq!(back.verdict, "active");
        assert_eq!(back.facts.slug, "feat-x");
    }
}

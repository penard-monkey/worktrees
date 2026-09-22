//! The per-machine RUN LEDGER: what an automation did, and when.
//!
//! Why this is not in the repo. The definitions sidecar
//! (`automation.rs`, `.worktrees.automations.json`) travels between machines
//! on purpose — the same jobs should exist on both laptops. A ledger must not:
//! two machines sharing one "last run" would double-run a daily job and
//! overwrite each other's entries, and "when did this last run" is a fact about
//! a MACHINE, not about a project. So it lives beside `init.rs`'s once-only
//! hint markers, under `$XDG_STATE_HOME/worktrees/runs/<hash of main root>`,
//! and reuses that module's `state_dir`/`fnv1a` rather than re-deriving the
//! path (two answers to "where does this repo's state live" is exactly the bug
//! this arrangement exists to avoid).
//!
//! Shape rules, inherited from the rest of core:
//!
//! - **Lenient reads, atomic writes.** A half-written entry must never be what
//!   a reader sees, so every write is tmp + rename in the same directory.
//! - **`extra` round-trips unknown keys** so a newer binary's fields survive an
//!   older one reading and rewriting the entry.
//! - **Every `Option` is serialized, never skipped.** A `Run` is a `--json`
//!   payload (`diag.rs`'s rule): a consumer that has to ask whether a key is
//!   absent or null is a consumer with two code paths for one fact.
//! - **Nothing is dropped silently.** A finding or proposal the validator
//!   refuses lands in `dropped` with the reason, because "claude proposed
//!   something we refused" is information and an empty findings list is not.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ui::{CaptureUi, Ui};
use crate::{ops, store, Project};

/// Schema version of a ledger entry.
pub const VERSION: u32 = 1;

/// How many entries are kept per automation. Older ones (entry + working dir)
/// are deleted after a finished run is written.
pub const KEEP_PER_AUTOMATION: usize = 50;

/// The ONLY tools a run's proposal may name.
///
/// Closed by construction rather than by a check on the model's output: a
/// proposal is a button a person presses, and the set of buttons is decided
/// here. `remove_worktree` is deliberately absent and can never be added — see
/// `automation.rs`'s module note and CLAUDE.md on `remove_place`'s `force`.
pub const PROPOSAL_TOOLS: [&str; 4] = ["set_lifecycle", "set_note", "set_pin", "close_session"];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Manual,
    Schedule,
    Mcp,
}

impl Trigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Trigger::Manual => "manual",
            Trigger::Schedule => "schedule",
            Trigger::Mcp => "mcp",
        }
    }
    pub fn parse(s: &str) -> Option<Trigger> {
        match s {
            "manual" => Some(Trigger::Manual),
            "schedule" => Some(Trigger::Schedule),
            "mcp" => Some(Trigger::Mcp),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Clean,
    Findings,
    Failed,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Running => "running",
            Status::Clean => "clean",
            Status::Findings => "findings",
            Status::Failed => "failed",
        }
    }
    /// A run that will never change again — the only kind retention may delete.
    pub fn is_finished(&self) -> bool {
        !matches!(self, Status::Running)
    }
}

/// One proposal: a tool from `PROPOSAL_TOOLS` and the arguments to call it with.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Proposal {
    pub tool: String,
    pub args: Map<String, Value>,
}

/// One finding: which place, what about it, and what could be done.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Finding {
    pub slug: String,
    pub text: String,
    #[serde(default)]
    pub proposals: Vec<Proposal>,
}

/// A finding or proposal the validator refused. NEVER silent (`diag.rs`'s rule):
/// "claude proposed removing a worktree and we said no" is the single most
/// important thing a run can report about itself.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Dropped {
    pub what: String,
    pub why: String,
}

/// A place the run did not look at, and why.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Skipped {
    pub slug: String,
    pub why: String,
}

/// A proposal a person actually applied, with what the command said.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Action {
    pub epoch: i64,
    pub tool: String,
    pub args: Map<String, Value>,
    pub ok: bool,
    pub output: String,
}

/// One run of one automation. The whole thing is the `--json` payload.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Run {
    pub version: u32,
    pub id: String,
    pub automation: String,
    pub trigger: Trigger,
    pub started_epoch: i64,
    pub finished_epoch: Option<i64>,
    pub status: Status,
    pub profile: Option<String>,
    pub places: Vec<String>,
    pub skipped: Vec<Skipped>,
    pub facts: Value,
    pub findings: Vec<Finding>,
    pub dropped: Vec<Dropped>,
    pub actions: Vec<Action>,
    pub report_md: Option<String>,
    pub turns: Option<u32>,
    pub seconds: Option<u64>,
    pub error: Option<String>,
    pub seen_epoch: Option<i64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Run {
    pub fn new(id: String, automation: String, trigger: Trigger, started_epoch: i64) -> Run {
        Run {
            version: VERSION,
            id,
            automation,
            trigger,
            started_epoch,
            finished_epoch: None,
            status: Status::Running,
            profile: None,
            places: Vec::new(),
            skipped: Vec::new(),
            facts: Value::Object(Map::new()),
            findings: Vec::new(),
            dropped: Vec::new(),
            actions: Vec::new(),
            report_md: None,
            turns: None,
            seconds: None,
            error: None,
            seen_epoch: None,
            extra: Map::new(),
        }
    }

    /// The row a list renders: everything but the bulky halves (`facts`,
    /// `report_md`), which a caller asks for per run.
    pub fn summary(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "automation": self.automation,
            "trigger": self.trigger,
            "started_epoch": self.started_epoch,
            "finished_epoch": self.finished_epoch,
            "status": self.status,
            "findings": self.findings.len(),
            "dropped": self.dropped.len(),
            "actions": self.actions.len(),
            "places": self.places.len(),
            "seconds": self.seconds,
            "error": self.error,
            "seen_epoch": self.seen_epoch,
        })
    }
}

// ── the clock seam ───────────────────────────────────────────────────────────

/// `WORKTREES_RUN_NOW` (epoch seconds) overrides the clock for run ids and
/// stamps — the same shape, and the same reason, as `WORKTREES_STATUS_NOW` in
/// `ops::cmd_status`: a bats case has to assert on an id, and an id built from
/// the wall clock is unassertable. A TEST SEAM, not user surface.
pub fn run_now() -> i64 {
    std::env::var("WORKTREES_RUN_NOW")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or_else(crate::sysclock::now_epoch)
}

/// `YYYY-MM-DDTHH-MM-SSZ` in UTC. Colons would be legal in a POSIX filename and
/// are a liability in one (a path that travels through a URL, an rsync spec or
/// a Windows-hosted backup), so the time separators are hyphens; the `T` and the
/// `Z` keep it unambiguously an instant rather than a local stamp.
pub fn utc_stamp(epoch: i64) -> String {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Howard Hinnant's `civil_from_days`, the standard branch-free conversion.
/// Hand-rolled because core takes no date crate (Cargo.toml says why), and the
/// only alternative — shelling out to `date -u` — would be a subprocess per run
/// id in a repo that counts them.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ── where the ledger lives ───────────────────────────────────────────────────

/// `$XDG_STATE_HOME/worktrees/runs/<fnv1a of the canonical main root>`.
///
/// `None` when neither `XDG_STATE_HOME` nor `HOME` is set — there is no
/// defensible fallback, and inventing one (cwd, /tmp) would put a project's
/// history somewhere it can never be found again.
pub fn ledger_dir(main_root: &str) -> Option<PathBuf> {
    let state = crate::init::state_dir()?;
    let canon =
        fs::canonicalize(main_root).unwrap_or_else(|_| PathBuf::from(main_root));
    Some(
        state
            .join("runs")
            .join(format!("{:016x}", crate::init::fnv1a(&canon.to_string_lossy()))),
    )
}

/// The same directory, created. The one error every caller has to handle.
pub fn ensure_ledger_dir(main_root: &str) -> Result<PathBuf, String> {
    let dir = ledger_dir(main_root)
        .ok_or("no state directory: neither XDG_STATE_HOME nor HOME is set")?;
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// `<ledger>/<id>.json` — the entry.
pub fn entry_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// `<ledger>/<id>/` — the run's working files (`brief.md`, `facts.json`,
/// `findings.json`, `report.md`, `claude.stdout`, `claude.stderr`).
pub fn work_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join(id)
}

/// A fresh id: `<UTC stamp>-<slug>`, with `-2`, `-3`… on collision.
///
/// Collisions are real, not theoretical: two runs of the same automation inside
/// one second is what an MCP call plus a manual retry looks like, and under the
/// `WORKTREES_RUN_NOW` seam every run in a bats case shares one second.
pub fn new_id(dir: &Path, slug: &str, now: i64) -> String {
    let base = format!("{}-{slug}", utc_stamp(now));
    if !entry_path(dir, &base).exists() && !work_dir(dir, &base).exists() {
        return base;
    }
    for n in 2..1000 {
        let c = format!("{base}-{n}");
        if !entry_path(dir, &c).exists() && !work_dir(dir, &c).exists() {
            return c;
        }
    }
    format!("{base}-{}", std::process::id())
}

// ── read / write ─────────────────────────────────────────────────────────────

/// Atomic: tmp in the SAME directory, then rename(2). A reader that catches a
/// half-written entry would report a `running` run as corrupt, and the tab
/// polls this file.
pub fn write(main_root: &str, run: &Run) -> Result<(), String> {
    let dir = ensure_ledger_dir(main_root)?;
    write_in(&dir, run)
}

/// The same write against an already-resolved ledger directory — for the runner,
/// which holds one for the life of a run and must not re-resolve it per write.
pub fn write_in(dir: &Path, run: &Run) -> Result<(), String> {
    let path = entry_path(dir, &run.id);
    let json = serde_json::to_string_pretty(run).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".{}.json.tmp", run.id));
    fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    if run.status.is_finished() {
        prune(dir, &run.automation);
    }
    Ok(())
}

pub fn read(main_root: &str, id: &str) -> Result<Run, String> {
    let dir = ledger_dir(main_root).ok_or("no state directory for this project")?;
    read_in(&dir, id)
}

pub fn read_in(dir: &Path, id: &str) -> Result<Run, String> {
    let path = entry_path(dir, id);
    let bytes = fs::read(&path).map_err(|_| format!("no such run: {id}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{id} is not a valid run entry: {e}"))
}

/// Newest first: `started_epoch` descending, then id descending, so two runs
/// stamped in the same second still have one stable order (the `-2` suffix
/// sorts after the bare id, and descending puts the later one on top).
pub fn list(main_root: &str, automation: Option<&str>) -> Vec<Run> {
    let Some(dir) = ledger_dir(main_root) else { return Vec::new() };
    list_in(&dir, automation)
}

pub fn list_in(dir: &Path, automation: Option<&str>) -> Vec<Run> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Run> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            // `<id>.spawn.log` and the `.` tmp files are not entries; a name
            // that does not end in `.json` is skipped without comment.
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                return None;
            }
            if p.file_name().and_then(|s| s.to_str()).is_some_and(|n| n.starts_with('.')) {
                return None;
            }
            serde_json::from_slice::<Run>(&fs::read(&p).ok()?).ok()
        })
        .filter(|r| automation.is_none_or(|a| r.automation == a))
        .collect();
    out.sort_by(|a, b| {
        b.started_epoch.cmp(&a.started_epoch).then_with(|| b.id.cmp(&a.id))
    });
    out
}

/// Keep the newest `KEEP_PER_AUTOMATION` entries for one automation; delete the
/// rest, entry AND working directory. Best-effort: a retention failure must not
/// fail the run that triggered it.
fn prune(dir: &Path, automation: &str) {
    let runs = list_in(dir, Some(automation));
    for r in runs.into_iter().skip(KEEP_PER_AUTOMATION) {
        let _ = fs::remove_file(entry_path(dir, &r.id));
        let _ = fs::remove_dir_all(work_dir(dir, &r.id));
    }
}

/// Every run entry the ledger holds, deleted with its automation.
pub fn delete_for_automation(main_root: &str, automation: &str) {
    let Some(dir) = ledger_dir(main_root) else { return };
    for r in list_in(&dir, Some(automation)) {
        let _ = fs::remove_file(entry_path(&dir, &r.id));
        let _ = fs::remove_dir_all(work_dir(&dir, &r.id));
    }
}

/// The last run of each automation, keyed by slug — what a list renders on the
/// right of a row. One directory walk for every automation rather than one per.
pub fn last_by_automation(main_root: &str) -> BTreeMap<String, Run> {
    let mut out: BTreeMap<String, Run> = BTreeMap::new();
    for r in list(main_root, None) {
        out.entry(r.automation.clone()).or_insert(r);
    }
    out
}

// ── applying a proposal ──────────────────────────────────────────────────────

/// Make the one call a run proposed, and record it in the run.
///
/// The closed set is enforced HERE and not at the call site, for the same reason
/// `safe_arg` lives in `mcp.rs`: this is the layer that knows the arguments came
/// from a model. `set_lifecycle`/`set_note`/`set_pin` go through `store::edit`
/// exactly as the MCP server's `meta` does — same lock, same atomic write, same
/// lifecycle vocabulary (`store::LIFECYCLE_LABELS`, moved there so there is one
/// answer) — and `close_session` is `ops::cmd_close`, the command a person would
/// have run.
pub fn apply_proposal(
    project: &Project,
    ui: &mut dyn Ui,
    run_id: &str,
    finding_ix: usize,
    proposal_ix: usize,
) -> Result<Action, String> {
    let dir = ensure_ledger_dir(&project.main_root)?;
    let mut run = read_in(&dir, run_id)?;

    let finding = run
        .findings
        .get(finding_ix)
        .ok_or_else(|| format!("run {run_id} has no finding {finding_ix}"))?;
    let proposal = finding
        .proposals
        .get(proposal_ix)
        .ok_or_else(|| format!("finding {finding_ix} has no proposal {proposal_ix}"))?
        .clone();

    if !PROPOSAL_TOOLS.contains(&proposal.tool.as_str()) {
        return Err(format!(
            "{} is not an applicable proposal (allowed: {})",
            proposal.tool,
            PROPOSAL_TOOLS.join(", ")
        ));
    }
    let slug = proposal
        .args
        .get("slug")
        .and_then(|v| v.as_str())
        .ok_or("the proposal names no slug")?
        .to_string();
    if !project.ls().places.iter().any(|p| p.slug == slug) {
        return Err(format!("no such place: {slug}"));
    }

    let (ok, output) = match proposal.tool.as_str() {
        "set_lifecycle" => {
            let life = proposal.args.get("lifecycle").and_then(|v| v.as_str()).unwrap_or("");
            if !store::LIFECYCLE_LABELS.contains(&life) {
                (false, format!("invalid lifecycle: {life}"))
            } else {
                match store::edit(&project.main_root, &slug, |d| {
                    d.lifecycle = Some(life.to_string())
                }) {
                    Ok(()) => (true, format!("{slug}: lifecycle = {life}")),
                    Err(e) => (false, e),
                }
            }
        }
        "set_note" => {
            let note = proposal.args.get("note").and_then(|v| v.as_str()).unwrap_or("").to_string();
            match store::edit(&project.main_root, &slug, |d| {
                d.note = if note.is_empty() { None } else { Some(note.clone()) }
            }) {
                Ok(()) => (true, format!("{slug}: note set")),
                Err(e) => (false, e),
            }
        }
        "set_pin" => {
            let Some(pinned) = proposal.args.get("pinned").and_then(|v| v.as_bool()) else {
                return Err("pinned must be true or false".into());
            };
            match store::edit(&project.main_root, &slug, |d| d.pinned = Some(pinned)) {
                Ok(()) => (true, format!("{slug}: pinned = {pinned}")),
                Err(e) => (false, e),
            }
        }
        "close_session" => {
            let mut cap = CaptureUi::default();
            let rc = ops::cmd_close(project, &mut cap, &[slug.clone()]);
            (rc == 0, cap.lines.join("\n"))
        }
        // Unreachable: the membership check above is the gate. Discharged with
        // a message rather than an `unreachable!` — this runs inside a server.
        other => (false, format!("{other} is not an applicable proposal")),
    };

    let action = Action {
        epoch: run_now(),
        tool: proposal.tool.clone(),
        args: proposal.args.clone(),
        ok,
        output: output.clone(),
    };
    if ok {
        ui.info(&output);
    } else {
        ui.error(&output);
    }
    run.actions.push(action.clone());
    write_in(&dir, &run)?;
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let t = std::env::temp_dir().join(format!(
            "wtruns-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        Tmp(t)
    }

    fn finished(id: &str, automation: &str, started: i64) -> Run {
        let mut r = Run::new(id.into(), automation.into(), Trigger::Manual, started);
        r.status = Status::Clean;
        r.finished_epoch = Some(started + 1);
        r
    }

    #[test]
    fn a_run_id_is_a_utc_instant_and_the_slug() {
        // The proposal's own §5 example id, so the shape there and the shape on
        // disk cannot drift apart. ⚠ Its EPOCH is not this instant: §5 pairs
        // `2026-09-22T08-02-11Z` with 1758528131, which is 2025-09-22 — a year
        // out (see the Answers section). The id is the contract; the number in
        // the document was hand-written.
        assert_eq!(utc_stamp(1_790_064_131), "2026-09-22T08-02-11Z");
        assert_eq!(utc_stamp(1_758_528_131), "2025-09-22T08-02-11Z");
        // Epoch itself, a leap day, and the last second of a year: the three
        // places a hand-rolled civil-date conversion goes wrong.
        assert_eq!(utc_stamp(0), "1970-01-01T00-00-00Z");
        assert_eq!(utc_stamp(1_709_164_800), "2024-02-29T00-00-00Z");
        assert_eq!(utc_stamp(1_767_225_599), "2025-12-31T23-59-59Z");
    }

    #[test]
    fn a_second_run_in_the_same_second_gets_a_suffix() {
        let t = tmp("id");
        let now = 1_790_064_131;
        let a = new_id(&t.0, "sweep", now);
        assert_eq!(a, "2026-09-22T08-02-11Z-sweep");
        write_in(&t.0, &finished(&a, "sweep", now)).unwrap();
        let b = new_id(&t.0, "sweep", now);
        assert_eq!(b, "2026-09-22T08-02-11Z-sweep-2");
    }

    /// Newest first, and the tiebreak is the ID — otherwise two runs stamped in
    /// the same second (an MCP call plus a manual retry) come back in whatever
    /// order `read_dir` happened to hand them over, which is not an order.
    #[test]
    fn runs_list_newest_first_and_filter_by_automation() {
        let t = tmp("list");
        write_in(&t.0, &finished("a", "sweep", 100)).unwrap();
        write_in(&t.0, &finished("b", "sweep", 300)).unwrap();
        write_in(&t.0, &finished("c", "other", 200)).unwrap();
        write_in(&t.0, &finished("d", "sweep", 300)).unwrap();

        let all: Vec<String> = list_in(&t.0, None).into_iter().map(|r| r.id).collect();
        assert_eq!(all, vec!["d", "b", "c", "a"], "epoch desc, then id desc");

        let mine: Vec<String> =
            list_in(&t.0, Some("sweep")).into_iter().map(|r| r.id).collect();
        assert_eq!(mine, vec!["d", "b", "a"]);
    }

    /// Retention is per AUTOMATION, and it takes the working directory with the
    /// entry — a ledger that keeps 50 entries and 400 directories of claude
    /// transcripts is not a ledger that keeps 50 runs.
    #[test]
    fn retention_keeps_the_newest_fifty_per_automation_with_their_dirs() {
        let t = tmp("prune");
        for i in 0..(KEEP_PER_AUTOMATION as i64 + 5) {
            let id = format!("sweep-{i:03}");
            fs::create_dir_all(work_dir(&t.0, &id)).unwrap();
            fs::write(work_dir(&t.0, &id).join("report.md"), "x").unwrap();
            write_in(&t.0, &finished(&id, "sweep", 1000 + i)).unwrap();
        }
        // A second automation must be untouched by the first's pruning.
        write_in(&t.0, &finished("other-000", "other", 1)).unwrap();

        let kept = list_in(&t.0, Some("sweep"));
        assert_eq!(kept.len(), KEEP_PER_AUTOMATION);
        assert_eq!(kept.first().unwrap().id, "sweep-054", "the newest survives");
        assert!(!entry_path(&t.0, "sweep-000").exists(), "the oldest entry is gone");
        assert!(!work_dir(&t.0, "sweep-000").exists(), "and so is its working dir");
        assert!(work_dir(&t.0, "sweep-054").exists());
        assert_eq!(list_in(&t.0, Some("other")).len(), 1, "another automation is untouched");
    }

    /// A RUNNING entry must never be pruned, whatever the count — it is the one
    /// entry something is still writing to.
    #[test]
    fn a_running_entry_does_not_trigger_retention() {
        let t = tmp("prune-running");
        for i in 0..(KEEP_PER_AUTOMATION as i64 + 3) {
            write_in(&t.0, &finished(&format!("s-{i:03}"), "sweep", 1000 + i)).unwrap();
        }
        assert_eq!(list_in(&t.0, Some("sweep")).len(), KEEP_PER_AUTOMATION);
        let running = Run::new("s-999".into(), "sweep".into(), Trigger::Manual, 9999);
        write_in(&t.0, &running).unwrap();
        assert_eq!(
            list_in(&t.0, Some("sweep")).len(),
            KEEP_PER_AUTOMATION + 1,
            "a running write must not prune"
        );
    }

    /// Every `Option` is a KEY, present as `null`. A `--json` consumer that has
    /// to distinguish "absent" from "null" has two code paths for one fact, and
    /// `diag.rs` settled that argument for this codebase already.
    #[test]
    fn an_empty_run_serializes_every_optional_key() {
        let r = Run::new("i".into(), "a".into(), Trigger::Mcp, 5);
        let v = serde_json::to_value(&r).unwrap();
        for k in [
            "finished_epoch", "profile", "report_md", "turns", "seconds", "error", "seen_epoch",
        ] {
            assert!(v.get(k).is_some(), "{k} must be a key even when None");
            assert!(v[k].is_null(), "{k} must serialize as null");
        }
        assert_eq!(v["trigger"], serde_json::json!("mcp"));
        assert_eq!(v["status"], serde_json::json!("running"));
    }

    /// Unknown keys round-trip: an entry written by a newer binary must survive
    /// an older one reading it, applying a proposal and writing it back.
    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let t = tmp("extra");
        fs::write(
            entry_path(&t.0, "x"),
            r#"{"version":1,"id":"x","automation":"a","trigger":"manual","started_epoch":1,
                "status":"clean","places":[],"skipped":[],"facts":{},"findings":[],
                "dropped":[],"actions":[],"from_the_future":42}"#,
        )
        .unwrap();
        let r = read_in(&t.0, "x").unwrap();
        assert_eq!(r.extra.get("from_the_future").and_then(|v| v.as_i64()), Some(42));
        write_in(&t.0, &r).unwrap();
        let raw = fs::read_to_string(entry_path(&t.0, "x")).unwrap();
        assert!(raw.contains("from_the_future"), "{raw}");
    }

    /// `remove_worktree` is not in the closed set, and must not become
    /// applicable by being written into an entry by hand.
    #[test]
    fn the_proposal_tool_set_is_closed() {
        assert!(!PROPOSAL_TOOLS.contains(&"remove_worktree"));
        assert!(!PROPOSAL_TOOLS.contains(&"create_worktree"));
        assert_eq!(PROPOSAL_TOOLS.len(), 4);
    }
}

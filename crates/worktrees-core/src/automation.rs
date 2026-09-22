//! Project AUTOMATIONS: a brief, a schedule, and the runner that executes one.
//!
//! An automation is a paragraph a person wrote, at project scope, with a *when*
//! — the same unit as `.planning/brief.md` (#183), one level out. There is no
//! vocabulary of built-in actions to learn, which is the whole design decision
//! (`docs/proposals/automations.md` §2): the thing you edit is prose, and the
//! thing that reads it is claude.
//!
//! Three rules shape this module, and each one is load-bearing:
//!
//! 1. **The brief is never argv.** ADR 0001 and `ops::BRIEF_OPENER`: a run
//!    writes the brief to a file and hands claude a FIXED opener that points at
//!    it. Nothing a repository (or a model) contains becomes a command line.
//! 2. **A run's `cwd` is the WORKTREE ROOT (`.worktrees/`), never a place —
//!    and the main root IS a place.** `health::assess` folds the newest claude
//!    transcript under a place's `~/.claude/projects/<mangled>` into its
//!    activity max, and `claude -p` writes one under its cwd (measured; it
//!    writes no session probe). A headless run *in* a worktree makes that
//!    worktree read `active` the next morning, and one in the main root does
//!    the same to `(main)` — a sweep over every place would blind the very
//!    signal it exists to report on (proposal §4.3). `.worktrees/` is owned by
//!    no place; the place paths travel as DATA in `facts.json`.
//! 3. **A run reports; a person acts.** Phase 1 has exactly one tier
//!    (`Tier::Report`). Findings carry *proposals* from a closed set
//!    (`runs::PROPOSAL_TOOLS`), and applying one is a separate, human-pressed
//!    call. `remove_worktree` is not in that set and never will be: it is the
//!    only path in this codebase that can destroy commits (CLAUDE.md on
//!    `remove_place`'s `force`), and an unattended caller does not go near it.
//!
//! The definitions sidecar is the twin of `.worktrees.places.json`: machine
//! JSON at the main root, git-excluded via `.git/info/exclude`, this tool the
//! sole writer, unknown keys round-tripped. It is NOT `.worktrees.toml` —
//! a cloned repo may not supply a prompt (ADR 0001).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::runs::{self, Dropped, Finding, Proposal, Run, Status, Trigger};
use crate::ui::{CaptureUi, Ui};
use crate::Project;

/// `<main root>/.worktrees.automations.json`.
pub const FILE: &str = ".worktrees.automations.json";

/// Schema version of the sidecar.
pub const VERSION: u32 = 1;

// ── the model ────────────────────────────────────────────────────────────────

/// When a run is due. Phase 1 stores and VALIDATES this and never evaluates it;
/// the clock is phase 2 (proposal §6.2), and shipping the field now is what lets
/// a definition written today be scheduled tomorrow without a migration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum When {
    Manual,
    Daily { at: String },
    Weekly { day: String, at: String },
}

impl Default for When {
    fn default() -> Self {
        When::Manual
    }
}

impl When {
    /// One-line human form: what `automations ls` puts under the name.
    pub fn label(&self) -> String {
        match self {
            When::Manual => "when I ask".into(),
            When::Daily { at } => format!("daily at {at}"),
            When::Weekly { day, at } => format!("{day} at {at}"),
        }
    }
    /// Refused at the seam, not at the clock. A `daily at 25:00` stored today is
    /// a job that silently never runs in phase 2, and the person who typed it
    /// will be looking at the ledger, not at the field.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            When::Manual => Ok(()),
            When::Daily { at } => validate_hhmm(at),
            When::Weekly { day, at } => {
                if !DAYS.contains(&day.as_str()) {
                    return Err(format!("day must be one of {} (got {day})", DAYS.join(", ")));
                }
                validate_hhmm(at)
            }
        }
    }
}

pub const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

fn validate_hhmm(at: &str) -> Result<(), String> {
    let bad = || format!("time must be HH:MM in 24-hour form (got {at})");
    let (h, m) = at.split_once(':').ok_or_else(bad)?;
    if h.len() != 2 || m.len() != 2 {
        return Err(bad());
    }
    let h: u32 = h.parse().map_err(|_| bad())?;
    let m: u32 = m.parse().map_err(|_| bad())?;
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok(())
}

/// Which places a run looks at. `Brief` means "the brief says" — the runner
/// still gathers facts for all of them; the prose decides what to ignore.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    All,
    Brief,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::All => "all",
            Scope::Brief => "brief",
        }
    }
    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "all" => Some(Scope::All),
            "brief" => Some(Scope::Brief),
            _ => None,
        }
    }
}

/// What a run may do. Phase 1 ships ONE tier on purpose (proposal §10.1): every
/// day the feature runs report-only is a day of evidence about what the briefs
/// actually propose, before anything acts unattended.
///
/// An entry carrying a tier this binary does not know is a READ ERROR for that
/// entry — skipped with a warning, never a panic and never silently downgraded
/// to `report`, which would run a job under permissions its author did not
/// choose.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    #[default]
    Report,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Report => "report",
        }
    }
    pub fn parse(s: &str) -> Option<Tier> {
        match s {
            "report" => Some(Tier::Report),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Automation {
    pub name: String,
    pub brief: String,
    #[serde(default)]
    pub when: When,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub tier: Tier,
    #[serde(default)]
    pub created_epoch: i64,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Store {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub automations: BTreeMap<String, Automation>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The editable fields. Every one optional: an edit that names nothing changes
/// nothing, and a create needs exactly `name` + `brief`.
#[derive(Clone, Debug, Default)]
pub struct Patch {
    pub name: Option<String>,
    pub brief: Option<String>,
    pub when: Option<When>,
    pub scope: Option<Scope>,
    pub tier: Option<Tier>,
    pub enabled: Option<bool>,
}

/// The slug an automation is filed under: lowercase, runs of anything that is
/// not `[a-z0-9]` collapsed to one `-`, trimmed.
///
/// Fixed at creation and IMMUTABLE thereafter, for the same reason a place's
/// slug is (`store::Declared::title`): it keys the ledger's entries, the lock
/// file and every run id already written. A rename changes `name` only.
pub fn slug_of(name: &str) -> Option<String> {
    let mut out = String::new();
    for ch in name.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// ── read / write ─────────────────────────────────────────────────────────────

fn base(main_root: &str) -> PathBuf {
    fs::canonicalize(main_root).unwrap_or_else(|_| PathBuf::from(main_root))
}

pub fn path(main_root: &str) -> PathBuf {
    base(main_root).join(FILE)
}

/// For display: a missing or corrupt file is an EMPTY store, never fatal — and
/// an entry this binary cannot read (an unknown `tier`, a malformed `when`) is
/// dropped from the set rather than taking the whole file with it. The reasons
/// come back as warnings so a caller can say them out loud; `read_lenient` is
/// the same read for a caller with nowhere to put them.
pub fn read_reporting(main_root: &str) -> (Store, Vec<String>) {
    let p = path(main_root);
    let Ok(bytes) = fs::read(&p) else { return (Store::default(), Vec::new()) };
    // Top level first, with the entries as raw values: one bad entry must not
    // erase the others, which a whole-file `from_slice::<Store>` would do.
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        version: u32,
        #[serde(default)]
        automations: BTreeMap<String, Value>,
        #[serde(flatten)]
        extra: Map<String, Value>,
    }
    let raw: Raw = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(e) => {
            return (
                Store::default(),
                vec![format!("{} is not valid JSON ({e}) — read as empty", p.display())],
            )
        }
    };
    let mut warnings = Vec::new();
    let mut automations = BTreeMap::new();
    for (slug, v) in raw.automations {
        match serde_json::from_value::<Automation>(v) {
            Ok(a) => {
                automations.insert(slug, a);
            }
            Err(e) => warnings.push(format!("automation '{slug}' skipped: {e}")),
        }
    }
    (Store { version: raw.version, automations, extra: raw.extra }, warnings)
}

pub fn read_lenient(main_root: &str) -> Store {
    read_reporting(main_root).0
}

/// For writes: a parse error must NOT be clobbered — a hand-edit typo has to
/// stay human-repairable — so it is surfaced instead of overwritten. Same rule,
/// and the same wording, as `store::read_strict`.
fn read_strict(p: &Path) -> Result<Store, String> {
    match fs::read(p) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{FILE} is not valid JSON ({e}) — not overwriting")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Atomic: tmp in the same directory, then rename(2). First write also teaches
/// the repo to ignore the file — the same `.git/info/exclude` maintenance the
/// places sidecar does, through the same function so there is one list and one
/// header.
pub fn write(main_root: &str, s: &Store) -> Result<(), String> {
    let b = base(main_root);
    let p = b.join(FILE);
    let creating = !p.exists();
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    let tmp = b.join(format!(".{FILE}.tmp"));
    fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &p).map_err(|e| format!("{}: {e}", p.display()))?;
    if creating {
        crate::store::exclude_app_state(&b);
    }
    Ok(())
}

/// Create (`slug = None`) or edit. Returns the slug and the stored automation.
pub fn upsert(
    main_root: &str,
    ui: &mut dyn Ui,
    slug: Option<&str>,
    patch: Patch,
) -> Result<(String, Automation), String> {
    let p = path(main_root);
    let mut store = read_strict(&p)?;
    if let Some(w) = patch.when.as_ref() {
        w.validate()?;
    }
    if let Some(b) = patch.brief.as_ref() {
        if b.trim().is_empty() {
            return Err("the brief is empty — say what Claude should do".into());
        }
    }

    let (slug, mut entry) = match slug {
        Some(s) => {
            let e = store
                .automations
                .get(s)
                .cloned()
                .ok_or_else(|| format!("no such automation: {s}"))?;
            (s.to_string(), e)
        }
        None => {
            let name = patch
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or("a new automation needs --name")?;
            let brief = patch
                .brief
                .as_deref()
                .filter(|b| !b.trim().is_empty())
                .ok_or("a new automation needs --brief (or --brief-file)")?;
            let slug = slug_of(name)
                .ok_or_else(|| format!("'{name}' has no letters or digits to make a slug from"))?;
            if store.automations.contains_key(&slug) {
                return Err(format!(
                    "an automation with the slug '{slug}' already exists — rename it or pick another name"
                ));
            }
            let e = Automation {
                name: name.to_string(),
                brief: brief.to_string(),
                when: When::Manual,
                scope: Scope::All,
                tier: Tier::Report,
                created_epoch: runs::run_now(),
                enabled: true,
                extra: Map::new(),
            };
            (slug, e)
        }
    };

    if let Some(n) = patch.name {
        let n = n.trim().to_string();
        if n.is_empty() {
            return Err("the name is empty".into());
        }
        // The SLUG does not move with it; say so rather than leaving the user to
        // discover that `automations run <new-name>` does not resolve.
        if entry.name != n && slug_of(&n).as_deref() != Some(slug.as_str()) {
            ui.warn(&format!("renamed — the slug stays '{slug}'"));
        }
        entry.name = n;
    }
    if let Some(b) = patch.brief {
        entry.brief = b;
    }
    if let Some(w) = patch.when {
        entry.when = w;
    }
    if let Some(s) = patch.scope {
        entry.scope = s;
    }
    if let Some(t) = patch.tier {
        entry.tier = t;
    }
    if let Some(e) = patch.enabled {
        entry.enabled = e;
    }

    store.version = VERSION;
    store.automations.insert(slug.clone(), entry.clone());
    write(main_root, &store)?;
    Ok((slug, entry))
}

/// Remove a definition AND every run it left behind. The ledger's entries are
/// keyed on the automation's slug, so leaving them would make the next
/// automation that happened to derive the same slug inherit a stranger's
/// history.
pub fn delete(main_root: &str, slug: &str) -> Result<(), String> {
    let p = path(main_root);
    let mut store = read_strict(&p)?;
    if store.automations.remove(slug).is_none() {
        return Err(format!("no such automation: {slug}"));
    }
    store.version = VERSION;
    write(main_root, &store)?;
    runs::delete_for_automation(main_root, slug);
    Ok(())
}

// ── the opener ───────────────────────────────────────────────────────────────

/// The prompt a run is launched on. FIXED TEXT with four path slots, for
/// exactly the reason `ops::BRIEF_OPENER` is fixed: the brief itself never
/// travels through argv, only a pointer to it does. A `ps` listing of a running
/// automation shows four paths and this sentence — never the prose, never a
/// place's contents, never anything a repository supplied.
///
/// The paths are backtick-quoted so the output contract is unambiguous to the
/// model AND greppable by the bats shim that stands in for claude.
pub const RUN_OPENER_TEMPLATE: &str = "Read `{brief}` and do what it says for the worktrees of this project. \
Facts about every worktree — health verdict, commits ahead/behind, dirty files, upstream, last activity, \
whether a Claude session is live there — are in `{facts}`; read that file, do not run git. \
Do not change anything: you are reporting. \
Write `{findings}` as JSON {\"findings\":[{\"slug\":\"…\",\"text\":\"one or two sentences\",\"proposals\":[{\"tool\":\"…\",\"args\":{…}}]}]} \
where a proposal's tool is one of set_lifecycle {slug, lifecycle: closed|saved|archived|abandoned}, \
set_note {slug, note}, set_pin {slug, pinned}, close_session {slug}; an empty findings list means all is well. \
Then write `{report}` as markdown: what you looked at, what you found, what you would do, under 300 words.";

pub fn run_opener(brief: &Path, facts: &Path, findings: &Path, report: &Path) -> String {
    RUN_OPENER_TEMPLATE
        .replace("{brief}", &brief.to_string_lossy())
        .replace("{facts}", &facts.to_string_lossy())
        .replace("{findings}", &findings.to_string_lossy())
        .replace("{report}", &report.to_string_lossy())
}

// ── the runner ───────────────────────────────────────────────────────────────

pub struct RunOpts {
    pub trigger: Trigger,
    /// Pre-assigned by a caller that must answer with an id before the run
    /// finishes (MCP's `run_automation`). `None` mints one.
    pub id: Option<String>,
    pub max_turns: u32,
    pub deadline_secs: u64,
    pub json: bool,
}

impl Default for RunOpts {
    fn default() -> Self {
        RunOpts {
            trigger: Trigger::Manual,
            id: None,
            max_turns: DEFAULT_MAX_TURNS,
            deadline_secs: DEFAULT_DEADLINE_SECS,
            json: false,
        }
    }
}

pub const DEFAULT_MAX_TURNS: u32 = 12;
pub const DEFAULT_DEADLINE_SECS: u64 = 300;

/// Held for the life of a run so two clocks (the app's tick, a launchd job, a
/// person) can never double-run one automation. Removed on EVERY exit path,
/// which is why it is a guard and not a pair of calls.
struct LockGuard(PathBuf);
impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// `Ok(None)` means a LIVE process holds it. A lock whose pid is dead is stale —
/// a crashed or killed run — and is removed rather than blocking the automation
/// forever.
fn acquire_lock(dir: &Path, slug: &str) -> Result<Option<LockGuard>, String> {
    let p = dir.join(format!("{slug}.lock"));
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                return Ok(Some(LockGuard(p)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let pid = fs::read_to_string(&p).ok().and_then(|s| s.trim().parse::<i32>().ok());
                match pid {
                    Some(pid) if crate::agent::pid_alive(pid) => return Ok(None),
                    _ => {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
            Err(e) => return Err(format!("{}: {e}", p.display())),
        }
    }
    Err(format!("could not take the run lock for {slug}"))
}

/// Is a run of this automation live right now?
///
/// Asked by a caller that must answer BEFORE spawning the runner — the MCP
/// tool, which has to reply in milliseconds and cannot wait to find out that
/// the child exited saying "already running". The same stale-pid rule as
/// `acquire_lock`, and deliberately non-destructive: this only looks.
pub fn is_running(main_root: &str, slug: &str) -> bool {
    let Some(dir) = runs::ledger_dir(main_root) else { return false };
    let Ok(body) = fs::read_to_string(dir.join(format!("{slug}.lock"))) else { return false };
    body.trim().parse::<i32>().map(crate::agent::pid_alive).unwrap_or(false)
}

/// An id is a PATH COMPONENT (`<ledger>/<id>.json`, `<ledger>/<id>/`), and the
/// MCP server lets a caller supply one.
fn check_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.starts_with('.')
    {
        return Err(format!("invalid run id: {id}"));
    }
    Ok(())
}

/// Can this project run an automation at all? The AI seam plus the "is it
/// really claude" guard, with the seam's warnings passed to `ui`.
///
/// `pub` because the app asks this BEFORE it spawns the runner on a thread: a
/// refusal here leaves no ledger entry (deliberately — see `run_inner`), so a
/// caller that only learns of it from the thread's exit code has nothing to
/// show the person. Asking first turns it into an ordinary error on the invoke.
/// The runner asks again on its own thread; the seam is idempotent.
///
/// The guard's message is passed through unchanged: it already names the cause
/// and the fix, and a prefix here once produced "automations need the claude
/// CLI: this project's ai_cmd is `none`, and a headless run needs the claude CLI".
pub fn preflight(p: &Project, ui: &mut dyn Ui) -> Result<crate::profile::AiLaunch, String> {
    let mut seam = CaptureUi::default();
    let ai = crate::ops::ai_launch_for(
        p,
        &mut seam,
        &p.main_root, // the profile seam keys on the project; the run CWD is chosen by the runner.
        &crate::config::resolve_ai_cmd(None),
    );
    for w in seam.warnings() {
        ui.warn(&w);
    }
    crate::profile::claude_launch_check(&ai.cmd, &ai.match_word)?;
    Ok(ai)
}

/// Run one automation, start to finish. Exit code: `0` clean, `2` findings
/// (`doctor`'s convention), `1` failed.
pub fn run(p: &Project, ui: &mut dyn Ui, slug: &str, opts: RunOpts) -> i32 {
    match run_inner(p, ui, slug, opts) {
        Ok(code) => code,
        Err(e) => {
            ui.error(&e);
            1
        }
    }
}

fn run_inner(p: &Project, ui: &mut dyn Ui, slug: &str, opts: RunOpts) -> Result<i32, String> {
    let store = read_lenient(&p.main_root);
    let auto = store
        .automations
        .get(slug)
        .cloned()
        .ok_or_else(|| format!("no such automation: {slug} — see `worktrees automations ls`"))?;

    let dir = runs::ensure_ledger_dir(&p.main_root)?;

    // 1. The lock, before anything observable. A second caller says so and
    //    exits 0: "already running" is not an error, it is the answer.
    let Some(_lock) = acquire_lock(&dir, slug)? else {
        ui.info(&format!("{slug} is already running"));
        return Ok(0);
    };

    // The AI seam BEFORE the entry is written: a project whose `ai_cmd` is not
    // claude cannot run an automation at all, and a ledger full of `failed`
    // entries for a machine that was never going to work is noise, not history.
    let ai = preflight(p, ui)?;

    let started = runs::run_now();
    let id = match opts.id {
        Some(id) => {
            check_id(&id)?;
            id
        }
        None => runs::new_id(&dir, slug, started),
    };
    let work = runs::work_dir(&dir, &id);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;

    let mut run = Run::new(id.clone(), slug.to_string(), opts.trigger, started);
    run.profile = ai.profile.as_ref().map(|(n, _)| n.clone());

    // 2. `running` on disk BEFORE anything slow: the surface that shows a
    //    spinner reads a FILE, not an in-process promise.
    runs::write_in(&dir, &run)?;

    // 3. Tier 0 — the facts, free and deterministic. Every place, the same
    //    `cmd_status --json` the CLI and the app both parse, so a run cannot
    //    disagree with `worktrees status` about what "at risk" means.
    let places = p.place_index();
    let probes = crate::agent::live_probes();
    let mut facts = Map::new();
    for pr in &places {
        run.places.push(pr.slug.clone());
        let mut cap = CaptureUi::default();
        let code = crate::ops::cmd_status(p, &mut cap, &[pr.slug.clone(), "--json".into()]);
        let parsed = cap
            .lines
            .iter()
            .rev()
            .find_map(|l| serde_json::from_str::<crate::health::Report>(l).ok());
        let live = !crate::agent::agents_at(&probes, &pr.path).is_empty();
        let mut v = match parsed {
            Some(r) => serde_json::to_value(&r).unwrap_or_else(|e| {
                serde_json::json!({ "error": format!("could not encode the report: {e}") })
            }),
            None => {
                let why = if cap.lines.is_empty() {
                    format!("status exited {code}")
                } else {
                    cap.lines.join("\n")
                };
                serde_json::json!({ "error": why })
            }
        };
        if let Some(o) = v.as_object_mut() {
            o.insert("path".into(), Value::String(pr.path.clone()));
            // Reported, and nothing more: phase 1 SKIPS nothing (the report tier
            // reads freely — proposal §4.4), so `skipped` stays empty and the
            // brief gets to decide what a live session means.
            o.insert("live_agent".into(), Value::Bool(live));
        }
        facts.insert(pr.slug.clone(), v);
    }
    run.facts = Value::Object(facts);

    // 4. The brief and the facts, as FILES. Never argv.
    let brief_path = work.join("brief.md");
    let facts_path = work.join("facts.json");
    let findings_path = work.join("findings.json");
    let report_path = work.join("report.md");
    let mut brief_body = auto.brief.clone();
    if !brief_body.ends_with('\n') {
        brief_body.push('\n');
    }
    fs::write(&brief_path, &brief_body).map_err(|e| format!("{}: {e}", brief_path.display()))?;
    fs::write(
        &facts_path,
        serde_json::to_string_pretty(&run.facts).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", facts_path.display()))?;
    runs::write_in(&dir, &run)?;

    // 5/6. Launch. `exec` is load-bearing: `proc::run_deadline` kills its DIRECT
    // child and only that, so without it a timed-out claude survives `sh` as an
    // orphan, still burning turns and still holding the profile's MCP servers.
    let opener = run_opener(&brief_path, &facts_path, &findings_path, &report_path);
    let line = format!(
        "exec {} -p {} --max-turns {}",
        ai.cmd,
        crate::tmux::sq(&opener),
        opts.max_turns
    );
    // cwd = the WORKTREE ROOT (`<main root>/.worktrees/`), which is not a place.
    // Measured 2026-09-22: `claude -p` writes no `sessions/<pid>.json` probe, but
    // it DOES write a transcript under `~/.claude/projects/<mangled cwd>/`, and
    // `(main)` is a place whose activity max reads that directory — so a run
    // from the main root would make `(main)` read `active` after every sweep.
    // From here the transcript lands in a directory no place owns, and both
    // `Project::discover` (the in-run MCP server) and claude's CLAUDE.md lookup
    // still resolve to the main root by walking up.
    let run_cwd = Path::new(&p.wt_root);
    fs::create_dir_all(run_cwd).map_err(|e| format!("{}: {e}", run_cwd.display()))?;
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", &line]).current_dir(run_cwd);
    for (k, v) in &ai.env {
        cmd.env(k, v);
    }
    cmd.env("WORKTREES_RUN_ID", &id);
    cmd.env("WORKTREES_RUN_TIER", auto.tier.as_str());

    let t0 = std::time::Instant::now();
    let out = crate::proc::run_deadline(cmd, opts.deadline_secs);
    let elapsed = t0.elapsed().as_secs();

    let out = match out {
        Ok(o) => o,
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::TimedOut {
                format!("claude timed out after {}s", opts.deadline_secs)
            } else {
                format!("could not run claude: {e}")
            };
            return finish(&dir, ui, run, Status::Failed, Some(msg), elapsed, opts.json);
        }
    };
    let _ = fs::write(work.join("claude.stdout"), &out.stdout);
    let _ = fs::write(work.join("claude.stderr"), &out.stderr);

    // 7. Fold the results in.
    run.report_md = fs::read_to_string(&report_path).ok().map(|s| s.trim_end().to_string());

    if !out.status.success() {
        let rc = out.status.code().unwrap_or(-1);
        let tail = stderr_tail(&out.stderr, 12);
        let msg = if tail.is_empty() {
            format!("claude exited {rc}")
        } else {
            format!("claude exited {rc}: {tail}")
        };
        return finish(&dir, ui, run, Status::Failed, Some(msg), elapsed, opts.json);
    }

    // A missing findings file is NOT "clean". The output contract is the whole
    // interface to the model, and a run that silently reported "all is well"
    // because claude never wrote the file would be the most expensive kind of
    // wrong: a sweep you stop reading because it never says anything.
    let raw = match fs::read_to_string(&findings_path) {
        Ok(s) => s,
        Err(_) => {
            return finish(
                &dir,
                ui,
                run,
                Status::Failed,
                Some("claude wrote no findings.json".into()),
                elapsed,
                opts.json,
            )
        }
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return finish(
                &dir,
                ui,
                run,
                Status::Failed,
                Some(format!("claude's findings.json is not valid JSON: {e}")),
                elapsed,
                opts.json,
            )
        }
    };
    // `{"findings": […]}` is the contract; a bare array is accepted because it
    // is the one deviation that is unambiguous, and refusing it would fail a run
    // whose content was perfectly good.
    let items: Vec<Value> = match parsed.get("findings").or(Some(&parsed)) {
        Some(Value::Array(a)) => a.clone(),
        _ => {
            return finish(
                &dir,
                ui,
                run,
                Status::Failed,
                Some("claude's findings.json has no `findings` array".into()),
                elapsed,
                opts.json,
            )
        }
    };

    let known: Vec<&str> = places.iter().map(|p| p.slug.as_str()).collect();
    let (findings, dropped) = validate(&items, &known);
    run.findings = findings;
    run.dropped = dropped;
    let status = if run.findings.is_empty() { Status::Clean } else { Status::Findings };
    finish(&dir, ui, run, status, None, elapsed, opts.json)
}

/// Every finding and proposal checked against the project, with a reason
/// recorded for each refusal. The two rules that matter: a slug must name a
/// place that EXISTS (a finding about `billing-refactor` when there is no such
/// worktree is a hallucination with a button under it), and a proposal's tool
/// must be in the closed set with its `args.slug` matching the finding it sits
/// under (otherwise a finding about one place carries a button that changes
/// another).
fn validate(items: &[Value], known: &[&str]) -> (Vec<Finding>, Vec<Dropped>) {
    let mut findings = Vec::new();
    let mut dropped = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let slug = item.get("slug").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if slug.is_empty() {
            dropped.push(Dropped {
                what: format!("finding #{i}"),
                why: "it names no place".into(),
            });
            continue;
        }
        if !known.contains(&slug.as_str()) {
            dropped.push(Dropped {
                what: format!("finding for '{slug}'"),
                why: "there is no such place in this project".into(),
            });
            continue;
        }
        if text.is_empty() {
            dropped.push(Dropped {
                what: format!("finding for '{slug}'"),
                why: "it says nothing".into(),
            });
            continue;
        }
        let mut proposals = Vec::new();
        if let Some(Value::Array(ps)) = item.get("proposals") {
            for pv in ps {
                let tool = pv.get("tool").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let args = match pv.get("args") {
                    Some(Value::Object(m)) => m.clone(),
                    _ => Map::new(),
                };
                if !runs::PROPOSAL_TOOLS.contains(&tool.as_str()) {
                    dropped.push(Dropped {
                        what: format!("proposal '{tool}' for '{slug}'"),
                        why: format!(
                            "a run may only propose {} — nothing that removes a worktree",
                            runs::PROPOSAL_TOOLS.join(", ")
                        ),
                    });
                    continue;
                }
                if args.get("slug").and_then(|v| v.as_str()) != Some(slug.as_str()) {
                    dropped.push(Dropped {
                        what: format!("proposal '{tool}' for '{slug}'"),
                        why: "its slug does not match the finding it belongs to".into(),
                    });
                    continue;
                }
                proposals.push(Proposal { tool, args });
            }
        }
        findings.push(Finding { slug, text, proposals });
    }
    (findings, dropped)
}

#[allow(clippy::too_many_arguments)]
fn finish(
    dir: &Path,
    ui: &mut dyn Ui,
    mut run: Run,
    status: Status,
    error: Option<String>,
    seconds: u64,
    json: bool,
) -> Result<i32, String> {
    run.status = status;
    run.error = error;
    run.finished_epoch = Some(runs::run_now());
    run.seconds = Some(seconds);
    // `turns` stays None: `claude -p` does not report it, and a guess written
    // into a ledger is indistinguishable from a measurement a week later.
    runs::write_in(dir, &run)?;

    if json {
        ui.plain(&serde_json::to_string(&run).map_err(|e| e.to_string())?);
    } else {
        for f in &run.findings {
            ui.plain(&format!("{} — {}", f.slug, f.text));
        }
        for d in &run.dropped {
            ui.warn(&format!("dropped {}: {}", d.what, d.why));
        }
        if let Some(e) = &run.error {
            ui.error(e);
        }
        ui.plain(&run.id);
    }
    Ok(match run.status {
        Status::Findings => 2,
        Status::Failed => 1,
        _ => 0,
    })
}

/// Last `n` non-empty lines of a subprocess's stderr — where a CLI puts the
/// reason, and short enough to belong in an error a person reads.
fn stderr_tail(bytes: &[u8], n: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
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
            "wtauto-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        Tmp(t)
    }

    #[test]
    fn slugs_are_lowercase_dashed_and_collapse_runs() {
        assert_eq!(slug_of("Close-out candidates").as_deref(), Some("close-out-candidates"));
        assert_eq!(slug_of("  What   happened / this week! ").as_deref(), Some("what-happened-this-week"));
        assert_eq!(slug_of("Sweep #2").as_deref(), Some("sweep-2"));
        assert_eq!(slug_of("---"), None);
        assert_eq!(slug_of(""), None);
    }

    #[test]
    fn a_when_is_validated_at_the_seam_not_at_the_clock() {
        assert!(When::Daily { at: "08:00".into() }.validate().is_ok());
        assert!(When::Daily { at: "25:00".into() }.validate().is_err());
        assert!(When::Daily { at: "8:00".into() }.validate().is_err(), "HH must be two digits");
        assert!(When::Daily { at: "08:60".into() }.validate().is_err());
        assert!(When::Daily { at: "0800".into() }.validate().is_err());
        assert!(When::Weekly { day: "mon".into(), at: "09:00".into() }.validate().is_ok());
        assert!(When::Weekly { day: "monday".into(), at: "09:00".into() }.validate().is_err());
        assert!(When::Manual.validate().is_ok());
    }

    #[test]
    fn a_when_round_trips_as_the_proposals_tagged_shape() {
        let v = serde_json::to_value(When::Daily { at: "08:00".into() }).unwrap();
        assert_eq!(v, serde_json::json!({ "kind": "daily", "at": "08:00" }));
        let v = serde_json::to_value(When::Manual).unwrap();
        assert_eq!(v, serde_json::json!({ "kind": "manual" }));
    }

    #[test]
    fn create_edit_and_delete_round_trip_with_unknown_keys() {
        let t = tmp("crud");
        let root = t.0.to_string_lossy().to_string();
        let mut ui = CaptureUi::default();

        let (slug, a) = upsert(
            &root,
            &mut ui,
            None,
            Patch {
                name: Some("Close-out candidates".into()),
                brief: Some("Look at every worktree.".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(slug, "close-out-candidates");
        assert_eq!(a.tier, Tier::Report);
        assert_eq!(a.scope, Scope::All);
        assert!(a.enabled);

        // A hand-added key must survive an edit written by this binary.
        let raw = fs::read_to_string(path(&root)).unwrap();
        let mut v: Value = serde_json::from_str(&raw).unwrap();
        v["automations"][&slug]["from_the_future"] = serde_json::json!(42);
        fs::write(path(&root), serde_json::to_string_pretty(&v).unwrap()).unwrap();

        upsert(&root, &mut ui, Some(&slug), Patch { enabled: Some(false), ..Default::default() })
            .unwrap();
        let back = read_lenient(&root);
        let e = &back.automations[&slug];
        assert!(!e.enabled);
        assert_eq!(e.extra.get("from_the_future").and_then(|v| v.as_i64()), Some(42));

        assert!(delete(&root, &slug).is_ok());
        assert!(read_lenient(&root).automations.is_empty());
        assert!(delete(&root, &slug).is_err(), "deleting twice is an error, not a no-op");
    }

    #[test]
    fn a_duplicate_slug_and_an_empty_brief_are_refused() {
        let t = tmp("dup");
        let root = t.0.to_string_lossy().to_string();
        let mut ui = CaptureUi::default();
        let mk = |ui: &mut CaptureUi, name: &str, brief: &str| {
            upsert(
                &root,
                ui,
                None,
                Patch { name: Some(name.into()), brief: Some(brief.into()), ..Default::default() },
            )
        };
        mk(&mut ui, "Nightly sweep", "do the thing").unwrap();
        // Different NAME, same derived slug — the collision people actually hit.
        let e = mk(&mut ui, "nightly   sweep", "do the thing").unwrap_err();
        assert!(e.contains("nightly-sweep"), "{e}");
        assert!(mk(&mut ui, "Other", "   ").unwrap_err().contains("brief"));
        assert!(mk(&mut ui, "   ", "x").is_err());
    }

    /// An entry this binary cannot read is skipped WITH A REASON, and takes
    /// nothing else with it. Downgrading it to `report` would run a job under
    /// permissions its author did not choose; erroring the whole file would make
    /// one bad row hide every good one.
    #[test]
    fn an_unknown_tier_skips_only_that_entry() {
        let t = tmp("tier");
        let root = t.0.to_string_lossy().to_string();
        fs::write(
            path(&root),
            r#"{"version":1,"automations":{
                "good":{"name":"Good","brief":"b","when":{"kind":"manual"},"scope":"all","tier":"report","created_epoch":1,"enabled":true},
                "future":{"name":"Future","brief":"b","when":{"kind":"manual"},"scope":"all","tier":"sessions","created_epoch":1,"enabled":true}
            }}"#,
        )
        .unwrap();
        let (s, warnings) = read_reporting(&root);
        assert!(s.automations.contains_key("good"));
        assert!(!s.automations.contains_key("future"), "an unknown tier is not readable");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("future"), "{:?}", warnings);
    }

    #[test]
    fn a_corrupt_file_reads_as_empty_and_says_so() {
        let t = tmp("corrupt");
        let root = t.0.to_string_lossy().to_string();
        fs::write(path(&root), "{not json").unwrap();
        let (s, w) = read_reporting(&root);
        assert!(s.automations.is_empty());
        assert_eq!(w.len(), 1);
        // …and a WRITE must refuse rather than clobber a repairable typo.
        let mut ui = CaptureUi::default();
        assert!(upsert(
            &root,
            &mut ui,
            None,
            Patch { name: Some("x".into()), brief: Some("y".into()), ..Default::default() }
        )
        .unwrap_err()
        .contains("not overwriting"));
    }

    /// The whole security story of the opener in one assertion: the prose a
    /// person wrote is not in it, and cannot be.
    #[test]
    fn the_opener_carries_paths_and_never_the_brief() {
        let o = run_opener(
            Path::new("/l/brief.md"),
            Path::new("/l/facts.json"),
            Path::new("/l/findings.json"),
            Path::new("/l/report.md"),
        );
        for p in ["/l/brief.md", "/l/facts.json", "/l/findings.json", "/l/report.md"] {
            assert!(o.contains(&format!("`{p}`")), "{p} must be backtick-quoted: {o}");
        }
        assert!(!o.contains("{brief}"), "every slot is filled: {o}");
        assert!(o.contains("do not run git"));
        assert!(o.contains("close_session"));
        assert!(!o.contains("remove_worktree"), "the opener must not name it at all");
    }

    #[test]
    fn a_finding_for_an_unknown_place_is_dropped_and_recorded() {
        let items = vec![
            serde_json::json!({ "slug": "alpha", "text": "fine", "proposals": [] }),
            serde_json::json!({ "slug": "ghost", "text": "gone" }),
            serde_json::json!({ "slug": "alpha", "text": "" }),
            serde_json::json!({ "text": "no slug" }),
        ];
        let (f, d) = validate(&items, &["alpha", "(main)"]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].slug, "alpha");
        assert_eq!(d.len(), 3, "{d:?}");
        assert!(d.iter().any(|x| x.what.contains("ghost")));
    }

    /// THE refusal this feature exists to make. A proposal naming
    /// `remove_worktree` is dropped AND recorded — never silently swallowed,
    /// because "the run wanted to delete a worktree" is the single most
    /// important thing it can tell you about itself.
    #[test]
    fn a_remove_worktree_proposal_is_dropped_and_recorded() {
        let items = vec![serde_json::json!({
            "slug": "alpha", "text": "merged long ago",
            "proposals": [
                { "tool": "remove_worktree", "args": { "slug": "alpha", "confirm": true } },
                { "tool": "set_lifecycle", "args": { "slug": "alpha", "lifecycle": "abandoned" } },
                { "tool": "set_note", "args": { "slug": "beta", "note": "wrong place" } }
            ]
        })];
        let (f, d) = validate(&items, &["alpha", "beta"]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].proposals.len(), 1, "only the applicable one survives");
        assert_eq!(f[0].proposals[0].tool, "set_lifecycle");
        assert_eq!(d.len(), 2);
        assert!(d[0].what.contains("remove_worktree"));
        assert!(d[0].why.contains("removes a worktree"));
        assert!(d[1].why.contains("does not match"), "{:?}", d[1]);
    }

    #[test]
    fn a_supplied_run_id_may_not_escape_the_ledger() {
        assert!(check_id("2026-09-22T08-02-11Z-sweep").is_ok());
        for bad in ["", "  ", "../x", "a/b", ".hidden", "a\\b"] {
            assert!(check_id(bad).is_err(), "{bad:?} must be refused");
        }
    }

    /// A stale lock (a crashed run) must not block the automation forever; a
    /// live one must.
    #[test]
    fn a_stale_lock_is_removed_and_a_live_one_is_honoured() {
        let t = tmp("lock");
        // pid 1 is always alive; a very high pid in a fresh namespace is not.
        fs::write(t.0.join("a.lock"), "1").unwrap();
        assert!(acquire_lock(&t.0, "a").unwrap().is_none(), "a live pid holds it");

        fs::write(t.0.join("b.lock"), "4000000").unwrap();
        let g = acquire_lock(&t.0, "b").unwrap();
        assert!(g.is_some(), "a dead pid's lock is stale");
        drop(g);
        assert!(!t.0.join("b.lock").exists(), "the guard removes it on every exit path");

        // A lock with nonsense in it is stale too — better than wedging forever.
        fs::write(t.0.join("c.lock"), "not a pid").unwrap();
        assert!(acquire_lock(&t.0, "c").unwrap().is_some());
    }
}

// ── the CLI verb ─────────────────────────────────────────────────────────────
// Lives in core beside the model, the way every other `cmd_*` does, so the app
// and the MCP server reach the same parser rather than each growing their own.

const AUTO_USAGE: &str = "\
usage: worktrees automations <subcommand>

  ls [--json]                              list this project's automations
  add --name <n> --brief <text>            create one (--brief-file <path>, - for stdin)
     [--daily HH:MM | --weekly <day> HH:MM] [--scope all|brief]
  edit <slug> [--name <n>] [--brief <t> | --brief-file <p>]
     [--daily HH:MM | --weekly <day> HH:MM | --manual] [--scope all|brief]
     [--enable | --disable]
  rm <slug>                                delete it and its run history
  run <slug> [--json]                      run it now (0 clean, 2 findings, 1 failed)
  runs [<slug>] [--json]                   the run ledger, newest first
  show <run-id> [--json]                   one run in full
  apply <run-id> <finding#> <proposal#>    make the call a run proposed (0-based)";

/// `worktrees automations …`. Exit: `0` ok, `2` a run produced findings, `1`
/// usage / failure.
pub fn cmd_automations(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let sub = args.first().map(String::as_str).unwrap_or("ls");
    let rest = args.get(1..).unwrap_or(&[]);

    // The same choke point `main.rs` applies to the mutating verbs, per
    // SUBcommand because half of these are reads: a tree that arrived on a sync
    // hub is another machine's mirror, so a definition written here is erased by
    // the next pull and a `run` would shell git at the wrong repo's worktrees.
    if matches!(sub, "add" | "edit" | "rm" | "remove" | "delete" | "run" | "apply") {
        if let Some(msg) = crate::sync::hub_copy_refusal(Path::new(&p.main_root)) {
            ui.error(&msg);
            return 1;
        }
    }

    match sub {
        "ls" | "list" => cmd_ls(p, ui, rest),
        "add" | "new" => cmd_add(p, ui, rest, None),
        "edit" => {
            let Some(slug) = rest.first().filter(|s| !s.starts_with('-')).cloned() else {
                ui.error("usage: worktrees automations edit <slug> [flags]");
                return 1;
            };
            cmd_add(p, ui, rest.get(1..).unwrap_or(&[]), Some(slug))
        }
        "rm" | "remove" | "delete" => match rest.first() {
            Some(slug) => match delete(&p.main_root, slug) {
                Ok(()) => {
                    ui.info(&format!("removed automation '{slug}' and its runs"));
                    0
                }
                Err(e) => {
                    ui.error(&e);
                    1
                }
            },
            None => {
                ui.error("usage: worktrees automations rm <slug>");
                1
            }
        },
        "run" => cmd_run_verb(p, ui, rest),
        "runs" => cmd_runs(p, ui, rest),
        "show" => cmd_show(p, ui, rest),
        "apply" => cmd_apply(p, ui, rest),
        "-h" | "--help" | "help" => {
            ui.plain(AUTO_USAGE);
            0
        }
        other => {
            ui.error(&format!("unknown automations subcommand: {other}"));
            ui.plain(AUTO_USAGE);
            1
        }
    }
}

/// The JSON row for one automation, with its last run folded in — what both the
/// CLI's `--json` and the MCP `list_automations` hand back, so the two cannot
/// describe the same automation differently.
pub fn row_json(slug: &str, a: &Automation, last: Option<&Run>) -> Value {
    serde_json::json!({
        "slug": slug,
        "name": a.name,
        "when": a.when,
        "scope": a.scope,
        "tier": a.tier,
        "enabled": a.enabled,
        "created_epoch": a.created_epoch,
        "last_run": last.map(|r| serde_json::json!({
            "id": r.id,
            "status": r.status,
            "finished_epoch": r.finished_epoch,
            "findings": r.findings.len(),
        })),
    })
}

fn cmd_ls(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let json = wants_json(args);
    let (store, warnings) = read_reporting(&p.main_root);
    for w in &warnings {
        ui.warn(w);
    }
    let last = runs::last_by_automation(&p.main_root);
    if json {
        let rows: Vec<Value> = store
            .automations
            .iter()
            .map(|(s, a)| row_json(s, a, last.get(s)))
            .collect();
        ui.plain(
            &serde_json::json!({ "version": VERSION, "automations": rows }).to_string(),
        );
        return 0;
    }
    if store.automations.is_empty() {
        ui.info("no automations yet — add one with `worktrees automations add`");
        return 0;
    }
    for (slug, a) in &store.automations {
        let state = if a.enabled { "" } else { " (disabled)" };
        ui.plain(&format!("{slug}  {}{state}", a.name));
        let tail = match last.get(slug) {
            Some(r) => format!("last: {} ({})", r.status.as_str(), r.id),
            None => "never ran".into(),
        };
        ui.plain(&format!(
            "    {} · scope {} · tier {} · {tail}",
            a.when.label(),
            a.scope.as_str(),
            a.tier.as_str()
        ));
    }
    0
}

fn wants_json(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json")
        || std::env::var("WORKTREES_JSON").ok().as_deref() == Some("1")
}

/// `add` and `edit` share every flag but the slug, so they share the parser —
/// the alternative is two tables that drift.
fn cmd_add(p: &Project, ui: &mut dyn Ui, args: &[String], slug: Option<String>) -> i32 {
    let mut patch = Patch::default();
    let mut i = 0;
    // `args[i + n]`, or the usage error naming the flag that wanted it. Written
    // out rather than closed over because the error needs `ui`, which the loop
    // is already holding.
    macro_rules! val {
        ($n:expr, $flag:expr, $what:expr) => {
            match args.get(i + $n) {
                Some(v) => v.clone(),
                None => return usage(ui, &format!("{} needs {}", $flag, $what)),
            }
        };
    }
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                patch.name = Some(val!(1, "--name", "a value"));
                i += 1;
            }
            "--brief" => {
                patch.brief = Some(val!(1, "--brief", "a value"));
                i += 1;
            }
            "--brief-file" => {
                let path = val!(1, "--brief-file", "a path (- for stdin)");
                match read_brief_file(&path) {
                    Ok(t) => patch.brief = Some(t),
                    Err(e) => {
                        ui.error(&e);
                        return 1;
                    }
                }
                i += 1;
            }
            "--daily" => {
                patch.when = Some(When::Daily { at: val!(1, "--daily", "HH:MM") });
                i += 1;
            }
            // Two values: `--weekly mon 09:00`. A day and a time are one
            // decision, so they are one flag rather than two that can disagree.
            "--weekly" => {
                let day = val!(1, "--weekly", "<day> HH:MM");
                let at = val!(2, "--weekly", "<day> HH:MM");
                patch.when = Some(When::Weekly { day, at });
                i += 2;
            }
            "--manual" => patch.when = Some(When::Manual),
            "--scope" => {
                let v = val!(1, "--scope", "all or brief");
                match Scope::parse(&v) {
                    Some(s) => patch.scope = Some(s),
                    None => return usage(ui, &format!("--scope must be all or brief (got {v})")),
                }
                i += 1;
            }
            "--enable" => patch.enabled = Some(true),
            "--disable" => patch.enabled = Some(false),
            "--json" => {}
            other => return usage(ui, &format!("unknown flag: {other}")),
        }
        i += 1;
    }

    match upsert(&p.main_root, ui, slug.as_deref(), patch) {
        Ok((slug, a)) => {
            ui.info(&format!("{slug}: {} — {}", a.name, a.when.label()));
            0
        }
        Err(e) => {
            ui.error(&e);
            1
        }
    }
}

fn usage(ui: &mut dyn Ui, msg: &str) -> i32 {
    ui.error(msg);
    ui.plain(AUTO_USAGE);
    1
}

/// `-` is stdin, which is how a brief composed by another program (or a
/// heredoc) gets in without becoming a command-line argument.
fn read_brief_file(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
            .map_err(|e| format!("reading the brief from stdin: {e}"))?;
        return Ok(s);
    }
    fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn cmd_run_verb(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let mut slug: Option<String> = None;
    let mut opts = RunOpts { json: wants_json(args), ..Default::default() };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {}
            "--id" => {
                i += 1;
                match args.get(i) {
                    Some(v) => opts.id = Some(v.clone()),
                    None => return usage(ui, "--id needs a value"),
                }
            }
            "--trigger" => {
                i += 1;
                match args.get(i).and_then(|v| Trigger::parse(v)) {
                    Some(t) => opts.trigger = t,
                    None => return usage(ui, "--trigger must be manual, schedule or mcp"),
                }
            }
            "--max-turns" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0) {
                    Some(n) => opts.max_turns = n,
                    None => return usage(ui, "--max-turns needs a positive number"),
                }
            }
            other if other.starts_with('-') => return usage(ui, &format!("unknown flag: {other}")),
            other => slug = Some(other.to_string()),
        }
        i += 1;
    }
    let Some(slug) = slug else {
        return usage(ui, "usage: worktrees automations run <slug>");
    };
    run(p, ui, &slug, opts)
}

fn cmd_runs(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let json = wants_json(args);
    let filter: Option<String> =
        args.iter().find(|a| !a.starts_with('-')).cloned();
    let list = runs::list(&p.main_root, filter.as_deref());
    if json {
        let rows: Vec<Value> = list.iter().map(|r| r.summary()).collect();
        ui.plain(&serde_json::json!({ "version": runs::VERSION, "runs": rows }).to_string());
        return 0;
    }
    if list.is_empty() {
        ui.info("no runs yet");
        return 0;
    }
    for r in &list {
        ui.plain(&format!(
            "{}  {}  {} finding(s){}",
            r.id,
            r.status.as_str(),
            r.findings.len(),
            r.error.as_ref().map(|e| format!("  — {e}")).unwrap_or_default()
        ));
    }
    0
}

fn cmd_show(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let json = wants_json(args);
    let Some(id) = args.iter().find(|a| !a.starts_with('-')) else {
        return usage(ui, "usage: worktrees automations show <run-id>");
    };
    let r = match runs::read(&p.main_root, id) {
        Ok(r) => r,
        Err(e) => {
            ui.error(&e);
            return 1;
        }
    };
    if json {
        ui.plain(&serde_json::to_string(&r).unwrap_or_default());
        return 0;
    }
    ui.plain(&format!("{}  {}  {}", r.id, r.automation, r.status.as_str()));
    for (i, f) in r.findings.iter().enumerate() {
        ui.plain(&format!("[{i}] {} — {}", f.slug, f.text));
        for (j, pr) in f.proposals.iter().enumerate() {
            ui.plain(&format!(
                "     [{j}] {} {}",
                pr.tool,
                serde_json::to_string(&pr.args).unwrap_or_default()
            ));
        }
    }
    for d in &r.dropped {
        ui.warn(&format!("dropped {}: {}", d.what, d.why));
    }
    if let Some(e) = &r.error {
        ui.error(e);
    }
    if let Some(md) = &r.report_md {
        ui.plain("");
        ui.plain(md);
    }
    0
}

fn cmd_apply(p: &Project, ui: &mut dyn Ui, args: &[String]) -> i32 {
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if pos.len() != 3 {
        return usage(ui, "usage: worktrees automations apply <run-id> <finding#> <proposal#>");
    }
    let (Ok(f), Ok(pr)) = (pos[1].parse::<usize>(), pos[2].parse::<usize>()) else {
        return usage(ui, "the finding and proposal indexes are 0-based numbers");
    };
    match runs::apply_proposal(p, ui, pos[0], f, pr) {
        Ok(a) => i32::from(!a.ok),
        Err(e) => {
            ui.error(&e);
            1
        }
    }
}

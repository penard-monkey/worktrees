//! Refuse to START an agent on a provider whose window is already nearly spent.
//!
//! # Why this exists
//!
//! Eight worktrees were started in parallel with Codex on high reasoning
//! effort against a Plus plan; the five-hour window was gone in fifteen
//! minutes. Nothing in the tool had an opinion — a launch happened with no
//! reference to what was left, and asking for eight is eight launches as fast
//! as tmux can start them. Since #385 the MCP instructions actively tell an
//! orchestrator to start every piece of work as a place, so "one session asks
//! for N lanes" is a shape this tool now prompts for.
//!
//! # What it is NOT: a burst limiter
//!
//! It will not stop the incident's exact shape, and pretending otherwise would
//! be worse than not having it:
//!
//! - Inside one long-lived `worktrees mcp` process the readers' caches
//!   (`codex_usage::CACHE`, `claude_usage::USAGE_CACHE`, 120s) are shared, so
//!   N parallel `create_worktree` calls all read the same "under 80%" and all
//!   launch.
//! - From a shell each `worktrees new` reads fresh, but a lane only bills
//!   after its first turns. Eight started within a minute are all admitted,
//!   and this first fires once the window is ALREADY at 80%.
//!
//! What it is, honestly: "do not start another one on a nearly-spent window".
//! The lag-free signal for a burst is CONCURRENCY — busy lanes per provider
//! right now, which `tmux::PaneList::agents_in` and `activity` already derive.
//! A user-settable cap there, plus a line in the agent guidance, is the thing
//! that would have caught the incident; see ROADMAP.
//!
//! # Where it sits, and why that is `Adapter::prepare`
//!
//! On the harness's existing launch-refusal path, beside pi's unreachable
//! model host (`docs/proposals/pi-harness.md`, Decisions Q5). That buys three
//! things a gate in `cmd_new` could not have:
//!
//! - **The place and its brief are CREATED; only the launch is refused.** For
//!   a window that resets in a bounded time that is exactly right — the
//!   handoff survives, nothing is re-typed, and `worktrees open <slug>
//!   --force` is the retry.
//! - **One override for every surface.** `launch.force` is already "proceed
//!   despite an advisory you have read": the CLI's `--force`, the app's
//!   "Launch anyway" button, and `open --force`. A gate of its own with a
//!   `--ignore-quota` flag refused the app user and the MCP caller with no way
//!   to override at all.
//! - **No per-surface dispatch table.** A harness answers `usage()` or does
//!   not (pi does not), instead of a string match on a provider word that the
//!   next harness has to remember to join — which is the drift
//!   `docs/adding-a-harness.md` exists to prevent.
//!
//! # It FAILS OPEN, deliberately
//!
//! No usage data means allow. Not being able to prove you are over budget is
//! not evidence that you are, and refusing whenever the probe is slow,
//! offline, logged out or simply absent would make the tool unusable for
//! everyone who does not use that provider.
//!
//! # The threshold is the PROVIDER'S, not one we invented
//!
//! Each reader already grades its own windows (`normal` / `warning` / `over`,
//! Codex at 80%). Reusing that grade means the number moves when the
//! provider's judgement moves, and there is no second definition of "nearly
//! out" to drift.
//!
//! # …unless the USER says otherwise (`[quota]` in config.toml)
//!
//! Some people find the provider's grade too eager — a weekly window at 81% is
//! days of work left for one person and nearly nothing for eight lanes. So the
//! user may override it, in their own `config.toml` and nowhere else (`quota`
//! is in `projcfg`'s `USER_ONLY_KEYS`: a cloned repo does not get to set your
//! spending policy):
//!
//! ```toml
//! [quota]
//! gate = false            # never refuse a launch on usage
//! weekly_warn_pct = 90    # weekly windows refuse at >= 90%, whatever the grade
//! ```
//!
//! Absent, both mean exactly the behaviour above. The percentage applies to
//! WEEKLY windows only (`Window::weekly`, decided from the reader's structure,
//! never its label); the five-hour window keeps the provider's grade — it is
//! the one a parallel spawn actually destroys, and the only way out of it is
//! turning the gate off. Read at GATE time, so a change reaches the app and a
//! long-lived MCP server on their next launch, with no restart.

use std::path::PathBuf;

/// The provider's own grade for one window. Deliberately NOT a percentage
/// threshold of ours — see the module header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Normal,
    /// The provider graded this above normal: `warning`, `over`, or any other
    /// word it chooses. Kept as one variant because the gate's question is
    /// binary and the vocabulary is the provider's to extend.
    Elevated,
}

impl Severity {
    /// Anything a reader did not call `normal`. An unknown word counts as
    /// elevated: Claude's usage endpoint is unversioned, and a grade we do not
    /// recognise is still the provider saying something other than "fine".
    /// (The empty/missing case never reaches here — `claude_usage` defaults a
    /// missing severity to `normal` at parse time.)
    pub fn from_provider(word: &str) -> Self {
        if word == "normal" {
            Severity::Normal
        } else {
            Severity::Elevated
        }
    }
}

/// How full one window of a harness account's allowance is.
///
/// One shape for every harness, so the grading below is written once. The
/// readers' own row types stay theirs; each adapter maps into this.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// What the provider calls it: `5h`, `7d`, `Fable 7d`.
    pub label: String,
    pub percent: f64,
    pub severity: Severity,
    /// Unix seconds, when the window rolls over. `None` = not reported.
    pub resets_at: Option<i64>,
    /// The model this bucket is scoped to, when the provider scopes it —
    /// Claude's weekly per-model buckets. `None` = it applies to every lane.
    ///
    /// Not in the shape the review sketched, and load-bearing: without it a
    /// weekly "Fable 85%" refuses a lane launching on Opus, which shares none
    /// of that bucket.
    ///
    /// The converse is fail-open BY DESIGN: a lane launched with no model of
    /// its own (`launch.model == None` — the harness's default, a profile's
    /// `--model`, a resume) is never graded against a model-scoped bucket,
    /// because which model it will actually run on is not known here. Only
    /// the unscoped windows can refuse it.
    pub model: Option<String>,
    /// A weekly-type window, which a user's `weekly_warn_pct` grades instead
    /// of the provider. Decided from the reader's STRUCTURE — Claude's `kind`,
    /// Codex's window length — because the labels say nothing reliable: Codex
    /// labels a bucket by its NAME ("Codex"), not its span.
    pub weekly: bool,
}

// ── the user's policy ───────────────────────────────────────────────────────

/// The config table this module owns.
pub const TABLE: &str = "quota";
/// The range `weekly_warn_pct` must fall in. The Settings select's options are
/// checked against these by `app/scripts/quota-check.mjs`.
pub const PCT_MIN: u8 = 1;
pub const PCT_MAX: u8 = 100;
/// Where the user changes it — said in every refusal, so the remedy is one
/// the reader can actually follow.
pub const HOW_TO_CHANGE: &str = "The user can change when this applies in Settings → Behavior → Plan limits, \
     or `[quota]` in ~/.config/worktrees/config.toml";

/// `[quota]` as the gate reads it. `Default` is today's behaviour exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// `false`: never refuse a launch on usage.
    pub gate: bool,
    /// Weekly windows refuse at `>= N`%, overriding the provider's grade.
    /// `None`: the provider's grade, as for every other window.
    pub weekly_warn_pct: Option<u8>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { gate: true, weekly_warn_pct: None }
    }
}

/// The policy, plus what was wrong with the file.
///
/// Lenient like the rest of the user config — a typo must not lock anyone out
/// of launching, nor silently turn the gate OFF, so a bad `gate` is ON and a
/// bad percentage is the provider's grade — but never SILENT: Settings shows
/// `problems`, so a hand-edit that did nothing says so.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserPolicy {
    pub policy: Policy,
    pub problems: Vec<String>,
}

pub fn policy_from(text: &str) -> UserPolicy {
    let mut out = UserPolicy::default();
    let root: std::collections::BTreeMap<String, toml::Value> = match toml::from_str(text) {
        Ok(r) => r,
        Err(_) => {
            out.problems.push("config.toml does not parse, so [quota] could not be read — the defaults apply".into());
            return out;
        }
    };
    let Some(v) = root.get(TABLE) else { return out };
    let Some(t) = v.as_table() else {
        out.problems.push("quota must be a table ([quota]) — ignored".into());
        return out;
    };
    for (k, v) in t {
        match k.as_str() {
            "gate" => match v.as_bool() {
                Some(b) => out.policy.gate = b,
                None => out.problems.push(format!("[quota] gate = {} is not true or false — ignored, the check stays on", show(v))),
            },
            "weekly_warn_pct" => match v.as_integer() {
                Some(n) if (PCT_MIN as i64..=PCT_MAX as i64).contains(&n) => out.policy.weekly_warn_pct = Some(n as u8),
                _ => out.problems.push(format!(
                    "[quota] weekly_warn_pct = {} is not a whole number from {PCT_MIN} to {PCT_MAX} — ignored, the provider's own grade applies",
                    show(v)
                )),
            },
            other => out.problems.push(format!("[quota] unknown key `{other}` — ignored")),
        }
    }
    out
}

/// A value as the user wrote it, for a problem line.
fn show(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => format!("\"{s}\""),
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        _ => "a table or list".into(),
    }
}

/// The user's policy, read now. An absent file is the default, not a problem.
pub fn user_policy() -> UserPolicy {
    match std::fs::read_to_string(crate::config::config_toml_path()) {
        Ok(t) => policy_from(&t),
        Err(_) => UserPolicy::default(),
    }
}

/// Set the user's `[quota]` — the user's act only (Settings, or by hand); the
/// MCP server never calls this. Edits the two keys inside `[quota]` in place,
/// or adds the table at the end, leaving every other line byte for byte; the
/// result is re-parsed and must read back as `p`, or nothing is written
/// (a dotted `quota.gate = …` or an inline `quota = {…}` refuses rather than
/// guess). `weekly_warn_pct: None` REMOVES the key: absent is the provider's
/// grade, and that has no spelling of its own.
pub fn set_user_policy(p: Policy) -> Result<(), String> {
    set_user_policy_at(&crate::config::config_toml_path(), p)
}

pub fn set_user_policy_at(path: &std::path::Path, p: Policy) -> Result<(), String> {
    crate::config::edit_user_config(path, |text| with_policy(text, p).map(Some)).map(|_| ())
}

/// `text` with `[quota]` set to `p`. Pure, for the tests.
pub fn with_policy(text: &str, p: Policy) -> Result<String, String> {
    if let Some(n) = p.weekly_warn_pct {
        if !(PCT_MIN..=PCT_MAX).contains(&n) {
            return Err(format!("weekly_warn_pct must be from {PCT_MIN} to {PCT_MAX} (got {n})"));
        }
    }
    let mut ours = vec![format!("gate = {}", p.gate)];
    if let Some(n) = p.weekly_warn_pct {
        ours.push(format!("weekly_warn_pct = {n}"));
    }
    let is_key = |l: &str, k: &str| l.trim_start().strip_prefix(k).is_some_and(|r| r.trim_start().starts_with('='));
    let mut out: Vec<String> = Vec::new();
    let mut in_quota = false;
    let mut placed = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with('[') {
            in_quota = t.trim_end() == "[quota]";
            out.push(l.to_string());
            if in_quota && !placed {
                out.extend(ours.iter().cloned());
                placed = true;
            }
            continue;
        }
        if in_quota && (is_key(l, "gate") || is_key(l, "weekly_warn_pct")) {
            continue;
        }
        out.push(l.to_string());
    }
    if !placed {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("[quota]".into());
        out.extend(ours);
    }
    let mut new = out.join("\n");
    new.push('\n');
    let back = policy_from(&new);
    if back.policy != p || toml::from_str::<toml::Table>(&new).is_err() {
        return Err(
            "could not set [quota] in your config.toml safely (an unusual layout?) — edit it by hand".into(),
        );
    }
    Ok(new)
}

/// Does this window, on its own, say "nearly spent" under `policy`?
fn trips(w: &Window, policy: &Policy) -> bool {
    match (w.weekly, policy.weekly_warn_pct) {
        (true, Some(n)) => w.percent >= f64::from(n),
        _ => w.severity == Severity::Elevated,
    }
}

/// Windows that still apply, for a lane on `model`.
///
/// Two filters, both of which are the difference between a true refusal and a
/// confusing one:
///
/// - **Expired buckets are dropped.** Claude's cached reading can live up to
///   30 minutes and core does not reset-suppress it the way `codex_usage`'s
///   `answer()` does, so a stale 85% would refuse a window that has since
///   rolled over.
/// - **A model-scoped bucket only counts for a lane on that model.** A weekly
///   Fable bucket at 85% says nothing about an Opus lane.
fn applicable<'a>(
    windows: &'a [Window],
    model: Option<&str>,
    now: i64,
) -> impl Iterator<Item = &'a Window> {
    let model = model.map(|m| m.to_ascii_lowercase());
    windows.iter().filter(move |w| {
        if w.resets_at.is_some_and(|t| t <= now) {
            return false;
        }
        match w.model.as_deref() {
            None => true,
            Some(scoped) => model
                .as_deref()
                .is_some_and(|m| m.contains(&scoped.to_ascii_lowercase())),
        }
    })
}

/// The window that should refuse this launch, or `None` to allow.
///
/// The FULLEST one decides. A five-hour bucket at 95% beside a weekly at 4% is
/// not a 4% situation, and the short window is the one a parallel spawn
/// actually destroys.
pub fn worst_window<'a>(
    windows: &'a [Window],
    model: Option<&str>,
    now: i64,
    policy: &Policy,
) -> Option<&'a Window> {
    if !policy.gate {
        return None;
    }
    applicable(windows, model, now)
        .filter(|w| trips(w, policy))
        .max_by(|a, b| a.percent.total_cmp(&b.percent))
}

/// How long until `resets_at`, in words. `None` when the provider did not say.
pub fn until(resets_at: Option<i64>, now: i64) -> Option<String> {
    let at = resets_at?;
    let secs = at - now;
    if secs <= 0 {
        return Some("any moment".into());
    }
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    Some(if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") })
}

// ── the test seam ───────────────────────────────────────────────────────────

/// Env var the suites drive, read in exactly ONE place (`seam`).
///
/// Without it nothing could prove the refusal: the bats fake-shim PATH has no
/// real `codex`, and it does not shim `/usr/bin/security` (an absolute path)
/// or `curl` — so every bats `new` took the allow path and "382/382 green"
/// said nothing whatever about this gate. It also kept every CLI `new` paying
/// for a real keychain read and a real GET.
pub const PROBE_ENV: &str = "WORKTREES_USAGE_PROBE";

/// `off` → no reading at all. A path → that JSON file IS the reading.
#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    Live,
    Off,
    Fixture(PathBuf),
}

pub fn probe() -> Probe {
    match std::env::var(PROBE_ENV) {
        Err(_) => Probe::Live,
        Ok(v) if v.is_empty() => Probe::Live,
        Ok(v) if v == "off" => Probe::Off,
        Ok(v) => Probe::Fixture(PathBuf::from(v)),
    }
}

/// What a harness's `usage()` answers, honouring the seam.
///
/// `live` is a closure so the real probe — a keychain read and a GET, or a
/// `codex app-server` spawn — is never paid for when the seam is set. That is
/// what lets the bats suite and the unit tests exercise this without touching
/// the network, the keychain, or the developer's own credentials.
pub fn seam(provider_id: &str, live: impl FnOnce() -> Vec<Window>) -> Option<Vec<Window>> {
    match probe() {
        Probe::Off => None,
        Probe::Fixture(path) => Some(fixture(&path, provider_id)),
        Probe::Live => Some(live()),
    }
}

/// `{"codex": [{"label":"5h","percent":85,"severity":"warning",
/// "resets_at":null,"model":null,"weekly":false}], "claude": […]}`
///
/// Unreadable, unparsable or absent provider ⇒ no windows ⇒ allow. A fixture
/// that cannot be read must not refuse: the seam is for proving the gate, not
/// for becoming a new way to fail.
fn fixture(path: &std::path::Path, provider_id: &str) -> Vec<Window> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let Some(rows) = v.get(provider_id).and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    rows.iter()
        .map(|r| Window {
            label: r.get("label").and_then(|x| x.as_str()).unwrap_or("window").to_string(),
            percent: r.get("percent").and_then(|x| x.as_f64()).unwrap_or(0.0),
            severity: Severity::from_provider(
                r.get("severity").and_then(|x| x.as_str()).unwrap_or("normal"),
            ),
            resets_at: r.get("resets_at").and_then(|x| x.as_i64()),
            model: r.get("model").and_then(|x| x.as_str()).map(str::to_string),
            weekly: r.get("weekly").and_then(|x| x.as_bool()).unwrap_or(false),
        })
        .collect()
}

// ── the refusal ─────────────────────────────────────────────────────────────

/// The sentence a refused launch carries.
///
/// It does NOT name the retry: `ops::launch`'s `Refusal::Soft` arm already
/// appends "The place is ready; to launch anyway: worktrees open <slug>
/// --force", and the MCP boundary rewrites that for its own callers. One
/// author per sentence.
///
/// `others` is the harnesses actually installed here, never a hardcoded "the
/// other provider" — there are three, and the next one is not far off.
pub fn refusal(provider_label: &str, w: &Window, now: i64, others: &[&str], policy: &Policy) -> String {
    let when = until(w.resets_at, now)
        .map(|s| format!(", resets in {s}"))
        .unwrap_or_default();
    let alt = match others {
        [] => String::new(),
        [one] => format!(" Another agent is available here: {one}."),
        many => format!(" Other agents available here: {}.", many.join(", ")),
    };
    // Which rule fired, so "too eager" points at the knob that would change it.
    let rule = match (w.weekly, policy.weekly_warn_pct) {
        (true, Some(n)) => format!(" (the user's limit: {n}% of a weekly window)"),
        _ => format!(" ({provider_label} graded it nearly spent)"),
    };
    format!(
        "{provider_label} is at {:.0}% of its {} window{when} — not starting another agent on it{rule}.{alt} \
         Ask the user before overriding: this spends an allowance they are nearly out of. {HOW_TO_CHANGE}",
        w.percent, w.label
    )
}

/// Every other harness whose CLI is actually present, in registry order.
///
/// "Present" is each adapter's own `installed()`. Claude's is the default
/// `Ok` even with no `claude` on PATH, and that is deliberate rather than an
/// oversight to patch here: Claude is the default harness, and it is routinely
/// reached through things a PATH lookup cannot see — the installer's shell
/// alias under `~/.claude/local`, a profile's wrapper, `WORKTREES_CLAUDE_CMD`.
/// A PATH check would drop it from the alternatives for exactly those users;
/// naming it where it turns out absent costs one failed launch the user chose.
pub fn other_harnesses(this: &'static crate::provider::Provider) -> Vec<&'static str> {
    crate::harness::ALL
        .iter()
        .filter(|a| a.provider().id != this.id)
        .filter(|a| a.installed().is_ok())
        .map(|a| a.provider().label)
        .collect()
}

/// The gate itself, shared by every harness that can measure a window.
///
/// `force` is checked FIRST so an override costs no probe at all, and so is
/// the user's `gate = false`.
pub fn gate(
    adapter: &dyn crate::harness::Adapter,
    launch: &crate::profile::AiLaunch,
    now: i64,
) -> Result<(), crate::harness::Refusal> {
    if launch.force {
        return Ok(());
    }
    // Before the probe as well: a user who turned the gate off pays for no
    // keychain read, no GET and no `codex app-server` spawn on every launch.
    let policy = user_policy().policy;
    if !policy.gate {
        return Ok(());
    }
    let Some(windows) = adapter.usage() else {
        return Ok(());
    };
    let Some(w) = worst_window(&windows, launch.model.as_deref(), now, &policy) else {
        return Ok(());
    };
    let others = other_harnesses(adapter.provider());
    Err(crate::harness::Refusal::Soft(refusal(
        adapter.provider().label,
        w,
        now,
        &others,
        &policy,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(label: &str, percent: f64, sev: &str) -> Window {
        Window {
            label: label.into(),
            percent,
            severity: Severity::from_provider(sev),
            resets_at: None,
            model: None,
            weekly: false,
        }
    }

    const D: Policy = Policy { gate: true, weekly_warn_pct: None };

    fn weekly(percent: f64, sev: &str) -> Window {
        Window { weekly: true, ..w("Weekly", percent, sev) }
    }
    fn at(pct: u8) -> Policy {
        Policy { gate: true, weekly_warn_pct: Some(pct) }
    }
    const OFF: Policy = Policy { gate: false, weekly_warn_pct: None };

    #[test]
    fn no_data_fails_open() {
        // The bats suite's PATH has no codex, and so does any machine that
        // does not use it. Unknown must never mean refuse.
        assert!(worst_window(&[], None, 0, &D).is_none());
    }

    #[test]
    fn headroom_is_allowed() {
        assert!(worst_window(&[w("5h", 79.0, "normal")], None, 0, &D).is_none());
    }

    #[test]
    fn a_normal_window_never_trips_it_however_full_it_reads() {
        // Severity is the provider's judgement; percent alone is not ours to
        // reinterpret, or there are two definitions of "nearly out".
        assert!(worst_window(&[w("5h", 99.0, "normal")], None, 0, &D).is_none());
    }

    #[test]
    fn the_providers_own_warning_grade_refuses() {
        let ws = [w("5h", 80.0, "warning")];
        let got = worst_window(&ws, None, 0, &D).expect("80% is the provider's own boundary");
        assert_eq!(got.label, "5h");
    }

    #[test]
    fn an_unrecognised_grade_counts_as_elevated() {
        // Claude's endpoint is unversioned. A word we do not know is still the
        // provider declining to say "normal".
        assert!(worst_window(&[w("5h", 90.0, "critical")], None, 0, &D).is_some());
    }

    #[test]
    fn the_fullest_window_decides_not_the_first() {
        // BOTH are over, and the fullest is NOT first — with a `normal` window
        // first, `filter` drops it before `max_by` runs, so "first" and
        // "fullest" would be the same element and swapping one for the other
        // would change nothing.
        let ws = [w("5h", 82.0, "warning"), w("7d", 95.0, "warning")];
        let got = worst_window(&ws, None, 0, &D).unwrap();
        assert_eq!(got.label, "7d", "the FULLEST window must be reported");
        assert_eq!(got.percent, 95.0);
    }

    #[test]
    fn a_window_that_has_already_reset_is_not_a_full_window() {
        // Claude's cached reading can be 30 minutes old and is not
        // reset-suppressed in core, so without this a stale 85% refuses a
        // window that rolled over twenty minutes ago.
        let mut over = w("5h", 85.0, "warning");
        over.resets_at = Some(1_000);
        assert!(worst_window(&[over.clone()], None, 2_000, &D).is_none(), "expired must be dropped");
        assert!(worst_window(&[over], None, 500, &D).is_some(), "still live must still refuse");
    }

    #[test]
    fn a_model_scoped_bucket_only_grades_a_lane_on_that_model() {
        let mut fable = w("Fable 7d", 85.0, "warning");
        fable.model = Some("fable".into());
        assert!(
            worst_window(std::slice::from_ref(&fable), Some("opus"), 0, &D).is_none(),
            "an Opus lane shares none of Fable's weekly bucket"
        );
        assert!(worst_window(std::slice::from_ref(&fable), Some("claude-fable-5-1"), 0, &D).is_some());
        assert!(
            worst_window(std::slice::from_ref(&fable), None, 0, &D).is_none(),
            "a lane on the CLI's default is not known to be on that model"
        );
    }

    #[test]
    fn until_speaks_in_hours_and_minutes_and_admits_silence() {
        assert_eq!(until(Some(3_600 + 120), 0).as_deref(), Some("1h 2m"));
        assert_eq!(until(Some(300), 0).as_deref(), Some("5m"));
        assert_eq!(until(Some(10), 60).as_deref(), Some("any moment"));
        assert_eq!(until(None, 0), None, "a provider that did not say must not be guessed at");
    }

    #[test]
    fn the_refusal_names_the_window_and_asks_before_overriding() {
        let mut over = w("5h", 92.0, "warning");
        over.resets_at = Some(3_600);
        let msg = refusal("Codex", &over, 0, &["Claude", "pi"], &D);
        assert!(msg.contains("Codex is at 92% of its 5h window"), "{msg}");
        assert!(msg.contains("resets in 1h 0m"), "{msg}");
        assert!(msg.contains("Other agents available here: Claude, pi."), "{msg}");
        assert!(msg.contains("Ask the user before overriding"), "{msg}");
    }

    #[test]
    fn the_refusal_never_invents_a_second_provider() {
        // The first cut said `if provider == "codex" {"claude"} else {"codex"}`
        // — with three harnesses in the registry that is a guess, and on a
        // machine with neither installed it is advice that cannot be followed.
        let msg = refusal("Codex", &w("5h", 90.0, "warning"), 0, &[], &D);
        assert!(!msg.contains("Claude"), "{msg}");
        assert!(!msg.contains("available here"), "{msg}");
    }

    #[test]
    fn the_seam_can_switch_the_probe_off_without_running_it() {
        // `Probe::Off` must not call `live` at all: under bats the live probe
        // is a keychain read and a network GET.
        let never = || -> Vec<Window> { panic!("the live probe must not run when the seam is off") };
        temp_env(PROBE_ENV, Some("off"), || assert_eq!(seam("codex", never), None));
    }

    #[test]
    fn a_fixture_is_the_reading() {
        let dir = std::env::temp_dir().join(format!("wt-quota-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("usage.json");
        std::fs::write(
            &f,
            r#"{"codex":[{"label":"5h","percent":85,"severity":"warning"}],
                "claude":[{"label":"5h","percent":40,"severity":"normal"}]}"#,
        )
        .unwrap();
        let never = || -> Vec<Window> { panic!("a fixture must not fall through to the live probe") };
        temp_env(PROBE_ENV, Some(f.to_str().unwrap()), || {
            let codex = seam("codex", never).unwrap();
            assert_eq!(codex.len(), 1);
            assert!(worst_window(&codex, None, 0, &D).is_some(), "85% warning must refuse");
            let claude = seam("claude", never).unwrap();
            assert!(worst_window(&claude, None, 0, &D).is_none(), "40% normal must allow");
            // A provider the fixture does not mention has no windows, which is
            // allow — not a parse failure and not a refusal.
            assert_eq!(seam("pi", never).unwrap().len(), 0);
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_fixture_allows_rather_than_refusing() {
        let never = || -> Vec<Window> { panic!("must not fall through to the live probe") };
        temp_env(PROBE_ENV, Some("/nonexistent/usage.json"), || {
            assert_eq!(seam("codex", never).unwrap().len(), 0);
        });
    }

    // ── the user's [quota] policy ───────────────────────────────────────────

    #[test]
    fn the_default_policy_is_todays_behaviour() {
        assert_eq!(Policy::default(), D);
        assert_eq!(policy_from(""), UserPolicy::default());
        // Unset threshold: the provider's grade, on weekly windows too.
        assert!(worst_window(&[weekly(81.0, "warning")], None, 0, &D).is_some());
        assert!(worst_window(&[weekly(99.0, "normal")], None, 0, &D).is_none());
    }

    #[test]
    fn gate_off_never_refuses_however_full() {
        let ws = [w("5h", 99.0, "over"), weekly(100.0, "over")];
        assert!(worst_window(&ws, None, 0, &OFF).is_none());
        // and a threshold beside it does not turn it back on
        let off_at_50 = Policy { gate: false, weekly_warn_pct: Some(50) };
        assert!(worst_window(&ws, None, 0, &off_at_50).is_none());
    }

    #[test]
    fn a_weekly_threshold_replaces_the_providers_grade_in_both_directions() {
        // 85% graded `warning` is UNDER the user's 90: allowed.
        assert!(worst_window(&[weekly(85.0, "warning")], None, 0, &at(90)).is_none());
        // 92% is over it: refused, whatever the grade says.
        assert!(worst_window(&[weekly(92.0, "warning")], None, 0, &at(90)).is_some());
        assert!(worst_window(&[weekly(92.0, "normal")], None, 0, &at(90)).is_some(), "the user's number, not the grade");
        // the boundary is inclusive: "warn at 90%" means 90% warns
        assert!(worst_window(&[weekly(90.0, "normal")], None, 0, &at(90)).is_some());
        assert!(worst_window(&[weekly(89.9, "warning")], None, 0, &at(90)).is_none());
    }

    #[test]
    fn the_five_hour_window_keeps_the_providers_grade_under_a_weekly_threshold() {
        // The knob is WEEKLY. A 5h window graded `warning` at 82% still
        // refuses with the threshold at 90, and a `normal` 95% still allows.
        assert!(worst_window(&[w("5h", 82.0, "warning")], None, 0, &at(90)).is_some());
        assert!(worst_window(&[w("5h", 95.0, "normal")], None, 0, &at(50)).is_none());
    }

    #[test]
    fn a_threshold_still_fails_open_and_still_drops_expired_and_other_models() {
        assert!(worst_window(&[], None, 0, &at(1)).is_none(), "no data is still allow");
        let mut stale = weekly(95.0, "over");
        stale.resets_at = Some(100);
        assert!(worst_window(&[stale], None, 200, &at(50)).is_none(), "a reset window is not full");
        let mut fable = weekly(95.0, "over");
        fable.model = Some("fable".into());
        assert!(worst_window(&[fable], Some("opus"), 0, &at(50)).is_none(), "another model's bucket");
    }

    #[test]
    fn the_policy_reads_its_table_and_reports_what_it_ignored() {
        let p = policy_from("ai_cmd = \"x\"\n[quota]\ngate = false\nweekly_warn_pct = 90\n");
        assert_eq!(p.policy, Policy { gate: false, weekly_warn_pct: Some(90) });
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        assert_eq!(policy_from("[quota]\nweekly_warn_pct = 1\n").policy.weekly_warn_pct, Some(1));
        assert_eq!(policy_from("[quota]\nweekly_warn_pct = 100\n").policy.weekly_warn_pct, Some(100));

        // Nonsense is IGNORED (a typo must not lock anyone out, nor turn the
        // gate off) and REPORTED (Settings shows it, so a no-op edit says so).
        for bad in ["0", "101", "-5", "90.5", "\"90\"", "true"] {
            let p = policy_from(&format!("[quota]\nweekly_warn_pct = {bad}\n"));
            assert_eq!(p.policy, D, "{bad}");
            assert_eq!(p.problems.len(), 1, "{bad}");
            assert!(p.problems[0].contains("from 1 to 100"), "{bad}: {}", p.problems[0]);
        }
        let p = policy_from("[quota]\ngate = \"no\"\n");
        assert!(p.policy.gate, "a gate value we cannot read is ON, never off");
        assert!(p.problems[0].contains("not true or false"), "{:?}", p.problems);
        let p = policy_from("[quota]\ngate_off = true\n");
        assert!(p.problems[0].contains("unknown key `gate_off`"), "{:?}", p.problems);
        let p = policy_from("quota = 3\n");
        assert_eq!(p.policy, D);
        assert_eq!(p.problems.len(), 1);
        let p = policy_from("[quota\n");
        assert_eq!(p.policy, D);
        assert!(p.problems[0].contains("does not parse"), "{:?}", p.problems);
    }

    #[test]
    fn the_refusal_says_which_rule_fired_and_where_to_change_it() {
        let msg = refusal("Claude", &weekly(92.0, "normal"), 0, &[], &at(90));
        assert!(msg.contains("Claude is at 92% of its Weekly window"), "{msg}");
        assert!(msg.contains("the user's limit: 90% of a weekly window"), "{msg}");
        assert!(msg.contains("Settings → Behavior → Plan limits"), "{msg}");
        assert!(msg.contains("`[quota]` in ~/.config/worktrees/config.toml"), "{msg}");
        let msg = refusal("Codex", &w("5h", 85.0, "warning"), 0, &[], &at(90));
        assert!(msg.contains("Codex graded it nearly spent"), "{msg}");
        assert!(msg.contains("Settings → Behavior → Plan limits"), "{msg}");
    }

    #[test]
    fn a_fixture_window_is_weekly_only_when_it_says_so() {
        let dir = std::env::temp_dir().join(format!("wt-quota-wk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("usage.json");
        std::fs::write(
            &f,
            r#"{"claude":[{"label":"Weekly","percent":85,"severity":"warning","weekly":true},
                          {"label":"Session","percent":85,"severity":"warning"}]}"#,
        )
        .unwrap();
        let never = || -> Vec<Window> { panic!("fixture only") };
        temp_env(PROBE_ENV, Some(f.to_str().unwrap()), || {
            let ws = seam("claude", never).unwrap();
            assert!(ws[0].weekly);
            assert!(!ws[1].weekly, "absent means not weekly");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setting_the_policy_edits_only_its_own_keys() {
        let before = "# mine\nai_cmd = \"codex\"\n\n[trust]\npi = [\"/r\"]\n";
        let p = Policy { gate: false, weekly_warn_pct: Some(90) };
        let after = with_policy(before, p).unwrap();
        assert!(after.starts_with(before), "everything else byte for byte:\n{after}");
        assert!(after.ends_with("\n[quota]\ngate = false\nweekly_warn_pct = 90\n"), "{after}");
        assert_eq!(policy_from(&after).policy, p);
        // a second write edits in place: no second table, no duplicate key
        let again = with_policy(&after, Policy { gate: true, weekly_warn_pct: None }).unwrap();
        assert_eq!(again.matches("[quota]").count(), 1, "{again}");
        assert!(!again.contains("weekly_warn_pct"), "None removes the key: {again}");
        assert_eq!(policy_from(&again).policy, D);
        // a comment inside the table, and a key after it, survive
        let hand = "[quota]\n# I keep this\ngate = true\n[model]\npi = \"x\"\n";
        let got = with_policy(hand, at(75)).unwrap();
        assert!(got.contains("# I keep this"), "{got}");
        assert!(got.contains("[model]\npi = \"x\""), "{got}");
        assert_eq!(policy_from(&got).policy, at(75));
        assert!(policy_from(&got).problems.is_empty());
        // from nothing
        assert_eq!(with_policy("", OFF).unwrap(), "[quota]\ngate = false\n");
    }

    #[test]
    fn setting_the_policy_refuses_nonsense_and_layouts_it_cannot_edit() {
        for n in [0u8, 101, 255] {
            let e = with_policy("", at(n)).unwrap_err();
            assert!(e.contains("from 1 to 100"), "{n}: {e}");
        }
        // dotted keys / an inline table: refuse rather than write a duplicate
        assert!(with_policy("quota.gate = true\n", OFF).is_err());
        assert!(with_policy("quota = { gate = true }\n", OFF).is_err());
    }

    #[test]
    fn set_user_policy_at_writes_atomically_and_creates_the_dir() {
        let dir = std::env::temp_dir().join(format!("wt-quota-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("worktrees").join("config.toml");
        set_user_policy_at(&f, at(88)).unwrap();
        assert_eq!(policy_from(&std::fs::read_to_string(&f).unwrap()).policy, at(88));
        assert!(set_user_policy_at(&f, at(0)).is_err());
        assert_eq!(policy_from(&std::fs::read_to_string(&f).unwrap()).policy, at(88), "a refused write writes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_user_policy_edits_a_symlinked_config_through_the_link() {
        crate::config::assert_writes_through_a_link(
            "quota",
            |p| set_user_policy_at(p, at(80)).unwrap(),
            |t| policy_from(t).policy == at(80),
        );
    }

    /// Set an env var for the duration of `f`. Tests that use this are run
    /// single-threaded by the `_env_serial` guard below — `std::env::set_var`
    /// is process-global and Rust runs tests in parallel by default.
    fn temp_env(key: &str, val: Option<&str>, f: impl FnOnce()) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(key).ok();
        match val {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        match prev {
            Some(p) => std::env::set_var(key, p),
            None => std::env::remove_var(key),
        }
        if let Err(e) = out {
            std::panic::resume_unwind(e);
        }
    }
}

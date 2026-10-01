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
pub fn worst_window<'a>(windows: &'a [Window], model: Option<&str>, now: i64) -> Option<&'a Window> {
    applicable(windows, model, now)
        .filter(|w| w.severity == Severity::Elevated)
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
/// "resets_at":null,"model":null}], "claude": […]}`
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
pub fn refusal(provider_label: &str, w: &Window, now: i64, others: &[&str]) -> String {
    let when = until(w.resets_at, now)
        .map(|s| format!(", resets in {s}"))
        .unwrap_or_default();
    let alt = match others {
        [] => String::new(),
        [one] => format!(" Another agent is available here: {one}."),
        many => format!(" Other agents available here: {}.", many.join(", ")),
    };
    format!(
        "{provider_label} is at {:.0}% of its {} window{when} — not starting another agent on it.{alt} \
         Ask the user before overriding: this spends an allowance they are nearly out of",
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
/// `force` is checked FIRST so an override costs no probe at all.
pub fn gate(
    adapter: &dyn crate::harness::Adapter,
    launch: &crate::profile::AiLaunch,
    now: i64,
) -> Result<(), crate::harness::Refusal> {
    if launch.force {
        return Ok(());
    }
    let Some(windows) = adapter.usage() else {
        return Ok(());
    };
    let Some(w) = worst_window(&windows, launch.model.as_deref(), now) else {
        return Ok(());
    };
    let others = other_harnesses(adapter.provider());
    Err(crate::harness::Refusal::Soft(refusal(
        adapter.provider().label,
        w,
        now,
        &others,
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
        }
    }

    #[test]
    fn no_data_fails_open() {
        // The bats suite's PATH has no codex, and so does any machine that
        // does not use it. Unknown must never mean refuse.
        assert!(worst_window(&[], None, 0).is_none());
    }

    #[test]
    fn headroom_is_allowed() {
        assert!(worst_window(&[w("5h", 79.0, "normal")], None, 0).is_none());
    }

    #[test]
    fn a_normal_window_never_trips_it_however_full_it_reads() {
        // Severity is the provider's judgement; percent alone is not ours to
        // reinterpret, or there are two definitions of "nearly out".
        assert!(worst_window(&[w("5h", 99.0, "normal")], None, 0).is_none());
    }

    #[test]
    fn the_providers_own_warning_grade_refuses() {
        let ws = [w("5h", 80.0, "warning")];
        let got = worst_window(&ws, None, 0).expect("80% is the provider's own boundary");
        assert_eq!(got.label, "5h");
    }

    #[test]
    fn an_unrecognised_grade_counts_as_elevated() {
        // Claude's endpoint is unversioned. A word we do not know is still the
        // provider declining to say "normal".
        assert!(worst_window(&[w("5h", 90.0, "critical")], None, 0).is_some());
    }

    #[test]
    fn the_fullest_window_decides_not_the_first() {
        // BOTH are over, and the fullest is NOT first — with a `normal` window
        // first, `filter` drops it before `max_by` runs, so "first" and
        // "fullest" would be the same element and swapping one for the other
        // would change nothing.
        let ws = [w("5h", 82.0, "warning"), w("7d", 95.0, "warning")];
        let got = worst_window(&ws, None, 0).unwrap();
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
        assert!(worst_window(&[over.clone()], None, 2_000).is_none(), "expired must be dropped");
        assert!(worst_window(&[over], None, 500).is_some(), "still live must still refuse");
    }

    #[test]
    fn a_model_scoped_bucket_only_grades_a_lane_on_that_model() {
        let mut fable = w("Fable 7d", 85.0, "warning");
        fable.model = Some("fable".into());
        assert!(
            worst_window(std::slice::from_ref(&fable), Some("opus"), 0).is_none(),
            "an Opus lane shares none of Fable's weekly bucket"
        );
        assert!(worst_window(std::slice::from_ref(&fable), Some("claude-fable-5-1"), 0).is_some());
        assert!(
            worst_window(std::slice::from_ref(&fable), None, 0).is_none(),
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
        let msg = refusal("Codex", &over, 0, &["Claude", "pi"]);
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
        let msg = refusal("Codex", &w("5h", 90.0, "warning"), 0, &[]);
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
            assert!(worst_window(&codex, None, 0).is_some(), "85% warning must refuse");
            let claude = seam("claude", never).unwrap();
            assert!(worst_window(&claude, None, 0).is_none(), "40% normal must allow");
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

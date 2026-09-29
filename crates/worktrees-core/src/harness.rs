//! Per-harness adapters: what each agent CLI in `provider::PROVIDERS` does
//! differently, behind one trait, so a caller loops over `ALL` instead of
//! spelling out "claude, then codex".
//!
//! `provider::Provider` stays the registry — static identity (id, label, the
//! program word, the sidecar suffix). Everything here is BEHAVIOUR: how a
//! launch is composed, how a resume is spelled and when it is allowed, how
//! activity is read, how a message is delivered. The two are kept apart on
//! purpose; a static table cannot express `--session-id <derived>` or "look at
//! the rollout, then maybe at the pane".
//!
//! Two shape rules, from the shared harness × model proposal (pi-harness §2.3,
//! opencode-harness §3.4), so a third harness does not have to re-cut this:
//!
//! - **A launch is not assumed to be argv-only or stateless.** `launch_env`
//!   exists beside `launch_args` (empty for both harnesses today), and every
//!   reader takes a `Scan` — the per-tick context a caller fetched once —
//!   rather than a bare pane list, so a harness whose activity and delivery
//!   are API calls against a per-launch runtime handle has somewhere to carry
//!   it without changing every signature again.
//! - **The pane is not the only channel.** `send` returns a `Delivery`, and
//!   the harness owns both the delivery and its confirmation.
//!
//! Only methods with a caller live on the trait. Model choice (`model_arg`,
//! `ModelRef`, a catalog) arrives with its first consumer.

use crate::activity::{self, Activity, State};
use crate::profile::{shell_quote, AiLaunch};
use crate::provider::{self, Provider};
use crate::{agent, tmux, Project};

/// What a caller has already fetched this tick, handed to every adapter so N
/// harnesses still cost ONE probe scan and ONE `list-panes`.
pub struct Scan<'a> {
    pub probes: &'a [agent::ClaudeProbe],
    pub panes: Option<&'a tmux::PaneList>,
}

/// Shell words a harness adds around the configured command, already quoted
/// for the inner `sh -ic`. `head` goes right after the executable — before a
/// resume SUBCOMMAND (`codex … resume --last`), which would otherwise swallow
/// them — and `tail` after the whole command, before the opener.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LaunchArgs {
    pub head: Vec<String>,
    pub tail: Vec<String>,
}

/// A `send` request: the place, the snapshot, and the already-attributed text.
pub struct SendRequest<'a> {
    pub panes: &'a tmux::PaneList,
    pub canonical: &'a str,
    pub path: &'a str,
    /// Subtree whose panes never count (the main checkout passes `.worktrees/`).
    pub exclude: Option<&'a str>,
    /// What to type, attribution label included.
    pub typed: &'a str,
    /// This harness's own reading for the place, from `Adapter::activity`.
    pub reading: &'a Activity,
}

/// How a `send` went. The harness decides, because only it knows whether its
/// input can be typed into and what counts as a receipt.
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Delivered into `session`; `outcome` says whether it was confirmed.
    Typed { session: String, outcome: SendOutcome },
    /// Refused outright: the reason, to hand back unchanged.
    Refused(String),
    /// This harness has its own bus and is not typed into. Used only when no
    /// other harness in the place took the message — the reason names where
    /// to send instead.
    Elsewhere(String),
}

/// Whether a typed message was confirmed submitted.
#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Submitted,
    Modal,
    Unconfirmed,
    EnterFailed(String),
}

impl SendOutcome {
    pub fn note(&self) -> String {
        match self {
            Self::Submitted => "Typed into its prompt and confirmed submitted; Codex queues it if a turn is running. Ask it to report back, then wait until: message.".into(),
            Self::Modal => "Codex opened an approval or a question while sending. No Enter was pressed into that prompt. The text may be sitting in its input; the user has to answer the prompt. Submission is not confirmed; the message copy is left unread if recorded. Submitting the composer later and reading messages can deliver the same instruction twice; check the composer and inbox before resending.".into(),
            Self::Unconfirmed => "Text was typed but submission could not be confirmed within the retry limit. It may still be in the composer. The message copy is left unread if recorded; read it with messages before retrying to avoid duplicates.".into(),
            Self::EnterFailed(e) => format!("Text was typed but Enter failed: {e}. Submission is not confirmed. The message copy is left unread if recorded; read it with messages before retrying to avoid duplicates."),
        }
    }
}

pub trait Adapter: Sync {
    /// This harness's registry row.
    fn provider(&self) -> &'static Provider;

    /// Flags derived from the PLACE at launch time (Codex's permission mode,
    /// which needs the worktree's git common dir). Carried on
    /// `AiLaunch::place_flags` and emitted by `launch_args`.
    fn place_flags(&self, _wt: &str) -> Vec<String> {
        Vec::new()
    }

    /// The words this harness adds to `launch`'s command when it runs as tmux
    /// session `session` (empty `session`: unnamed).
    fn launch_args(&self, launch: &AiLaunch, session: &str) -> LaunchArgs;

    /// Environment a launch needs beyond the profile's. Empty for Claude and
    /// Codex. Emitted today with the profile's env, as the pane shell's prefix
    /// (`AiLaunch::shell_prefix`); a per-launch SECRET (opencode's server
    /// password) must not ride there, since that string is tmux's argv — the
    /// harness that needs one chooses the channel.
    fn launch_env(&self, _launch: &AiLaunch) -> Vec<(String, String)> {
        Vec::new()
    }

    /// The resume words appended to the command (`-r`, `resume --last`). A
    /// method, not a registry string: a harness that resumes an exact session
    /// derives the id from the place.
    fn resume_arg(&self, cwd: &str) -> String;

    /// Whether this harness has a conversation on disk for `cwd`.
    fn session_present(&self, project: &Project, cwd: &str) -> bool;

    /// Whether a resume requested in `cwd` may actually be launched. Claude's
    /// `-r` is harmless with nothing to resume; Codex's `resume --last` would
    /// pick up a conversation from ANOTHER directory, so it needs one here.
    fn may_resume(&self, _project: &Project, _cwd: &str) -> bool {
        true
    }

    /// What this harness is doing in the place, or `None` when it is not there.
    fn activity(&self, scan: &Scan, canonical: &str, path: &str) -> Option<Activity>;

    /// `place_status`'s `agents` rows for this harness. The default is one row
    /// from the reading; Claude lists every probe (forks, a hand-started one).
    fn agents(&self, _scan: &Scan, _path: &str, reading: Option<&Activity>) -> Vec<serde_json::Value> {
        reading
            .map(|x| {
                serde_json::json!({
                    "provider": self.provider().id,
                    "state": x.state,
                    "tmux": x.session,
                    "last_done": x.last_done,
                })
            })
            .into_iter()
            .collect()
    }

    /// Deliver `req.typed` to this harness's agent in the place.
    fn send(&self, req: &SendRequest) -> Delivery;
}

pub struct Claude;
pub struct Codex;

pub const CLAUDE: &Claude = &Claude;
pub const CODEX: &Codex = &Codex;

/// Every adapter, in registry order (`provider::PROVIDERS`). A new harness is
/// a row there and an entry here; `registry_and_adapters_line_up` pins it.
pub const ALL: &[&dyn Adapter] = &[CLAUDE, CODEX];

pub fn by_id(id: &str) -> Option<&'static dyn Adapter> {
    ALL.iter().copied().find(|a| a.provider().id == id)
}

pub fn by_word(word: &str) -> Option<&'static dyn Adapter> {
    ALL.iter().copied().find(|a| a.provider().match_word == word)
}

/// The adapter for a configured AI command, by its program word.
pub fn for_cmd(ai_cmd: &str) -> Option<&'static dyn Adapter> {
    by_word(&crate::profile::ai_word_of(ai_cmd))
}

/// The harness a place gets when nothing names one — also whose resume words
/// a non-harness `ai_cmd` inherits.
pub fn default_adapter() -> &'static dyn Adapter {
    ALL.iter().copied().find(|a| a.provider().canonical_default).unwrap_or(CLAUDE)
}

impl Adapter for Claude {
    fn provider(&self) -> &'static Provider {
        provider::CLAUDE
    }

    /// `--name <session>`: what makes the agent addressable by claude's own
    /// cross-session messaging (see `AiLaunch::launch_cmd`).
    fn launch_args(&self, _launch: &AiLaunch, session: &str) -> LaunchArgs {
        let tail = match self.provider().name_arg {
            Some(arg) if !session.is_empty() => vec![arg.to_string(), shell_quote(session)],
            _ => Vec::new(),
        };
        LaunchArgs { head: Vec::new(), tail }
    }

    fn resume_arg(&self, _cwd: &str) -> String {
        "-r".into()
    }

    fn session_present(&self, project: &Project, cwd: &str) -> bool {
        project.claude_session_present(cwd)
    }

    fn activity(&self, scan: &Scan, _canonical: &str, path: &str) -> Option<Activity> {
        activity::claude_activity(scan.probes, path)
    }

    fn agents(&self, scan: &Scan, path: &str, _reading: Option<&Activity>) -> Vec<serde_json::Value> {
        agent::agents_at(scan.probes, path).iter().filter_map(|a| serde_json::to_value(a).ok()).collect()
    }

    fn send(&self, req: &SendRequest) -> Delivery {
        let name = req.reading.session.clone().unwrap_or_else(|| req.canonical.to_string());
        Delivery::Elsewhere(format!(
            "runs Claude, which is not typed into. Claude sessions have their own \
             messaging: use SendMessage to \"{name}\" (its full tmux session name), or post \
             with report and it reads it via messages."
        ))
    }
}

impl Adapter for Codex {
    fn provider(&self) -> &'static Provider {
        provider::CODEX
    }

    fn place_flags(&self, wt: &str) -> Vec<String> {
        crate::codex::launch_flags(wt)
    }

    /// Worktrees-owned Codex panes use ChatGPT account sign-in, and
    /// `project_doc_fallback_filenames` makes Codex read a directory's
    /// CLAUDE.md when it has no AGENTS.md (measured with `codex debug
    /// prompt-input`, 0.157.1: nested directories too, and AGENTS.md still
    /// wins where both exist) — so a CLAUDE.md-only project briefs Codex with
    /// no repo change. Per-launch `-c`, never a write to Codex's config, and
    /// Codex's own browser OAuth flow and credential store stay the CLI's. No
    /// Worktrees API key is created or read.
    fn launch_args(&self, launch: &AiLaunch, _session: &str) -> LaunchArgs {
        let mut head = vec![
            "-c".to_string(),
            "forced_login_method=chatgpt".to_string(),
            "-c".to_string(),
            shell_quote(crate::profile::CODEX_DOC_FALLBACK),
        ];
        head.extend(launch.place_flags.iter().cloned());
        LaunchArgs { head, tail: Vec::new() }
    }

    fn resume_arg(&self, _cwd: &str) -> String {
        "resume --last".into()
    }

    fn session_present(&self, _project: &Project, cwd: &str) -> bool {
        crate::codex::session_present(cwd)
    }

    fn may_resume(&self, project: &Project, cwd: &str) -> bool {
        self.session_present(project, cwd)
    }

    fn activity(&self, scan: &Scan, canonical: &str, path: &str) -> Option<Activity> {
        scan.panes.and_then(|p| activity::codex_activity(p, canonical, path))
    }

    fn send(&self, req: &SendRequest) -> Delivery {
        let codex = activity::codex_session_for(req.panes, req.canonical);
        // A modal (an approval, a plan-mode question) takes typed keys as its
        // ANSWER: text plus Enter confirms the highlighted option, which is
        // "Yes, proceed". So a Codex that is waiting on someone is never typed
        // into, and neither is one that has exited.
        if let Err(e) = may_type(req.reading.state) {
            return Delivery::Refused(e);
        }
        // Ownership by what the pane IS, not by its session's name: in this
        // place, and running codex. No fallback.
        let word = self.provider().match_word;
        let Some(pane) = tmux::agent_pane(&codex, req.path, req.exclude, word) else {
            return Delivery::Refused(format!(
                "{codex} has no pane running codex in {}; send only types into this \
                 project's own Codex pane. Use report instead.",
                req.path
            ));
        };
        if let Err(e) = tmux::send_literal(&pane, req.typed) {
            return Delivery::Refused(format!("could not type into {codex}: {e}"));
        }
        let t0 = std::time::Instant::now();
        let outcome = submit_codex(
            || tmux::capture(&pane),
            || tmux::press_enter(&pane),
            || t0.elapsed().as_millis() as u64,
            |ms| std::thread::sleep(std::time::Duration::from_millis(ms)),
        );
        Delivery::Typed { session: codex, outcome }
    }
}

/// Every harness's reading for one place, in registry order, from a probe scan
/// and a pane snapshot the caller already holds (so a caller asking about
/// several places pays for each once). Only harnesses that are there.
pub fn place_activities(
    project: &Project,
    slug: &str,
    path: &str,
    scan: &Scan,
) -> Vec<(&'static dyn Adapter, Activity)> {
    let canonical = project.session_name(slug);
    ALL.iter().filter_map(|a| a.activity(scan, &canonical, path).map(|x| (*a, x))).collect()
}

// Bounds cover both paste conversion and lost Enter retries, without letting
// a changing or unreadable screen hold an MCP request forever.
pub const SEND_POLL_MS: u64 = 100;
pub const SEND_STABLE_MS: u64 = 300;
pub const SEND_VERIFY_MS: u64 = 1_000;
pub const SEND_TIMEOUT_MS: u64 = 8_000;
pub const SEND_ENTER_TRIES: usize = 3;

/// Poll only the live composer, so transcript animation cannot prevent a
/// settle. Every Enter (including retries) is gated by a fresh, recognized,
/// nonempty composer and the modal guard. Failed captures never authorize a
/// keypress or count as confirmation. Clock and I/O seams keep timing tests
/// deterministic; production uses exactly this loop.
pub fn submit_codex(
    mut capture: impl FnMut() -> Option<String>,
    mut enter: impl FnMut() -> Result<(), String>,
    now: impl Fn() -> u64,
    mut sleep: impl FnMut(u64),
) -> SendOutcome {
    use crate::codex::{composer_settled, composer_submitted, waiting_on_screen};
    let start = now();
    let mut previous = String::new();
    let mut stable_since = start;
    let mut tries = 0;
    let mut last_enter = None;
    loop {
        let time = now();
        if time.saturating_sub(start) >= SEND_TIMEOUT_MS {
            return SendOutcome::Unconfirmed;
        }
        if let Some(screen) = capture() {
            if waiting_on_screen(&screen) {
                return SendOutcome::Modal;
            }
            if tries > 0 && composer_submitted(&screen) {
                return SendOutcome::Submitted;
            }
            if !composer_settled(&previous, &screen) {
                stable_since = time;
            } else if time.saturating_sub(stable_since) >= SEND_STABLE_MS
                && last_enter.is_none_or(|at| time.saturating_sub(at) >= SEND_VERIFY_MS)
            {
                if tries == SEND_ENTER_TRIES {
                    return SendOutcome::Unconfirmed;
                }
                if let Err(e) = enter() {
                    return SendOutcome::EnterFailed(e);
                }
                tries += 1;
                last_enter = Some(time);
                stable_since = time;
            }
            previous = screen;
        } else {
            previous.clear();
            stable_since = time;
        }
        sleep(SEND_POLL_MS);
    }
}

/// Whether a Codex in `state` may be typed into. Only a Codex that is running
/// and not stopped on someone: `Waiting` means a modal is up and would take the
/// keys as its answer; `None` means codex has exited and the pane is a shell.
pub fn may_type(state: State) -> Result<(), String> {
    match state {
        State::Busy | State::Idle => Ok(()),
        State::Waiting => Err(
            "Codex is waiting on you (an approval or a question) — typing would answer it. Answer \
             it, or wait until: idle first."
                .into(),
        ),
        State::None => Err("Codex is not running there (its pane is back at a shell). Use report.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_and_adapters_line_up() {
        let ids: Vec<&str> = ALL.iter().map(|a| a.provider().id).collect();
        assert_eq!(ids, provider::ids(), "one adapter per registry row, in registry order");
        assert_eq!(default_adapter().provider().id, "claude");
    }

    #[test]
    fn resume_words_are_the_registry_strings_they_replace() {
        assert_eq!(CLAUDE.resume_arg("/w"), "-r");
        assert_eq!(CODEX.resume_arg("/w"), "resume --last");
    }

    #[test]
    fn launch_args_split_head_and_tail() {
        let plain = AiLaunch::plain("claude");
        assert_eq!(
            CLAUDE.launch_args(&plain, "p-feat"),
            LaunchArgs { head: vec![], tail: vec!["--name".into(), "'p-feat'".into()] }
        );
        assert_eq!(CLAUDE.launch_args(&plain, ""), LaunchArgs::default(), "an unnamed launch gets no --name");
        let mut codex = AiLaunch::plain("codex");
        codex.place_flags = vec!["--sandbox".into(), "workspace-write".into()];
        let a = CODEX.launch_args(&codex, "p-feat");
        assert!(a.tail.is_empty(), "codex takes no --name");
        assert_eq!(&a.head[..2], &["-c".to_string(), "forced_login_method=chatgpt".to_string()]);
        assert_eq!(&a.head[4..], &["--sandbox".to_string(), "workspace-write".to_string()]);
        assert!(ALL.iter().all(|a| a.launch_env(&plain).is_empty()), "no harness needs launch env yet");
    }

    const SEND_EMPTY: &str =
        include_str!("../tests/fixtures/codex-send/empty.txt");
    const SEND_TYPED: &str =
        include_str!("../tests/fixtures/codex-send/typed.txt");
    const SEND_PASTED: &str =
        include_str!("../tests/fixtures/codex-send/pasted.txt");
    const SEND_MODAL: &str = "  Press enter to confirm or esc to cancel\n";

    #[test]
    fn send_waits_for_conversion_and_retries_a_lost_enter() {
        use std::cell::{Cell, RefCell};
        let clock = Cell::new(0);
        let presses = RefCell::new(Vec::new());
        let outcome = submit_codex(
            || {
                Some(
                    if presses.borrow().len() >= 2 {
                        SEND_EMPTY
                    } else if clock.get() < 200 {
                        SEND_EMPTY
                    } else if clock.get() < 400 {
                        SEND_TYPED
                    } else {
                        SEND_PASTED
                    }
                    .into(),
                )
            },
            || {
                presses.borrow_mut().push(clock.get());
                Ok(())
            },
            || clock.get(),
            |ms| clock.set(clock.get() + ms),
        );
        assert_eq!(outcome, SendOutcome::Submitted);
        assert_eq!(*presses.borrow(), vec![700, 1700]);
    }

    #[test]
    fn send_review_busy_input_queues_and_confirms_without_retry() {
        use std::cell::Cell;
        for (typed, queued) in [
            (include_str!("../tests/fixtures/codex-send/review-busy-typed.txt"),
             include_str!("../tests/fixtures/codex-send/review-busy-queued.txt")),
            (include_str!("../tests/fixtures/codex-send/busy-no-status-typed.txt"),
             include_str!("../tests/fixtures/codex-send/busy-no-status-queued.txt")),
        ] {
            let clock = Cell::new(0);
            let presses = Cell::new(0);
            let outcome = submit_codex(
                || Some(if presses.get() == 0 {
                    typed
                } else {
                    queued
                }.into()),
                || { presses.set(presses.get() + 1); Ok(()) },
                || clock.get(), |ms| clock.set(clock.get() + ms),
            );
            assert_eq!(outcome, SendOutcome::Submitted);
            assert_eq!(presses.get(), 1);
        }
    }

    #[test]
    fn send_stuck_paste_is_bounded_and_unknown_screens_never_confirm() {
        use std::cell::Cell;
        for screen in [Some(SEND_PASTED), Some(SEND_EMPTY), Some(""), None] {
            let clock = Cell::new(0);
            let presses = Cell::new(0);
            let outcome = submit_codex(
                || screen.map(str::to_string),
                || {
                    presses.set(presses.get() + 1);
                    Ok(())
                },
                || clock.get(),
                |ms| clock.set(clock.get() + ms),
            );
            assert_eq!(outcome, SendOutcome::Unconfirmed);
            assert_eq!(
                presses.get(),
                if screen == Some(SEND_PASTED) { 3 } else { 0 }
            );
            assert!(clock.get() <= SEND_TIMEOUT_MS);
        }
        let clock = Cell::new(0);
        let presses = Cell::new(0);
        assert_eq!(
            submit_codex(
                || if presses.get() == 0 {
                    Some(SEND_TYPED.into())
                } else {
                    None
                },
                || {
                    presses.set(presses.get() + 1);
                    Ok(())
                },
                || clock.get(),
                |ms| clock.set(clock.get() + ms),
            ),
            SendOutcome::Unconfirmed
        );
        assert_eq!(
            presses.get(),
            1,
            "capture failure after Enter must not authorize a retry"
        );
    }

    #[test]
    fn send_async_question_banner_confirms_only_after_the_composer_clears() {
        use std::cell::Cell;
        let typed = include_str!("../tests/fixtures/codex-send/probe-question-banner-typed.txt");
        let pasted = include_str!("../tests/fixtures/codex-send/probe-question-banner-pasted.txt");
        let submitted = include_str!("../tests/fixtures/codex-send/probe-question-submitted.txt");
        for input in [typed, pasted] {
            // One lost Enter retries; a stuck composer never becomes delivered.
            for clears in [true, false] {
                let clock = Cell::new(0);
                let presses = Cell::new(0);
                let outcome = submit_codex(
                    || Some(if clears && presses.get() >= 2 { submitted } else { input }.into()),
                    || { presses.set(presses.get() + 1); Ok(()) },
                    || clock.get(),
                    |ms| clock.set(clock.get() + ms),
                );
                assert_eq!(outcome, if clears { SendOutcome::Submitted } else { SendOutcome::Unconfirmed });
                assert_eq!(presses.get(), if clears { 2 } else { SEND_ENTER_TRIES });
                assert!(clock.get() <= SEND_TIMEOUT_MS);
            }
        }
    }

    #[test]
    fn send_checks_modals_before_initial_enter_and_every_retry() {
        use std::cell::Cell;
        for before_first in [true, false] {
            let clock = Cell::new(0);
            let presses = Cell::new(0);
            assert_eq!(
                submit_codex(
                    || Some(
                        if before_first || presses.get() > 0 {
                            SEND_MODAL
                        } else {
                            SEND_PASTED
                        }
                        .into()
                    ),
                    || {
                        presses.set(presses.get() + 1);
                        Ok(())
                    },
                    || clock.get(),
                    |ms| clock.set(clock.get() + ms),
                ),
                SendOutcome::Modal
            );
            assert_eq!(presses.get(), if before_first { 0 } else { 1 });
        }
    }

    #[test]
    fn send_changing_composer_times_out_and_enter_errors_are_unconfirmed() {
        use std::cell::Cell;
        let clock = Cell::new(0);
        assert_eq!(
            submit_codex(
                || Some(
                    if clock.get() % 200 == 0 {
                        SEND_TYPED
                    } else {
                        SEND_PASTED
                    }
                    .into()
                ),
                || panic!("changing composer must not get Enter"),
                || clock.get(),
                |ms| clock.set(clock.get() + ms),
            ),
            SendOutcome::Unconfirmed
        );
        assert_eq!(
            submit_codex(
                || Some(SEND_TYPED.into()),
                || Err("pane gone".into()),
                || clock.get(),
                |ms| clock.set(clock.get() + ms),
            ),
            SendOutcome::EnterFailed("pane gone".into())
        );
    }

    /// A modal takes typed keys as its ANSWER — text plus Enter confirms
    /// "Yes, proceed" — so a waiting Codex is never typed into, nor one that
    /// has exited.
    #[test]
    fn a_waiting_or_gone_codex_may_not_be_typed_into() {
        assert!(may_type(State::Idle).is_ok());
        assert!(may_type(State::Busy).is_ok(), "a busy composer queues typed input");
        let e = may_type(State::Waiting).unwrap_err();
        assert!(e.contains("waiting on you"), "{e}");
        assert!(may_type(State::None).is_err());
    }
}

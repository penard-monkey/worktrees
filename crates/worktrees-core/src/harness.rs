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
//! Only methods with a caller live on the trait. A model reaches argv through
//! the registry's `model_arg`, emitted here and only on a fresh launch; what a
//! picker lists is `choice::options_for`.

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

/// Why a harness will not start a session. `Hard`: it cannot run at all
/// (pi missing, node below pi's floor, no model to pass) — forcing it would
/// only start something broken. `Soft`: its model host did not answer; the
/// user may launch anyway (`AiLaunch::force`), and the exit code says so
/// (`diag::EXIT_LAUNCH_REFUSED`).
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Hard(String),
    Soft(String),
}

/// Whether a typed message was confirmed submitted.
#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Submitted,
    /// pi was mid-turn: the text left the composer and sits in pi's steering
    /// queue, delivered when the current step ends. Consumed, not yet read.
    Queued,
    Modal,
    Unconfirmed,
    EnterFailed(String),
}

impl SendOutcome {
    /// The harness took the text: it is in the conversation or in its queue,
    /// and a retry would deliver it twice.
    pub fn confirmed(&self) -> bool {
        matches!(self, Self::Submitted | Self::Queued)
    }

    /// `note`, in the words of the harness that was typed into.
    pub fn note_for(&self, provider: &str) -> String {
        if provider != crate::provider::PI.id {
            return self.note();
        }
        match self {
            Self::Submitted => "Typed into pi's prompt and confirmed: it is in pi's session as a user message. Ask it to report back, then wait until: message.".into(),
            Self::Queued => "pi was working, so it queued this as a steering message; it is delivered when the current step ends (it shows as \"Steering:\" above pi's prompt until then). pi keeps that queue in memory: if pi exits or is closed before delivering it, the text is lost and is NOT in messages — check the lane before assuming it arrived. Ask it to report back, then wait until: message.".into(),
            Self::Modal => "pi showed its project-trust prompt while sending. No Enter was pressed into it (Enter there trusts the repo). The text may be sitting in its input; the user has to answer the prompt. The message copy is left unread; check the composer and inbox before resending.".into(),
            other => other.note(),
        }
    }

    pub fn note(&self) -> String {
        match self {
            Self::Submitted => "Typed into its prompt and confirmed submitted; Codex queues it if a turn is running. Ask it to report back, then wait until: message.".into(),
            Self::Queued => "Typed while the agent was working and confirmed queued; it is delivered when the current step ends. Ask it to report back, then wait until: message.".into(),
            Self::Modal => "Codex opened an approval or a question while sending. No Enter was pressed into that prompt. The text may be sitting in its input; the user has to answer the prompt. Submission is not confirmed; the message copy is left unread if recorded. Submitting the composer later and reading messages can deliver the same instruction twice; check the composer and inbox before resending.".into(),
            Self::Unconfirmed => "Text was typed but submission could not be confirmed within the retry limit. It may still be in the composer. The message copy is left unread if recorded; read it with messages before retrying to avoid duplicates.".into(),
            Self::EnterFailed(e) => format!("Text was typed but Enter failed: {e}. Submission is not confirmed. The message copy is left unread if recorded; read it with messages before retrying to avoid duplicates."),
        }
    }
}

pub trait Adapter: Sync {
    /// This harness's registry row.
    fn provider(&self) -> &'static Provider;

    /// Whether this harness's CLI is here to launch at all — the reason, when
    /// it is not, is the one the app shows. Checked before any launch path
    /// that the UI drives; `prepare` still refuses on its own for the CLI/MCP.
    fn installed(&self) -> Result<(), String> {
        Ok(())
    }

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
    /// derives the id from the place. Empty for such a harness — its resume is
    /// carried by `AiLaunch::resume` and emitted by `prepare`/`launch_args`.
    fn resume_arg(&self, cwd: &str) -> String;

    /// How Settings shows this harness's resume, with no place to derive one
    /// from. Equal to `resume_arg` for a harness whose resume is a fixed word.
    fn resume_display(&self) -> String {
        self.resume_arg("")
    }

    /// Whether resume names an EXACT session, so a user-configured
    /// `ai_resume_arg` must not replace it (`config::resolve_ai_resume_arg_for`).
    fn exact_resume(&self) -> bool {
        false
    }

    /// Last word before a session is CREATED for `launch` in place `slug`:
    /// decide what argv needs the place's declared state for (pi's session id
    /// and generation), and refuse a launch that cannot or should not start.
    /// Runs before `ops::launch` closes the place's other agent, so a refusal
    /// costs nothing that was running. Never runs for an attach.
    fn prepare(&self, _p: &Project, _slug: &str, _wt: &str, _launch: &mut AiLaunch) -> Result<(), Refusal> {
        Ok(())
    }

    /// Whether this harness has never STARTED a session in place `slug` — so
    /// a fresh `open` there is the first launch, and a brief waiting in the
    /// place has never been read. Only a harness whose first launch can be
    /// refused after the brief was written (pi: a dead model host) needs to
    /// know; the rest answer false and `open` stays as it was.
    fn never_launched(&self, _p: &Project, _slug: &str) -> bool {
        false
    }

    /// The model the place's running session is on, from the harness's own
    /// record, when it keeps one we read.
    fn running_model(&self, _canonical: &str, _path: &str) -> Option<String> {
        None
    }

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
pub struct Pi;

pub const CLAUDE: &Claude = &Claude;
pub const CODEX: &Codex = &Codex;
pub const PI: &Pi = &Pi;

/// Every adapter, in registry order (`provider::PROVIDERS`). A new harness is
/// a row there and an entry here; `registry_and_adapters_line_up` pins it.
pub const ALL: &[&dyn Adapter] = &[CLAUDE, CODEX, PI];

/// `<model_arg> '<model>'` for a fresh launch that names one; nothing on a
/// resume (a resumed session keeps its own model) or without a model.
fn model_words(p: &Provider, launch: &AiLaunch) -> Vec<String> {
    match (p.model_arg, launch.model.as_deref().filter(|m| !m.trim().is_empty())) {
        (Some(arg), Some(m)) if !launch.resume => vec![arg.to_string(), shell_quote(m)],
        _ => Vec::new(),
    }
}

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
    ///
    /// A chosen model goes in `head`; a profile's own `--model` is on the END
    /// of `cmd` (`profile::claude_launch`), and claude takes the last one — so
    /// `Profile.model` keeps winning, as §2.3.6 decided.
    fn launch_args(&self, launch: &AiLaunch, session: &str) -> LaunchArgs {
        let tail = match self.provider().name_arg {
            Some(arg) if !session.is_empty() => vec![arg.to_string(), shell_quote(session)],
            _ => Vec::new(),
        };
        LaunchArgs { head: model_words(self.provider(), launch), tail }
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

    fn installed(&self) -> Result<(), String> {
        crate::profile::codex_bin()
            .map(|_| ())
            .ok_or_else(|| "Codex CLI is not installed. Install it, then sign in with `codex login`.".into())
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
        head.extend(model_words(self.provider(), launch));
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

    fn running_model(&self, _canonical: &str, path: &str) -> Option<String> {
        crate::codex::latest_rollout(path).and_then(|r| activity::codex_tail(&r).0)
    }

    fn send(&self, req: &SendRequest) -> Delivery {
        let codex = activity::codex_session_for(req.panes, req.canonical);
        // A modal (an approval, a plan-mode question) takes typed keys as its
        // ANSWER: text plus Enter confirms the highlighted option, which is
        // "Yes, proceed". So a Codex that is waiting on someone is never typed
        // into, and neither is one that has exited.
        if let Err(e) = may_type(self.provider().label, req.reading.state) {
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

/// The place's current pi session generation (0: never launched).
fn pi_generation(p: &Project, slug: &str) -> u32 {
    crate::store::read_lenient(&p.main_root).places.get(slug).and_then(|d| d.pi_session_gen).unwrap_or(0)
}

/// pi (pi-harness §1–§3, §7–§9). Runs as `node`, in its `~agent~pi` sidecar,
/// never as a place's canonical session.
///
/// Its launch is `pi --session-dir <dir> [--no-approve|--approve]
/// --session-id <id> [--model <backend>/<id>] "<opener>"`, and a resume is the
/// same minus `--model`. The exact session id is the point: resume, activity
/// and the model label all read THE SAME NAMED FILE, where Codex has only
/// "the newest rollout for this cwd".
impl Adapter for Pi {
    fn provider(&self) -> &'static Provider {
        provider::PI
    }

    fn installed(&self) -> Result<(), String> {
        crate::pimodels::pi_bin()
            .map(|_| ())
            .ok_or_else(|| "pi is not installed. Install it with: curl -fsSL https://pi.dev/install.sh | sh".into())
    }

    /// `--session-dir` is passed explicitly, set to pi's own default for the
    /// place: a repo's `.pi/settings.json` `sessionDir` is read BEFORE trust is
    /// decided, and the flag is the one thing that outranks it — so a repo
    /// cannot move the file activity is read from. Then the trust flag
    /// (`trust::pi_flag`).
    fn place_flags(&self, wt: &str) -> Vec<String> {
        let mut f = vec!["--session-dir".to_string(), shell_quote(&crate::pi::session_dir(wt).to_string_lossy())];
        f.extend(crate::trust::pi_flag(wt).map(str::to_string));
        f
    }

    fn launch_args(&self, launch: &AiLaunch, _session: &str) -> LaunchArgs {
        let mut head = launch.place_flags.clone();
        head.extend(model_words(self.provider(), launch));
        LaunchArgs { head, tail: Vec::new() }
    }

    fn resume_arg(&self, _cwd: &str) -> String {
        String::new()
    }

    fn resume_display(&self) -> String {
        "--session-id <per place>".into()
    }

    fn exact_resume(&self) -> bool {
        true
    }

    /// A place worktrees has launched pi in, with a session of its own
    /// (`pi::current_session`: the one it actually has — ours, or one started
    /// by hand since — and none older than the place). Without a launch of
    /// ours there is nothing to RESUME: the brief opener has not run, and the
    /// CLI's `new` and the app's "Switch agent → pi" must both give it one.
    fn session_present(&self, project: &Project, cwd: &str) -> bool {
        let slug = if std::path::Path::new(cwd) == std::path::Path::new(&project.main_root) {
            "(main)".to_string()
        } else {
            cwd.rsplit('/').next().unwrap_or(cwd).to_string()
        };
        pi_generation(project, &slug) > 0 && crate::pi::current_session(&crate::pi::session_dir(cwd), cwd).is_some()
    }

    /// Only with a file to reopen: `--session-id` for a session pi never wrote
    /// would CREATE one, and a resume carries no `--model` — so it would start
    /// on pi's own default, which is exactly the trap a fresh launch avoids.
    fn may_resume(&self, project: &Project, cwd: &str) -> bool {
        self.session_present(project, cwd)
    }

    /// The launch gate. In order: can pi run in the pane at all (preflight,
    /// hard); which model (fresh: `--model` > the place's last > the user's
    /// `[model] pi` > refuse — never pi's default); does its host answer
    /// (soft, `--force` passes). Only then is the generation bumped, so a
    /// refused launch leaves the next resume pointing at the old session.
    fn prepare(&self, p: &Project, slug: &str, wt: &str, launch: &mut AiLaunch) -> Result<(), Refusal> {
        if let Some(why) = crate::pimodels::preflight().problem {
            return Err(Refusal::Hard(why));
        }
        let canonical = p.session_name(slug);
        let (gen, model) = if launch.resume {
            (pi_generation(p, slug), self.running_model(&canonical, wt))
        } else {
            // The place's last model only from a store THIS machine wrote: a
            // committed `.worktrees.places.json` is repo input (ADR 0001).
            let model = launch
                .model
                .clone()
                .or_else(|| crate::store::declared_model(&p.main_root, slug, self.provider().id))
                .or_else(|| crate::config::default_model(self.provider().id));
            let Some(model) = model else {
                let ready = crate::pimodels::ready_names(&crate::pimodels::options());
                return Err(Refusal::Hard(format!(
                    "pi needs a model, and none was chosen for this place: pass --model <backend>/<id>, or set \
                     `[model] pi = \"…\"` in ~/.config/worktrees/config.toml. pi's own default is never \
                     used. Ready: {}",
                    if ready.is_empty() { "none".to_string() } else { ready.join(", ") }
                )));
            };
            crate::choice::validate_model(&model).map_err(Refusal::Hard)?;
            (0, Some(model))
        };
        if !launch.force {
            if let Some((_, why)) = model.as_deref().and_then(crate::pimodels::launch_reason) {
                return Err(Refusal::Soft(format!("{why} — pi was not started")));
            }
        }
        let gen = if launch.resume {
            gen
        } else {
            let model = model.clone().unwrap_or_default();
            let next = std::cell::Cell::new(0);
            crate::store::edit(&p.main_root, slug, |d| {
                next.set(d.pi_session_gen.unwrap_or(0) + 1);
                d.pi_session_gen = Some(next.get());
                d.agent = Some(crate::store::AgentDecl { harness: self.provider().id.into(), model: Some(model.clone()) });
            })
            .map_err(|e| Refusal::Hard(format!("could not record this place's pi session: {e}")))?;
            launch.model = model_arg_value(model);
            next.get()
        };
        // A resume reopens the conversation the place actually has, by its
        // header id — after a `/new` or a hand restart that is not the derived
        // one, and resuming the derived id would reopen a stale session (or,
        // with no file, create an empty one on pi's default model).
        let id = if launch.resume {
            crate::pi::current_session(&crate::pi::session_dir(wt), wt)
                .map(|s| s.id)
                .filter(|id| crate::pi::valid_session_id(id))
                .unwrap_or_else(|| crate::pi::session_id(&canonical, gen))
        } else {
            crate::pi::session_id(&canonical, gen)
        };
        launch.place_flags.push("--session-id".into());
        launch.place_flags.push(shell_quote(&id));
        Ok(())
    }

    fn running_model(&self, canonical: &str, path: &str) -> Option<String> {
        crate::pi::running_model(canonical, path)
    }

    /// Nothing of ours to resume here, so the brief opener has not run in THIS
    /// place: no generation recorded (`prepare` bumps it only once a launch is
    /// going ahead, so a refused create leaves 0), or none of the place's
    /// sessions — which is also the case for a slug re-used after `rm`, whose
    /// declared record keeps the dead lane's generation.
    fn never_launched(&self, p: &Project, slug: &str) -> bool {
        !self.session_present(p, &p.place_dir(slug))
    }

    fn activity(&self, scan: &Scan, canonical: &str, path: &str) -> Option<Activity> {
        scan.panes.and_then(|p| crate::pi::pi_activity(p, canonical, path))
    }

    fn agents(&self, _scan: &Scan, path: &str, reading: Option<&Activity>) -> Vec<serde_json::Value> {
        reading
            .map(|x| {
                let canonical = x.session.as_deref().and_then(|s| s.strip_suffix(self.provider().sidecar_suffix)).unwrap_or("");
                serde_json::json!({
                    "provider": self.provider().id,
                    "state": x.state,
                    "tmux": x.session,
                    "last_done": x.last_done,
                    "model": self.running_model(canonical, path),
                })
            })
            .into_iter()
            .collect()
    }

    /// Type into pi's composer and confirm it (pi-harness §4.5): idle →
    /// a new user entry in the place's current session file that starts with
    /// the attributed header; mid-turn → the composer clears and a `Steering:`
    /// line with the header sits above it (the file only gets it at delivery).
    /// The trust modal is refused first and by name: Enter there trusts the
    /// repo. Literal keys only — no paste (it folds), no control keys (Ctrl-C
    /// clears the composer, a second Ctrl-D exits pi).
    fn send(&self, req: &SendRequest) -> Delivery {
        if let Err(e) = may_type(self.provider().label, req.reading.state) {
            return Delivery::Refused(e);
        }
        let session = self.provider().sidecar_name(req.canonical);
        // pi runs as `node`, which `agent_pane` counts as an agent: in the
        // place's own `~agent~pi` session, that pane is pi.
        let Some(pane) = tmux::agent_pane(&session, req.path, req.exclude, self.provider().match_word) else {
            return Delivery::Refused(format!(
                "{session} has no pane running pi in {}; send only types into this project's own pi pane. \
                 Use report instead.",
                req.path
            ));
        };
        let dir = crate::pi::session_dir(req.path);
        let prefix = crate::pi::send_prefix(req.typed);
        let entries = || {
            crate::pi::current_session(&dir, req.path).map_or(0, |s| crate::pi::user_entries_starting(&s.path, &prefix))
        };
        let baseline = entries();
        if let Err(e) = tmux::send_literal(&pane, req.typed) {
            return Delivery::Refused(format!("could not type into {session}: {e}"));
        }
        let t0 = std::time::Instant::now();
        let outcome = crate::pi::submit_pi(
            &prefix,
            baseline,
            || tmux::capture(&pane),
            entries,
            || tmux::press_enter(&pane),
            || t0.elapsed().as_millis() as u64,
            |ms| std::thread::sleep(std::time::Duration::from_millis(ms)),
        );
        Delivery::Typed { session, outcome }
    }

}

fn model_arg_value(model: String) -> Option<String> {
    Some(model).filter(|m| !m.is_empty())
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
pub(crate) const SEND_POLL_MS: u64 = 100;
pub(crate) const SEND_STABLE_MS: u64 = 300;
pub(crate) const SEND_VERIFY_MS: u64 = 1_000;
pub(crate) const SEND_TIMEOUT_MS: u64 = 8_000;
pub(crate) const SEND_ENTER_TRIES: usize = 3;

/// Poll only the live composer, so transcript animation cannot prevent a
/// settle. Every Enter (including retries) is gated by a fresh, recognized,
/// nonempty composer and the modal guard. Failed captures never authorize a
/// keypress or count as confirmation. Clock and I/O seams keep timing tests
/// deterministic; production uses exactly this loop.
pub(crate) fn submit_codex(
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

/// Whether a harness in `state` may be typed into. Only one that is running
/// and not stopped on someone: `Waiting` means a modal is up and would take the
/// keys as its answer — Codex's approval ("Yes, proceed" is highlighted), pi's
/// project-trust prompt (**Trust** is highlighted, so one Enter lets the repo
/// run its own extensions inside pi) — and `None` means the harness has exited
/// and the pane is a shell.
pub(crate) fn may_type(label: &str, state: State) -> Result<(), String> {
    match state {
        State::Busy | State::Idle => Ok(()),
        State::Waiting => Err(format!(
            "{label} is waiting on you (an approval, a question or a trust prompt) — typing would \
             answer it. Answer it, or wait until: idle first."
        )),
        State::None => Err(format!("{label} is not running there (its pane is back at a shell). Use report.")),
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
        // pi resumes an EXACT session: no words, and no user override either.
        assert_eq!(PI.resume_arg("/w"), "");
        assert!(PI.exact_resume() && !CLAUDE.exact_resume() && !CODEX.exact_resume());
        assert_eq!(crate::ops::resume_command("pi", "/w"), "pi", "no trailing space into argv");
        // Settings' display default is never a session id derived from "".
        assert_eq!(CLAUDE.resume_display(), "-r");
        assert_eq!(PI.resume_display(), "--session-id <per place>");
        assert!(!PI.resume_display().contains("place-"), "not a derived id");
    }

    /// The launch a place's pi gets, and the resume: identical but for
    /// `--model`, which a resume never carries (pi reopens a session on its
    /// own model, and a `--model` would write a `model_change` into it).
    #[test]
    fn a_pi_launch_names_its_session_dir_trust_id_and_model_and_a_resume_drops_the_model() {
        let mut l = AiLaunch::plain("pi");
        l.place_flags = vec![
            "--session-dir".into(),
            "'/h/.pi/agent/sessions/--w--'".into(),
            "--no-approve".into(),
            "--session-id".into(),
            "'p-feat-abc123-g2'".into(),
        ];
        l.model = Some("lm-studio/qwen3.6-27b".into());
        l.opener = Some(crate::ops::BRIEF_OPENER.into());
        assert_eq!(
            l.launch_cmd("p-feat~agent~pi"),
            "pi --session-dir '/h/.pi/agent/sessions/--w--' --no-approve --session-id 'p-feat-abc123-g2' \
             --model 'lm-studio/qwen3.6-27b' 'Read .planning/brief.md and begin.'"
        );
        l.resume = true;
        l.opener = None;
        assert_eq!(
            l.launch_cmd("p-feat~agent~pi"),
            "pi --session-dir '/h/.pi/agent/sessions/--w--' --no-approve --session-id 'p-feat-abc123-g2'"
        );
        assert_eq!(PI.provider().name_arg, None, "pi's --name is a label, not an address");
    }

    /// `--model` reaches claude and codex through their registry `model_arg`,
    /// only when chosen and never on a resume — and with none chosen their
    /// launch is byte-identical to before.
    #[test]
    fn a_chosen_model_reaches_claude_and_codex_and_nothing_changes_without_one() {
        let mut c = AiLaunch::plain("claude");
        assert_eq!(c.launch_cmd("s"), "claude --name 's'");
        c.model = Some("opus".into());
        assert_eq!(c.launch_cmd("s"), "claude --model 'opus' --name 's'");
        let mut r = AiLaunch::plain("claude -r");
        r.model = Some("opus".into());
        r.resume = true;
        assert_eq!(r.launch_cmd("s"), "claude -r --name 's'");
        let mut x = AiLaunch::plain("codex");
        x.model = Some("gpt-5-codex".into());
        assert!(x.launch_cmd("s").ends_with(" -m 'gpt-5-codex'"), "{}", x.launch_cmd("s"));
        let mut xr = AiLaunch::plain("codex resume --last");
        xr.model = Some("gpt-5-codex".into());
        xr.resume = true;
        assert!(!xr.launch_cmd("s").contains(" -m "), "{}", xr.launch_cmd("s"));
        assert!(xr.launch_cmd("s").ends_with("resume --last"));
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
        assert_eq!(default_adapter().provider().id, "claude", "pi is never the default");
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
        assert!(may_type("Codex", State::Idle).is_ok());
        assert!(may_type("Codex", State::Busy).is_ok(), "a busy composer queues typed input");
        let e = may_type("Codex", State::Waiting).unwrap_err();
        assert!(e.contains("Codex is waiting on you"), "{e}");
        assert!(may_type("Codex", State::None).is_err());
    }
}

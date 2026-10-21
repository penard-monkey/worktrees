//! Settings, planning, and the mutating verbs.
//!
//! Three concerns that share a file because they share `Ctx` and little
//! else: `ui-state.json` (app-global, free-form, the frontend owns the
//! schema), the owned-planning surface, and every verb that reaches
//! `worktrees_core::ops`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use worktrees_core::ui::CaptureUi;
use worktrees_core::{argv, harness, mcpsetup, mention, ops, provider, store, sync, sysclock, tmux, Project, Ui};

use crate::argv::Argv;
use crate::log::log as applog;
use crate::Ctx;


pub fn ui_state_file(ctx: &Ctx) -> Result<PathBuf, String> {
    let dir = ctx.config_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("ui-state.json"))
}

#[derive(Serialize)]
pub struct SettingsInfo {
    pub dir: String,
    pub file: String,
}

/// Config dir + the ui-state.json path — for Settings → Data (reveal the file).
pub fn settings_info(ctx: &Ctx) -> Result<SettingsInfo, String> {
    let file = ui_state_file(ctx)?;
    let dir = ctx.config_dir.clone();
    Ok(SettingsInfo { dir: dir.to_string_lossy().into(), file: file.to_string_lossy().into() })
}

pub fn get_settings(ctx: &Ctx) -> Result<Option<serde_json::Value>, String> {
    let p = ui_state_file(ctx)?;
    match std::fs::read(&p) {
        Ok(b) => Ok(serde_json::from_slice(&b).ok()),
        Err(_) => Ok(None),
    }
}

pub fn set_settings(ctx: &Ctx, settings: serde_json::Value) -> Result<(), String> {
    let p = ui_state_file(ctx)?;
    let json = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(p, json).map_err(|e| e.to_string())
}

pub const LIFECYCLE_LABELS: [&str; 4] = ["closed", "saved", "archived", "abandoned"];

/// Set — or with an empty label CLEAR — a place's declared lifecycle.
///
/// Clearing exists because the nav's tier groups are draggable and two of them
/// (Active, Idle) are DERIVED by `store::reconcile` rather than declared. There
/// is no label that means "idle"; the closest true statement is "no declared
/// label, let the live state speak", and that is what dropping a row on Idle
/// writes. Faking it by stamping `last_opened_epoch` would lie to the Recent
/// lens about when the place was last used.
pub fn set_lifecycle(repo: String, slug: String, label: String) -> Result<(), String> {
    if !label.is_empty() && !LIFECYCLE_LABELS.contains(&label.as_str()) {
        return Err(format!("invalid lifecycle label: {label}"));
    }
    store::edit(&repo, &slug, |d| {
        d.lifecycle = if label.is_empty() { None } else { Some(label) }
    })
}

pub fn set_pin(repo: String, slug: String, on: bool) -> Result<(), String> {
    store::edit(&repo, &slug, |d| d.pinned = Some(on))
}

pub fn set_note(repo: String, slug: String, note: String) -> Result<(), String> {
    store::edit(&repo, &slug, |d| {
        d.note = if note.trim().is_empty() { None } else { Some(note) }
    })
}

/// Drop a reference to one place into ANOTHER place's agent session.
///
/// The nav drag's landing. Within one project, into a Claude pane, it is the
/// same `@worktrees:place://…` token the `@` menu completes, built in Rust from
/// `worktrees_core::mention` so there is exactly one implementation of it — the
/// client matches a mention against its cached resource list by exact string
/// equality, so a frontend copy that drifted by a character would reference the
/// wrong place silently rather than erroring.
///
/// Anything else — a place from ANOTHER project, or any place into a Codex or
/// pi pane — is a plain-text address (`place <project>:<slug>`,
/// `mention::address`), decided by `reach::plan_drop` (cross-project §6.3). A
/// token there would need the session's server to list that resource, which it
/// does not, and would resolve to nothing silently.
///
/// `repo` is the DRAGGED place's project and `into_repo` the receiving
/// session's: the session's server name is looked up from the receiving
/// place's profile stamp, which lives in ITS project's sidecar. (Reading it
/// from the dragged project's sidecar was right only while the two were always
/// the same repo.)
///
/// `into_session` rather than a slug because the receiving place may be on an
/// ADOPTED session whose name is not the canonical one; the frontend already
/// holds the real name from `ls`.
pub fn drop_reference(
    repo: String,
    slug: String,
    into_repo: String,
    into_slug: String,
    into_session: String,
    provider: Option<String>,
) -> Result<String, String> {
    let provider = provider.unwrap_or_else(|| worktrees_core::provider::CLAUDE.id.to_string());
    let prov = worktrees_core::provider::by_id(&provider).ok_or_else(|| format!("unknown agent: {provider}"))?;
    let project = Project::discover(std::path::Path::new(&repo)).map_err(|e| e.msg)?;
    let into = Project::discover(std::path::Path::new(&into_repo)).map_err(|e| e.msg)?;
    let places = project.place_index();
    if !places.iter().any(|p| p.slug == slug) {
        return Err(format!("no such place: {slug}"));
    }
    let plan = worktrees_core::reach::plan_drop(
        &worktrees_core::registry::read_lenient(),
        worktrees_core::reach::user_level(),
        &project.main_root,
        &slug,
        &into.main_root,
        prov.id,
    )?;
    let text = match plan {
        // `mcpsetup::claude_json_path()` rather than hand-building it from
        // $HOME: that module owns where claude's config lives.
        worktrees_core::reach::Drop::Token => drop_token(&places, &slug, &into.main_root, &into_slug, &mcpsetup::claude_json_path())?,
        worktrees_core::reach::Drop::Address(a) => {
            // Codex and pi: refuse while the agent is on an approval or a
            // question. A Claude permission prompt ignores a BRACKETED paste
            // (tmux::paste_to_ai); that is not verified for Codex's or pi's
            // composer, so a paste there could read as the answer.
            if prov.id != worktrees_core::provider::CLAUDE.id {
                let probes = worktrees_core::agent::live_probes();
                let panes = tmux::PaneList::fetch();
                let scan = worktrees_core::harness::Scan { probes: &probes, panes: panes.as_ref() };
                let path = into.place_dir(&into_slug);
                let waiting = worktrees_core::harness::place_activities(&into, &into_slug, &path, &scan)
                    .into_iter()
                    .any(|(a, r)| a.provider().id == prov.id && r.state == worktrees_core::activity::State::Waiting);
                if waiting {
                    return Err(format!(
                        "{}'s {} is waiting on an approval or a question — drop it again once it has an answer",
                        into_slug, prov.label
                    ));
                }
            }
            a
        }
    };
    // Spaces on BOTH sides. The client's extractor requires whitespace (or
    // start-of-input) before the `@`, and this cannot see the prompt to know
    // whether there already is any; the trailing one closes the `\b` and
    // dismisses the completion popup the `@` opens as it arrives. An address
    // wants the same separation from whatever is already typed.
    //
    // Addressed by the AI's pane, not by an index — see `tmux::ai_pane` for the
    // three ordinary ways pane 0 turns out not to be the agent.
    tmux::paste_to_ai(&into_session, prov.match_word, &format!(" {text} "))?;
    applog("info", &format!("drop_reference: {text} -> {into_session} ({})", prov.id));
    Ok(text)
}

/// The `@<server>:place://…` token for `slug` (among `places`, the DRAGGED
/// project's index), named by the server the RECEIVING session has — read
/// from `into_root`'s sidecar, where that session's profile stamp lives.
pub fn drop_token(places: &[worktrees_core::model::PlaceRef], slug: &str, into_root: &str, into_slug: &str, claude_json: &Path) -> Result<String, String> {
    let uri = mention::uri_for(places, slug).ok_or_else(|| format!("no such place: {slug}"))?;
    let server = mention::server_name_for(into_root, into_slug, claude_json)?;
    Ok(mention::mention(&server, &uri))
}

/// Settings → Agent guidance's cross-project section, and the offer's input:
/// the user's level and the registered projects (names, `private`). Read-only
/// and machine-level — the same answer whichever project is in focus.
#[derive(Serialize)]
pub struct CrossProjectStatus {
    pub level: &'static str,
    pub config_path: String,
    pub projects: Vec<CrossProjectEntry>,
}
#[derive(Serialize)]
pub struct CrossProjectEntry {
    pub root: String,
    pub name: String,
    pub private: bool,
}
pub fn cross_project_now() -> CrossProjectStatus {
    CrossProjectStatus {
        level: worktrees_core::reach::user_level().as_str(),
        config_path: worktrees_core::config::config_toml_path().to_string_lossy().into_owned(),
        projects: worktrees_core::registry::read_lenient()
            .projects
            .into_iter()
            .map(|e| CrossProjectEntry { root: e.root, name: e.name, private: e.private })
            .collect(),
    }
}

pub fn cross_project_status() -> Result<CrossProjectStatus, String> {
    Ok(cross_project_now())
}

/// The user's act: set `cross_project` in `~/.config/worktrees/config.toml`.
/// Sessions read it when they start.
pub fn set_cross_project(level: String) -> Result<CrossProjectStatus, String> {
    let l = worktrees_core::reach::Level::parse(&level).ok_or_else(|| format!("not a level: {level}"))?;
    worktrees_core::reach::set_user_level(l)?;
    applog("info", &format!("cross_project = {}", l.as_str()));
    Ok(cross_project_now())
}


/// One registered project's planning, as Settings → Planning lists it.
#[derive(Serialize)]
pub struct PlanningProject {
    pub root: String,
    pub name: String,
    /// `None` = inherits the default.
    pub level: Option<worktrees_core::planning::Level>,
    pub plan_path: Option<String>,
    pub plan_scope: Option<worktrees_core::planning::Scope>,
    pub effective: worktrees_core::planning::Level,
    /// What the write that produced THIS status did about `/.planning/` in
    /// info/exclude, for the projects it made (or found) full. Absent on a
    /// plain read: it is the result of an act, not a property of the repo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<worktrees_core::planning::Exclude>,
}

/// `planning_status`: machine-level, like `cross_project_status` — the global
/// default, every registered project's own choice, and the CLI the plan hook
/// runs beside this app's own version (owned-planning §7 item 1: two
/// binaries, so two releases, and only then can the hook and the tab differ).
#[derive(Serialize)]
pub struct PlanningStatus {
    pub default: worktrees_core::planning::GlobalDefault,
    pub version: u32,
    pub config_path: String,
    pub projects: Vec<PlanningProject>,
    pub cli_path: Option<String>,
    pub cli_version: Option<String>,
    pub app_version: String,
}

pub fn planning_now() -> PlanningStatus {
    planning_after(&[])
}

/// [`planning_now`], carrying the exclude outcomes of the write just made.
pub fn planning_after(excluded: &[(String, worktrees_core::planning::Exclude)]) -> PlanningStatus {
    use worktrees_core::planning;
    let g = planning::read_global();
    let reg = worktrees_core::registry::read_lenient();
    let projects = reg
        .projects
        .iter()
        .map(|e| PlanningProject {
            root: e.root.clone(),
            name: e.name.clone(),
            level: e.planning,
            plan_path: e.plan_path.clone(),
            plan_scope: e.plan_scope,
            effective: planning::effective_from(Some(e), &g).level,
            exclude: excluded.iter().find(|(r, _)| *r == e.root).map(|(_, x)| x.clone()),
        })
        .collect();
    let cli = worktrees_core::plancmd::probe_cli();
    PlanningStatus {
        default: g.default,
        version: planning::VERSION,
        config_path: planning::global_path().to_string_lossy().into_owned(),
        projects,
        cli_path: cli.as_ref().map(|(p, _)| p.to_string_lossy().into_owned()),
        cli_version: cli.map(|(_, v)| v),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

pub fn planning_status() -> Result<PlanningStatus, String> {
    Ok(planning_now())
}

/// The user's act: the global default (`unset` / `full` / `off`). Any choice
/// retires the `planning` offer.
pub fn set_planning_default(default: String) -> Result<PlanningStatus, String> {
    use worktrees_core::planning::GlobalDefault;
    let d = match default.as_str() {
        "unset" => GlobalDefault::Unset,
        "full" => GlobalDefault::Full,
        "off" => GlobalDefault::Off,
        other => return Err(format!("not a planning default: {other}")),
    };
    let (_, swept) = worktrees_core::planning::set_global(d)?;
    applog("info", &format!("planning default = {default}"));
    for (e, x) in &swept {
        applog("info", &format!("{}: {}", e.name, x.words()));
    }
    let swept: Vec<_> = swept.into_iter().map(|(e, x)| (e.root, x)).collect();
    Ok(planning_after(&swept))
}

/// The user's act: one project's level (`None` = inherit), with show-only's
/// path and scope. Core refuses full over a tracked `.planning/` and a path
/// that leaves the project; the error is the row's own text.
pub fn set_project_planning(
    root: String,
    level: Option<String>,
    plan_path: Option<String>,
    scope: Option<String>,
) -> Result<PlanningStatus, String> {
    use worktrees_core::planning::{Choice, Level, Scope};
    let level = match level.as_deref() {
        None | Some("inherit") => None,
        Some("off") => Some(Level::Off),
        Some("show") => Some(Level::Show),
        Some("full") => Some(Level::Full),
        Some(other) => return Err(format!("not a planning level: {other}")),
    };
    let plan_scope = match scope.as_deref() {
        None | Some("place") => None,
        Some("main") => Some(Scope::Main),
        Some(other) => return Err(format!("not a scope: {other}")),
    };
    let set = worktrees_core::planning::set_project(&root, &Choice { level, plan_path, plan_scope })?;
    let e = &set.entry;
    applog("info", &format!("planning {} = {}", e.name, e.planning.map(Level::as_str).unwrap_or("inherit")));
    let excluded: Vec<_> = set.exclude.iter().map(|x| (e.root.clone(), x.clone())).collect();
    if let Some(x) = &set.exclude {
        applog("info", &format!("{}: {}", e.name, x.words()));
    }
    Ok(planning_after(&excluded))
}

/// After a clone: does the new repo suggest owned planning (`[plan]` in its
/// `.worktrees.toml`)? Clone's dialog cannot know before the clone exists, so
/// the suggestion becomes a post-clone line (owned-planning §2.3).
pub fn planning_suggested(root: String) -> Result<bool, String> {
    Ok(worktrees_core::planning::repo_suggests(Path::new(&root)))
}

/// Settings → Behavior → Plan limits: the user's `[quota]` launch-gate policy
/// (`worktrees_core::quota`), and what was wrong with a hand-edited one.
/// Machine-level, and read by the gate at launch time — a change reaches the
/// next launch everywhere, running MCP servers included.
#[derive(Serialize)]
pub struct QuotaSettings {
    pub gate: bool,
    pub weekly_warn_pct: Option<u8>,
    pub problems: Vec<String>,
    pub config_path: String,
}
pub fn quota_now() -> QuotaSettings {
    let u = worktrees_core::quota::user_policy();
    QuotaSettings {
        gate: u.policy.gate,
        weekly_warn_pct: u.policy.weekly_warn_pct,
        problems: u.problems,
        config_path: worktrees_core::config::config_toml_path().to_string_lossy().into_owned(),
    }
}

pub fn quota_settings() -> Result<QuotaSettings, String> {
    Ok(quota_now())
}

/// The user's act: set `[quota]` in `~/.config/worktrees/config.toml`.
pub fn set_quota_settings(gate: bool, weekly_warn_pct: Option<u8>) -> Result<QuotaSettings, String> {
    worktrees_core::quota::set_user_policy(worktrees_core::quota::Policy { gate, weekly_warn_pct })
        .inspect_err(|e| applog("error", &format!("set_quota_settings: {e}")))?;
    applog("info", &format!("quota: gate = {gate}, weekly_warn_pct = {weekly_warn_pct:?}"));
    Ok(quota_now())
}

/// Mark a registered project private (out of reach in both directions) or not.
pub fn set_project_private(root: String, private: bool) -> Result<CrossProjectStatus, String> {
    worktrees_core::registry::set_private(&root, private)?;
    Ok(cross_project_now())
}

/// Every "Copy …" in the app (`clipboard.ts`). Native because the web API
/// cannot be relied on here: `navigator.clipboard.writeText` needs the click's
/// transient activation, and macOS 27's WebKit throws that away on EVERY
/// `evaluateJavaScript:` — which is how Tauri delivers each `emit` and each
/// small `Channel` message, terminal output included. With a session
/// streaming, one lands between mousedown and click almost every time and the
/// write rejects with a raw `NotAllowedError`. Not ours to fix upstream
/// (WebKit 321448@main), and not a race a retry wins while output keeps coming.
pub fn copy_text(text: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Blocking, like every other body here — the Tauri wrapper is what
        // keeps it off the main thread, and the pipe below is short-lived
        // anyway (it is `pbcopy` reading one buffer).
        let r = (|| -> Result<(), String> {
            use std::io::Write;
            let mut child = pbcopy_command().spawn().map_err(|e| format!("pbcopy: {e}"))?;
            // Take stdin so it is dropped (EOF) before the wait.
            let mut stdin = child.stdin.take().ok_or("pbcopy: no stdin")?;
            stdin.write_all(text.as_bytes()).map_err(|e| format!("pbcopy: {e}"))?;
            drop(stdin);
            let out = child.wait_with_output().map_err(|e| format!("pbcopy: {e}"))?;
            if out.status.success() {
                Ok(())
            } else {
                Err(format!("pbcopy: {}", String::from_utf8_lossy(&out.stderr).trim()))
            }
        })();
        // Logged here as well as returned: the frontend then tries the web
        // clipboard, and on macOS THAT refusal is the one the banner would show.
        if let Err(e) = &r {
            applog("warn", &format!("copy_text: {e}"));
        }
        r
    }
    // No native path elsewhere; the frontend falls back to the web API.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        Err("no native clipboard on this platform".into())
    }
}

/// `pbcopy` reads its stdin in the LOCALE's encoding: with none, a path like
/// `año/✎` lands on the clipboard as `a√±o/‚úé` (measured). `fixup_gui_locale`
/// already sets `LANG` at startup when no UTF-8 locale is present, so this is
/// belt-and-braces — and it covers the case that check lets through: a UTF-8
/// `LANG` beside a non-UTF-8 `LC_ALL` (say `C`, from `launchctl setenv`), which
/// overrides both `LANG` and `LC_CTYPE`, hence the `env_remove`. Absolute path
/// for the same launchd reason as `fixup_gui_path`.
#[cfg(target_os = "macos")]
pub fn pbcopy_command() -> std::process::Command {
    use std::process::Stdio;
    let mut c = worktrees_core::childenv::command("/usr/bin/pbcopy");
    c.env_remove("LC_ALL")
        .env("LC_CTYPE", "UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    c
}

/// Rename a place's LABEL. Empty clears it, which is how the UI goes back to
/// showing the slug — see `Declared::title` for why this is a label and not a
/// rename of the worktree.
pub fn set_title(repo: String, slug: String, title: String) -> Result<(), String> {
    store::edit(&repo, &slug, |d| {
        let t = title.trim();
        d.title = if t.is_empty() { None } else { Some(t.to_string()) }
    })
}

/// Stamp last-opened (drives the `idle` window). Called when a place is opened.
pub fn touch_place(repo: String, slug: String) -> Result<(), String> {
    store::edit(&repo, &slug, |d| d.last_opened_epoch = Some(sysclock::now_epoch()))
}

/// Stamp "the user has SEEN this place's afterglow", forward-only.
///
/// The epoch comes from the FRONTEND, not from `now_epoch()`: the frontend is
/// what knows the moment the dwell expired, and it has already written the same
/// number into its optimistic map. Two clocks for one fact is how an ack and the
/// dot it is acking end up disagreeing by a second.
///
/// Reads before it writes, like `stamp_worked`: selecting a place is a gesture
/// the user makes all day, and an unconditional `store::edit` would rewrite
/// `.worktrees.places.json` on every one of them. A stale-or-equal epoch is a
/// no-op, NOT an error — the caller fire-and-forgets it.
///
/// ⚠ Touches `last_seen_epoch` and nothing else. `last_worked_epoch` is the
/// fact being acknowledged and `last_opened_epoch` means "I entered here";
/// moving either from this path would erase the signal or lie to the Recent lens.
pub fn mark_seen(repo: String, slug: String, epoch: i64) -> Result<(), String> {
    if store::read_lenient(&repo)
        .places
        .get(&slug)
        .and_then(|d| d.last_seen_epoch)
        .unwrap_or(0)
        >= epoch
    {
        return Ok(()); // already seen at or after this moment
    }
    store::edit(&repo, &slug, |d| {
        if d.last_seen_epoch.unwrap_or(0) < epoch {
            d.last_seen_epoch = Some(epoch);
        }
    })
}


/// Outcome of a core op: exit code + the op's own messages (the loud guards),
/// surfaced to the UI verbatim.
#[derive(Serialize)]
pub struct CmdResult {
    pub ok: bool,
    pub code: i32,
    pub output: String,
    /// For `new_place` only: the ACTUAL slug core landed the place on (branch
    /// slugified, origin/ stripped, holder-reuse applied). The frontend selects
    /// this instead of re-deriving and guessing wrong. `None` for every other op.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// For `close_place` only: core stopped short (`diag::EXIT_NEEDS_CONFIRM`)
    /// because killing this place's session needs the user's word — it was
    /// ADOPTED, so `kill-session` takes a whole session the user did not start
    /// through this tool. The value is the session that would die, passed
    /// STRUCTURALLY so the frontend names it in its own confirm without parsing
    /// core's prose. `None` for every other op and outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_confirm: Option<String>,
    /// Messages the op emitted at warn/error severity. `output` flattens every
    /// severity into one blob and the UI only shows it when the op FAILED, so a
    /// warning on a successful op used to vanish — which is the failure mode
    /// `doctor` exists to end (proposal §7).
    pub warnings: Vec<String>,
}

pub fn run_op<F: FnOnce(&Project, &mut CaptureUi) -> i32>(op: &str, repo: &str, f: F) -> Result<CmdResult, String> {
    let project = Project::discover(Path::new(repo)).map_err(|e| {
        applog("error", &format!("{op} repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    // Guard A for the app. The CLI refuses every mutating command inside a tree
    // that arrived on a sync hub (main.rs's one dispatch choke point); the app
    // calls the same `ops::cmd_*` IN-PROCESS and so never passes it. This is the
    // app's equivalent choke point — every op-shaped command goes through here.
    //
    // Reported as a FAILED CmdResult rather than an Err: the frontend renders op
    // failures from `output`, and an Err would surface as a thrown invoke with no
    // banner of its own.
    if let Some(msg) = sync::hub_copy_refusal(Path::new(&project.main_root)) {
        return Ok(refused(op, repo, msg));
    }
    let mut ui = CaptureUi::default();
    let code = f(&project, &mut ui);
    let warnings = ui.warnings();
    if code == 0 {
        applog("info", &format!("{op} ok repo={repo}"));
        // A SUCCESSFUL op can still warn (a declared file absent from main, a
        // drifted copy). Log it separately — otherwise the only record of it is
        // an `output` string the UI never renders on success.
        if !warnings.is_empty() {
            applog("warn", &format!("{op} warnings repo={repo}: {}", warnings.join(" | ")));
        }
    } else {
        applog("warn", &format!("{op} rc={code} repo={repo}: {}", ui.lines.join(" | ")));
    }
    Ok(CmdResult { ok: code == 0, code, output: ui.lines.join("\n"), slug: None, needs_confirm: None, warnings })
}

/// An op refused before it ran, shaped like one that failed: the frontend
/// renders `output`, and a failed create puts the new-worktree form back with
/// what was typed in it — a thrown error would lose both.
pub fn refused(op: &str, repo: &str, msg: String) -> CmdResult {
    applog("warn", &format!("{op} refused repo={repo}: {msg}"));
    CmdResult { ok: false, code: 1, output: msg, slug: None, needs_confirm: None, warnings: Vec::new() }
}

/// Create a worktree (`new`). `--no-attach`: the session is created (pane 0 AI,
/// pane 1 shell) but the app embeds it via its own PTY rather than attaching.
pub fn new_place(
    repo: String,
    branch: String,
    base: Option<String>,
    name: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    force: Option<bool>,
) -> Result<CmdResult, String> {
    // R3's second layer, before anything else runs (`argv`).
    let checked = (|| -> Result<_, String> {
        let branch = argv::branch_arg(&branch)?;
        let base = base.filter(|s| !s.trim().is_empty()).map(|b| argv::safe_arg(&b, "base")).transpose()?;
        let name = name.filter(|s| !s.trim().is_empty()).map(|n| argv::safe_arg(&n, "name")).transpose()?;
        Ok((branch, base, name))
    })();
    let (branch, base, name) = match checked {
        Ok(v) => v,
        Err(msg) => return Ok(refused("new", &repo, msg)),
    };
    let provider = provider.unwrap_or_else(|| harness::default_adapter().provider().id.into());
    known_harness(&provider)?.installed()?;
    let model = model.filter(|m| !m.trim().is_empty());
    if let Some(m) = &model {
        worktrees_core::choice::validate_model(m)?;
    }
    // Resolve the FINAL slug BEFORE the op — wt_for_branch reflects the holder
    // that reuse targets, and the derived-slug dir doesn't exist yet either way.
    // After the op the holder may have moved (branch now in the new dir), so the
    // pre-op resolution is what the frontend should select.
    let resolved_slug = Project::discover(Path::new(&repo))
        .ok()
        .map(|p| p.resolve_new_slug(&branch, name.as_deref()));
    let op = format!("new {branch}");
    // Single-pane like `open_place` below: Claude gets the full width and the
    // scratch shell lives in the dock's Terminal tab (which is also where deps
    // get installed — no spare pane means no auto-install). Single pane is the
    // CLI's default too now; `--no-spare` stays explicit so the app's layout
    // never rides on that default again.
    let args = Argv::new()
        .value_opt("--name", name.as_deref())
        .value("--ai", &provider)
        .value_opt("--model", model.as_deref())
        .flag_if(force.unwrap_or(false), "--force")
        .flag("--no-attach")
        .flag("--no-spare")
        .pos(branch)
        .pos_opt(base);
    let mut r = run_op(&op, &repo, |p, ui| args.call(p, ui, ops::cmd_new))?;
    if r.ok {
        r.slug = resolved_slug;
    }
    Ok(r)
}

/// Move a place to another branch (`switch <slug> <branch> [base]`). `-y` skips
/// the inside-a-worktree ambiguity prompt (the UI targets a place explicitly).
pub fn switch_place(
    repo: String,
    slug: String,
    branch: String,
    base: Option<String>,
) -> Result<CmdResult, String> {
    let checked = (|| -> Result<_, String> {
        let slug = argv::safe_arg(&slug, "slug")?;
        let branch = argv::branch_arg(&branch)?;
        let base = base.filter(|s| !s.trim().is_empty()).map(|b| argv::safe_arg(&b, "base")).transpose()?;
        Ok((slug, branch, base))
    })();
    let (slug, branch, base) = match checked {
        Ok(v) => v,
        Err(msg) => return Ok(refused("switch", &repo, msg)),
    };
    let op = format!("switch {slug}");
    let args = Argv::new().flag("-y").pos(slug).pos(branch).pos_opt(base);
    run_op(&op, &repo, |p, ui| args.call(p, ui, ops::cmd_switch))
}

/// Branches the status-bar switcher offers, plus the base a NEW branch would be
/// cut from. `switch` is DWIM (local → switch, remote-only → track, otherwise
/// create), so the control is a combobox and not a `<select>`: picking from the
/// list and typing a name that doesn't exist yet are both first-class.
#[derive(Serialize)]
pub struct BranchList {
    pub branches: Vec<String>,
    pub current: String,
    pub default_base: String,
}

pub fn list_branches(repo: String, slug: String) -> Result<BranchList, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| e.msg)?;
    let current = p.wt_branch(&p.place_dir(&slug));
    Ok(BranchList { branches: p.branch_names(), current, default_base: p.default_base() })
}

/// The adapter for a provider id the frontend sent, or the refusal naming the
/// ones there are.
pub fn known_harness(id: &str) -> Result<&'static dyn harness::Adapter, String> {
    harness::by_id(id).ok_or_else(|| format!("provider must be {}", provider::choices()))
}

/// Enter a place: ensure its tmux session exists (create if down) WITHOUT attaching
/// — the app embeds it via its own PTY. Worktrees go through `open` (reuses the
/// existing launch path); the main checkout is launched directly since `open` only
/// targets worktrees under `.worktrees/`.
pub fn open_place(
    repo: String,
    slug: String,
    fresh: Option<bool>,
    provider: Option<String>,
    model: Option<String>,
    force: Option<bool>,
) -> Result<CmdResult, String> {
    let slug = match argv::safe_arg(&slug, "slug") {
        Ok(s) => s,
        Err(msg) => return Ok(refused("open", &repo, msg)),
    };
    let provider = provider.unwrap_or_else(|| harness::default_adapter().provider().id.into());
    let adapter = known_harness(&provider)?;
    adapter.installed()?;
    let model = model.filter(|m| !m.trim().is_empty());
    if let Some(m) = &model {
        worktrees_core::choice::validate_model(m)?;
    }
    let force = force.unwrap_or(false);
    run_op(&format!("open {slug} fresh={}", fresh.unwrap_or(false)), &repo, move |p, ui| {
        // Auto-resume only a conversation belonging to this place, as the
        // selected harness defines it. Codex resolves an exact session UUID
        // before launch; its repository-wide --last selector is never used.
        // `fresh` (right-click "Open fresh") skips resume.
        let wt = p.place_dir(&slug);
        let resume = !fresh.unwrap_or(false) && adapter.session_present(p, &wt);
        if slug == "(main)" {
            if !worktrees_core::tmux::have_tmux() {
                ui.error("tmux not found");
                return 1;
            }
            let session = ops::agent_session_name(&p.session_name("(main)"), &provider);
            let mut ai_cmd = provider.clone();
            // (main) does not go through `cmd_open`, so it has to resolve its
            // own resume target — without this it would set `resume` with no
            // id and `Codex::prepare` would correctly degrade to a fresh
            // launch, silently dropping (main)'s own conversation.
            let (allowed, resume_id) = if resume && !ai_cmd.is_empty() {
                ops::resume_plan(p, &ai_cmd, &p.main_root)
            } else {
                (false, None)
            };
            let resume = resume && allowed;
            if resume {
                ai_cmd = ops::resume_command(&ai_cmd, &p.main_root);
            }
            // Propagate launch's rc: a failed new-session must reach the UI
            // banner / app.log, not silently report success. Single-pane
            // (spare_shell=false): Claude gets full width; the scratch shell
            // lives in the dock's Terminal tab.
            let mut ai = ops::ai_launch_for(p, ui, &p.main_root, &ai_cmd);
            (ai.model, ai.resume, ai.force) = (model.clone(), resume, force);
            ai.resume_id = resume_id;
            ops::launch(p, ui, &p.main_root, &session, "", &ai, false, false)
        } else {
            let args = Argv::new()
                .flag("--no-attach")
                .flag("--no-spare")
                .value("--ai", &provider)
                .flag_if(resume, "-r")
                .value_opt("--model", model.as_deref())
                .flag_if(force, "--force")
                .pos(slug);
            args.call(p, ui, ops::cmd_open)
        }
    })
}

// `close_place` stays in the Tauri shim for now: it sweeps the dock's owned
// shells, and that registry has not moved to `Ctx` yet. It follows the PTY
// section rather than being split in two and rewritten twice.



pub const REPO_SLUG: &str = "penard-monkey/worktrees";

#[derive(Serialize)]
pub struct UpdateInfo {
    pub app_version: String,
    pub cli_version: Option<String>,
    pub cli_path: Option<String>,
    pub latest: Option<String>, // release tag, e.g. "v0.2.0"
}

/// Latest release tag via the releases/latest REDIRECT (install.sh's trick —
/// no API, no rate limit, no auth). None when offline or no releases exist.
pub fn latest_release_tag() -> Option<String> {
    let out = worktrees_core::childenv::command("curl")
        .args([
            "-fsSLI", "--max-time", "6", "-o", "/dev/null", "-w", "%{url_effective}",
            &format!("https://github.com/{REPO_SLUG}/releases/latest"),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout);
    let url = url.trim();
    if !url.contains("/tag/") {
        return None;
    }
    url.rsplit("/tag/").next().map(String::from)
}

/// The hard-deadline subprocess runner, now core's (`proc.rs`).
///
/// It moved when the (since-removed) automation runner needed the identical
/// thing: a headless `claude -p` with no pane to Ctrl-C and nobody watching.
/// Imported rather than re-exported so every call site below reads exactly as
/// it did.
use worktrees_core::proc::run_deadline;


/// Fetch a single project's origin. Runs `git fetch --prune origin` in the MAIN
/// ROOT only — every worktree shares the repo's ref store, so one fetch keeps the
/// whole project's ahead/behind fresh. Hardening: `GIT_TERMINAL_PROMPT=0` +
/// `GIT_SSH_COMMAND=ssh -oBatchMode=yes` so a credential/host-key prompt can
/// never hang a thread forever (the GUI has no tty under launchd), and a ~60s
/// deadline (run_deadline) so a wedged network can't pin the thread either.
/// Result → app.log only (info ok / warn + git's stderr) — background work never
/// pops a user-facing banner. `Err` carries git's stderr for the manual command.
pub fn fetch_origin_root(main_root: &str) -> Result<(), String> {
    let mut cmd = worktrees_core::childenv::command("git");
    cmd.arg("-C")
        .arg(main_root)
        .args(["fetch", "--prune", "origin"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
    match run_deadline(cmd, 60) {
        Ok(out) if out.status.success() => {
            applog("info", &format!("fetch ok repo={main_root}"));
            Ok(())
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            applog("warn", &format!("fetch rc={} repo={main_root}: {stderr}", out.status.code().unwrap_or(-1)));
            Err(if stderr.is_empty() { format!("git fetch exited {}", out.status.code().unwrap_or(-1)) } else { stderr })
        }
        Err(e) => {
            applog("warn", &format!("fetch failed repo={main_root}: {e}"));
            Err(e.to_string())
        }
    }
}

/// Auto-fetch interval in SECONDS (0 = off). The settings blob is backend-opaque
/// (get_settings/set_settings just shuttle JSON), so the watcher thread can't read
/// it — the frontend pushes the value here via `set_fetch_interval` after every
/// settings hydration and change.
pub static FETCH_INTERVAL_SECS: AtomicU64 = AtomicU64::new(0);

/// Frontend → backend sync of the auto-fetch interval. `mins` is 0 (off) or one
/// of the offered cadences (5 / 15 / 60); stored in whole seconds for the watcher.
pub fn set_fetch_interval(mins: u64) -> Result<(), String> {
    FETCH_INTERVAL_SECS.store(mins.saturating_mul(60), Ordering::Relaxed);
    Ok(())
}

/// Manual "Fetch origin" verb (project right-click). Discovers the repo from any
/// path under it, then fetches its main root via the shared fn. The error string
/// is git's own stderr so the frontend's fail() shows the real reason.
pub fn fetch_origin(root: String) -> Result<(), String> {
    let project = Project::discover(Path::new(&root)).map_err(|e| e.msg)?;
    fetch_origin_root(&project.main_root)
}


/// How many rows above the cursor to capture. The prompt box is the bottom of
/// the pane and the parser needs the box's TOP edge in frame (that edge is what
/// tells an input box from a `❯` selector menu), so this has to clear the
/// longest wrapped draft anyone types. `-S -20` starts 20 rows INTO the
/// scrollback and runs to the bottom of the visible pane, so what comes back is
/// the whole screen plus a little history — a few KB per pane. Too few costs a
/// draft, never a wrong one.
pub const DRAFT_CAPTURE_LINES: &str = "-20";

/// Payload row for `sessions:drafts` / `list_drafts`.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Draft {
    /// The session's cwd — the same key as `sessions:busy` and a place's `path`.
    pub path: String,
    pub text: String,
    /// The session is mid-turn, so claude will SEND this when the turn ends
    /// rather than sit on it. A different sentence in the UI, not a different
    /// signal.
    pub queued: bool,
}

/// Payload for `sessions:drafts` — the CURRENT set, reconciled every sample.
#[derive(Serialize, Clone)]
pub struct Drafts {
    pub drafts: Vec<Draft>,
}

/// Capture every live claude pane in one `tmux` call and parse the unsent text
/// out of each. `sessions` is `tmux::session_fingerprint()`'s output (sorted
/// names, one per line) — passed in rather than re-fetched so the pass costs a
/// single spawn.
///
/// Panes whose session is not in that list are dropped BEFORE the call: a
/// `capture-pane` on a dead pane makes tmux print an error and abandon the rest
/// of the chain, which would silently blank every pane after it.
pub fn scan_drafts(sessions: &str) -> Result<Vec<Draft>, String> {
    let live: Vec<&str> = sessions.lines().filter(|l| !l.is_empty()).collect();
    let probes = worktrees_core::agent::live_probes();
    // (cwd, pane id, queued) for the panes worth capturing.
    let mut targets: Vec<(String, String, bool)> = Vec::new();
    for p in &probes {
        let state = worktrees_core::agent::effective_state(p);
        // `shell` is not a claude prompt at all; `delegated` is a session whose
        // turn is somewhere else, and whose box is not the user's to answer.
        if state == "shell" || state == "delegated" {
            continue;
        }
        let Some(tmux) = p.tmux.as_deref() else { continue }; // started outside tmux → no screen
        let (Some(pane), Some(sess)) = (
            worktrees_core::agent::pane_id(tmux),
            worktrees_core::agent::session_name(tmux),
        ) else {
            continue;
        };
        if !live.contains(&sess) {
            continue;
        }
        targets.push((p.cwd.clone(), pane.to_string(), state == "busy"));
    }
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    // `display-message -p` expands `#{…}` AND eats `%` in its format, so the
    // marker is built from the target's index and nothing else.
    let marks: Vec<String> = (0..targets.len()).map(|i| format!("@@ {i} @@")).collect();
    let mut args: Vec<&str> = Vec::with_capacity(targets.len() * 10);
    for (i, (_, pane, _)) in targets.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        // Marker FIRST: a chain that dies part-way then leaves the surviving
        // segments still correctly attributed to their panes.
        args.extend(["display-message", "-p", &marks[i], ";"]);
        // `-e` keeps the styling: claude paints its own suggested follow-up
        // into the box in DIM, and the text alone cannot tell it from typing
        // (`agent::draft_from_screen` reads the attribute and strips the rest).
        args.extend(["capture-pane", "-e", "-p", "-t", pane, "-S", DRAFT_CAPTURE_LINES]);
    }
    let out = worktrees_core::tmux::tmux(&args).map_err(|e| format!("tmux: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "tmux capture-pane failed".into() } else { err });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut drafts: Vec<Draft> = Vec::new();
    for (i, (path, _, queued)) in targets.iter().enumerate() {
        let Some(rest) = text.split(&format!("{}\n", marks[i])).nth(1) else { continue };
        // The next marker ends this pane's screen (the last one runs to EOF).
        let screen = match marks.get(i + 1).and_then(|m| rest.find(m.as_str())) {
            Some(end) => &rest[..end],
            None => rest,
        };
        if let Some(t) = worktrees_core::agent::draft_from_screen(screen) {
            drafts.push(Draft { path: path.clone(), text: t, queued: *queued });
        }
    }
    // Two claude sessions can share a cwd (a fork, or a second one started by
    // hand). The row has one ✎, so keep one draft per path — the busy one
    // first, since "queued" is the more surprising of the two sentences.
    drafts.sort_by(|a, b| a.path.cmp(&b.path).then(b.queued.cmp(&a.queued)));
    drafts.dedup_by(|a, b| a.path == b.path);
    Ok(drafts)
}

/// Mount-time pull for `sessions:drafts`. The event is change-gated and only
/// fires every 15s, so a frontend that mounts (or reloads) between two changes
/// would otherwise show nothing until the user typed something.
pub fn list_drafts() -> Result<Vec<Draft>, String> {
    let fp = worktrees_core::tmux::session_fingerprint();
    Ok(scan_drafts(&fp).unwrap_or_default())
}

/// Draft TEXT is something a person typed, so it never reaches the log — same
/// rule the usage metrics live under. `applog` records counts only.
///
/// `WORKTREES_TRACE_DRAFTS=<path substring>` is the one exception, for proving
/// the pipeline end to end against a scratch place. It is a FILTER rather than
/// an on/off switch on purpose: `app.log` is keyed to `APP_IDENT`, which the
/// dev/sandbox build shares with the installed app, and a session that scans
/// every config root would otherwise write the user's real drafts into the
/// file they paste into bug reports.
pub fn draft_trace_filter() -> Option<String> {
    std::env::var("WORKTREES_TRACE_DRAFTS").ok().filter(|v| !v.is_empty())
}

/// `WORKTREES_TRACE_SPAWNS=1` logs what the draft pass costs in subprocesses —
/// the app cannot be measured with the CLI's PATH shim (`fixup_gui_path` puts
/// the real tmux back in front), so the counter has to live here.
pub fn trace_spawns() -> bool {
    std::env::var("WORKTREES_TRACE_SPAWNS").is_ok_and(|v| v == "1")
}


/// Close a place. It moved here last of the mutating verbs, because it sweeps
/// the dock's owned shells and that registry only reached `Ctx` with the PTY
/// section — splitting the handler earlier would have meant writing the split
/// and then undoing it.
pub fn close_place(
    repo: String,
    slug: String,
    yes: bool,
    session: Option<String>,
    provider: Option<String>,
    shells: &crate::term::Shells,
) -> Result<CmdResult, String> {
    let slug = match argv::safe_arg(&slug, "slug") {
        Ok(s) => s,
        Err(msg) => return Ok(refused("close", &repo, msg)),
    };
    if let Some(p) = provider.as_deref() {
        known_harness(p)?;
    }
    // core cmd_close sweeps this place's tmux-era sidecars; the owned dock
    // shells are app state, so they're swept here — same rule as before, the
    // dock's scratch shells die with the place.
    let expect = session.filter(|s| !s.is_empty());
    // The consented session goes in the log line too: when a kill is questioned
    // later, the record must say WHICH session the user was asked about.
    let op = match &expect {
        Some(s) => format!("close {slug} yes={yes} session={s}"),
        None => format!("close {slug} yes={yes}"),
    };
    // `--session` is glued on and only ever COMPARED with the live name, so it
    // needs no validation of its own: a wrong one is "changed since confirmation".
    let args = Argv::new()
        .value_opt("--ai", provider.as_deref())
        .flag_if(yes, "-y")
        .value_opt("--session", expect.as_deref())
        .pos(slug.clone());
    let mut r = run_op(&op, &repo, |p, ui| args.call(p, ui, ops::cmd_close))?;
    if r.code == worktrees_core::diag::EXIT_NEEDS_CONFIRM {
        // Not a failure — a question. Resolve the session core balked at the
        // same way core did, so the UI can name it verbatim. This is a SECOND
        // resolution and can differ from core's (the session may have exited in
        // between) — harmless now that the answer comes back name-bound: the
        // name shown is the name core is held to, and `None` means there is
        // nothing left to ask about (the frontend treats it as "already gone").
        r.needs_confirm = Project::discover(Path::new(&repo)).ok().and_then(|p| ops::place_session(&p, &slug));
    } else if r.ok && provider.is_none() {
        crate::term::kill_place_shells(shells, &repo, &slug);
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The receiving session's server is looked up in the RECEIVING
    /// project's sidecar. Both repos have a place called `lane`; the dragged
    /// one is stamped with a profile that no longer exists, the receiving one
    /// is unprofiled, and claude's user config has no worktrees server — so
    /// each sidecar gives a DIFFERENT refusal, and the message says which one
    /// was read. Reading the dragged repo's (the code before the split) names
    /// the ghost profile for a drop into `b`.
    #[test]
    fn a_drop_token_names_the_receiving_sessions_server() {
        let base = std::env::temp_dir().join(format!("wt-droptoken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (a, b) = (base.join("a"), base.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let ghost = format!("ghost-{}", std::process::id());
        worktrees_core::store::edit(&a.to_string_lossy(), "lane", |d| d.profile_id = Some(ghost.clone())).unwrap();
        let cj = base.join("claude.json");
        std::fs::write(&cj, "{}").unwrap();
        let places = vec![worktrees_core::model::PlaceRef {
            slug: "lane".into(),
            path: a.join(".worktrees/lane").to_string_lossy().into_owned(),
            branch: None,
            is_main: false,
            registered: true,
        }];
        let into_b = drop_token(&places, "lane", &b.to_string_lossy(), "lane", &cj).unwrap_err();
        assert!(into_b.contains("no worktrees MCP server") && !into_b.contains(&ghost), "{into_b}");
        let into_a = drop_token(&places, "lane", &a.to_string_lossy(), "lane", &cj).unwrap_err();
        assert!(into_a.contains(&ghost), "{into_a}");
        // With a server in user scope, the token itself.
        std::fs::write(&cj, r#"{"mcpServers":{"worktrees":{"command":"/x/bin/worktrees","args":["mcp","--mutations"]}}}"#).unwrap();
        assert_eq!(drop_token(&places, "lane", &b.to_string_lossy(), "lane", &cj).unwrap(), "@worktrees:place://lane");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A GUI launch has no locale, and pbcopy without one mangles every
    /// non-ASCII byte of a copied path. Asserted on the command rather than by
    /// running it: a test must not overwrite the developer's clipboard.
    #[cfg(target_os = "macos")]
    #[test]
    fn pbcopy_is_told_its_input_is_utf8() {
        let c = pbcopy_command();
        let envs: Vec<_> = c.get_envs().collect();
        assert!(
            envs.contains(&(std::ffi::OsStr::new("LC_CTYPE"), Some(std::ffi::OsStr::new("UTF-8")))),
            "{envs:?}"
        );
        // An inherited LC_ALL would override LC_CTYPE; `None` = removed.
        assert!(envs.contains(&(std::ffi::OsStr::new("LC_ALL"), None)), "{envs:?}");
        assert_eq!(c.get_program(), "/usr/bin/pbcopy");
    }
}

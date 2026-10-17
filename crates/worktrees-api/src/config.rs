//! Config surfaces: the shared CLI config the app only reads, Claude's MCP
//! install, the per-project `.worktrees.toml`, and "Ask Claude".

use std::path::Path;

use serde::Serialize;
use worktrees_core::proc::run_deadline;
use worktrees_core::ui::CaptureUi;
use worktrees_core::{ops, store, sysclock, Project};

use crate::log::log as applog;
use crate::ops::{run_op, CmdResult};


#[derive(Serialize)]
pub struct AiConfig {
    /// Effective resolved ai_cmd (flag=None → env > config > default `claude`).
    pub ai_cmd: String,
    /// Effective resolved resume arg (env > config > default `-r`).
    pub ai_resume_arg: String,
    /// The shared config file path (respects $XDG_CONFIG_HOME).
    pub path: String,
    /// Whether that config file actually exists on disk.
    pub exists: bool,
}

/// The engine's effective AI command + resume arg, plus the shared config path.
/// Read-only (Phase 1). The frontend shows it under the ai_auto_resume checkbox
/// and offers a reveal button (reveal the PARENT dir when the file is absent —
/// revealItemInDir rejects a non-existent path).
pub fn get_ai_config() -> Result<AiConfig, String> {
    let path = worktrees_core::config::config_path();
    let ai_cmd = worktrees_core::config::resolve_ai_cmd(None);
    Ok(AiConfig {
        // What this command will ACTUALLY resume with. A harness that names an
        // exact session (Codex, pi) ignores `ai_resume_arg` entirely
        // (`config::resolve_ai_resume_arg_for`), so printing the configured
        // value here advertised an override that does nothing — and for Codex
        // the override it advertised was the cross-worktree `resume --last`
        // this release exists to stop.
        ai_resume_arg: match worktrees_core::harness::for_cmd(&ai_cmd).filter(|a| a.exact_resume()) {
            Some(a) => a.resume_display(),
            None => worktrees_core::config::resolve_ai_resume_arg(),
        },
        ai_cmd,
        path: path.to_string_lossy().into_owned(),
        exists: path.exists(),
    })
}


/// Is the worktrees MCP server wired into this machine's claude?
///
/// Cheap by construction — it parses `~/.claude.json` rather than running
/// `claude mcp list`, which health-checks by launching every server in the cwd
/// (~1.1s, and a false ✘ whenever the cwd is not a repo). Safe to call on a
/// sheet open or a Home render; still `async`, like every command here, because
/// it touches the filesystem.
///
/// Logged, unlike most read-only commands. `mcp_install`/`mcp_uninstall` already
/// applog, but the state that decides whether the offer EVER appears is
/// computed here — and every non-`absent` answer is a silent one, by design. A
/// user reporting "it never offered" leaves no other trace: without this line
/// the only way to tell `absent` (card suppressed by something in the UI) from
/// `cli-missing`/`elsewhere` (card correctly withheld) is to ask them to open
/// Settings and read it out.
pub fn mcp_status(repo: Option<String>) -> Result<worktrees_core::mcpsetup::Status, String> {
    let s = worktrees_core::mcpsetup::status(repo.as_deref());
    applog(
        "info",
        &format!(
            "mcp_status repo={} -> state={:?} found_in={:?} claude={} worktrees={}",
            repo.as_deref().unwrap_or("-"),
            s.state,
            s.found_in,
            s.claude_bin.as_deref().unwrap_or("-"),
            s.worktrees_bin.as_deref().unwrap_or("-"),
        ),
    );
    Ok(s)
}

/// Wire it in (or repair, or re-install with a different `--mutations`).
///
/// Spawns `claude mcp add`, so it is slow enough to need a busy state in the UI
/// and is deadline-guarded in core. The returned `Outcome` carries the RE-READ
/// status, which is the real success signal — the frontend renders that rather
/// than assuming the click worked.
pub fn mcp_install(repo: Option<String>, mutations: bool) -> Result<worktrees_core::mcpsetup::Outcome, String> {
    let r = worktrees_core::mcpsetup::install(repo.as_deref(), mutations);
    match &r {
        Ok(o) => applog(
            if o.ok { "info" } else { "warn" },
            &format!("mcp_install mutations={mutations}: ok={} state={:?}", o.ok, o.status.state),
        ),
        Err(e) => applog("error", &format!("mcp_install mutations={mutations}: {e}")),
    }
    r
}

/// Remove the user-scope server, so the panel is not a one-way door.
pub fn mcp_uninstall(repo: Option<String>) -> Result<worktrees_core::mcpsetup::Outcome, String> {
    let r = worktrees_core::mcpsetup::uninstall(repo.as_deref());
    if let Err(e) = &r {
        applog("error", &format!("mcp_uninstall: {e}"));
    }
    r
}

pub fn codex_mcp_status() -> Result<worktrees_core::codexmcp::Status, String> {
    Ok(worktrees_core::codexmcp::status())
}

pub fn codex_mcp_install(mutations: bool) -> Result<worktrees_core::codexmcp::Outcome, String> {
    worktrees_core::codexmcp::install(mutations)
}

pub fn codex_mcp_uninstall() -> Result<worktrees_core::codexmcp::Outcome, String> {
    worktrees_core::codexmcp::uninstall()
}

/// pi's twins of the three above (`pimcp`): status READS pi's mcp.json; the
/// writes are `pi mcp add/remove`, never ours.
pub fn pi_mcp_status() -> Result<worktrees_core::pimcp::Status, String> {
    Ok(worktrees_core::pimcp::status())
}

pub fn pi_mcp_install(mutations: bool) -> Result<worktrees_core::pimcp::Outcome, String> {
    worktrees_core::pimcp::install(mutations).inspect_err(|e| applog("error", &format!("pi_mcp_install: {e}")))
}

pub fn pi_mcp_uninstall() -> Result<worktrees_core::pimcp::Outcome, String> {
    worktrees_core::pimcp::uninstall().inspect_err(|e| applog("error", &format!("pi_mcp_uninstall: {e}")))
}

pub fn codex_mcp_migration_plan() -> Result<Vec<worktrees_core::mcpmigrate::Row>, String> {
    let result = worktrees_core::mcpmigrate::migration_plan();
    if let Err(e) = &result { applog("error", &format!("codex_mcp_migration_plan: {e}")); }
    result
}

pub fn codex_mcp_migrate(names: Vec<String>) -> Result<Vec<worktrees_core::mcpmigrate::Outcome>, String> {
    let result = worktrees_core::mcpmigrate::apply(&names);
    match &result {
        Err(e) => applog("error", &format!("codex_mcp_migrate: {e}")),
        Ok(rows) => {
            let failed = rows.iter().filter(|r| !r.ok).count();
            applog(if failed > 0 { "warn" } else { "info" }, &format!("codex_mcp_migrate: {} attempted, {failed} not copied", rows.len()));
        }
    }
    result
}


#[derive(Serialize)]
pub struct ProjectFileView {
    pub path: String,
    /// `"link"` | `"copy"` — the per-entry mode (§3).
    pub mode: String,
}

#[derive(Serialize)]
pub struct ProjectPortsView {
    pub stride: u32,
    pub max_slots: u32,
    /// `(NAME, base_port)`, in the config's own (BTreeMap-sorted) order.
    pub base: Vec<(String, u32)>,
}

#[derive(Serialize)]
pub struct ProjectComposeView {
    /// The `-f` list in DOCKER's order (a later file overrides an earlier one).
    /// A LIST because `projcfg::Compose` is one: `down -v` only removes volumes
    /// declared in the files it is handed, so the base compose file has to travel
    /// with the worktree override. `file = "…"` is still the one-element form.
    pub files: Vec<String>,
    /// The `{prefix}`/`{slug}` template, unexpanded — this is the declaration.
    pub project: String,
}

/// `[docs]` as the sheet shows it. Present here for the same reason every other
/// section is: a declared section the sheet does not mention leaves the user
/// unable to tell whether it took effect — and `[docs]` is the one section
/// whose effect is on ANOTHER surface (the Docs tab), so it is the easiest to
/// write wrong and never notice.
#[derive(Serialize)]
pub struct ProjectDocsView {
    /// Declared order, which is also listing order (`docs::index_with`).
    pub paths: Vec<String>,
    /// Where reading starts, when declared.
    pub index: Option<String>,
}

/// The direct analogue of `get_ai_config`: read-only visibility of a config the
/// app does not (yet) edit. Never a `CmdResult` — the sheet renders structure.
#[derive(Serialize)]
pub struct ProjectConfigView {
    /// Absolute path of `.worktrees.toml` — where it IS, or where it would go.
    pub path: String,
    pub exists: bool,
    pub files: Vec<ProjectFileView>,
    pub ports: Option<ProjectPortsView>,
    pub compose: Option<ProjectComposeView>,
    pub docs: Option<ProjectDocsView>,
    /// A fatal parse/validation failure, rendered as its `file:line: message`.
    /// This is the most important thing the sheet can say when set: it is *why*
    /// every op in the repo is refusing.
    pub error: Option<String>,
    /// Non-fatal parse findings, already rendered as `file:line: message`. Today
    /// that is exactly `projcfg`'s `unknown-key` family — an unknown key, an
    /// unknown table, or a wrong-shaped `[project]`. (Prefix disagreement is a
    /// `doctor` finding, not a parse one: it needs the repo, not the file.)
    pub warnings: Vec<String>,
}

pub fn project_config_read(repo: String) -> Result<ProjectConfigView, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("project_config_read repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let main = Path::new(&p.main_root);
    let path = main.join(worktrees_core::projcfg::CONFIG_FILE);
    let mut view = ProjectConfigView {
        path: path.to_string_lossy().into_owned(),
        exists: path.exists(),
        files: Vec::new(),
        ports: None,
        compose: None,
        docs: None,
        error: None,
        warnings: Vec::new(),
    };
    match worktrees_core::projcfg::load(main) {
        Ok((Some(cfg), findings)) => {
            view.files = cfg
                .files
                .iter()
                .map(|f| ProjectFileView {
                    path: f.path.as_str().to_string(),
                    mode: match f.mode {
                        worktrees_core::projcfg::Mode::Link => "link",
                        worktrees_core::projcfg::Mode::Copy => "copy",
                    }
                    .to_string(),
                })
                .collect();
            view.ports = cfg.ports.as_ref().map(|x| ProjectPortsView {
                stride: x.stride,
                max_slots: x.max_slots,
                base: x.base.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            });
            view.compose = cfg.compose.as_ref().map(|c| ProjectComposeView {
                files: c.files.iter().map(|f| f.as_str().to_string()).collect(),
                project: c.project.clone(),
            });
            view.docs = cfg.docs.as_ref().map(|d| ProjectDocsView {
                paths: d.paths.iter().map(|p| p.as_str().to_string()).collect(),
                index: d.index.as_ref().map(|i| i.as_str().to_string()),
            });
            view.warnings = findings.iter().map(|f| f.message.clone()).collect();
        }
        // No config is a healthy repo, not a broken one (§2.4).
        Ok((None, _)) => {}
        Err(e) => {
            applog("warn", &format!("project_config_read repo={repo}: {e}"));
            view.error = Some(e.to_string());
        }
    }
    Ok(view)
}

/// `doctor`'s TYPED report — badges need structure, so this is deliberately not
/// a `CmdResult` (§10). `findings` is `diag::Finding` verbatim, so the app's
/// vocabulary of severities and codes is the CLI's by construction.
#[derive(Serialize)]
pub struct DoctorReport {
    /// `0` clean · `1` a guard only the user can fix (bad worktree name, an
    /// unreadable config) · `2` findings present.
    pub code: i32,
    pub schema_version: u32,
    pub findings: Vec<worktrees_core::diag::Finding>,
    /// Set when the run produced no report at all (`code == 1`): the guard
    /// message, already in app.log. Never swallowed.
    pub error: Option<String>,
}

/// Report drift for one place, or for every place in the project (`slug: None`).
///
/// Runs the REAL `cmd_doctor` in `--json` mode and parses the line it emits,
/// rather than re-assembling findings here: the CLI and the app must never
/// disagree about what "drift" means.
pub fn doctor(repo: String, slug: Option<String>) -> Result<DoctorReport, String> {
    let project = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("doctor repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let mut args: Vec<String> = Vec::new();
    if let Some(s) = slug.filter(|s| !s.trim().is_empty()) {
        args.push(s);
    }
    args.push("--json".into());
    let mut ui = CaptureUi::default();
    let code = ops::cmd_doctor(&project, &mut ui, &args);
    let parsed = ui
        .lines
        .iter()
        .rev()
        .find_map(|l| serde_json::from_str::<worktrees_core::diag::Report>(l).ok());
    match parsed {
        Some(r) => {
            if !r.findings.is_empty() {
                applog("info", &format!("doctor rc={code} repo={repo}: {} finding(s)", r.findings.len()));
            }
            Ok(DoctorReport { code, schema_version: r.schema_version, findings: r.findings, error: None })
        }
        None => {
            // rc 1 before the report was emitted (a usage guard, or a config that
            // does not parse). The lines ARE the diagnosis.
            let msg = ui.lines.join("\n");
            applog("warn", &format!("doctor rc={code} repo={repo}: {msg}"));
            Ok(DoctorReport {
                code,
                schema_version: worktrees_core::diag::SCHEMA_VERSION,
                findings: Vec::new(),
                error: Some(if msg.is_empty() { format!("doctor exited {code}") } else { msg }),
            })
        }
    }
}

/// One place's health verdict — `doctor`'s shape, for `status`.
///
/// `report` is `health::Report` verbatim, so the sheet's vocabulary of verdicts
/// and reasons is the CLI's by construction; `error` carries the guard message
/// when `cmd_status` exited before emitting any JSON (never swallowed).
#[derive(Serialize)]
pub struct HealthReport {
    /// `0` a report was emitted · `1` usage / unknown / unregistered place.
    pub code: i32,
    pub report: Option<worktrees_core::health::Report>,
    pub error: Option<String>,
}

/// Run the REAL `cmd_status --json` in-process for one place and parse the line
/// it emits → `(exit code, report, the captured lines)`.
///
/// Factored out because TWO commands need the same verdict and must not each
/// grow their own parse: `place_health` renders it, and `ai_status_report`
/// embeds its JSON in the prompt it hands claude. A second copy would be a
/// second answer to "what did the check say" — the same reason `place_health`
/// shells through core at all instead of re-assembling facts here.
///
/// `None` for the report means `cmd_status` exited on a guard before emitting
/// any JSON (unknown or unregistered place); the lines ARE the diagnosis then.
pub fn status_of(project: &Project, slug: &str) -> (i32, Option<worktrees_core::health::Report>, String) {
    let args: Vec<String> = vec![slug.to_string(), "--json".into()];
    let mut ui = CaptureUi::default();
    let code = ops::cmd_status(project, &mut ui, &args);
    let parsed = ui
        .lines
        .iter()
        .rev()
        .find_map(|l| serde_json::from_str::<worktrees_core::health::Report>(l).ok());
    (code, parsed, ui.lines.join("\n"))
}

/// Health verdict for one place. Runs the REAL `cmd_status` in `--json` mode
/// in-process and parses the line it emits, for the same reason `doctor` does:
/// the CLI and the app must never disagree about what "at risk" means.
pub fn place_health(repo: String, slug: String) -> Result<HealthReport, String> {
    let project = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("place_health repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let (code, parsed, lines) = status_of(&project, &slug);
    match parsed {
        Some(r) => Ok(HealthReport { code, report: Some(r), error: None }),
        None => {
            // rc 1 before the report was emitted (an unknown or unregistered
            // place). The captured lines ARE the diagnosis.
            applog("warn", &format!("place_health rc={code} repo={repo} slug={slug}: {lines}"));
            Ok(HealthReport {
                code,
                report: None,
                error: Some(if lines.is_empty() { format!("status exited {code}") } else { lines }),
            })
        }
    }
}

/// `Project::discover`, with the failure logged — the shape `place_health` uses.
pub fn project_at(repo: &str, what: &str) -> Result<Project, String> {
    Project::discover(Path::new(repo)).map_err(|e| {
        applog("error", &format!("{what} repo={repo}: discover failed: {}", e.msg));
        e.msg
    })
}


/// What the frontend caches under `declared.status_report`.
#[derive(Serialize)]
pub struct AiStatusReport {
    pub text: String,
    pub epoch: i64,
    /// The verdict from the SAME `cmd_status` run whose JSON went into the
    /// prompt — so a cached read can say what the machine thought at the time,
    /// beside what claude thought about it.
    pub verdict: String,
}

/// The "is this really claude" guard, now core's (`profile.rs`).
///
/// It moved to core when a second headless caller (the since-removed automation
/// runner) needed it, and stays there: the reasoning it encodes (read the COMPOSED command, never `match_word`, because
/// `ai_launch_for` fails closed with a `printf` sentinel that keeps the word)
/// is the kind that must have exactly one home. The test below still drives it.
use worktrees_core::profile::claude_launch_check;

/// The prompt, composed from the health report and the worktree's path.
///
/// Pure and unit-tested because it is the whole interface to the model: the JSON
/// goes in VERBATIM (claude must not have to re-derive facts the check already
/// measured), and the ban on running commands is what keeps a headless run —
/// which cannot answer a permission prompt — from stalling on a tool it will
/// never be granted. Reads are allowed, which is why the planning files are
/// named: they are where a worktree says what it was FOR, and no git fact does.
pub fn status_prompt(report_json: &str, path: &str) -> String {
    format!(
        "{report_json}\n\
         \n\
         You are reporting on the git worktree at {path}. The JSON above is its \
         measured state. If task_plan.md, findings.md, progress.md or ROADMAP.md \
         exist there, read them.\n\
         \n\
         Answer in under 250 words:\n\
         (1) what was this worktree for,\n\
         (2) what state did the work end in,\n\
         (3) recommend exactly one of: resume / push-then-abandon / abandon, with \
         one line of justification.\n\
         \n\
         Do not run commands; the git facts are already in the JSON."
    )
}

/// Last `n` lines of a subprocess's stderr, for an error the user has to read.
/// Whole-stderr in a toast is unreadable and whole-stderr in an `Err` string is
/// worse; the tail is where a CLI puts the reason.
pub fn stderr_tail(bytes: &[u8], n: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Hand this worktree's measured state to the repo's AI profile, headless, and
/// cache the paragraph that comes back.
///
/// App-only on purpose: there is no CLI verb for it (the seam would support one
/// — see the spec's out-of-scope list — but a headless spawn with a 180s
/// deadline is a very different contract from a CLI that a user can Ctrl-C).
/// NEVER runs on its own: the sheet has a button, and nothing else calls this.
pub fn ai_status_report(repo: String, slug: String) -> Result<AiStatusReport, String> {
    let project = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("ai_status_report repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    // `place_dir`, not `wt_root.join(slug)`: `(main)` is a place too, and it
    // lives at the main checkout rather than under `.worktrees/`.
    let wt_path = project.place_dir(&slug);

    // THE seam (ops.rs): the same call every interactive launch makes, so this
    // report runs under exactly the profile a session in this place would.
    let mut ui = CaptureUi::default();
    let ai = ops::ai_launch_for(
        &project,
        &mut ui,
        &wt_path,
        &worktrees_core::config::resolve_ai_cmd(None),
    );
    // The seam's own voice — a skipped skill, or the fail-closed reason. Never
    // swallowed: without this, a broken profile's guard message below would be
    // the ONLY trace, and it deliberately points at this log.
    for w in ui.warnings() {
        applog("warn", &format!("ai_status_report repo={repo} slug={slug}: {w}"));
    }
    // And what it DID: at full the seam can write `/.planning/` to
    // info/exclude, and a write nobody hears of is a write nobody can trace.
    for (_, m) in ui.events.iter().filter(|(s, _)| *s == worktrees_core::diag::Severity::Info) {
        applog("info", &format!("ai_status_report repo={repo} slug={slug}: {m}"));
    }
    if let Err(msg) = claude_launch_check(&ai.cmd, &ai.match_word) {
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: {msg}"));
        return Err(msg);
    }

    let (code, report, lines) = status_of(&project, &slug);
    let Some(report) = report else {
        let why = if lines.is_empty() { format!("status exited {code}") } else { lines };
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: no status report: {why}"));
        return Err(format!("could not read this worktree's status: {why}"));
    };
    let verdict = report.verdict.clone();
    let json = serde_json::to_string(&report).map_err(|e| {
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: encode failed: {e}"));
        e.to_string()
    })?;
    let prompt = status_prompt(&json, &wt_path);

    // ⚠ `exec` is load-bearing. `run_deadline` kills its DIRECT child and only
    // that (one pid, no process group). Without `exec` the direct child is
    // `sh`, and a timed-out claude survives it as an orphan — still burning
    // turns, still holding any MCP servers the profile started. `exec` makes sh
    // replace itself, so the deadline's kill lands on claude.
    //
    // `sh -c` because `ai.cmd` is an already-composed, shell-quoted flag string
    // (profile::claude_launch). Env goes through `Command::env`, NOT inlined as
    // a `K=V ` prefix: that separation is the point of `AiLaunch`.
    let line = format!("exec {} -p {} --max-turns 12", ai.cmd, worktrees_core::tmux::sq(&prompt));
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", &line]).current_dir(&wt_path);
    for (k, v) in &ai.env {
        cmd.env(k, v); // CLAUDE_CONFIG_DIR, when a profile applies
    }
    let out = run_deadline(cmd, 180).map_err(|e| {
        let msg = if e.kind() == std::io::ErrorKind::TimedOut {
            "claude timed out after 180s".to_string()
        } else {
            format!("could not run claude: {e}")
        };
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: {msg}"));
        msg
    })?;

    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let tail = stderr_tail(&out.stderr, 12);
        let rc = out.status.code().unwrap_or(-1);
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: claude rc={rc}: {tail}"));
        return Err(if tail.is_empty() { format!("claude exited {rc}") } else { format!("claude exited {rc}:\n{tail}") });
    }
    // A real failure shape, not a curiosity: claude can exit 0 having printed
    // nothing (a denied tool, an empty turn budget). Caching that would leave a
    // blank "Claude's read" section that looks like a successful answer, and the
    // Re-run button would be the only way to discover it was not one.
    if text.is_empty() {
        let tail = stderr_tail(&out.stderr, 12);
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: claude exited 0 with no output: {tail}"));
        return Err(if tail.is_empty() {
            "claude exited 0 but printed nothing".into()
        } else {
            format!("claude exited 0 but printed nothing:\n{tail}")
        });
    }

    let epoch = sysclock::now_epoch();
    // The declared sidecar, via the same locked + atomic edit every other
    // declared write uses. `extra` round-trips unknown keys by design, so this
    // needs no schema surgery — and it is NOT `ui-state.json`, which the
    // frontend owns whole-blob (a backend write there is erased by the next
    // settings save).
    //
    // ⚠ `last_worked_epoch` is deliberately untouched. That stamp means "claude
    // finished a task HERE", and it drives the nav's afterglow; a report ABOUT a
    // worktree is not work IN it, and stamping it would relight a cold place.
    let stamp = serde_json::json!({ "text": text, "epoch": epoch, "verdict": verdict });
    store::edit(&repo, &slug, |d| {
        d.extra.insert("status_report".into(), stamp);
    })
    .map_err(|e| {
        applog("error", &format!("ai_status_report repo={repo} slug={slug}: store write failed: {e}"));
        e
    })?;
    applog("info", &format!("ai_status_report repo={repo} slug={slug}: {} chars, verdict={verdict}", text.len()));
    Ok(AiStatusReport { text, epoch, verdict })
}

/// Re-apply the file plan (`relink [<wt>|--all] [--force]`). `force` is the
/// documented escape hatch for a shadowing regular file — core writes a `.bak`
/// alongside before it touches one (§7), so the drifted content is never the
/// only casualty.
pub fn relink(repo: String, slug: Option<String>, force: bool) -> Result<CmdResult, String> {
    let target = slug.filter(|s| !s.trim().is_empty());
    let mut args: Vec<String> = vec![target.clone().unwrap_or_else(|| "--all".into())];
    if force {
        args.push("--force".into());
    }
    let label = target.as_deref().unwrap_or("--all").to_string();
    run_op(&format!("relink {label}"), &repo, |p, ui| ops::cmd_relink(p, ui, &args))
}

/// Allocate/repair port slots (`provision [<wt>|--all]`). `--reallocate` is
/// deliberately NOT exposed: §6 makes moving a slot under a running stack a
/// refuse-by-default, and a GUI button is the last place that decision belongs.
pub fn provision(repo: String, slug: Option<String>) -> Result<CmdResult, String> {
    let target = slug.filter(|s| !s.trim().is_empty());
    let args: Vec<String> = vec![target.clone().unwrap_or_else(|| "--all".into())];
    let label = target.as_deref().unwrap_or("--all").to_string();
    run_op(&format!("provision {label}"), &repo, |p, ui| ops::cmd_provision(p, ui, &args))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prompt is the WHOLE interface to the model, and every clause in it is
    /// load-bearing: drop the JSON and claude re-derives facts the check already
    /// measured; drop the ban on commands and a headless run — which cannot
    /// answer a permission prompt — stalls on a tool it will never be granted;
    /// drop the path and it reports on whatever directory it guessed.
    #[test]
    fn the_status_prompt_carries_the_facts_the_questions_and_the_ban() {
        let json = r#"{"schema_version":1,"verdict":"at-risk","reasons":["3 commits not on origin/main"]}"#;
        let p = status_prompt(json, "/tmp/wt/feat-x");

        // VERBATIM — not re-serialized, not summarized. The report the sheet
        // renders and the report claude reads have to be the same bytes.
        assert!(p.contains(json), "the health JSON must appear verbatim");
        assert!(p.contains("/tmp/wt/feat-x"), "the worktree path must be named");

        // The three questions, in order.
        assert!(p.contains("what was this worktree for"));
        assert!(p.contains("what state did the work end in"));
        assert!(p.contains("resume / push-then-abandon / abandon"));
        let (q1, q3) = (p.find("what was this worktree for"), p.find("push-then-abandon"));
        assert!(q1 < q3, "the questions must stay in order");

        // The reading list, the budget, and the ban.
        for planning in ["task_plan.md", "findings.md", "progress.md", "ROADMAP.md"] {
            assert!(p.contains(planning), "the prompt should point at {planning}");
        }
        assert!(p.contains("under 250 words"));
        assert!(p.contains("Do not run commands"));
    }

    /// The guard that decides whether anything is spawned at all.
    ///
    /// The case that matters most is the third: `ops::ai_launch_for` fails
    /// CLOSED by swapping in a `printf … >&2` sentinel while KEEPING
    /// `match_word: "claude"`. A guard written against `match_word` — the
    /// obvious choice, and what every other caller in the codebase reads — waves
    /// that through, and the app then caches printf's error message as claude's
    /// considered read of the worktree.
    #[test]
    fn the_guard_reads_the_composed_command_not_the_match_word() {
        // plain claude
        assert!(claude_launch_check("claude", "claude").is_ok());
        // profiled claude — claude_launch appends flags to the same first word
        assert!(claude_launch_check(
            "claude --append-system-prompt-file '/x/rules.md' --settings '/x/s.json' --strict-mcp-config",
            "claude",
        )
        .is_ok());
        // an absolute path still basenames to claude
        assert!(claude_launch_check("/opt/homebrew/bin/claude -r", "claude").is_ok());

        // fail-closed sentinel: match_word STILL says claude, the command does not
        let sentinel = "printf '%s\\n' 'profile could not be prepared' >&2";
        let e = claude_launch_check(sentinel, "claude").expect_err("the printf sentinel must be refused");
        assert!(e.contains("could not be prepared"), "say WHICH failure it was: {e}");
        assert!(e.contains("app log"), "point at where the reason is: {e}");

        // a different AI tool — a different fix, so a different message
        let e = claude_launch_check("aider --model x", "aider").expect_err("non-claude ai_cmd must be refused");
        assert!(e.contains("ai_cmd"), "{e}");
        assert!(e.contains("aider"), "name the tool it found: {e}");

        // ai_cmd = none (plain shell). ⚠ `ai_word_of("")` DEFAULTS to "claude",
        // so an empty command passes the word check and has to be caught first.
        let e = claude_launch_check("", "claude").expect_err("an empty command must be refused");
        assert!(e.contains("none"), "{e}");
        assert!(claude_launch_check("   ", "claude").is_err(), "whitespace is empty too");
    }

    /// The tail is what the user reads when a run fails, so it has to survive
    /// the two shapes a CLI's stderr actually takes: nothing at all, and pages.
    #[test]
    fn the_stderr_tail_keeps_the_end_and_drops_the_blanks() {
        assert_eq!(stderr_tail(b"", 3), "");
        assert_eq!(stderr_tail(b"\n\n  \n", 3), "", "blank lines are not a reason");
        assert_eq!(stderr_tail(b"one\ntwo\nthree", 5), "one\ntwo\nthree");
        assert_eq!(stderr_tail(b"one\ntwo\nthree\nfour", 2), "three\nfour", "the END is the reason");
    }

}

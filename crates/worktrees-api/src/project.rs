//! Project-level operations that reach outside the repo: courier sync through
//! a mounted hub, the agent-instruction files, diagnostics, and pull requests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use worktrees_core::proc::run_deadline;
use worktrees_core::ui::CaptureUi;
use worktrees_core::{ops, sync, Project};

use crate::argv::Argv;
use crate::log::log as applog;
use crate::config::project_at;
use crate::editors;
use crate::hostpolicy;
use crate::ops::{known_harness, run_op, CmdResult};
use crate::status::cli_binary;
use crate::Ctx;


/// `repo` = a project in the workspace; `name` = a project that exists only on
/// the hub (adoption — the machine has no tree for it, so there is no root to
/// pass). Exactly one is expected; `repo` wins if both arrive, since a caller
/// that knows a root is not adopting.
pub fn sync_request(
    repo: Option<&str>,
    name: Option<&str>,
    direction: &str,
    with_sessions: bool,
) -> Result<sync::SyncRequest, String> {
    let source = match (repo, name) {
        (Some(r), _) => {
            let p = Project::discover(Path::new(r)).map_err(|e| e.msg)?;
            sync::SyncSource::Root(PathBuf::from(&p.main_root))
        }
        (None, Some(n)) => sync::SyncSource::HubName(n.to_string()),
        (None, None) => return Err("sync needs a project: pass repo, or name to import one from the hub".into()),
    };
    Ok(sync::SyncRequest {
        source,
        direction: sync::SyncDirection::parse(direction)?,
        hub: None,
        with_sessions: Some(with_sessions),
    })
}

/// `sync status` for one project, plus whether this checkout IS a hub copy —
/// the menu disables the whole sync group there (pushing FROM a copy sends it
/// back over the original; pulling INTO one is hub→hub).
#[derive(Serialize)]
pub struct SyncStatusView {
    #[serde(flatten)]
    pub status: sync::StatusJson,
    pub hub_copy: Option<String>,
}

/// Cheap enough for a menu-open: one `rsync --version`, one hub resolve, one
/// manifest read, two git calls. No rsync transfer of any kind.
pub fn sync_status(repo: String) -> Result<SyncStatusView, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("sync_status repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let root = PathBuf::from(&p.main_root);
    let mut ui = CaptureUi::default();
    let status = sync::sync_status_data(&mut ui, Some(&root), None, true);
    Ok(SyncStatusView { status, hub_copy: sync::hub_copy_refusal(&root) })
}

/// The hub's own status: what has been pushed to this drive, from anywhere.
/// This is the ONE sync command with no project — the import picker calls it on
/// a machine whose workspace may be empty, which is precisely the machine that
/// needs it (`sync status` outside a repo, the CLI's adoption listing).
///
/// The walk is core's: `sync_status_data(from: None)` already lists every
/// manifest on the hub, so nothing here re-implements it. `hub_error` carries
/// the reason there is no hub, because "no projects" and "no drive" are
/// different answers and the picker must not show them the same way.
pub fn sync_hub_list() -> Result<sync::StatusJson, String> {
    let mut ui = CaptureUi::default();
    let out = sync::sync_status_data(&mut ui, None, None, true);
    if let Some(e) = &out.hub_error {
        applog("info", &format!("sync_hub_list: no hub ({e})"));
    }
    Ok(out)
}

/// What a sync WOULD do — the modal's whole content. Two rsync dry passes plus
/// (on a pull) the live-session lookup, so it is seconds on a real tree.
///
/// `name` (with no `repo`) is an adoption preview: the destination in the
/// answer comes from the hub manifest, and it is the only thing telling the
/// user where a tree they have never had is about to appear.
pub fn sync_preview(
    repo: Option<String>,
    name: Option<String>,
    direction: String,
    with_sessions: bool,
) -> Result<sync::SyncPreviewOut, String> {
    let req = sync_request(repo.as_deref(), name.as_deref(), &direction, with_sessions)?;
    sync::sync_preview(&req).inspect_err(|e| {
        let who = repo.clone().or_else(|| name.clone()).unwrap_or_default();
        applog("warn", &format!("sync_preview {direction} {who}: {e}"));
    })
}

/// Apply it. `confirmed` is the user's answer to the modal's live-session
/// warning — core re-lists the sessions here and refuses again if the answer no
/// longer covers what is running.
///
/// Mapped into `CmdResult` (not `Result::Err`) so the frontend's `runCmd` shows
/// it the way it shows every other op: `needs_confirm` carries the live sessions
/// NEWLINE-separated, structurally, so the modal names them without parsing prose.
///
/// `on_progress` streams rsync's own progress while the transfer runs — the same
/// `Channel` mechanism `term_open` uses for pty bytes. An 8.5G first push is
/// minutes long, and a button that says "Pushing…" for four minutes is
/// indistinguishable from a hung one. Core throttles to ~15 events/sec and
/// always sends the last one, so this end can simply forward.
pub fn sync_apply(
    ctx: &Ctx,
    repo: Option<String>,
    name: Option<String>,
    direction: String,
    with_sessions: bool,
    confirmed: bool,
    install: bool,
    on_progress: &mut dyn FnMut(sync::SyncProgress),
) -> Result<CmdResult, String> {
    let req = sync_request(repo.as_deref(), name.as_deref(), &direction, with_sessions)?;
    let repo = repo.or_else(|| name.clone()).unwrap_or_default();
    let op = format!("sync {direction}");
    // A client that went away mid-sync is not a reason to stop the transfer or
    // to lose the result, and it is not swallowed either — but that is the
    // TRANSPORT's knowledge, so the Tauri shim owns it now and logs it once.
    let outcome = sync::sync_apply(&req, confirmed, install, Some(on_progress));
    match outcome {
        Err(e) => {
            applog("warn", &format!("{op} rc=1 repo={repo}: {e}"));
            Ok(CmdResult {
                ok: false,
                code: 1,
                output: e,
                slug: None,
                needs_confirm: None,
                warnings: Vec::new(),
            })
        }
        Ok(out) if out.needs_confirm => {
            let live = out.live_sessions.join(", ");
            applog("info", &format!("{op} needs confirm repo={repo}: {live}"));
            Ok(CmdResult {
                ok: false,
                code: worktrees_core::diag::EXIT_NEEDS_CONFIRM,
                output: format!(
                    "{} live tmux session(s) in {}: {live} — a pull mirrors over the tree they are using.",
                    out.live_sessions.len(),
                    out.dst
                ),
                slug: None,
                needs_confirm: Some(out.live_sessions.join("\n")),
                warnings: out.warnings,
            })
        }
        Ok(out) => {
            applog(
                "info",
                &format!(
                    "{op} ok repo={repo}: {} sent/updated, {} deleted → {}",
                    out.plan.sends, out.plan.deletes, out.dst
                ),
            );
            if !out.warnings.is_empty() {
                applog("warn", &format!("{op} warnings repo={repo}: {}", out.warnings.join(" | ")));
            }
            let mut lines = vec![format!(
                "sync {} — {}: {} sent/updated, {} deleted",
                out.direction, out.name, out.plan.sends, out.plan.deletes
            )];
            // An adoption just created a tree this workspace has never seen. The
            // app's whole surface hangs off a project ROW, so a transfer that
            // stopped here would leave the user with files and no way to reach
            // them — the add is part of the import, not a follow-up.
            //
            // It goes through `add_project` itself (the same discover + dedupe +
            // persist the "Add project" button runs), so an import cannot admit
            // something the button would have refused.
            if out.adopting {
                match crate::workspace::add_project(ctx, out.dst.clone()) {
                    Ok(_) => lines.push(format!("added to the workspace: {}", out.dst)),
                    Err(e) => {
                        // Half-success is the one outcome that must never read as
                        // either a success or a plain failure: the FILES are
                        // there, and only the bookkeeping failed. Say both, and
                        // say what to do about it.
                        applog(
                            "error",
                            &format!("{op} transferred but add_project failed dst={}: {e}", out.dst),
                        );
                        ctx.emit("places:changed", serde_json::Value::Null);
                        return Ok(CmdResult {
                            ok: false,
                            code: 1,
                            output: format!(
                                "{}\nThe transfer finished — the files are at {}.\nAdding it to the workspace failed: {e}\nAdd it by hand with “Add project” and pick that folder.",
                                lines.join("\n"),
                                out.dst
                            ),
                            slug: None,
                            needs_confirm: None,
                            warnings: out.warnings,
                        });
                    }
                }
            }
            // A pull rewrites the tree under every place in it — branches, dirty
            // counts and sessions are all potentially different now. Emitted
            // AFTER the workspace add, so the refresh it triggers is the one
            // that already contains the imported project.
            if req.direction == sync::SyncDirection::Pull {
                ctx.emit("places:changed", serde_json::Value::Null);
            }
            Ok(CmdResult {
                ok: true,
                code: 0,
                output: lines.join("\n"),
                slug: None,
                needs_confirm: None,
                warnings: out.warnings,
            })
        }
    }
}

#[derive(Serialize)]
pub struct SuggestedFile {
    pub path: String,
    /// The class that fails SILENTLY (§1.2) — flagged louder in the UI.
    pub credential: bool,
}

/// What `worktrees init` would suggest, WITHOUT writing anything.
#[derive(Serialize)]
pub struct InitSuggestion {
    /// Where `.worktrees.toml` would be written.
    pub path: String,
    /// A config is already there (then `qualifies` is irrelevant — nothing is
    /// suggested over an existing file).
    pub exists: bool,
    /// Anything at all to configure.
    pub qualifies: bool,
    pub files: Vec<SuggestedFile>,
    pub credentials: usize,
    pub ports: bool,
    pub compose: bool,
    /// Existing worktrees already missing at least one suggested file.
    pub stale_places: Vec<String>,
    /// A walk bound was hit, so the suggestion may be incomplete.
    pub truncated: bool,
    /// The rendered file, comments and all — the sheet shows it before writing.
    pub toml: String,
    /// §9's dismissal key: a repo that later gains a credential file gets a NEW
    /// hash and correctly re-suggests, which a boolean "dismissed" could never do.
    /// Hashed from a CANONICAL PROJECTION of the suggestion, not from `toml` —
    /// see `suggestion_key` for which differences are deliberately invisible here.
    pub hash: String,
}

/// A 64-bit string mixer, FNV-1a **in shape only**: the multiplier below is
/// `0x1000_0000_01b3`, which is NOT the FNV-64 prime (`0x100_0000_01b3`). It is
/// deliberately byte-for-byte the same mixer as `init.rs`'s `fnv1a`, so the CLI's
/// once-only marker and the app's dismissal key can never disagree about what
/// "the same suggestion" means. Renamed off `fnv1a_hex` because claiming FNV
/// while using a different constant is how the next reader gets misled.
///
/// Not a security boundary — it only answers "is this the suggestion I already
/// declined?". If core's constant is ever corrected, correct this one in the same
/// change (both are pre-release, so no stored marker survives either way).
pub fn digest_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{h:016x}")
}

/// The dismissal key's INPUT: a canonical projection of the suggestion, not the
/// rendered TOML.
///
/// Hashing the rendered file made the key sensitive to things that are not a
/// change in what is being suggested. `init::walk` picks the `[compose] file`
/// from a `read_dir`-ordered candidate list, and truncates its hit list at
/// `MAX_HITS`/`MAX_ENTRIES` before sorting — so on a repo with two compose files
/// (or one big enough to truncate) the same checkout can render two different
/// TOMLs. A flapping key resurrects a banner the user already dismissed.
///
/// So: sort the file list, reduce compose to the boolean that actually drives the
/// suggestion, and drop the port numbers (they come from the same heuristic scan).
/// What survives is exactly §9's contract — "a repo that later gains a credential
/// file re-suggests" — and nothing else.
///
/// ⚠ This cannot fix the remaining case: when the walk truncates, the SET of files
/// can differ between runs, and no projection of a wrong set is stable. That fix
/// belongs in `worktrees_core::init::walk` (sort each `read_dir` before the
/// bound is applied).
pub fn suggestion_key(s: &InitSuggestion) -> String {
    let mut lines: Vec<String> = s
        .files
        .iter()
        .map(|f| format!("file\t{}\t{}", if f.credential { "cred" } else { "plain" }, f.path))
        .collect();
    lines.sort();
    lines.push(format!("ports\t{}", s.ports));
    lines.push(format!("compose\t{}", s.compose));
    lines.push(format!("truncated\t{}", s.truncated));
    digest_hex(&lines.join("\n"))
}

pub fn init_suggest(repo: String) -> Result<InitSuggestion, String> {
    use worktrees_core::init;
    let p = Project::discover(Path::new(&repo)).map_err(|e| {
        applog("error", &format!("init_suggest repo={repo}: discover failed: {}", e.msg));
        e.msg
    })?;
    let main = Path::new(&p.main_root);
    let path = main.join(worktrees_core::projcfg::CONFIG_FILE);
    let facts = init::probe(main, Path::new(p.wt_root_dir()));
    let sug = init::detect(&facts);
    let qualifies = !sug.is_empty();
    let toml = if qualifies { init::render(&sug) } else { String::new() };
    let mut view = InitSuggestion {
        path: path.to_string_lossy().into_owned(),
        exists: path.exists(),
        qualifies,
        files: sug
            .files
            .iter()
            .map(|c| SuggestedFile { path: c.rel.clone(), credential: c.kind == init::Kind::Credential })
            .collect(),
        credentials: sug.credentials(),
        ports: sug.ports.is_some(),
        compose: sug.compose.is_some(),
        stale_places: sug.stale_places.clone(),
        truncated: sug.truncated,
        hash: String::new(),
        toml,
    };
    // Derived from the projection, never from `view.toml` — see `suggestion_key`.
    view.hash = suggestion_key(&view);
    Ok(view)
}

/// Write the suggested `.worktrees.toml`.
///
/// `-y` carries the consent: `CaptureUi::confirm` always answers NO (a
/// programmatic caller must never have a config appear under it), so without the
/// flag this command could only ever print. The SHEET confirms first — that is
/// where the human says yes. NOT `--force`: an existing config is still refused,
/// loudly, exactly as on the CLI.
pub fn init_write(repo: String) -> Result<CmdResult, String> {
    run_op("init", &repo, |p, ui| Argv::new().flag("-y").call(p, ui, ops::cmd_init))
}


#[derive(Serialize)]
pub struct AgentSetupStatus {
    pub repo: worktrees_core::agentfiles::Report,
    pub user_skills: Vec<worktrees_core::agentfiles::UserSkill>,
    /// The worktrees rules (ADR 0002), from the same ref. `None` when they
    /// could not be read — logged, and the sheet says so rather than offering
    /// an apply it cannot judge.
    pub rules: Option<worktrees_core::rules::Status>,
}

/// How long one `gh pr view` answer stands for a project.
pub const PR_MERGED_TTL: std::time::Duration = std::time::Duration::from_secs(300);
pub static PR_MERGED: Mutex<Option<HashMap<String, (std::time::Instant, bool)>>> = Mutex::new(None);

pub fn pr_merged_cached(root: &str) -> bool {
    {
        let g = PR_MERGED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, merged)) = g.as_ref().and_then(|m| m.get(root)) {
            if at.elapsed() < PR_MERGED_TTL {
                return *merged;
            }
        }
    }
    // Asked outside the lock: a slow gh must not stall another project's read.
    let merged = worktrees_core::review::pr_state(root).as_deref() == Some("MERGED");
    let mut g = PR_MERGED.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(root.to_string(), (std::time::Instant::now(), merged));
    merged
}

pub fn agent_setup_status(repo: String) -> Result<AgentSetupStatus, String> {
    let p = project_at(&repo, "agent_setup_status")?;
    let report = worktrees_core::agentfiles::inspect(&p).map_err(|e| {
        applog("error", &format!("agent_setup_status repo={repo}: {e}"));
        e
    })?;
    let rules = worktrees_core::rules::status(&p)
        .map_err(|e| applog("error", &format!("agent_setup_status rules repo={repo}: {e}")))
        .ok();
    let mut report = report;
    // A pushed branch whose review place is still open: has its PR merged?
    // Asked of GitHub — a squash merge leaves the branch looking unmerged, and
    // a matching subject proves nothing. Only in this one state, and cached
    // (`PR_MERGED_TTL`): it is a network call, and the sheet re-reads on every
    // open. A stale "not yet" costs a few minutes; a stale "merged" costs
    // nothing, because `rules_lane_remove` asks GitHub again before it acts.
    if report.pending_on_origin && report.pending_place.is_some() {
        report.pending_merged = pr_merged_cached(&p.main_root);
    }
    Ok(AgentSetupStatus { repo: report, user_skills: worktrees_core::agentfiles::user_skills(), rules })
}

pub fn agent_setup_fix(ctx: &Ctx, repo: String, push: Option<bool>) -> Result<worktrees_core::agentfiles::FixOutcome, String> {
    let p = project_at(&repo, "agent_setup_fix")?;
    let out = worktrees_core::agentfiles::fix(&p, push.unwrap_or(false));
    // A new local branch (and maybe a fetch) either way — re-pull the snapshot.
    ctx.emit("places:changed", serde_json::Value::Null);
    match out {
        Ok(o) => {
            applog("info", &format!("agent_setup_fix repo={repo}: commit={:?} pushed={} pr={:?}", o.commit, o.pushed, o.pr_url));
            Ok(o)
        }
        Err(e) => {
            applog("error", &format!("agent_setup_fix repo={repo}: {e}"));
            Err(e)
        }
    }
}

/// `worktrees rules apply`: the layout fix plus the worktrees rules, one commit
/// on `agent-instructions`. Local unless `push`; `force` replaces a block or
/// skill someone edited by hand (never a close-out skill worktrees did not
/// write).
pub fn rules_apply(ctx: &Ctx, repo: String, push: Option<bool>, force: Option<bool>) -> Result<worktrees_core::agentfiles::FixOutcome, String> {
    let p = project_at(&repo, "rules_apply")?;
    let (push, force) = (push.unwrap_or(false), force.unwrap_or(false));
    let out = worktrees_core::rules::apply(&p, push, force);
    ctx.emit("places:changed", serde_json::Value::Null);
    match out {
        Ok(o) => {
            applog("info", &format!("rules_apply repo={repo} push={push} force={force}: commit={:?} pushed={} pr={:?}", o.commit, o.pushed, o.pr_url));
            Ok(o)
        }
        Err(e) => {
            applog("error", &format!("rules_apply repo={repo}: {e}"));
            Err(e)
        }
    }
}

/// Open the review lane for the `agent-instructions` branch: a place with a
/// brief and the default agent on it. Called right after Apply / Fix commit
/// locally, and from the To do row's Review button for a branch that already
/// exists. A launch the machine guard refuses comes back as
/// `EXIT_LAUNCH_REFUSED` with the place ready — the frontend asks "Launch
/// anyway?"; nothing is forced here.
pub fn rules_review(repo: String, provider: Option<String>, model: Option<String>) -> Result<CmdResult, String> {
    let provider = provider.unwrap_or_else(|| worktrees_core::harness::default_adapter().provider().id.into());
    known_harness(&provider)?.installed()?;
    let model = model.filter(|m| !m.trim().is_empty());
    if let Some(m) = &model {
        worktrees_core::choice::validate_model(m)?;
    }
    let mut slug = None;
    let mut r = run_op("rules review", &repo, |p, ui| {
        let (rc, s) = worktrees_core::review::open_lane(p, ui, true, Some(&provider), model.as_deref());
        slug = Some(s);
        rc
    })?;
    r.slug = slug;
    Ok(r)
}

/// The To do row's "Remove the place" once the branch's PR is MERGED. Core
/// re-asks GitHub before it deletes anything.
pub fn rules_lane_remove(ctx: &Ctx, repo: String) -> Result<CmdResult, String> {
    let r = run_op("rules lane remove", &repo, |p, ui| worktrees_core::review::retire_merged(p, ui))?;
    ctx.emit("places:changed", serde_json::Value::Null);
    Ok(r)
}

/// The user's own skills alone — machine-level, no repo. What Settings → Codex
/// and the `codex-skills` after-update offer read: taking it from some
/// project's `agent_setup_status` would make a machine-wide suggestion need a
/// project to exist. A directory scan of ~/.claude/skills, cheap enough to
/// probe at startup.
/// Settings → Agent guidance, and the `agent-guidance` offer's input. Machine
/// level: the worktrees settings file and which agents are installed. It never
/// probes Codex (that starts Codex's MCP servers): Codex's line is the answer
/// its last launch acted on. Blocking pool all the same — it materialises
/// files and asks the worktrees CLI whether it has the guard.
pub fn agent_guidance_status() -> Result<worktrees_core::guidance::Status, String> {
    Ok(worktrees_core::guidance::status())
}

/// Write the two toggles to `~/.config/worktrees/agent-guidance.json` — a
/// worktrees-owned file every launcher reads (the CLI and MCP-launched lanes
/// too, which an in-process override would never reach) — and return the
/// status as it now stands.
pub fn set_agent_guidance(enabled: bool, guard: bool) -> Result<worktrees_core::guidance::Status, String> {
    (|| -> Result<_, String> {
        worktrees_core::guidance::save_settings(&worktrees_core::guidance::Settings { enabled, guard })?;
        Ok(worktrees_core::guidance::status())
    })()
    .map_err(|e: String| {
        applog("error", &format!("set_agent_guidance: {e}"));
        e
    })
}

/// Save the user's own text for the worktrees skill (`text`), or go back to the
/// default (`None`). Written to `~/.config/worktrees/guidance/` with a record
/// of the shipped default it was based on — the CLI and MCP-launched lanes read
/// the same file — and the status returned as it now stands. Saving the edit
/// you already have re-bases it on this build's default ("keep mine").
pub fn set_agent_guidance_skill(text: Option<String>) -> Result<worktrees_core::guidance::Status, String> {
    (|| -> Result<_, String> {
        worktrees_core::guidance::save_edit(text.as_deref())?;
        Ok(worktrees_core::guidance::status())
    })()
    .map_err(|e: String| {
        applog("error", &format!("set_agent_guidance_skill: {e}"));
        e
    })
}

/// `old` → `new` as a full-context unified diff, for the agent-guidance compare
/// view (it renders through the Files tab's `DiffView`). Empty = identical.
pub fn agent_guidance_diff(old: String, new: String) -> Result<String, String> {
    worktrees_core::guidance::diff_texts(&old, &new).map_err(|e| {
        applog("error", &format!("agent_guidance_diff: {e}"));
        e
    })
}

/// The user's edited skill merged onto this build's default (`git merge-file`),
/// for the editor to start from. Never saved by itself.
pub fn agent_guidance_merge() -> Result<worktrees_core::guidance::Merged, String> {
    worktrees_core::guidance::merge_edit()
        .map_err(|e| {
            applog("error", &format!("agent_guidance_merge: {e}"));
            e
        })
}

pub fn agent_user_skills() -> Result<Vec<worktrees_core::agentfiles::UserSkill>, String> {
    Ok(worktrees_core::agentfiles::user_skills())
}

pub fn agent_link_skills() -> Result<Vec<String>, String> {
    worktrees_core::agentfiles::link_user_skills().map_err(|e| {
        applog("error", &format!("agent_link_skills: {e}"));
        e
    })
}

/// Settings → Codex → Permissions: HOST policy (`hostpolicy`), persisted in
/// the config dir and then applied in-process so the NEXT Codex launch uses
/// it. Only the Settings click calls this; loading a client never does. An
/// unknown value is refused and nothing is written: a typo must not quietly
/// widen (or narrow) what Codex may do.
pub fn set_codex_permissions(ctx: &Ctx, mode: String) -> Result<(), String> {
    let Some(p) = worktrees_core::codex::Permissions::parse(&mode) else {
        let e = format!("unknown Codex permission mode '{mode}' (expected ask, auto-review or full)");
        applog("error", &format!("set_codex_permissions: {e}"));
        return Err(e);
    };
    hostpolicy::set(&ctx.config_dir, hostpolicy::CODEX_PERMISSIONS, p.as_str())
        .inspect_err(|e| applog("error", &format!("set_codex_permissions: {e}")))?;
    worktrees_core::codex::set_permissions_override(Some(p));
    applog("info", &format!("host-policy: codex_permissions set to {}", p.as_str()));
    Ok(())
}

/// What the next Codex launch on this host will use, for Settings to render:
/// the host's own value, else whatever env or user config decide.
pub fn get_codex_permissions() -> Result<String, String> {
    Ok(worktrees_core::codex::permissions().as_str().to_string())
}

/// What a model picker lists for `harness` (`choice::options_for`): Claude's
/// aliases, nothing for Codex (free text), pi's catalog with each unusable
/// model's reason. pi's shells out and probes its hosts — cached a minute in
/// core, and async here so a slow host never holds the main thread.
pub fn agent_models(harness: String) -> Result<Vec<worktrees_core::choice::ModelOption>, String> {
    known_harness(&harness)?;
    Ok(worktrees_core::choice::options_for(&harness))
}

/// Settings → pi: which pi and node a pane gets, the trust posture, and the
/// user's allowance — plus, for `repo`, its root and whether it is allowed.
#[derive(Serialize)]
pub struct PiStatus {
    pub preflight: worktrees_core::pimodels::Preflight,
    pub trust: worktrees_core::trust::PiTrust,
    pub allowed: Vec<String>,
    pub repo_root: Option<String>,
    pub repo_allowed: bool,
    /// How a pi lane in `repo`'s main checkout would be trusted, and why —
    /// the same answer the launch and `doctor --pi` use.
    pub launch: Option<worktrees_core::trust::PiLaunchTrust>,
    /// pi's own trust.json, which worktrees reads and never writes.
    pub pi_trust_path: String,
}

pub fn pi_status(repo: Option<String>) -> Result<PiStatus, String> {
    let repo_root = repo.as_deref().and_then(worktrees_core::trust::repo_root);
    let allowed = worktrees_core::trust::allowed("pi");
    let repo_allowed = repo_root.as_ref().is_some_and(|r| allowed.contains(r));
    let launch = repo_root.as_deref().map(worktrees_core::trust::pi_launch_trust);
    Ok(PiStatus {
        preflight: worktrees_core::pimodels::preflight(),
        trust: worktrees_core::trust::pi_trust(),
        allowed,
        repo_root,
        repo_allowed,
        launch,
        pi_trust_path: worktrees_core::trust::pi_trust_path().to_string_lossy().into_owned(),
    })
}

/// Settings → Kimi and the new-place dialog: whether kimi is here, how a lane
/// in `repo` would be trusted (the user's allowance — kimi itself always asks
/// otherwise), and whether lanes get the worktrees MCP server. Reads nothing
/// under kimi's home but the `[models.*]` half of its config, and only in
/// `agent_models`.
#[derive(Serialize)]
pub struct KimiStatus {
    pub kimi_bin: Option<String>,
    /// Why kimi's model list is empty, when it is: `no config.toml` or
    /// `config.toml unreadable` (`kimimodels::catalog`); `None` when it read.
    pub models_reason: Option<&'static str>,
    pub mcp: worktrees_core::kimiplace::Status,
    pub allowed: Vec<String>,
    pub repo_root: Option<String>,
    pub repo_allowed: bool,
    pub launch: Option<worktrees_core::trust::KimiLaunchTrust>,
}

pub fn kimi_status(repo: Option<String>) -> Result<KimiStatus, String> {
    let repo_root = repo.as_deref().and_then(worktrees_core::trust::repo_root);
    let allowed = worktrees_core::trust::allowed("kimi");
    let repo_allowed = repo_root.as_ref().is_some_and(|r| allowed.contains(r));
    let launch = repo_root.as_deref().map(worktrees_core::trust::kimi_launch_trust);
    // Cheap on purpose — the new-place dialog asks this for availability on
    // every open and every Start-with flip, so it starts no kimi process (the
    // version is `kimi_version`, which only Settings → Kimi asks for).
    Ok(KimiStatus {
        kimi_bin: worktrees_core::kimi::kimi_bin().map(|p| p.to_string_lossy().into_owned()),
        models_reason: worktrees_core::kimimodels::catalog().err(),
        mcp: worktrees_core::kimiplace::status(),
        allowed,
        repo_root,
        repo_allowed,
        launch,
    })
}

/// Settings → Kimi's version line: `kimi --version`, normalised to the bare
/// version. It starts a kimi process (bounded at 10s), so it is its own
/// command, asked by the Settings panel alone — and blocking, so the app's
/// wrapper puts it on the blocking pool.
pub fn kimi_version() -> Result<String, String> {
    worktrees_core::kimi::version().inspect_err(|e| applog("warn", &format!("kimi_version: {e}")))
}

/// Settings → pi's trust mode: host policy like Codex's permissions, and read
/// back through `pi_status`'s `trust`. An unknown value is refused, never
/// defaulted, and nothing is written.
pub fn set_pi_trust(ctx: &Ctx, mode: String) -> Result<(), String> {
    let Some(t) = worktrees_core::trust::PiTrust::parse(&mode) else {
        let e = format!("unknown pi trust mode '{mode}' (expected never or ask)");
        applog("error", &format!("set_pi_trust: {e}"));
        return Err(e);
    };
    hostpolicy::set(&ctx.config_dir, hostpolicy::PI_PROJECT_TRUST, t.as_str())
        .inspect_err(|e| applog("error", &format!("set_pi_trust: {e}")))?;
    worktrees_core::trust::set_pi_trust_override(Some(t));
    applog("info", &format!("host-policy: pi_project_trust set to {}", t.as_str()));
    Ok(())
}

/// Allow (or revoke) a repo's own pi resources. The user's act, from Settings
/// → pi; the MCP server has no path here. `repo` is any path inside the repo —
/// core keys the allowance on the parent of its git common dir.
pub fn set_pi_allowed(repo: String, allow: bool) -> Result<bool, String> {
    // A revoke takes the LISTED string as it is (Settings' Revoke button sends
    // exactly that), so a repo that has moved or been deleted can still be
    // revoked; a grant resolves the repo's root through git.
    let root = worktrees_core::trust::root_for("pi", &repo, allow).ok_or_else(|| {
        let e = format!("{repo} is not inside a git repository");
        applog("warn", &format!("set_pi_allowed: {e}"));
        e
    })?;
    worktrees_core::trust::set_allowed("pi", &root, allow).inspect_err(|e| applog("error", &format!("set_pi_allowed: {e}")))
}

/// Allow (or revoke) this repo's kimi lanes to start trusted. The user's act,
/// from Settings → Kimi; the MCP server has no path here. Same keying as pi's.
pub fn set_kimi_allowed(repo: String, allow: bool) -> Result<bool, String> {
    let root = worktrees_core::trust::root_for("kimi", &repo, allow).ok_or_else(|| {
        let e = format!("{repo} is not inside a git repository");
        applog("warn", &format!("set_kimi_allowed: {e}"));
        e
    })?;
    worktrees_core::trust::set_allowed("kimi", &root, allow).inspect_err(|e| applog("error", &format!("set_kimi_allowed: {e}")))
}


/// `which <tool>` (resolved path) + first line of `<tool> --version`, each under
/// a short deadline so a wedged tool can't stall the button. Returns the two as
/// display strings ("(not found)" when absent).
pub fn tool_report(tool: &str) -> (String, String) {
    let mut which = worktrees_core::childenv::command("which");
    which.arg(tool);
    let path = match run_deadline(which, 10) {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "(not found)".to_string(),
    };
    let mut ver = worktrees_core::childenv::command(tool);
    ver.arg("--version");
    let version = match run_deadline(ver, 10) {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string()
        }
        _ => "(unknown)".to_string(),
    };
    (path, version)
}

/// Assemble the diagnostics block. `async` (a login-shell CLI probe can take
/// ~5s and two ~10s tool probes — must never run on the main thread).
/// `viewer_path` is resolved by the caller: where a bundled resource lives is
/// a host fact (Tauri's `resource_dir`), and the server's answer is its own.
pub fn diagnostics(_ctx: &Ctx, gh: Option<bool>, viewer_path: String) -> Result<String, String> {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let (cli_path, cli_version) = match cli_binary() {
        Some((p, v)) => (p, v),
        None => ("(not found)".to_string(), "(unknown)".to_string()),
    };
    let path = std::env::var("PATH").unwrap_or_default();
    let (git_path, git_version) = tool_report("git");
    let (tmux_path, tmux_version) = tool_report("tmux");
    // `gh` feeds the Pull requests tab. Version + per-host login and token
    // source, from `gh auth status --json hosts` — never the token. That call
    // validates tokens against the API, so with Pull requests switched off it
    // is not made at all: "off stops every gh call" includes this button.
    let gh_line = if gh == Some(false) {
        "(not checked — Pull requests is off in Settings → Behavior)".to_string()
    } else {
        let gh = worktrees_core::github::gh_bin();
        let mut which = worktrees_core::childenv::command("which");
        which.arg(&gh);
        let at = match run_deadline(which, 10) {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            _ => "(not found)".to_string(),
        };
        format!("{} @ {at}", worktrees_core::github::diag_line(&gh))
    };
    // The documentation viewer's BROWSER BUNDLE, by STAT — never by running
    // anything. "It should be there and it is not" belongs here, on demand,
    // beside the other tools, and NOT on any path the app takes at launch: a
    // probe that decides whether the Docs tab exists would let a missing file
    // change what the app looks like before the user has asked for anything.
    // `tool_report` is the wrong shape for the same reason — it runs
    // `<tool> --version`, and this is a JavaScript file.

    let ai_cmd = worktrees_core::config::resolve_ai_cmd(None);
    let ai_resume_arg = worktrees_core::config::resolve_ai_resume_arg();
    let cfg_path = worktrees_core::config::config_path();
    let cfg_exists = cfg_path.exists();

    let log_tail = {
        let text = std::fs::read_to_string(crate::log::paths().1).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        let start = all.len().saturating_sub(200);
        all[start..].join("\n")
    };

    let block = format!(
        "worktrees diagnostics\n\
         =====================\n\
         app version : {app_version}\n\
         cli version : {cli_version}\n\
         cli path    : {cli_path}\n\
         \n\
         PATH        : {path}\n\
         git         : {git_version} @ {git_path}\n\
         tmux        : {tmux_version} @ {tmux_path}\n\
         gh          : {gh_line}\n\
         docs viewer : {viewer_path}\n\
         \n\
         core config\n\
         -----------\n\
         ai_cmd        : {ai_cmd}\n\
         ai_resume_arg : {ai_resume_arg}\n\
         config file   : {cfg} ({exists})\n\
         \n\
         log (last 200 lines)\n\
         --------------------\n\
         {log_tail}\n",
        cfg = cfg_path.to_string_lossy(),
        exists = if cfg_exists { "exists" } else { "absent" },
    );
    Ok(block)
}

/// The https web home of a project's `origin` remote, or None when there is no
/// origin (or it is a local path / exotic protocol). This is the ONLY thing the
/// backend knows about the remote; which PAGE to open — the repo home, or a
/// branch's tree — is decided in the frontend (`remote.ts::remoteWebUrl`) from
/// the snapshot it already holds (`Place.upstream`), so the rule lives once.
/// The UI opens the result via the opener plugin.
pub fn remote_url(repo: String) -> Result<Option<String>, String> {
    let p = Project::discover(Path::new(&repo)).map_err(|e| e.msg)?;
    let Some(remote) = worktrees_core::git::git_out(&p.main_root, &["remote", "get-url", "origin"])
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    Ok(worktrees_core::github::web_base(&remote))
}


/// What a project's PR fetch last concluded. `snap` survives an offline
/// failure, so a dropped network keeps the last good list ("as of …").
#[derive(Clone)]
pub struct PrCacheEntry {
    /// when this conclusion was reached (ms)
    pub at: u64,
    pub state: &'static str,
    pub host: Option<String>,
    pub web: Option<String>,
    pub push_owner: String,
    pub snap: Option<(u64, worktrees_core::github::Snapshot)>,
    pub message: Option<String>,
    pub viewer: Option<String>,
}

/// The per-project answers and the per-project locks callers queue on. A type
/// rather than two bare statics so a test can own one (`PR_CACHE` is the app's;
/// a process-global asserted by tests is order-dependent — AGENTS.md).
#[derive(Default)]
pub struct PrCache {
    pub entries: Mutex<HashMap<String, PrCacheEntry>>,
    pub locks: Mutex<HashMap<String, std::sync::Arc<Mutex<()>>>>,
}

pub static PR_CACHE: std::sync::LazyLock<PrCache> = std::sync::LazyLock::new(PrCache::default);

impl PrCache {
    /// The answer for `repo`: the cached one when it is at most `max_age_secs`
    /// old OR was concluded after this caller asked (it queued behind an
    /// in-flight fetch — one fetch serves both, even at `max_age_secs == 0`),
    /// else `conclude`'s, which is cached.
    ///
    /// `at` is stamped when the conclusion is REACHED, not when it began: a
    /// start stamp predates a caller that queued mid-fetch, which then fetched
    /// again on its own.
    fn answer(
        &self,
        repo: &str,
        max_age_secs: u64,
        conclude: impl FnOnce(Option<&PrCacheEntry>) -> PrCacheEntry,
    ) -> PrCacheEntry {
        self.answer_with(repo, max_age_secs, now_ms, conclude)
    }

    /// `answer` with the clock injected, so a test can script the stamps a
    /// caller's "asked" and a conclusion's "done" carry instead of racing two
    /// sleeps against the scheduler — which is how the test below failed on a
    /// macOS CI runner while passing 40 of 40 locally.
    fn answer_with(
        &self,
        repo: &str,
        max_age_secs: u64,
        now: impl Fn() -> u64,
        conclude: impl FnOnce(Option<&PrCacheEntry>) -> PrCacheEntry,
    ) -> PrCacheEntry {
        let asked = now();
        let lock = self.locks.lock().unwrap_or_else(|e| e.into_inner()).entry(repo.to_string()).or_default().clone();
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let cached = self.entries.lock().unwrap_or_else(|e| e.into_inner()).get(repo).cloned();
        if let Some(c) = cached.as_ref() {
            if c.at >= asked || asked.saturating_sub(c.at) <= max_age_secs.saturating_mul(1000) {
                return c.clone();
            }
        }
        let started = now();
        let mut e = conclude(cached.as_ref());
        let done = now();
        // a snapshot fetched by THIS conclusion is dated with it; one carried
        // over from before (offline) keeps its own, older, time
        if let Some((t, _)) = e.snap.as_mut() {
            if *t >= started {
                *t = done;
            }
        }
        e.at = done;
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).insert(repo.to_string(), e.clone());
        e
    }
}

#[derive(Serialize)]
pub struct PrsReply {
    /// `ok` / `not_github` / `gh_missing` / `logged_out` / `no_host_token` /
    /// `not_found` / `error`. `not_github` renders NOTHING — no tab, no chip.
    pub state: &'static str,
    pub host: Option<String>,
    pub web: Option<String>,
    pub view: Option<worktrees_core::github::View>,
    /// when the shown list was fetched (ms) — older than `checked_at` when an
    /// offline failure is being papered over
    pub fetched_at: Option<u64>,
    pub checked_at: u64,
    pub stale: bool,
    pub message: Option<String>,
    pub viewer: Option<String>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Reach a conclusion for one project, from scratch. Blocking (git + gh).
pub fn pr_conclude(root: &str, prev: Option<&PrCacheEntry>) -> PrCacheEntry {
    use worktrees_core::github::{self as gh, Auth, FetchError};
    let at = now_ms();
    let blank = |state: &'static str| PrCacheEntry {
        at,
        state,
        host: None,
        web: None,
        push_owner: String::new(),
        snap: None,
        message: None,
        viewer: None,
    };
    let Some(res) = gh::resolve(root) else { return blank("not_github") };
    let bin = gh::gh_bin();
    let host = res.repo.host.clone();
    let mut e = PrCacheEntry {
        host: Some(host.clone()),
        web: Some(res.repo.web()),
        push_owner: res.push_owner.clone(),
        ..blank("ok")
    };
    let auth = gh::probe_auth(&bin, &host);
    if !gh::is_github_host(&host, &auth) {
        return blank("not_github");
    }
    e.state = match auth {
        Auth::Ok => "ok",
        Auth::Missing => "gh_missing",
        Auth::LoggedOut => "logged_out",
        Auth::NoHostToken => "no_host_token",
    };
    if e.state != "ok" {
        return e;
    }
    // a previous good list, for this same repo, survives a network failure
    let prev_snap = prev.and_then(|p| p.snap.clone()).filter(|(_, s)| s.repo == res.repo);
    match gh::fetch(&bin, &res.repo) {
        Ok(snap) => {
            e.viewer = snap.viewer.clone();
            e.snap = Some((at, snap));
        }
        Err(FetchError::Offline { message }) => {
            e.viewer = prev_snap.as_ref().and_then(|(_, s)| s.viewer.clone());
            e.state = if prev_snap.is_some() { "ok" } else { "error" };
            e.snap = prev_snap;
            e.message = Some(message);
        }
        Err(FetchError::NotFound { viewer }) => {
            e.state = "not_found";
            e.viewer = viewer;
        }
        Err(FetchError::Missing) => e.state = "gh_missing",
        Err(FetchError::LoggedOut) => e.state = "logged_out",
        Err(FetchError::Other { message }) => {
            e.state = if prev_snap.is_some() { "ok" } else { "error" };
            e.snap = prev_snap;
            e.message = Some(message);
        }
    }
    // One line per real fetch (at most every ~2 min, visible window, selected
    // project only), so cadence and auth state can be read back from app.log.
    match &e.message {
        Some(m) => applog("warn", &format!("pull requests: {root}: {} — {m}", e.state)),
        None => applog(
            "info",
            &format!(
                "pull requests: {root}: {} ({} open) in {}ms",
                e.state,
                e.snap.as_ref().map(|(_, s)| s.open.len()).unwrap_or(0),
                now_ms().saturating_sub(at)
            ),
        ),
    }
    e
}

/// The project's pull requests, joined to `places` (each place's slug and
/// CURRENT local branch — the frontend holds them, and a branch switch re-maps
/// against the cached list without a fetch). Answers from the cache when it is
/// at most `max_age_secs` old; `0` forces a fetch, which a concurrent caller
/// that queued behind it still shares.
pub fn project_prs(
    repo: String,
    max_age_secs: u64,
    places: Vec<worktrees_core::github::PlaceBranch>,
) -> Result<PrsReply, String> {
    {
        let entry = PR_CACHE.answer(&repo, max_age_secs, |prev| {
            let root = Project::discover(Path::new(&repo)).map(|p| p.main_root).unwrap_or_else(|_| repo.clone());
            pr_conclude(&root, prev)
        });
        let view = entry.snap.as_ref().map(|(_, s)| worktrees_core::github::view(s, &entry.push_owner, &places));
        let fetched_at = entry.snap.as_ref().map(|(t, _)| *t);
        Ok(PrsReply {
            state: entry.state,
            host: entry.host,
            web: entry.web,
            stale: fetched_at.is_some_and(|t| t < entry.at),
            view,
            fetched_at,
            checked_at: entry.at,
            message: entry.message,
            viewer: entry.viewer,
        })
    }
}

/// Open a place in the user's editor (`editor_cmd` from Settings, e.g. `code`).
/// The command is the user's own configured tool — same trust model as ai_cmd.
/// Run through `/bin/sh -c` so quoted/spaced commands work (`open -a "Visual
/// Studio Code"`); the path is passed as a positional arg ($0) so it never needs
/// quoting inside the command string.
pub fn open_editor(path: String, cmd: String) -> Result<(), String> {
    if cmd.trim().is_empty() {
        return Err("no editor configured (Settings → Editor command)".into());
    }
    let line = format!("{cmd} \"$0\"");
    launch_watched(&line, Some(&path), "Editor command", &cmd, LAUNCH_GRACE)
}

/// Installed editors for Settings → Commands' picker (`editors.rs`). A stat
/// per known path, off the async runtime because there are a few dozen.
pub fn detect_editors() -> Result<Vec<editors::Editor>, String> {
    Ok(editors::detect())
}

/// Open a place's tmux session in the user's external terminal app
/// (`terminal_cmd` from Settings, e.g. `ghostty -e tmux attach -t {session}`).
/// Every `{session}` token is replaced with the SINGLE-QUOTED session name —
/// session names carry parens (`<prefix>-(main)`) that break unquoted sh — then
/// the whole command runs via `/bin/sh -c`. Same trust model as editor_cmd.
pub fn open_terminal(cmd: String, session: String) -> Result<(), String> {
    if cmd.trim().is_empty() {
        return Err("no terminal command configured (Settings → Terminal command)".into());
    }
    let quoted = worktrees_core::tmux::sq(&session);
    let line = cmd.replace("{session}", &quoted);
    launch_watched(&line, None, "Terminal command", &cmd, LAUNCH_GRACE)
}

/// How long a launched editor/terminal command has to fail before we stop
/// watching. A GUI launcher (`code`, `cursor`, `open -a …`, `ghostty -e …`)
/// either hands off and exits within a few hundred ms or fails just as fast —
/// `command not found` is immediate. A command that is STILL running at the
/// deadline (`code --wait`, a terminal app that stays in the foreground) is a
/// success by definition: we cannot tell "working" from "slow" past this
/// point, and blocking the click on it would be worse than missing a late
/// failure. So the rule is: a non-zero exit inside the grace is an error the
/// user sees; anything else is fine.
pub const LAUNCH_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Run `line` through `/bin/sh -c` (with `arg0` as `$0` when given) and report
/// a failure that happens within `grace`. The child used to be spawned and
/// reaped blind, so a `code` that was not installed exited 127 and the click
/// did nothing at all. `setting` names the Settings field ("Editor command")
/// and `cmd` is the user's text, so the message says what to change.
pub fn launch_watched(
    line: &str,
    arg0: Option<&str>,
    setting: &str,
    cmd: &str,
    grace: std::time::Duration,
) -> Result<(), String> {
    use std::io::Read;
    use std::process::Stdio;
    let mut c = worktrees_core::childenv::command("/bin/sh");
    c.arg("-c").arg(line);
    if let Some(a) = arg0 {
        c.arg(a);
    }
    let mut child = c
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't launch '{cmd}': {e}"))?;
    // Drain stderr on its own thread for the child's whole life: a launcher
    // that outlives the grace must never block on a full pipe. Only the tail
    // is kept — it is for a one-line error, not a log. The thread lives as long
    // as ANY process holds the write end: a GUI editor forked by the launcher
    // inherits it, so one parked thread per launch can outlast the click by
    // the editor's whole session. It does nothing but block in read(), and
    // ends at EOF when the last holder exits.
    let tail = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    if let Some(mut err) = child.stderr.take() {
        let tail = tail.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut t = tail.lock().unwrap();
                t.extend_from_slice(&buf[..n]);
                let over = t.len().saturating_sub(2048);
                t.drain(..over);
            }
            let _ = done_tx.send(());
        });
    }
    let deadline = std::time::Instant::now() + grace;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            _ => break None,
        }
    };
    let Some(status) = status else {
        // still running (or unknowable): reap in the background — a dropped
        // Child is never waited on (zombie per click)
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    };
    if status.success() {
        return Ok(());
    }
    // A forked grandchild can hold stderr open past sh's exit; take what has
    // arrived rather than waiting for an EOF that may never come.
    let _ = done_rx.recv_timeout(std::time::Duration::from_millis(200));
    let said = String::from_utf8_lossy(&tail.lock().unwrap()).trim().to_string();
    let said = said.lines().last().unwrap_or("").to_string();
    let first = cmd.split_whitespace().next().unwrap_or(cmd);
    Err(match status.code() {
        Some(127) => format!(
            "{setting} `{first}` was not found — {}",
            if setting == "Editor command" {
                "choose an installed editor in Settings → Commands"
            } else {
                "install it, or change it in Settings → Commands"
            }
        ),
        Some(126) => format!(
            "{setting} `{first}` is not executable — change it in Settings → Commands"
        ),
        Some(n) if said.is_empty() => format!("{setting} `{cmd}` failed (exit {n})"),
        Some(n) => format!("{setting} `{cmd}` failed (exit {n}): {said}"),
        None => format!("{setting} `{cmd}` was killed by a signal"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scripted clock: each call returns the next stamp, the last one
    /// repeating. The stamps are the input the rule is about, so they are
    /// chosen rather than raced.
    fn clock(ticks: &'static [u64]) -> impl Fn() -> u64 + Send + Sync {
        let i = std::sync::atomic::AtomicUsize::new(0);
        move || ticks[i.fetch_add(1, std::sync::atomic::Ordering::SeqCst).min(ticks.len() - 1)]
    }

    /// Two callers that both force (`max_age 0`), the second asking while the
    /// first is mid-fetch, spend ONE fetch: the queued one finds a conclusion
    /// reached after it asked. A start-stamped conclusion predates it and the
    /// second fetches again (`e.at = started` makes this red).
    ///
    /// The stamps are scripted, not measured. The first version slept 40ms and
    /// expected that to land inside a 150ms fetch on another thread; a macOS
    /// CI runner spawned the thread late enough that it did not, and the
    /// failure read as the cache fetching twice. The `Barrier` keeps the SHAPE
    /// — the second caller is at the door while the first is concluding — and
    /// the clocks decide the outcome.
    #[test]
    fn a_forced_ask_queued_behind_a_fetch_shares_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};
        let cache = Arc::new(PrCache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let entry = |at| PrCacheEntry {
            at,
            state: "ok",
            host: None,
            web: None,
            push_owner: String::new(),
            snap: None,
            message: None,
            viewer: None,
        };
        // A asks at 1000, starts at 1001, concludes at 2000 — and does not
        // conclude until B is at the door.
        let door = Arc::new(Barrier::new(2));
        let a = {
            let (cache, calls, door) = (cache.clone(), calls.clone(), door.clone());
            std::thread::spawn(move || {
                cache.answer_with("/repo", 0, clock(&[1000, 1001, 2000]), |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    door.wait();
                    entry(0)
                })
            })
        };
        // B asks at 1500: after A began, before A concluded.
        let b = {
            let (cache, calls, door) = (cache.clone(), calls.clone(), door.clone());
            std::thread::spawn(move || {
                door.wait();
                cache.answer_with("/repo", 0, clock(&[1500, 1501, 1502]), |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    entry(0)
                })
            })
        };
        let got_a = a.join().unwrap();
        let got_b = b.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the queued forced ask shares the in-flight fetch");
        assert_eq!(got_a.at, 2000, "a conclusion is stamped when it is REACHED, not when it began");
        assert_eq!(got_b.at, 2000, "the queued caller is handed that conclusion");
        // a forced ask that comes AFTER both does fetch
        cache.answer_with("/repo", 0, clock(&[2001, 2002, 2003]), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            entry(0)
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// A launcher that fails fast is REPORTED. Before: spawned and reaped
    /// blind, so an uninstalled `code` (sh exit 127) made the Editor button do
    /// nothing at all, with no error anywhere.
    #[test]
    fn a_missing_editor_command_is_an_error_naming_it() {
        let cmd = "wt-no-such-editor-xyz";
        let e = super::launch_watched(
            &format!("{cmd} \"$0\""), Some("/tmp/f.txt"), "Editor command", cmd,
            std::time::Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(e.contains("`wt-no-such-editor-xyz` was not found"), "{e}");
        assert!(e.contains("choose an installed editor in Settings → Commands"), "{e}");
    }

    /// Any other quick non-zero exit is an error too, carrying the last line
    /// the command wrote to stderr (`open -a Nope` says why on stderr).
    #[test]
    fn a_quick_failure_carries_its_stderr() {
        let e = super::launch_watched(
            "echo noise >&2; echo 'Unable to find application' >&2; exit 1", None,
            "Terminal command", "fake", std::time::Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(e.contains("failed (exit 1): Unable to find application"), "{e}");
    }

    /// Success, and a command that is still running at the deadline, are both
    /// fine — and the call returns at the deadline, not when the child exits.
    #[test]
    fn a_clean_exit_or_a_long_runner_is_ok() {
        assert!(super::launch_watched("true", Some("x"), "Editor command", "true",
            std::time::Duration::from_secs(5)).is_ok());
        let t = std::time::Instant::now();
        assert!(super::launch_watched("sleep 5", None, "Terminal command", "sleep",
            std::time::Duration::from_millis(200)).is_ok());
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    }

    /// `$0` still carries the path unquoted-safe: a space in it survives.
    #[test]
    fn the_path_reaches_the_command_as_one_argument() {
        let e = super::launch_watched(
            "test \"$0\" = 'a b' && exit 7", Some("a b"), "Editor command", "x",
            std::time::Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(e.contains("exit 7"), "{e}");
    }
}

//! The 150 commands, each with its exposure, in ONE table.
//!
//! A row is `name: Exposure => body`, and the macro makes two things of the
//! list: `TABLE`, the exhaustive `(name, exposure)` slice, and the `match` the
//! socket dispatches on. One list means the table cannot say one thing while
//! the dispatcher does another — a command missing here is a request that
//! fails with `unknown command`, never a silent default. The precedent is
//! `projcfg`'s user-only key list: a missing row is a hard error, not a
//! fallback. `app/scripts/serve-dispatch-check.mjs` holds this list against
//! the shim's `generate_handler!`, so a command added to the app without a
//! row here fails CI rather than failing a phone.
//!
//! Arguments are read by name from the request's `args` object exactly as
//! Tauri reads them: each parameter deserialized on its own, a missing key
//! read as JSON `null` (so an `Option` is `None` and a required `String` is
//! an error), and the key spelled the way the frontend spells it —
//! camelCase, which is Tauri 2's default — or in snake_case, both accepted.
//! Nested types (`UiEvent`, `PlaceBranch`, `ViewerPlace`, `ShellRef`) carry
//! their own serde attributes and are the same types Tauri deserializes from
//! the same JSON, so nothing is re-stated here.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use worktrees_api::{config, files, hostenv, meta, ops, prefs, profiles, project, status, term, viewer, workspace, Ctx};

use super::wire::{self, Frames, CLIENT_LOCAL};

/// What a command is, seen from a socket. The classes are the plan's §III.4
/// (`docs/proposals/server-edition.md`), tagged now so Phase 2 can gate by
/// class rather than by re-reading 150 bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposure {
    /// The point of the server: reads, and mutations scoped to a registered
    /// project or place. Served to any authenticated client.
    Engine,
    /// Acts on the CLIENT's machine — its clipboard, its editor, its
    /// terminal emulator, its browser, its window, its app bundle. Refused
    /// over the socket with `err: "client-local"`, never run on the host:
    /// `copy_text` would write the SERVER's clipboard and `open_editor` would
    /// open a window nobody is sitting at. The shim keeps these.
    ClientLocal,
    /// Changes this MACHINE's user-tier state outside any one project — the
    /// harness configs (`~/.claude.json`, codex, pi), profiles and skills,
    /// the project registry's membership, planning/quota/reach settings,
    /// agent guidance — or installs software, or creates repositories on
    /// disk. Allowed on loopback (a loopback caller is the local user), and
    /// tagged so a remote tier can require more than a token for them.
    HostAdmin,
    /// Owns or drives a byte stream: a tmux attach or a dock shell. Served,
    /// with its bytes going back as channel frames on the same connection.
    Stream,
}

/// What a request is answered with: the engine's `Ctx`, and the connection's
/// frame queue for anything the command streams back.
pub struct Call {
    pub ctx: Ctx,
    pub frames: Frames,
}

/// The request's `args`, read one parameter at a time.
pub struct Args(pub Map<String, Value>);

impl Args {
    /// Look `key` up as written, then as camelCase. Missing is JSON `null`,
    /// which is what Tauri hands a parameter whose key is absent.
    fn raw(&self, key: &str) -> Value {
        if let Some(v) = self.0.get(key) {
            return v.clone();
        }
        let camel = camel(key);
        self.0.get(&camel).cloned().unwrap_or(Value::Null)
    }

    /// One parameter. The error names the key the way Tauri's `invalid args`
    /// does, so a frontend bug reads the same over either transport.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T, String> {
        serde_json::from_value(self.raw(key)).map_err(|e| format!("invalid args `{key}`: {e}"))
    }

    /// A channel parameter, as the frontend's `Channel` serializes it.
    pub fn channel(&self, key: &str) -> Result<u64, String> {
        wire::channel_id(&self.raw(key)).map_err(|e| format!("invalid args `{key}`: {e}"))
    }
}

/// `show_ignored` → `showIgnored`. Only the first form is ever written in the
/// table, so the mapping needs to go one way.
pub fn camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut up = false;
    for c in snake.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// A command's `Result` as the wire's: both halves serialized.
fn reply<T: Serialize, E: Serialize>(r: Result<T, E>) -> Result<Value, Value> {
    match r {
        Ok(t) => serde_json::to_value(t).map_err(|e| Value::String(format!("serialize: {e}"))),
        Err(e) => Err(serde_json::to_value(e).unwrap_or_else(|e| Value::String(format!("serialize error: {e}")))),
    }
}

/// For bodies that return a plain value rather than a `Result`.
fn ok<T>(t: T) -> Result<T, String> {
    Ok(t)
}

/// The body of a `ClientLocal` row. Never evaluated — the exposure check runs
/// first — but a row needs a body to be a row.
fn refused() -> Result<(), String> {
    Err(CLIENT_LOCAL.into())
}

/// What `diagnostics` is told about the viewer bundle. The shim resolves it
/// from Tauri's `resource_dir`; a server has no bundle dir of its own, so only
/// the `WORKTREES_VIEWER_BUNDLE` override can name one.
fn viewer_path() -> String {
    match viewer::bundle_path(None) {
        Some(p) => p.to_string_lossy().into_owned(),
        None => "(no browser bundle — the Docs tab lists and reads without it)".to_string(),
    }
}

macro_rules! commands {
    ($cx:ident, $a:ident; $( $name:ident : $exp:ident => $body:expr ),* $(,)?) => {
        /// Every command the shim registers, with its exposure. Exhaustive by
        /// construction: the dispatcher below is generated from the same list.
        pub const TABLE: &[(&str, Exposure)] = &[ $( (stringify!($name), Exposure::$exp) ),* ];

        /// `None` = not a command. `Some(Err("client-local"))` for a
        /// `ClientLocal` row, before its body is so much as looked at.
        fn run(name: &str, $cx: &Call, $a: &Args) -> Option<Result<Value, Value>> {
            match name {
                $( stringify!($name) => Some(match Exposure::$exp {
                    Exposure::ClientLocal => Err(Value::from(CLIENT_LOCAL)),
                    _ => (|| -> Result<Value, Value> { reply($body) })(),
                }), )*
                _ => None,
            }
        }
    };
}

/// A command only test builds have, whose body panics with its `why`: the
/// seam for the tests of what a panic is answered with (`conn::answer`).
/// Never in `TABLE`, so nothing built for use can reach it.
#[cfg(test)]
pub const TEST_PANIC: &str = "__test_panic";

/// Dispatch one request. `Err` is the wire's `err` value.
pub fn call(cx: &Call, cmd: &str, args: &Args) -> Result<Value, Value> {
    #[cfg(test)]
    if cmd == TEST_PANIC {
        panic!("{}", args.raw("why").as_str().unwrap_or("a test body panicked"));
    }
    match run(cmd, cx, args) {
        Some(r) => r,
        None => Err(Value::from(format!("unknown command: {cmd}"))),
    }
}

pub fn exposure(cmd: &str) -> Option<Exposure> {
    TABLE.iter().find(|(n, _)| *n == cmd).map(|(_, e)| *e)
}

/// The lane a request runs on, or `None` for the concurrent pool.
///
/// Requests to ONE stream must reach it in the order they were sent — a
/// keystroke behind a keystroke, a resize behind the bytes typed before it —
/// and a dispatcher that gives every frame its own blocking task does not
/// keep that (measured: 26 in-order `shell_write`s reaching zsh as
/// `0124356789…`). So every `Stream` row that names an existing stream is
/// keyed by that stream: a tmux attach by its `id`, a dock shell by
/// `(repo, slug, index)`; the connection runs one lane per key, serially, and
/// everything else stays concurrent, so a long clone still cannot hold up a
/// keystroke. `term_open` has no id yet and needs no order: nothing can write
/// to an attach before the reply names it. The key is built from the RAW
/// args, so a malformed request lands on a lane and fails IN ORDER with its
/// proper error instead of racing past the requests around it; two keys
/// that happened to collide would only share a lane, never lose order.
pub fn lane(cmd: &str, args: &Args) -> Option<String> {
    // The panicking test body runs on the lane of the attach its `id` names,
    // when it names one, so a test can queue a request behind it.
    #[cfg(test)]
    if cmd == TEST_PANIC {
        let id = args.raw("id");
        return (!id.is_null()).then(|| format!("term:{id}"));
    }
    if exposure(cmd)? != Exposure::Stream {
        return None;
    }
    if let Some(_) = cmd.strip_prefix("term_") {
        if cmd == "term_open" {
            return None;
        }
        return Some(format!("term:{}", args.raw("id")));
    }
    if cmd.starts_with("shell_") {
        return Some(format!("shell:{}:{}:{}", args.raw("repo"), args.raw("slug"), args.raw("index")));
    }
    None
}

commands! { cx, a;
    // ── projects and their snapshots (workspace) ─────────────────────────
    list_places: Engine => workspace::list_places(&cx.ctx, a.get("repo")?),
    list_workspace: Engine => workspace::list_workspace(&cx.ctx),
    add_project: HostAdmin => workspace::add_project(&cx.ctx, a.get("dir")?),
    probe_dir: Engine => workspace::probe_dir(a.get("dir")?),
    create_project: HostAdmin => workspace::create_project(&cx.ctx, a.get("location")?, a.get("name")?),
    clone_project: HostAdmin => {
        let mut sink = cx.frames.typed::<worktrees_core::clone::CloneProgress>(a.channel("on_progress")?);
        workspace::clone_project(&cx.ctx, a.get("id")?, a.get("url")?, a.get("parent")?, a.get("name")?, &mut sink)
    },
    clone_cancel: HostAdmin => workspace::clone_cancel(a.get("id")?),
    init_repo: HostAdmin => workspace::init_repo(&cx.ctx, a.get("dir")?),
    create_initial_commit: HostAdmin => workspace::create_initial_commit(&cx.ctx, a.get("repo")?),
    remove_project: HostAdmin => workspace::remove_project(&cx.ctx, a.get("root")?),
    reorder_projects: Engine => workspace::reorder_projects(&cx.ctx, a.get("roots")?),

    // ── settings, planning, and the mutating verbs (ops) ─────────────────
    settings_info: Engine => ops::settings_info(&cx.ctx),
    get_settings: Engine => ops::get_settings(&cx.ctx),
    set_settings: Engine => ops::set_settings(&cx.ctx, a.get("settings")?),
    set_lifecycle: Engine => ops::set_lifecycle(a.get("repo")?, a.get("slug")?, a.get("label")?),
    set_pin: Engine => ops::set_pin(a.get("repo")?, a.get("slug")?, a.get("on")?),
    set_note: Engine => ops::set_note(a.get("repo")?, a.get("slug")?, a.get("note")?),
    drop_reference: Engine => ops::drop_reference(
        a.get("repo")?, a.get("slug")?, a.get("into_repo")?, a.get("into_slug")?, a.get("into_session")?, a.get("provider")?,
    ),
    cross_project_status: Engine => ops::cross_project_status(),
    set_cross_project: HostAdmin => ops::set_cross_project(a.get("level")?),
    planning_status: Engine => ops::planning_status(),
    set_planning_default: HostAdmin => ops::set_planning_default(a.get("default")?),
    set_project_planning: HostAdmin => ops::set_project_planning(a.get("root")?, a.get("level")?, a.get("plan_path")?, a.get("scope")?),
    planning_suggested: Engine => ops::planning_suggested(a.get("root")?),
    quota_settings: Engine => ops::quota_settings(),
    set_quota_settings: HostAdmin => ops::set_quota_settings(a.get("gate")?, a.get("weekly_warn_pct")?),
    set_project_private: HostAdmin => ops::set_project_private(a.get("root")?, a.get("private")?),
    copy_text: ClientLocal => refused(),
    set_title: Engine => ops::set_title(a.get("repo")?, a.get("slug")?, a.get("title")?),
    touch_place: Engine => ops::touch_place(a.get("repo")?, a.get("slug")?),
    mark_seen: Engine => ops::mark_seen(a.get("repo")?, a.get("slug")?, a.get("epoch")?),
    new_place: Engine => ops::new_place(
        a.get("repo")?, a.get("branch")?, a.get("base")?, a.get("name")?, a.get("provider")?, a.get("model")?, a.get("force")?,
    ),
    switch_place: Engine => ops::switch_place(a.get("repo")?, a.get("slug")?, a.get("branch")?, a.get("base")?),
    list_branches: Engine => ops::list_branches(a.get("repo")?, a.get("slug")?),
    open_place: Engine => ops::open_place(
        a.get("repo")?, a.get("slug")?, a.get("fresh")?, a.get("provider")?, a.get("model")?, a.get("force")?,
    ),
    set_fetch_interval: Engine => ops::set_fetch_interval(a.get("mins")?),
    fetch_origin: Engine => ops::fetch_origin(a.get("root")?),
    list_drafts: Engine => ops::list_drafts(),
    close_place: Engine => ops::close_place(
        a.get("repo")?, a.get("slug")?, a.get("yes")?, a.get("session")?, a.get("provider")?, &cx.ctx.shells,
    ),
    remove_place: Engine => prefs::remove_place(
        &cx.ctx, a.get("repo")?, a.get("slug")?, a.get("del_branch")?, a.get("force")?, &cx.ctx.shells,
    ),

    // ── usage, service status, releases (status) ─────────────────────────
    claude_usage: Engine => status::claude_usage(),
    codex_usage: Engine => status::codex_usage(),
    // Spawns `kimi web`, asks once, ends it; every row already runs on the
    // blocking pool, which is what the shim's spawn_blocking buys it.
    kimi_usage: Engine => status::kimi_usage(),
    claude_status: Engine => status::claude_status(),
    check_update: Engine => status::check_update(),
    update_cli: HostAdmin => status::update_cli(a.get("tag")?),
    list_releases: Engine => status::list_releases(),
    install_app_version: ClientLocal => refused(),

    // ── config surfaces, MCP setup, Ask Claude (config) ──────────────────
    get_ai_config: Engine => config::get_ai_config(),
    mcp_status: HostAdmin => config::mcp_status(a.get("repo")?),
    mcp_install: HostAdmin => config::mcp_install(a.get("repo")?, a.get("mutations")?),
    mcp_uninstall: HostAdmin => config::mcp_uninstall(a.get("repo")?),
    codex_mcp_status: HostAdmin => config::codex_mcp_status(),
    codex_mcp_install: HostAdmin => config::codex_mcp_install(a.get("mutations")?),
    codex_mcp_uninstall: HostAdmin => config::codex_mcp_uninstall(),
    codex_mcp_migration_plan: HostAdmin => config::codex_mcp_migration_plan(),
    pi_mcp_status: HostAdmin => config::pi_mcp_status(),
    pi_mcp_install: HostAdmin => config::pi_mcp_install(a.get("mutations")?),
    pi_mcp_uninstall: HostAdmin => config::pi_mcp_uninstall(),
    kimi_mcp_status: HostAdmin => config::kimi_mcp_status(),
    kimi_mcp_set: HostAdmin => config::kimi_mcp_set(a.get("mode")?),
    codex_mcp_migrate: HostAdmin => config::codex_mcp_migrate(a.get("names")?),
    project_config_read: Engine => config::project_config_read(a.get("repo")?),
    doctor: Engine => config::doctor(a.get("repo")?, a.get("slug")?),
    place_health: Engine => config::place_health(a.get("repo")?, a.get("slug")?),
    ai_status_report: Engine => config::ai_status_report(a.get("repo")?, a.get("slug")?),
    relink: Engine => config::relink(a.get("repo")?, a.get("slug")?, a.get("force")?),
    provision: Engine => config::provision(a.get("repo")?, a.get("slug")?),

    // ── sync, agent setup, diagnostics, pull requests (project) ──────────
    sync_status: Engine => project::sync_status(a.get("repo")?),
    sync_hub_list: Engine => project::sync_hub_list(),
    sync_preview: Engine => project::sync_preview(a.get("repo")?, a.get("name")?, a.get("direction")?, a.get("with_sessions")?),
    sync_apply: HostAdmin => {
        let mut sink = cx.frames.typed::<worktrees_core::sync::SyncProgress>(a.channel("on_progress")?);
        project::sync_apply(
            &cx.ctx, a.get("repo")?, a.get("name")?, a.get("direction")?, a.get("with_sessions")?,
            a.get("confirmed")?, a.get("install")?, &mut sink,
        )
    },
    init_suggest: Engine => project::init_suggest(a.get("repo")?),
    init_write: Engine => project::init_write(a.get("repo")?),
    agent_setup_status: Engine => project::agent_setup_status(a.get("repo")?),
    agent_setup_fix: Engine => project::agent_setup_fix(&cx.ctx, a.get("repo")?, a.get("push")?),
    rules_apply: Engine => project::rules_apply(&cx.ctx, a.get("repo")?, a.get("push")?, a.get("force")?),
    rules_review: Engine => project::rules_review(a.get("repo")?, a.get("provider")?, a.get("model")?),
    rules_lane_remove: Engine => project::rules_lane_remove(&cx.ctx, a.get("repo")?),
    agent_link_skills: HostAdmin => project::agent_link_skills(),
    agent_user_skills: Engine => project::agent_user_skills(),
    // Host policy (`worktrees_api::hostpolicy`): written only by this explicit
    // act, read by the engine; the getter is a read like any other.
    set_codex_permissions: HostAdmin => project::set_codex_permissions(&cx.ctx, a.get("mode")?),
    get_codex_permissions: Engine => project::get_codex_permissions(),
    agent_guidance_status: Engine => project::agent_guidance_status(),
    set_agent_guidance: HostAdmin => project::set_agent_guidance(a.get("enabled")?, a.get("guard")?),
    set_agent_guidance_skill: HostAdmin => project::set_agent_guidance_skill(a.get("text")?),
    agent_guidance_diff: Engine => project::agent_guidance_diff(a.get("old")?, a.get("new")?),
    agent_guidance_merge: HostAdmin => project::agent_guidance_merge(),
    agent_models: Engine => project::agent_models(a.get("harness")?),
    pi_status: Engine => project::pi_status(a.get("repo")?),
    kimi_status: Engine => project::kimi_status(a.get("repo")?),
    // Starts a kimi process for its --version; a read.
    kimi_version: Engine => project::kimi_version(),
    set_pi_trust: HostAdmin => project::set_pi_trust(&cx.ctx, a.get("mode")?),
    set_pi_allowed: HostAdmin => project::set_pi_allowed(a.get("repo")?, a.get("allow")?),
    set_kimi_allowed: HostAdmin => project::set_kimi_allowed(a.get("repo")?, a.get("allow")?),
    remote_url: Engine => project::remote_url(a.get("repo")?),
    project_prs: Engine => project::project_prs(a.get("repo")?, a.get("max_age_secs")?, a.get("places")?),
    open_editor: ClientLocal => refused(),
    detect_editors: ClientLocal => refused(),
    open_terminal: ClientLocal => refused(),
    diagnostics: Engine => project::diagnostics(&cx.ctx, a.get("gh")?, viewer_path()),
    tmux_check: Engine => hostenv::tmux_check(a.get("refresh")?),
    set_zoom: ClientLocal => refused(),

    // ── the app's own metadata (meta) ────────────────────────────────────
    log_info: Engine => ok(meta::log_info()),
    log_event: Engine => { meta::log_event(&a.get::<String>("level")?, a.get("msg")?); ok(()) },
    log_tail: Engine => ok(meta::log_tail(a.get("lines")?)),
    get_changelog: Engine => ok(meta::get_changelog()),

    // ── the Files tab: tree, diffs, reads and writes (files) ─────────────
    list_dir: Engine => files::list_dir(&cx.ctx, a.get("path")?, a.get("show_ignored")?),
    changed_files: Engine => files::changed_files(&cx.ctx, a.get("root")?),
    file_diff: Engine => files::file_diff(&cx.ctx, a.get("path")?, a.get("against")?),
    read_file: Engine => files::read_file(&cx.ctx, a.get("path")?, a.get("max_bytes")?),
    file_readable: Engine => files::file_readable(&cx.ctx, a.get("path")?),
    resolve_term_paths: Engine => files::resolve_term_paths(
        &cx.ctx, &cx.ctx.shells, a.get("root")?, a.get("shell")?, a.get("session")?, a.get("paths")?,
    ),
    resolve_doc_paths: Engine => files::resolve_doc_paths(
        &cx.ctx, &cx.ctx.docnames, a.get("root")?, a.get("doc")?, a.get("generation")?, a.get("paths")?,
    ),
    list_docs: Engine => files::list_docs(&cx.ctx, a.get("repo")?, a.get("root")?),
    place_plan: Engine => files::place_plan(&cx.ctx, a.get("root")?),
    plan_prompt: Engine => files::plan_prompt(a.get("session")?, a.get("provider")?),
    plan_ask_main: Engine => files::plan_ask_main(&cx.ctx, a.get("root")?, a.get("session")?, a.get("provider")?),
    open_docs_viewer: ClientLocal => refused(),
    read_file_base64: Engine => files::read_file_base64(&cx.ctx, a.get("path")?, a.get("max_bytes")?),
    write_file: Engine => files::write_file(&cx.ctx, a.get("path")?, a.get("content")?, a.get("expected_mtime")?),

    // ── the dock: shells, scrollback, and the tmux attach (term) ─────────
    list_shell_sessions: Engine => term::list_shell_sessions(a.get("repo")?, a.get("slug")?, &cx.ctx.shells),
    close_shell_session: Engine => term::close_shell_session(
        &cx.ctx, a.get("repo")?, a.get("slug")?, a.get("index")?, a.get("keep_cwd")?, &cx.ctx.shells,
    ),
    shell_open: Stream => term::shell_open(
        &cx.ctx, a.get("repo")?, a.get("slug")?, a.get("index")?, a.get("cols")?, a.get("rows")?,
        cx.frames.bytes(a.channel("on_bytes")?), &cx.ctx.shells,
    ),
    shell_write: Stream => term::shell_write(a.get("repo")?, a.get("slug")?, a.get("index")?, a.get("data")?, &cx.ctx.shells),
    shell_resize: Stream => term::shell_resize(
        a.get("repo")?, a.get("slug")?, a.get("index")?, a.get("cols")?, a.get("rows")?, &cx.ctx.shells,
    ),
    shell_detach: Stream => term::shell_detach(a.get("repo")?, a.get("slug")?, a.get("index")?, a.get("gen")?, &cx.ctx.shells),
    term_open: Stream => {
        let id = term::term_open(
            a.get("session")?, a.get("cols")?, a.get("rows")?, cx.frames.bytes(a.channel("on_bytes")?), &cx.ctx.terminals,
        );
        // Remembered per connection, so a client that vanishes takes its
        // attaches with it instead of leaving a tmux client on every session
        // it ever looked at.
        if let Ok(id) = &id {
            cx.frames.note_term(*id);
        }
        id
    },
    term_write: Stream => term::term_write(a.get("id")?, a.get("data")?, &cx.ctx.terminals),
    term_wheel: Stream => term::term_wheel(a.get("id")?, a.get("lines")?, &cx.ctx.terminals),
    term_resize: Stream => term::term_resize(a.get("id")?, a.get("cols")?, a.get("rows")?, &cx.ctx.terminals),
    term_close: Stream => {
        let id: u32 = a.get("id")?;
        cx.frames.forget_term(id);
        term::term_close(id, &cx.ctx.terminals)
    },

    // ── backend preferences, the Data pane, local usage metrics (prefs) ──
    set_term_history_opts: Engine => prefs::set_term_history_opts(a.get("persist_scrollback")?, a.get("per_tab_history")?),
    term_history_info: Engine => prefs::term_history_info(&cx.ctx),
    term_history_clear: Engine => prefs::term_history_clear(&cx.ctx, &cx.ctx.shells),
    ui_events_append: Engine => prefs::ui_events_append(&cx.ctx, a.get("events")?),
    ui_usage: Engine => prefs::ui_usage(&cx.ctx, a.get("days")?, a.get("tz_offset_min")?),
    ui_events_clear: Engine => prefs::ui_events_clear(&cx.ctx),

    // ── AI profiles + the skill store (profiles) ─────────────────────────
    profiles_info: Engine => profiles::profiles_info(a.get("repo")?),
    save_profile: HostAdmin => profiles::save_profile(a.get("profile")?),
    new_profile_id: Engine => profiles::new_profile_id(a.get("name")?),
    delete_profile: HostAdmin => profiles::delete_profile(a.get("id")?),
    set_project_profile: HostAdmin => profiles::set_project_profile(a.get("repo")?, a.get("id")?),
    set_default_profile: HostAdmin => profiles::set_default_profile(a.get("id")?),
    skills_list: Engine => profiles::skills_list(),
    skill_inspect: Engine => profiles::skill_inspect(a.get("path")?),
    skill_install_local: HostAdmin => profiles::skill_install_local(a.get("path")?),
    skill_preview_git: Engine => profiles::skill_preview_git(a.get("url")?, a.get("rev")?),
    skill_install_git: HostAdmin => profiles::skill_install_git(a.get("url")?, a.get("rev")?, a.get("sha")?, a.get("name")?),
    skill_remove: HostAdmin => profiles::skill_remove(a.get("name")?),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;
    use std::sync::Arc;

    /// A `Call` over a temp config dir with a queue nobody drains — enough to
    /// dispatch the commands that touch only the config dir.
    fn call(tag: &str) -> (Call, std::path::PathBuf, tokio::sync::mpsc::UnboundedReceiver<tokio_tungstenite::tungstenite::Message>) {
        let dir = std::env::temp_dir().join(format!("wt-dispatch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        struct Quiet;
        impl worktrees_api::Host for Quiet {
            fn emit(&self, _: &str, _: Value) {}
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(dir.clone(), Arc::new(Quiet));
        (Call { ctx, frames: Frames::new(wire::Outbox(tx)) }, dir, rx)
    }

    fn args(v: Value) -> Args {
        Args(v.as_object().cloned().unwrap_or_default())
    }

    /// 150 rows, no duplicates: the number the shim's handler list has
    /// (143 at the extraction; host policy's getter and Kimi's six since),
    /// and the check script holds the names against it.
    #[test]
    fn the_table_has_every_command_once() {
        let names: HashSet<&str> = TABLE.iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), TABLE.len(), "a command is listed twice");
        assert_eq!(TABLE.len(), 150, "the shim registers 150 commands");
    }

    /// The client-local set, spelled out: these must never run on the host.
    #[test]
    fn client_local_commands_are_refused_without_running() {
        let (c, dir, _rx) = call("local");
        let want: HashSet<&str> = ["copy_text", "open_editor", "open_terminal", "detect_editors", "open_docs_viewer", "set_zoom", "install_app_version"].into();
        let tagged: HashSet<&str> = TABLE.iter().filter(|(_, e)| *e == Exposure::ClientLocal).map(|(n, _)| *n).collect();
        assert_eq!(tagged, want);
        for n in &want {
            // Args that would make the body misbehave if it ran: copy_text's
            // text would land on this machine's clipboard.
            let r = super::call(&c, n, &args(json!({"text": "NEVER", "path": "/", "cmd": "x", "session": "s", "factor": 1.0, "tag": "v0"})));
            assert_eq!(r, Err(Value::from(CLIENT_LOCAL)), "{n}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_command_is_an_error_not_a_default() {
        let (c, dir, _rx) = call("unknown");
        assert_eq!(super::call(&c, "not_a_command", &args(json!({}))), Err(Value::from("unknown command: not_a_command")));
        assert!(exposure("not_a_command").is_none());
        assert_eq!(exposure("term_open"), Some(Exposure::Stream));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Args by name, both spellings, missing = null: a settings round trip
    /// through the dispatcher with the keys the frontend actually sends.
    #[test]
    fn args_are_read_by_key_in_either_spelling_and_missing_is_null() {
        let (c, dir, _rx) = call("args");
        assert_eq!(super::call(&c, "get_settings", &args(json!({}))).unwrap(), Value::Null, "nothing stored yet → None → null");
        super::call(&c, "set_settings", &args(json!({"settings": {"theme": "nord"}}))).unwrap();
        assert_eq!(super::call(&c, "get_settings", &args(json!({}))).unwrap()["theme"], json!("nord"));
        // camelCase as the frontend spells it, snake_case as the table does.
        let a1 = args(json!({"days": 7, "tzOffsetMin": 0}));
        let a2 = args(json!({"days": 7, "tz_offset_min": 0}));
        assert!(super::call(&c, "ui_usage", &a1).is_ok());
        assert!(super::call(&c, "ui_usage", &a2).is_ok());
        // A required key missing is Tauri's `invalid args`, naming the key.
        let e = super::call(&c, "ui_usage", &args(json!({"days": 7}))).unwrap_err();
        assert!(e.as_str().unwrap().starts_with("invalid args `tz_offset_min`"), "{e}");
        assert_eq!(camel("tz_offset_min"), "tzOffsetMin");
        assert_eq!(camel("repo"), "repo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `clone_project`'s error is the struct its dialog branches on, not a
    /// string — `err` carries what the Tauri `Err` serializes to.
    #[test]
    fn a_structured_error_crosses_the_wire_as_its_object() {
        let (c, dir, _rx) = call("cloneerr");
        let e = super::call(&c, "clone_project", &args(json!({
            "id": "j1", "url": "not a url at all", "parent": dir.to_string_lossy(), "name": null, "onProgress": "__CHANNEL__:1"
        })))
        .unwrap_err();
        assert!(e.is_object(), "{e}");
        assert!(e.get("kind").is_some(), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream command needs its channel; without one it is an args error
    /// naming the key, before anything is opened.
    #[test]
    fn a_stream_command_without_a_channel_is_an_args_error() {
        let (c, dir, _rx) = call("chan");
        let e = super::call(&c, "term_open", &args(json!({"session": "nope", "cols": 80, "rows": 24}))).unwrap_err();
        assert!(e.as_str().unwrap().starts_with("invalid args `on_bytes`"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Which requests share a lane: the writes, resizes, wheel, close and
    /// detach of one stream, by that stream; opens of attaches and everything
    /// that is not a stream run concurrently.
    #[test]
    fn stream_requests_are_keyed_by_their_stream_and_nothing_else_is() {
        let t = args(json!({ "id": 4, "data": [97] }));
        assert_eq!(lane("term_write", &t).as_deref(), Some("term:4"));
        assert_eq!(lane("term_resize", &args(json!({ "id": 4, "cols": 1, "rows": 1 }))).as_deref(), Some("term:4"));
        assert_eq!(lane("term_wheel", &t), lane("term_close", &t));
        assert_ne!(lane("term_write", &t), lane("term_write", &args(json!({ "id": 5 }))));
        assert_eq!(lane("term_open", &args(json!({ "session": "s" }))), None);
        let sh = args(json!({ "repo": "/r", "slug": "s", "index": 2, "data": [97] }));
        assert_eq!(lane("shell_write", &sh).as_deref(), Some("shell:\"/r\":\"s\":2"));
        assert_eq!(lane("shell_open", &sh), lane("shell_detach", &sh));
        assert_eq!(lane("shell_resize", &sh), lane("shell_write", &sh));
        assert_ne!(lane("shell_write", &sh), lane("shell_write", &args(json!({ "repo": "/r", "slug": "s", "index": 3 }))));
        assert_eq!(lane("list_workspace", &args(json!({}))), None);
        assert_eq!(lane("clone_project", &args(json!({}))), None);
        assert_eq!(lane("no_such", &args(json!({}))), None);
    }
}

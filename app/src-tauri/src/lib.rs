// worktrees UI — the Tauri shim. A CLIENT of this host's engine, not an engine:
// the nav-dot poll, the dock shells, the docs server and every writer of the
// engine's files run in `worktrees serve` (one engine per host — decision 2 of
// docs/proposals/server-edition.md, and `worktrees_api`'s crate doc). The
// frontend reaches it over an authenticated loopback WebSocket (app/src/ipc.ts,
// app/src/ipcws.ts). What is left here is what only the machine the window runs
// on can do:
//   1. the window — page zoom, window state, the Writing Tools fix, raising it
//      when a session asks to show a document;
//   2. the client-local commands — serve's `ClientLocal` rows: the clipboard,
//      the editor and terminal launchers, the app's own updater;
//   3. finding or starting `worktrees serve` (engine.rs) and handing the page
//      the port and token to reach it — the one Tauri command that answers
//      with the token.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use worktrees_api::ops::REPO_SLUG;
use worktrees_api::status::{list_published_releases, valid_tag};
use worktrees_api::util::fmt_utc;
use worktrees_core::sysclock;

mod engine;
// This window driving ANOTHER host's serve: pairing, the hosts kept, and the
// pinned pipe the page's protocol runs over (Phase 2 §4.4).
mod remote;
// Window state: where the window was, restored on launch. Its own file because
// it is the one part of this shim with real logic of its own.
mod winstate;

// ── app log ──────────────────────────────────────────────────────────────────
// The sink behind `worktrees_api::log`: a plain append-only file at the
// platform's log location (macOS: ~/Library/Logs/<identifier>/app.log —
// Console.app finds it). Deliberately AppHandle-free so the panic hook can use
// it. Since the app became serve's client, only the shim's own lines and
// panics land here (startup, `engine:`, the window); command failures and the
// page's `log_event` lines go to serve.log, which is what Settings → Logs shows.

/// The identifier to log under when nothing has said otherwise — what a
/// `tauri build` bundle carries.
const APP_IDENT: &str = worktrees_api::hostenv::APP_IDENT;

/// The identifier this PROCESS is actually running as, which is not always
/// `APP_IDENT`: `sandbox.sh --app` passes
/// `--config '{"identifier":"net.casadelvalle.worktrees.sbx"}'`, and while the
/// log read a `const` instead, every sandbox wrote into the REAL app's log —
/// the gap AGENTS.md records under `sandbox.sh`. Resolved once at startup from
/// the live config. A `OnceLock` and not a parameter because the panic hook
/// has no handle to pass one.
static IDENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn ident() -> &'static str {
    IDENT.get().map(String::as_str).unwrap_or(APP_IDENT)
}

fn log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Logs").join(ident())
    } else {
        // XDG-ish fallback (linux dev)
        PathBuf::from(home).join(".local/share").join(ident()).join("logs")
    }
}

fn log_file() -> PathBuf {
    log_dir().join("app.log")
}

/// The app's log sink. `worktrees_api::log::install` takes one of these at
/// startup, and from then on every `crate::log::log` in the command layer —
/// and `applog` below — lands in this file.
struct AppLog;

impl worktrees_api::log::LogSink for AppLog {
    fn log(&self, level: &str, msg: &str) {
        append_line(&log_dir(), &log_file(), level, msg);
    }

    fn location(&self) -> Option<(PathBuf, PathBuf)> {
        Some((log_dir(), log_file()))
    }
}

/// One line of app.log, appended to `path` (rotating it into `dir` past ~1MB).
///
/// Owner-only, the same three ways as serve.log: 0600 at creation, an existing
/// file tightened on every write (a log from before this release is 0644), and
/// the rotated copy tightened after the `rename`, which keeps whatever mode the
/// file had.
fn append_line(dir: &std::path::Path, path: &std::path::Path, level: &str, msg: &str) {
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::create_dir_all(dir);
    // single-slot rotation at ~1MB so the log can't grow unbounded
    if std::fs::metadata(path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let rotated = dir.join("app.log.1");
        if std::fs::rename(path, &rotated).is_ok() {
            if let Ok(f) = std::fs::File::open(&rotated) {
                worktrees_api::log::owner_only(&f);
            }
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path) {
        worktrees_api::log::owner_only(&f);
        let ts = fmt_utc(sysclock::now_epoch());
        // ONE write per line: `writeln!` on an unbuffered `File` is a write
        // per piece of the format, and concurrent writers interleave pieces
        // (serve.log's ecf6851, the same fix).
        let _ = f.write_all(format!("{ts}Z [{level}] {msg}\n").as_bytes());
    }
}

/// Unchanged for its ~150 call sites, and now one line: the log moved to
/// `worktrees_api::log` so that handlers extracted out of this file keep
/// logging without carrying a handle for it.
fn applog(level: &str, msg: &str) {
    worktrees_api::log::log(level, msg);
}

// ── the client-local commands ────────────────────────────────────────────────
// serve's `ClientLocal` rows: each acts on the machine the window runs on, so
// serve refuses it and the page routes it here (app/src/ipc.ts).

#[tauri::command]
async fn copy_text(text: String) -> Result<(), String> {
    worktrees_api::ops::copy_text(text)
}

#[tauri::command]
async fn open_editor(path: String, cmd: String) -> Result<(), String> {
    worktrees_api::project::open_editor(path, cmd)
}

#[tauri::command]
async fn detect_editors() -> Result<Vec<worktrees_api::editors::Editor>, String> {
    worktrees_api::project::detect_editors()
}

#[tauri::command]
async fn open_terminal(cmd: String, session: String) -> Result<(), String> {
    worktrees_api::project::open_terminal(cmd, session)
}

// Stays here: it drives `tauri_plugin_updater` against one release's own
// manifest — the app replacing its own bundle, which is client-local and
// means nothing to a server whose clients are browsers.

/// Install the signed app bundle of exactly `tag` — up or down. The updater is
/// pointed at that release's own manifest and its comparator accepts only that
/// version, so the signature check is the stock one and nothing else can land.
/// The caller relaunches.
#[tauri::command]
async fn install_app_version(app: AppHandle, tag: String) -> Result<String, String> {
    use tauri_plugin_updater::UpdaterExt;
    if !valid_tag(&tag) {
        return Err(format!("suspicious tag '{tag}'"));
    }
    let want = tag[1..].to_string();
    if want == env!("CARGO_PKG_VERSION") {
        return Err(format!("already running {tag}"));
    }
    if !list_published_releases()?.iter().any(|r| r.tag == tag) {
        return Err(format!("{tag} is not a published release"));
    }
    let url = tauri::Url::parse(&format!("https://github.com/{REPO_SLUG}/releases/download/{tag}/latest.json"))
        .map_err(|e| e.to_string())?;
    let cmp = want.clone();
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .version_comparator(move |_current, remote| remote.version.to_string() == cmp)
        .build()
        .map_err(|e| e.to_string())?;
    let up = updater
        .check()
        .await
        .map_err(|e| format!("{tag}: {e}"))?
        .ok_or_else(|| format!("{tag}'s manifest does not describe version {want}"))?;
    applog("info", &format!("install_app_version: {} → {tag}", env!("CARGO_PKG_VERSION")));
    up.download_and_install(|_, _| {}, || {}).await.map_err(|e| {
        applog("error", &format!("install_app_version {tag}: {e}"));
        format!("{tag}: {e}")
    })?;
    Ok(up.version)
}

/// App-wide zoom — WKWebView's `setPageZoom`, driven by ⌘+ / ⌘− / ⌘0.
///
/// This is deliberately NOT tauri's `zoomHotkeysEnabled`. That option injects a
/// script (`tauri/src/webview/scripts/zoom-hotkey.js`) whose `window` keydown
/// listener never checks `defaultPrevented` — so it would fire ALONGSIDE the
/// frontend's own handler — keeps the level in a script-local variable that
/// desyncs from any programmatic call, steps by a coarse 0.2, and forgets the
/// level on every restart. The frontend owns the step table and persists it in
/// `ui-state.json`; this command is the one line of platform underneath.
///
/// Page zoom is what makes the knob reach the TERMINAL: it shrinks the CSS-px
/// viewport, TerminalPane's ResizeObserver refits, and the tmux pane is resized
/// to the new cols/rows. `--ui-rem` alone can never do that (tokens.css keeps
/// `--term-size` independent on purpose).
#[tauri::command]
async fn set_zoom(window: tauri::WebviewWindow, factor: f64) -> Result<(), String> {
    // A NaN or a wild factor here is a blank window the user cannot zoom back
    // out of — the clamp is the frontend's, repeated because this is an IPC
    // boundary and the frontend is not the only possible caller.
    if !factor.is_finite() {
        return Err("zoom factor is not a finite number".into());
    }
    let factor = factor.clamp(0.5, 3.0);
    window.set_zoom(factor).map_err(|e| {
        applog("error", &format!("set_zoom({factor}): {e}"));
        e.to_string()
    })
}

// ── the shim's own commands ──────────────────────────────────────────────────
// Neither is a row in serve's dispatch table, and the frontend routes both
// here by name (app/src/ipc.ts `SHIM_COMMANDS`; engine-route-check.mjs holds
// that neither ever becomes one).

/// Find or start this host's `worktrees serve` and say how to reach it. This
/// answer is the ONLY way the token reaches the page: never a URL, never
/// storage, never a log line. `reason` is the page's, when it is reconnecting
/// after losing the engine, so app.log says why a second serve was looked for.
/// `start` is the page's reconnect policy (app/src/reconnect.ts): under
/// `false` nothing is started or replaced. `manual` marks a press of Reconnect
/// now. `on_phase` hears what a long call is doing, for the banner. Blocking
/// work (a spawn waits for serve to prove itself), so off the main thread.
#[tauri::command]
async fn engine_connect(
    engine: tauri::State<'_, Arc<engine::Engine>>,
    reason: Option<String>,
    start: bool,
    manual: bool,
    on_phase: tauri::ipc::Channel<String>,
) -> Result<engine::Endpoint, engine::ConnectError> {
    let e = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let ask = engine::Ask { reason: reason.as_deref(), may_start: start, manual };
        e.connect(ask, &|p: &str| {
            let _ = on_phase.send(p.to_string());
        })
    })
    .await
    .map_err(|e| engine::ConnectError::failed(e.to_string()))?
}

/// Bring the window forward. The engine's inbox emits `app:open-doc` when a
/// session runs `worktrees show` / `show_doc`, and the page calls this so the
/// person sees the document — the half of that delivery that needs a window,
/// which serve (a phone's engine as much as this desktop's) does not have.
#[tauri::command]
async fn raise_window(app: AppHandle) -> Result<(), String> {
    let w = app.get_webview_window("main").ok_or("no main window")?;
    let _ = w.unminimize();
    let _ = w.show();
    w.set_focus().map_err(|e| {
        applog("warn", &format!("inbox: could not focus the window: {e}"));
        e.to_string()
    })
}

// ── the remote door (remote.rs) ─────────────────────────────────────────────
// The shim's own commands, like engine_connect: never serve rows. The page
// never sees a key or a pin through any of them.

/// The hosts this machine is paired with, and which one the window drives
/// (`null`: this machine's own serve).
#[tauri::command]
async fn remote_hosts(remote: tauri::State<'_, Arc<remote::Remote>>) -> Result<remote::HostList, String> {
    remote.hosts()
}

/// Choose the engine the window drives: a kept host's id, or `null` for this
/// machine's own. The page reloads to act on it.
#[tauri::command]
async fn remote_choose(remote: tauri::State<'_, Arc<remote::Remote>>, host_id: Option<String>) -> Result<(), String> {
    remote.choose(host_id.as_deref())
}

#[tauri::command]
async fn remote_forget(remote: tauri::State<'_, Arc<remote::Remote>>, host_id: String) -> Result<worktrees_remote::hosts::HostSummary, String> {
    remote.forget(&host_id).await
}

/// Pair with the host a `worktrees-pair:v1` link names (§2.2). `on_event`
/// hears `{dialling}` and then `{code}`, the six digits to compare with the
/// host's screen; the answer is the kept host, or the refusal in words.
#[tauri::command]
async fn remote_pair(
    remote: tauri::State<'_, Arc<remote::Remote>>,
    link: String,
    name: String,
    on_event: tauri::ipc::Channel<serde_json::Value>,
) -> Result<worktrees_remote::hosts::HostSummary, String> {
    remote.pair(&link, &name, &on_event).await
}

/// Follow a kept host to a new address by its address code (§4.1): kept
/// only once a pinned connection there succeeds; the moved host, or the
/// refusal in words.
#[tauri::command]
async fn remote_readdress(remote: tauri::State<'_, Arc<remote::Remote>>, host_id: String, code: String) -> Result<worktrees_remote::hosts::HostSummary, String> {
    remote.readdress(&host_id, &code).await
}

/// Open the pinned socket to a kept host: `{conn, token, host}`, then its
/// frames on `on_frame` as `{frame}` and, last, `{closed}`. Nothing answering
/// at its address sends the shim looking for it on the LAN (`reach`).
#[tauri::command]
async fn remote_connect(
    remote: tauri::State<'_, Arc<remote::Remote>>,
    host_id: String,
    on_frame: tauri::ipc::Channel<serde_json::Value>,
) -> Result<remote::Connected, String> {
    remote.connect(&host_id, on_frame).await
}

#[tauri::command]
async fn remote_send(remote: tauri::State<'_, Arc<remote::Remote>>, conn: u64, text: String) -> Result<(), String> {
    remote.send(conn, text).await
}

#[tauri::command]
async fn remote_close(remote: tauri::State<'_, Arc<remote::Remote>>, conn: u64) -> Result<(), String> {
    remote.close(conn).await
}

/// A remote window's own settings blob (§9.1), in place of the host's
/// `get_settings`/`set_settings`: written whole by the page, never edited here.
#[tauri::command]
async fn remote_settings_get(remote: tauri::State<'_, Arc<remote::Remote>>) -> Result<Option<serde_json::Value>, String> {
    remote.settings_get()
}

#[tauri::command]
async fn remote_settings_set(remote: tauri::State<'_, Arc<remote::Remote>>, settings: serde_json::Value) -> Result<(), String> {
    remote.settings_set(settings)
}

/// The origin of the page `tauri dev` loads (`devUrl`, http://localhost:1420),
/// which serve does not allow by name: a serve this dev build starts is told
/// `--allow-origin` it, and the shim's probe presents it.
#[cfg(dev)]
fn dev_origin(app: &tauri::App) -> Option<String> {
    app.config().build.dev_url.as_ref().map(|u| u.origin().ascii_serialization())
}

/// A release build's page is `tauri://localhost`, which serve allows by name:
/// there is no dev origin to pass, ever. This twin is what keeps the dev-only
/// `--allow-origin` path out of a shipped app (engine-route-check.mjs).
#[cfg(not(dev))]
fn dev_origin(_app: &tauri::App) -> Option<String> {
    None
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // FIRST, and in one call: the log sink, the panic hook, the GUI-bare PATH
    // and locale repairs, core's log, the startup line. The shim is not an
    // engine, but it still logs, can panic, and launches editors and
    // terminals that need the login shell's PATH — and serve, which it starts
    // from `engine_connect`, inherits that repaired PATH. `worktrees serve`
    // runs the same preamble (`worktrees_api::host`).
    worktrees_api::host::boot(std::sync::Arc::new(AppLog));
    tauri::Builder::default()
        // `open_path` (the Files tab's right-click → Open) needs BOTH halves,
        // and JSON cannot carry the reason: `opener:allow-open-path` in
        // capabilities/default.json plus a path scope, because the plugin's
        // `is_path_allowed` ANDs the fs scope with "some allowed entry names a
        // path" — the permission on its own allows nothing. The scope is `**`,
        // and tauri.conf.json turns `requireLiteralLeadingDot` off so that
        // pattern also covers a path with a dot component: every worktree this
        // app opens lives under `.worktrees/`, and with the unix default the
        // invoke would reject exactly those — silently.
        //
        // `open_url` has the same shape one level up: `opener:default` scopes
        // it to http(s), mailto and tel, so the privacy card's System Settings
        // link (`x-apple.systempreferences:`) needs its own allow entry or it
        // rejects with `ForbiddenUrl`. The test
        // `every_privacy_pane_is_one_the_opener_may_open` holds every pane
        // against that file.
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // The window tracker IS the window. The engine finder is managed in
        // `.setup()` below, once the config dir can be resolved; those two are
        // the only state this process holds (api-boundary-check.mjs).
        .manage(winstate::Tracker::default())
        // ONE `.setup`. tauri's `Builder::setup` REPLACES the stored closure
        // (`self.setup = Box::new(setup)`) rather than chaining, so a second
        // call silently discards the first. Two of them shipped from #358 on:
        // the dead one held core's log sink, and then, in #508, `IDENT` and the
        // `Ctx` itself, so every `ctx()` panicked "state() called before
        // manage()". api-boundary-check.mjs counts them.
        .setup(|app| {
            // The identifier this process is really running as. `sandbox.sh
            // --app` overrides it via `--config`, and `log_dir()` reads this
            // rather than the `APP_IDENT` const so a sandbox stops writing
            // into the real app's log. Lines emitted ABOVE this point (the
            // startup line) still land under the default identifier — this is
            // the earliest an app handle exists to ask.
            let _ = IDENT.set(app.config().identifier.clone());
            // The config dir serve is told to serve: the app's own, so a
            // sandbox identifier gets a sandbox engine. A failure is logged and
            // falls back to the current directory rather than aborting startup.
            let config_dir = app.path().app_config_dir().unwrap_or_else(|e| {
                applog("error", &format!("app_config_dir: {e}"));
                PathBuf::from(".")
            });
            // How to find or start the engine. Host policy is NOT loaded here:
            // `hostpolicy::boot` is the engine's act, and serve's `cmd_serve`
            // does it (hostpolicy-check.mjs).
            let dev = dev_origin(app);
            let origin = dev.clone().unwrap_or_else(|| engine::APP_ORIGIN.to_string());
            let resource_dir = app.path().resource_dir().ok();
            let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
            let bin = engine::serve_bin(
                cfg!(dev),
                std::env::var_os("WORKTREES_SERVE_BIN").map(PathBuf::from).as_deref(),
                Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target")),
                exe_dir.as_deref(),
                resource_dir.as_deref(),
            );
            match &bin {
                Ok((p, how)) => applog("info", &format!("engine: serve binary {} ({how})", p.display())),
                Err(e) => applog("error", &format!("engine: no serve binary: {e}")),
            }
            // The remote door's state: the hosts kept beside the app's other
            // files, and the one remote socket this window may hold.
            app.manage(Arc::new(remote::Remote::new(&config_dir)));
            app.manage(Arc::new(engine::Engine::new(
                config_dir.clone(),
                origin,
                dev,
                bin,
                worktrees_api::viewer::bundle_path(resource_dir.as_deref()),
                app.config().identifier.clone(),
                env!("CARGO_PKG_VERSION").to_string(),
            )));
            // Settings → Behavior → Restore window. Read straight from
            // ui-state.json because this runs before the frontend exists;
            // see winstate.rs for why that file is only ever READ here.
            if let Some(w) = app.get_webview_window("main") {
                let ui = std::fs::read(config_dir.join("ui-state.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
                winstate::attach(&w, &config_dir, winstate::enabled(ui.as_ref()));
            }
            // macOS 26 floats a Writing Tools affordance (AppKit's Campo
            // lightweight UI) over any selection, and hovering the one it puts
            // over the webview trips an assertion inside AppKit —
            // NSCampoLightweightUIController.m:1429, three times in app.log.
            // The NSException that assertion raises unwinds out through tao's
            // `sendEvent:` override, which is `extern "C"` and therefore
            // nounwind, so Rust turns the foreign unwind into an abort: SIGABRT
            // mid-keystroke, tmux sessions untouched. The only trace is a bare
            // "panic in a function that cannot unwind" with NO panic line before
            // it — because no Rust panic ever happened.
            //
            // Selecting text is routine here (the tab-rename box selects on
            // focus, the terminal selects on drag), so take the affordance away.
            // AppKit asks the view for `allowsWritingToolsAffordance`; WKWebView
            // answers YES and offers no setter, and the settable knob
            // (`WKWebViewConfiguration.writingToolsBehavior`) only exists before
            // the webview is built — tauri creates this window from
            // tauri.conf.json, so we never hold that configuration. Overriding
            // the getter on wry's own WKWebView subclass is the small version:
            // one added method, our instances only, no Apple class touched.
            #[cfg(target_os = "macos")]
            for (_, win) in app.webview_windows() {
                // Every webview shares the one registered wry class, so the
                // method goes on once per process: a second window's add would
                // fail (the class implements it by then) and warn about a fix
                // that is in place and working.
                static AFFORDANCE: std::sync::Once = std::sync::Once::new();
                let r = win.with_webview(|pw| unsafe {
                    AFFORDANCE.call_once(|| {
                        let view = &*(pw.inner() as *mut objc2::runtime::AnyObject);
                        // Step past the KVO subclass AppKit swizzles in (wry
                        // observes the document title): the instance's isa
                        // reverts to the original class when observation ends,
                        // and a method added on the notifying class goes out of
                        // reach with it. wry's own class is the stable place —
                        // a KVO subclass made later inherits the override.
                        let mut cls = Some(view.class());
                        while let Some(c) = cls {
                            let name = c.name().to_string_lossy().to_string();
                            if name.contains("WryWebView") && !name.contains("NSKVONotifying") {
                                break;
                            }
                            cls = c.superclass();
                        }
                        let Some(c) = cls else {
                            applog("warn", "writing-tools affordance: wry webview class not found");
                            return;
                        };
                        extern "C" fn no(
                            _this: &objc2::runtime::AnyObject,
                            _sel: objc2::runtime::Sel,
                        ) -> objc2::runtime::Bool {
                            objc2::runtime::Bool::NO
                        }
                        // BOOL encodes as `B` on arm64 and `c` on x86_64 — take
                        // it from objc2 rather than picking one.
                        let types = std::ffi::CString::new(format!(
                            "{}@:",
                            <objc2::runtime::Bool as objc2::encode::Encode>::ENCODING
                        ))
                        .expect("no interior nul in an objc type encoding");
                        let added = objc2::ffi::class_addMethod(
                            c as *const objc2::runtime::AnyClass as *mut objc2::runtime::AnyClass,
                            objc2::sel!(allowsWritingToolsAffordance),
                            std::mem::transmute::<
                                extern "C" fn(&objc2::runtime::AnyObject, objc2::runtime::Sel) -> objc2::runtime::Bool,
                                unsafe extern "C-unwind" fn(),
                            >(no),
                            types.as_ptr(),
                        );
                        if !added.as_bool() {
                            applog("warn", "writing-tools affordance: class_addMethod failed");
                        }
                    });
                });
                // A failure here leaves the app crashing on a selection with a
                // log that looks exactly like the fix landed — say so instead.
                if let Err(e) = r {
                    applog("warn", &format!("writing-tools affordance: with_webview failed: {e}"));
                }
            }
            Ok(())
        })
        // The client-local commands (serve refuses each as `client-local`) and
        // the shim's own two. Every other command the page sends goes to serve.
        .invoke_handler(tauri::generate_handler![
            copy_text,
            install_app_version,
            open_editor,
            detect_editors,
            open_terminal,
            set_zoom,
            engine_connect,
            raise_window,
            remote_hosts,
            remote_choose,
            remote_forget,
            remote_pair,
            remote_readdress,
            remote_connect,
            remote_send,
            remote_close,
            remote_settings_get,
            remote_settings_set
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|handle, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                winstate::ready(handle);
            }
            if matches!(event, tauri::RunEvent::Exit) {
                winstate::stop(handle);
                // Stop the serve this app manages — one it started, or one an
                // app started and this one adopted after a crash — and only
                // that: its `host::stop` ends the dock shells and flushes their
                // rings, as quitting always has. A serve someone else started
                // (the CLI, the dev stack) is left running.
                handle.state::<Arc<engine::Engine>>().stop_owned();
            }
        });
}

#[cfg(test)]
mod tests {
    // ── app.log's mode ───────────────────────────────────────────────────────
    // The same three cases as serve.log's (`worktrees serve`, OWASP L1): the
    // app's log carries the same kind of thing — PATH, the places opened, the
    // engine's own errors — and sat at 0644 in a 0755 directory.

    fn log_scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wt-app-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn mode(p: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    fn set_mode(p: &std::path::Path, m: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    }

    /// One `write(2)` per line, as serve.log has had since ecf6851: a
    /// `writeln!` on an unbuffered `File` is a write per piece of the format,
    /// and two threads' pieces interleave inside one line.
    #[test]
    fn app_log_lines_from_concurrent_writers_are_whole() {
        let d = log_scratch("torn");
        let log = d.join("app.log");
        const WRITERS: usize = 8;
        const LINES: usize = 300;
        let tail = "x".repeat(200);
        let hs: Vec<_> = (0..WRITERS)
            .map(|w| {
                let (d, log, tail) = (d.clone(), log.clone(), tail.clone());
                std::thread::spawn(move || {
                    for i in 0..LINES {
                        super::append_line(&d, &log, "info", &format!("w{w}-{i}-{tail}"));
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), WRITERS * LINES, "one line per call");
        let mut seen = std::collections::HashSet::new();
        for line in &lines {
            // `<19-char UTC stamp>Z [info] w<writer>-<i>-<200 x>`; anything
            // else is two writers' pieces in one line.
            let (ts, rest) = line.split_at(line.len().min(19));
            assert!(ts.len() == 19 && ts.as_bytes()[4] == b'-' && ts.as_bytes()[10] == b' ', "stamp: {line:?}");
            let body = rest.strip_prefix("Z [info] w").unwrap_or_else(|| panic!("torn: {line:?}"));
            let (id, x) = body.rsplit_once('-').unwrap_or_else(|| panic!("torn: {line:?}"));
            assert_eq!(x, tail, "torn: {line:?}");
            assert!(seen.insert(id.to_string()), "twice: {line:?}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_new_app_log_is_owner_only() {
        let d = log_scratch("fresh");
        let log = d.join("app.log");
        super::append_line(&d, &log, "info", "hello");
        assert_eq!(mode(&log), 0o600, "app.log is {:o}", mode(&log));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_wider_app_log_already_there_is_tightened_and_kept() {
        let d = log_scratch("wide");
        let log = d.join("app.log");
        std::fs::write(&log, "an earlier line\n").unwrap();
        set_mode(&log, 0o644);
        super::append_line(&d, &log, "info", "hello");
        assert_eq!(mode(&log), 0o600, "app.log is {:o}", mode(&log));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.starts_with("an earlier line\n") && text.ends_with("hello\n"), "{text}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A `rename` keeps the mode the file had.
    #[test]
    fn the_rotated_app_log_is_owner_only_too() {
        let d = log_scratch("rotate");
        let log = d.join("app.log");
        std::fs::write(&log, vec![b'x'; 1_000_001]).unwrap();
        set_mode(&log, 0o644);
        super::append_line(&d, &log, "info", "hello");
        let old = d.join("app.log.1");
        assert_eq!(std::fs::metadata(&old).unwrap().len(), 1_000_001, "the full log was not the one rotated");
        assert_eq!(mode(&old), 0o600, "app.log.1 is {:o}", mode(&old));
        assert_eq!(mode(&log), 0o600, "app.log is {:o}", mode(&log));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn clone_errors_serialize_with_a_kind_the_dialog_can_branch_on() {
        let e = worktrees_core::clone::CloneError {
            kind: worktrees_core::clone::CloneErrorKind::NotFound,
            message: "No repository".into(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "not_found");
        assert_eq!(v["message"], "No repository");
    }

    // ── the privacy card's System Settings link ─────────────────────────────

    #[test]
    fn every_privacy_pane_is_one_the_opener_may_open() {
        // `opener:default` scopes `open_url` to http(s), mailto and tel, so a
        // pane the capability file does not name is a Settings button that
        // rejects with `ForbiddenUrl` — in the real app only, since the mock's
        // `openUrl` has no scope. The panes are serve's (`macprivacy`); the
        // scope is this window's, which is why the check lives here. The
        // plugin matches each `url` as a glob; the entries here are all
        // `<prefix>*`, and a fancier one should be read by hand rather than
        // guessed at.
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let prefixes: Vec<&str> = caps["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["identifier"] == "opener:allow-open-url")
            .flat_map(|p| p["allow"].as_array().into_iter().flatten())
            .filter_map(|e| e["url"].as_str())
            .map(|u| {
                let pre = u.strip_suffix('*').unwrap_or_else(|| panic!("not a <prefix>* glob: {u}"));
                assert!(!pre.contains(['*', '?', '[']), "not a <prefix>* glob: {u}");
                pre
            })
            .collect();
        for a in worktrees_api::macprivacy::Area::ALL {
            assert!(
                prefixes.iter().any(|pre| a.pane().starts_with(pre)),
                "{a:?}: capabilities/default.json does not let the opener open {}",
                a.pane()
            );
        }
    }

    // ── rollback: which releases are offered ────────────────────────────────

    // ── Claude service status ───────────────────────────────────────────────

    // ── dock shell cwd memory ────────────────────────────────────────────────

    // ── dock shell resizes ───────────────────────────────────────────────────

    // ── dock shell history: scrollback + per-tab commands ───────────────────

    // ── "Ask Claude" (ai_status_report) — the two pure pieces ────────────────
    // The spawn itself cannot be tested here: there is no fake `claude` anywhere
    // in this repo (the bats harness fakes tmux and git, never the AI), which is
    // why docs/ai-profiles-manual-checks.md §11 exists. What IS testable is the
    // text we hand the model and the check that decides whether to spawn at all.

    // ── local usage metrics ────────────────────────────────────────────────

}

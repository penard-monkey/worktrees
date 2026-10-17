//! The command layer, driven with no window attached.
//!
//! This is an INTEGRATION test on purpose: it links `worktrees-api` from
//! outside and may touch only what the crate makes public. That is the same
//! position `worktrees serve` will be in, so "it compiles and passes" is a
//! statement about the seam and not just about the code behind it. A unit
//! test inside the crate could reach private helpers and would prove less.
//!
//! What it deliberately does NOT do is reach for a repo, a tmux server or a
//! pty. Those need fixtures the unit tests already build; the question here
//! is narrower — can something that is not Tauri construct a `Ctx`, call
//! handlers, and get its events back.

use std::sync::{Arc, Mutex};

use worktrees_api::{Ctx, Host};

/// A host with no window. Everything Tauri's `AppHandle` was for, minus the
/// window: events go somewhere a test can read them.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<(String, serde_json::Value)>>,
}

impl Host for Recorder {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events.lock().unwrap().push((event.into(), payload));
    }
}

/// A `Ctx` over a config dir of this test's own, so nothing here can read or
/// write the real app's state.
fn ctx(tag: &str) -> (Ctx, Arc<Recorder>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("wt-headless-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rec = Arc::new(Recorder::default());
    (Ctx::new(dir.clone(), rec.clone()), rec, dir)
}

/// The whole point of the extraction, as one assertion: a caller that is not
/// a WebView builds the context and the commands answer.
#[test]
fn a_caller_with_no_window_can_build_a_ctx_and_be_answered() {
    let (c, rec, dir) = ctx("ctx");
    assert_eq!(c.projects_file(), dir.join("projects.json"));

    // Events reach the host rather than a window.
    c.emit("places:changed", serde_json::Value::Null);
    assert_eq!(rec.events.lock().unwrap()[0].0, "places:changed");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// `ui-state.json` is the app's settings blob, written whole by the frontend.
/// It round-trips through the config dir the `Ctx` names — which is the part
/// that moved, from `AppHandle::path()` to a plain `PathBuf`.
#[test]
fn settings_round_trip_through_the_ctx_config_dir() {
    let (c, _, dir) = ctx("settings");
    assert!(worktrees_api::ops::get_settings(&c).unwrap().is_none(), "nothing stored yet");

    let want = serde_json::json!({ "theme": "tokyo-night", "dock_width": 420 });
    worktrees_api::ops::set_settings(&c, want.clone()).unwrap();
    assert_eq!(worktrees_api::ops::get_settings(&c).unwrap(), Some(want));

    // …and the file is where `settings_info` says it is, because Settings →
    // Data offers to reveal exactly that path.
    let info = worktrees_api::ops::settings_info(&c).unwrap();
    assert_eq!(info.file, dir.join("ui-state.json").to_string_lossy());
    assert!(std::path::Path::new(&info.file).exists());

    std::fs::remove_dir_all(&dir).unwrap();
}

/// The usage metrics behind the nav footer: appended through one command,
/// read back through another, both resolving their file from the `Ctx`.
#[test]
fn ui_events_append_and_read_back_as_usage() {
    let (c, _, dir) = ctx("events");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let ev = |k: &str, key: &str| worktrees_api::prefs::UiEvent {
        t: now_ms,
        k: k.into(),
        key: key.into(),
        s: "nav".into(),
        ms: 1000,
    };
    worktrees_api::prefs::ui_events_append(&c, vec![ev("act", "place_open"), ev("dwell", "files")])
        .unwrap();

    let usage = worktrees_api::prefs::ui_usage(&c, 7, 0).unwrap();
    assert_eq!(usage.days.len(), 7, "exactly `days` buckets, gaps filled");
    assert!(usage.file.ends_with("ui-events.jsonl"));
    assert!(
        usage.acts.iter().any(|r| r.key == "place_open"),
        "the act we appended is in the rollup: {:?}",
        usage.acts
    );

    worktrees_api::prefs::ui_events_clear(&c).unwrap();
    let after = worktrees_api::prefs::ui_usage(&c, 7, 0).unwrap();
    assert!(after.acts.is_empty(), "clear empties the rollup");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// `include_str!("../../../CHANGELOG.md")` resolves from a different crate
/// now. A wrong path is not a compile error if some other file is found, and
/// the only symptom in the app is an empty "What's new" — so assert on the
/// content, not merely that it is non-empty.
#[test]
fn the_changelog_ships_in_the_binary_and_is_this_repos() {
    let c = worktrees_api::meta::get_changelog();
    assert_eq!(c.version, env!("CARGO_PKG_VERSION"));
    assert!(c.changelog.contains("## [Unreleased]"), "not a CHANGELOG we recognise");
    assert!(c.changelog.len() > 10_000, "suspiciously short: {} bytes", c.changelog.len());
}

/// Logging before a host installs a sink must be silent, because every test
/// above reaches a handler with no sink at all — and so does a `worktrees
/// serve` that has not finished starting.
#[test]
fn the_log_is_a_no_op_until_a_host_installs_a_sink() {
    worktrees_api::log::warn("nobody is listening");
    assert_eq!(worktrees_api::log::paths(), (String::new(), String::new()));
}

/// The nav-dot plumbing, end to end through a `Ctx` CLONE.
///
/// This is the failure the extraction could most easily have introduced and
/// the one no existing test would catch: the registries were three
/// process-global `static`s, and they are now fields on `Ctx`. Every handler
/// gets its `Ctx` from a clone, and the 3s poll holds its own — so if a clone
/// did not share the registry, the snapshot would record a lane into one map
/// and the tick would read an empty other one. No error, no log line, just
/// dots that never light.
#[test]
fn a_lane_watched_through_one_ctx_is_visible_through_a_clone_of_it() {
    let (c, _, dir) = ctx("watch");
    let cwd = "/nonexistent/headless-lane";
    let session = "headless~agent~pi".to_string();

    // What `snapshot()` does for every live session it finds.
    let watched = worktrees_api::watch::watch_lane(
        &c.watch, "pi", cwd, Some(&session), Some("1"), &None,
    );
    assert!(watched, "pi is a polled harness");

    // What the poll thread does, holding its own clone of the same `Ctx`.
    //
    // Asserted on the REGISTRY, not on the tick's output: a tick can only
    // report busy/waiting/done for a lane with a real session file behind it,
    // so a path that does not exist would answer "nothing" whether the
    // registries were shared or not. That version of this test passed for the
    // wrong reason, which is the failure mode AGENTS.md calls a test passing
    // because its fixture omits the thing.
    let poll = c.clone();
    assert_eq!(
        poll.watch.watching(),
        vec![("pi", cwd.to_string())],
        "the clone's registry did not see the lane the original watched — \
         `Ctx::clone` has stopped sharing `watch`, and the nav dots are dead"
    );

    // And dropping the lane through the clone is visible to the original.
    worktrees_api::watch::watch_lane(&poll.watch, "pi", cwd, None, None, &None);
    assert!(
        c.watch.watching().is_empty(),
        "a lane dropped through the clone is still watched by the original"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every harness in `provider::PROVIDERS` must be wired into the poll. The
/// unit tests assert this too; it is repeated here because the drift it
/// guards (pi shipped with a correct core and no dot at all, v0.34.0) is the
/// exact shape a server edition would repeat for a harness added later.
#[test]
fn every_provider_is_polled_through_a_public_ctx() {
    let (c, _, dir) = ctx("providers");
    for p in worktrees_core::provider::PROVIDERS {
        assert!(
            worktrees_api::watch::watch_lane(&c.watch, p.id, "/nonexistent/x", None, None, &None),
            "{} is in PROVIDERS but the snapshot never hands its sessions to the poll",
            p.id
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

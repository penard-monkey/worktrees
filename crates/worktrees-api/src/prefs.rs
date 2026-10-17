//! Preferences the backend keeps, the Data pane's verbs, and the local usage
//! metrics behind the nav footer (`ui-events.jsonl` in the config dir).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::io::Write;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use worktrees_core::{ops, sysclock, Project};

use crate::log::log as applog;
use crate::util::fmt_utc;
use crate::ops::{run_op, CmdResult};
use crate::term::{
    cwd_key, edit_cwds, forget_place_history, hist_write_atomic, kill_place_shells, term_hist_dir,
    Shells, PERSIST_SCROLLBACK, PER_TAB_HISTORY, TERM_HIST_LOCK,
};
use crate::viewer;
use crate::Ctx;


/// Push the two terminal-history preferences down from the frontend.
///
/// The settings blob is backend-opaque and frontend-owned, so the shell path
/// cannot look them up — the frontend pushes instead, on hydration and on every
/// change. Same shape as `set_fetch_interval`, and for the same reason.
pub fn set_term_history_opts(persist_scrollback: bool, per_tab_history: bool) -> Result<(), String> {
    PERSIST_SCROLLBACK.store(persist_scrollback, Ordering::Relaxed);
    PER_TAB_HISTORY.store(per_tab_history, Ordering::Relaxed);
    Ok(())
}

/// Bytes under one directory, recursively. This tree is only ever
/// `<tab>/{meta.json, scrollback.bin, zdotdir/*}`, so the recursion is shallow.
pub fn dir_bytes(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// What Settings → Data shows beside "Clear saved terminal history".
#[derive(Serialize)]
pub struct TermHistoryInfo {
    pub dir: String,
    pub bytes: u64,
    pub tabs: usize,
}

pub fn term_history_info(ctx: &Ctx) -> Result<TermHistoryInfo, String> {
    let root = term_hist_dir(ctx)?;
    let (mut bytes, mut tabs) = (0u64, 0usize);
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                tabs += 1;
                bytes += dir_bytes(&p);
            }
        }
    }
    Ok(TermHistoryInfo { dir: root.to_string_lossy().into_owned(), bytes, tabs })
}

/// Forget every tab's saved output and command history.
///
/// The live rings go too. Leaving them would have the next sampler tick write
/// the same bytes straight back, which would make the button a lie — but what is
/// already PAINTED in an open terminal stays, because xterm owns that and
/// wiping someone's visible screen is not what this asks for.
///
/// A tab's `zdotdir` is emptied rather than removed: a running shell holds
/// `HISTFILE` inside it, and deleting the directory would leave that shell
/// silently unable to record anything until it was restarted. `meta.json` stays
/// for the same practical reason — it is identity and width, never user
/// content, and a directory without it is one no sweep can ever collect.
pub fn term_history_clear(ctx: &Ctx, shells: &Shells) -> Result<(), String> {
    // Scoped: nothing that holds the registry may wait on the file lock below.
    {
        let map = shells.0.lock().unwrap();
        for sh in map.values() {
            sh.ring.lock().unwrap().clear();
            sh.dirty.store(false, Ordering::Relaxed);
        }
    }
    let root = term_hist_dir(ctx)?;
    let _guard = TERM_HIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(rd) = std::fs::read_dir(&root) else { return Ok(()) };
    let mut cleared = 0usize;
    for e in rd.flatten() {
        let tab = e.path();
        if !tab.is_dir() {
            continue;
        }
        cleared += 1;
        let _ = std::fs::remove_file(tab.join("scrollback.bin"));
        let zdot = tab.join("zdotdir");
        if zdot.is_dir() {
            let _ = hist_write_atomic(&zdot.join(".zsh_history"), b"");
        } else {
            let _ = std::fs::remove_file(tab.join(".bash_history"));
        }
    }
    applog("info", &format!("term-history: cleared {cleared} tab(s) at the user's request"));
    Ok(())
}

/// Remove a place (`rm <slug> -y` [+ --branch/--force]); the UI confirms first.
pub fn remove_place(
    ctx: &Ctx,
    repo: String,
    slug: String,
    del_branch: bool,
    force: bool,
    shells: &Shells,
) -> Result<CmdResult, String> {
    let slug_log = slug.clone();
    let slug_sweep = slug.clone();
    let mut args: Vec<String> = vec![slug, "-y".into()];
    if del_branch {
        args.push("--branch".into());
    }
    if force {
        args.push("--force".into());
    }
    // Resolved BEFORE the removal, because it cannot be afterwards: the derived
    // tree is keyed by a hash of the canonical directory, and `canonicalize`
    // needs the directory to exist. A place nobody opened in the browser has no
    // tree and this resolves to one that is not there, which is fine.
    let viewer_root = Project::discover(Path::new(&repo))
        .ok()
        .map(|p| p.place_dir(&slug_sweep))
        .and_then(|d| std::fs::canonicalize(d).ok());
    // cmd_rm sweeps this place's dock shell sidecars itself (core, only once the
    // removal proceeds past its dirty/confirm guards — a refused rm keeps them).
    let r = run_op(&format!("rm {slug_log}"), &repo, move |p, ui| ops::cmd_rm(p, ui, &args))?;
    if r.ok {
        kill_place_shells(&shells, &repo, &slug_sweep);
        // …and the documents the viewer derived from it. `cmd_rm` deleted the
        // originals; without this the derived copy is the last readable one and
        // it is on a port.
        if let (cfg, Some(root)) = (ctx.config_dir.clone(), viewer_root) {
            viewer::forget_place(&ctx.viewer, &cfg, &slug_sweep, &root);
        }
        // The place is gone for good, so its remembered directories are too.
        // (A `close` deliberately does NOT do this: closing keeps the tab names,
        // so it has to keep what those tabs point at.)
        edit_cwds(ctx, |map| map.remove(&cwd_key(&repo, &slug_sweep)).is_some());
        if let Ok(root) = term_hist_dir(ctx) {
            forget_place_history(&root, &repo, &slug_sweep);
        }
    }
    Ok(r)
}


/// One line of `ui-events.jsonl`. `ms` is dwell only — an `act` has no duration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UiEvent {
    /// unix MILLIseconds (the frontend's clock; a day bucket needs no more)
    pub t: i64,
    /// `act` (a click or a chord) · `dwell` (foreground ms on one surface)
    pub k: String,
    pub key: String,
    pub s: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ms: i64,
}

pub fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Is this a name someone WROTE, rather than something someone typed?
///
/// Deliberately an allowlist, not the brief's "no `/`, no spaces" denylist: a
/// denylist has to imagine every shape user text arrives in, and slugs, branch
/// names and note text are exactly the shapes nobody imagines. Keys and
/// surfaces are hand-written identifiers — `nav.row.select`, `chord.cmd-b`,
/// `dock.files` — so ASCII alphanumerics plus `.`, `-` and `_` is the whole
/// vocabulary, and 64 chars is far more than any of them needs.
///
/// It cannot tell `ui-tweaks` from `dock-files`, and it is not meant to: it is
/// the last line, not the first. The first is that the frontend resolves keys
/// from `data-track` attributes, which are constants in the source.
pub fn valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

pub fn valid_event(e: &UiEvent) -> bool {
    matches!(e.k.as_str(), "act" | "dwell") && valid_token(&e.key) && valid_token(&e.s) && e.ms >= 0 && e.t > 0
}

/// Serialises appenders against the rotation + the clear, all of which
/// read-modify-write the same two files.
pub static UI_EVENTS_LOCK: Mutex<()> = Mutex::new(());

/// Past this the file rotates to `.1` (one generation, then it is dropped).
/// A month of heavy use is a few hundred KB, so 4 MB is a year of headroom and
/// still small enough that `ui_usage` can read both generations on demand.
pub const UI_EVENTS_MAX: u64 = 4 * 1024 * 1024;

pub fn ui_events_path(ctx: &Ctx) -> Result<PathBuf, String> {
    let dir = ctx.config_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("ui-events.jsonl"))
}

pub fn ui_events_append_at(path: &Path, events: &[UiEvent]) {
    let _guard = UI_EVENTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let kept: Vec<&UiEvent> = events.iter().filter(|e| valid_event(e)).collect();
    let dropped = events.len() - kept.len();
    if dropped > 0 {
        // Loud, because the only way to reach it is a frontend that started
        // building a key out of something it read off the screen.
        applog("warn", &format!("ui-events: refused {dropped} event(s) that did not look like fixed keys"));
    }
    if kept.is_empty() {
        return;
    }
    if std::fs::metadata(path).map(|m| m.len() > UI_EVENTS_MAX).unwrap_or(false) {
        let _ = std::fs::rename(path, path.with_extension("jsonl.1"));
    }
    let mut buf = String::new();
    for e in kept {
        match serde_json::to_string(e) {
            Ok(line) => {
                buf.push_str(&line);
                buf.push('\n');
            }
            Err(e) => applog("warn", &format!("ui-events encode failed: {e}")),
        }
    }
    // Append, not write-then-rename: a lost tail costs a few clicks out of a
    // histogram, and a metric may never be the reason the UI stalls.
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(buf.as_bytes()) {
                applog("warn", &format!("ui-events write failed: {e}"));
            }
        }
        Err(e) => applog("warn", &format!("ui-events open failed: {e}")),
    }
}

/// Append a batch. NEVER fails the UI: a metric that can produce an error toast
/// is a metric that changes the behaviour it is measuring.
pub fn ui_events_append(ctx: &Ctx, events: Vec<UiEvent>) -> Result<(), String> {
    match ui_events_path(ctx) {
        Ok(path) => ui_events_append_at(&path, &events),
        Err(e) => applog("warn", &format!("ui-events path unavailable: {e}")),
    }
    Ok(())
}


/// Which LOCAL day an event landed in, as a day number since the epoch. The
/// offset comes from the frontend (`-new Date().getTimezoneOffset()`), because
/// the backend has no timezone database and the user reads the heatmap in the
/// only timezone that matters — theirs. `div_euclid`, not `/`: a pre-1970 or
/// west-of-UTC value must floor, not truncate toward zero.
pub fn day_index(t_ms: i64, tz_offset_min: i32) -> i64 {
    (t_ms.div_euclid(1000) + i64::from(tz_offset_min) * 60).div_euclid(86_400)
}

/// Day number → `YYYY-MM-DD`. `fmt_utc` already does civil-from-days; taking
/// its date half at midnight of that day is the same arithmetic.
pub fn day_label(day: i64) -> String {
    fmt_utc(day * 86_400)[..10].to_string()
}

#[derive(Serialize, PartialEq, Debug)]
pub struct ActRow {
    pub key: String,
    pub total: u64,
    pub per_day: Vec<u64>,
}

#[derive(Serialize, PartialEq, Debug)]
pub struct DwellRow {
    pub surface: String,
    pub ms_total: i64,
    pub per_day: Vec<i64>,
}

#[derive(Serialize, PartialEq, Debug)]
pub struct UiUsage {
    /// oldest first, exactly `days` entries, gaps filled with zeros
    pub days: Vec<String>,
    pub acts: Vec<ActRow>,
    pub dwell: Vec<DwellRow>,
    pub file: String,
}

pub fn read_events(path: &Path) -> Vec<UiEvent> {
    let mut out = Vec::new();
    // Both generations, oldest first. A half-written last line (a crash mid
    // append) is one dropped event, not a failed read — hence per-line parsing.
    for p in [path.with_extension("jsonl.1"), path.to_path_buf()] {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        out.extend(text.lines().filter_map(|l| serde_json::from_str::<UiEvent>(l).ok()).filter(valid_event));
    }
    out
}

/// The whole report, from a set of events. Split out from the command so the
/// tests can drive it without a clock or an AppHandle.
pub fn usage_from(events: &[UiEvent], now_ms: i64, days: u32, tz_offset_min: i32, file: String) -> UiUsage {
    let n = days.clamp(1, 90) as usize;
    let last = day_index(now_ms, tz_offset_min);
    let first = last - (n as i64 - 1);

    let mut acts: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut dwell: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for e in events {
        // A key the file has EVER seen gets a row, even when every one of its
        // events is older than the range — a row of zeros is not noise here,
        // it is the answer: that control has not been touched in a month.
        // (Dwell is the opposite: a surface at zero is a row of nothing, and
        // the bar chart drops it.)
        if e.k == "act" {
            acts.entry(e.key.clone()).or_insert_with(|| vec![0; n]);
        }
        let d = day_index(e.t, tz_offset_min);
        if d < first || d > last {
            continue;
        }
        let col = (d - first) as usize;
        match e.k.as_str() {
            "act" => acts.entry(e.key.clone()).or_insert_with(|| vec![0; n])[col] += 1,
            "dwell" => dwell.entry(e.s.clone()).or_insert_with(|| vec![0; n])[col] += e.ms,
            _ => {}
        }
    }

    let mut act_rows: Vec<ActRow> =
        acts.into_iter().map(|(key, per_day)| ActRow { total: per_day.iter().sum(), key, per_day }).collect();
    // Descending by total, then by key — a stable order, so two runs of the same
    // data draw the same table and a row does not jump between refreshes.
    act_rows.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.key.cmp(&b.key)));

    let mut dwell_rows: Vec<DwellRow> =
        dwell.into_iter().map(|(surface, per_day)| DwellRow { ms_total: per_day.iter().sum(), surface, per_day }).collect();
    dwell_rows.sort_by(|a, b| b.ms_total.cmp(&a.ms_total).then_with(|| a.surface.cmp(&b.surface)));

    UiUsage {
        days: (first..=last).map(day_label).collect(),
        acts: act_rows,
        dwell: dwell_rows,
        file,
    }
}

/// Settings → Usage reads this on open and on every range change. No cache: a
/// month of heavy use is a few hundred KB and the whole pass is one read plus a
/// map, which is cheaper than any invalidation rule would be to get right.
pub fn ui_usage(ctx: &Ctx, days: u32, tz_offset_min: i32) -> Result<UiUsage, String> {
    let path = ui_events_path(ctx)?;
    let events = read_events(&path);
    let now_ms = sysclock::now_epoch() * 1000;
    Ok(usage_from(&events, now_ms, days, tz_offset_min, path.to_string_lossy().into()))
}

/// Both generations. The button that calls this is two-click armed.
pub fn ui_events_clear(ctx: &Ctx) -> Result<(), String> {
    let path = ui_events_path(ctx)?;
    let _guard = UI_EVENTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for p in [path.with_extension("jsonl.1"), path.clone()] {
        if let Err(e) = std::fs::remove_file(&p) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(format!("could not remove {}: {e}", p.display()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The day a click belongs to is the day the USER was having, and the
    /// frontend is the only thing that knows which that is. Same instant,
    /// three timezones, three different columns.
    #[test]
    fn a_day_bucket_is_the_users_day_not_utcs() {
        // 2026-09-05T23:30:00Z
        let late = 1_788_651_000_000i64;
        assert_eq!(day_label(day_index(late, 0)), "2026-09-05");
        assert_eq!(day_label(day_index(late, 120)), "2026-09-06", "UTC+2 is already tomorrow");
        assert_eq!(day_label(day_index(late, -420)), "2026-09-05", "UTC-7 is still today");
        // 2026-09-06T02:00:00Z — the same wall clock from the other side
        let early = late + 2 * 3600 * 1000 + 30 * 60 * 1000;
        assert_eq!(day_label(day_index(early, 0)), "2026-09-06");
        assert_eq!(day_label(day_index(early, -420)), "2026-09-05", "UTC-7 has not gone to bed yet");
    }

    fn ev(k: &str, key: &str, s: &str, t_ms: i64, ms: i64) -> UiEvent {
        UiEvent { t: t_ms, k: k.into(), key: key.into(), s: s.into(), ms }
    }

    /// The one rule this whole feature rests on: what lands in the file is a
    /// name from the source, never something a person typed. Every rejected
    /// shape here is user text wearing a key's clothes.
    #[test]
    fn a_key_is_a_written_name_not_typed_text() {
        for ok in ["nav.row.select", "chord.cmd-b", "dock.files", "main", "a", "files_row"] {
            assert!(valid_token(ok), "{ok} is a hand-written key");
        }
        for bad in [
            "",                                   // no key at all
            "/Users/davidpena/workspace/repo",    // a path
            "feat/redesign",                      // a branch
            "standup and daily work",             // a typed note
            "a\tb",                               // whitespace of any kind
            "café",                               // non-ASCII: user text, or a title
            "…",
        ] {
            assert!(!valid_token(bad), "{bad:?} must be refused");
        }
        // 64 is the cap; 65 is not
        assert!(valid_token(&"a".repeat(64)));
        assert!(!valid_token(&"a".repeat(65)));
        assert!(!valid_token(&"k".repeat(100)));
        // and the whole-event gate rides on it
        assert!(valid_event(&ev("act", "nav.row", "nav", 1, 0)));
        assert!(!valid_event(&ev("act", "nav.row", "nav/x", 1, 0)), "the surface is checked too");
        assert!(!valid_event(&ev("keystroke", "a", "main", 1, 0)), "only act and dwell exist");
        assert!(!valid_event(&ev("act", "nav.row", "nav", 0, 0)), "an event with no clock is not one");
    }

    /// A heatmap with a hole in it is a lie about the days it skipped, so the
    /// report is a dense grid: exactly `days` columns, oldest first, zeros where
    /// nothing happened.
    #[test]
    fn the_report_is_a_dense_grid_with_the_gaps_filled() {
        let day = 86_400_000i64;
        let now = 1_788_651_000_000i64; // 2026-09-05T23:30:00Z
        let events = vec![
            ev("act", "nav.row", "nav", now, 0),
            ev("act", "nav.row", "nav", now, 0),
            ev("act", "nav.row", "nav", now - 2 * day, 0),
            ev("act", "places", "nav", now - 2 * day, 0),
            ev("dwell", "dwell", "main", now, 3000),
            ev("dwell", "dwell", "main", now - day, 1000),
            ev("dwell", "dwell", "home", now, 2000),
            ev("act", "ancient", "nav", now - 30 * day, 0), // outside the range
        ];
        let u = usage_from(&events, now, 3, 0, "/tmp/ui-events.jsonl".into());
        assert_eq!(u.days, vec!["2026-09-03", "2026-09-04", "2026-09-05"]);
        assert_eq!(u.acts.len(), 3);
        assert_eq!(u.acts[0], ActRow { key: "nav.row".into(), total: 3, per_day: vec![1, 0, 2] });
        assert_eq!(u.acts[1], ActRow { key: "places".into(), total: 1, per_day: vec![1, 0, 0] });
        // the whole point of the exercise: a control that HAS been used, but
        // not once in the range, is a row of zeros rather than an absence
        assert_eq!(u.acts[2], ActRow { key: "ancient".into(), total: 0, per_day: vec![0, 0, 0] });
        assert_eq!(u.dwell[0], DwellRow { surface: "main".into(), ms_total: 4000, per_day: vec![0, 1000, 3000] });
        assert_eq!(u.dwell[1], DwellRow { surface: "home".into(), ms_total: 2000, per_day: vec![0, 0, 2000] });
        // and the columns really are the user's days: 23:30Z is already
        // tomorrow at UTC+2, so the whole grid — the labels AND every event in
        // it — slides one day forward rather than the counts moving between
        // columns. A report bucketed in UTC would put today's clicks under
        // yesterday for anyone east of Greenwich after 22:00.
        let shifted = usage_from(&events, now, 3, 120, "/tmp/x".into());
        assert_eq!(shifted.days, vec!["2026-09-04", "2026-09-05", "2026-09-06"]);
        assert_eq!(shifted.acts[0], ActRow { key: "nav.row".into(), total: 3, per_day: vec![1, 0, 2] });
        // one hour further into the past and the oldest pair drops out of the
        // window entirely — the range is inclusive of exactly `days` days
        let narrow = usage_from(&events, now, 2, 0, "/tmp/x".into());
        assert_eq!(narrow.days, vec!["2026-09-04", "2026-09-05"]);
        assert_eq!(narrow.acts[0], ActRow { key: "nav.row".into(), total: 2, per_day: vec![0, 2] });
        assert_eq!(narrow.acts.last().map(|r| (r.key.as_str(), r.total)), Some(("places", 0)),
                   "a key with nothing left in range falls to the bottom at zero");
    }

    /// Append refuses what does not look like a key, and rotates ONCE — the
    /// file is a convenience, not an archive.
    #[test]
    fn the_events_file_refuses_text_and_rotates_once() {
        let dir = std::env::temp_dir().join(format!("wt-uievents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ui-events.jsonl");

        ui_events_append_at(&file, &[
            ev("act", "nav.row", "nav", 1_788_651_000_000, 0),
            ev("act", "/Users/me/workspace/secret-project", "nav", 1_788_651_000_000, 0),
            ev("act", "ok.two", "main", 1_788_651_000_000, 0),
        ]);
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 2, "the path-shaped key must not be stored: {text}");
        assert!(!text.contains("secret-project"), "{text}");
        // `ms` is dwell-only, so an act line does not carry one
        assert!(!text.lines().next().unwrap().contains("\"ms\""), "{text}");

        // over the cap → one generation aside, and the next append starts clean
        std::fs::write(&file, "x".repeat(UI_EVENTS_MAX as usize + 1)).unwrap();
        ui_events_append_at(&file, &[ev("act", "after.rotate", "nav", 1_788_651_000_000, 0)]);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);
        assert!(dir.join("ui-events.jsonl.1").exists(), "the old generation is kept, once");

        // …and a read spans both generations
        std::fs::write(dir.join("ui-events.jsonl.1"), "{\"t\":1788651000000,\"k\":\"act\",\"key\":\"older\",\"s\":\"nav\"}\nnot json\n").unwrap();
        let all = read_events(&file);
        assert_eq!(all.len(), 2, "one from each generation, the torn line dropped");
        assert_eq!(all[0].key, "older", "the older generation comes first");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

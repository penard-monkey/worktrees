//! The one-way channel a Claude session uses to say "show me this document".
//!
//! A session's `worktrees` (the MCP server, or `worktrees show`) and the app are
//! different processes with nothing between them. The app has no inbound
//! surface at all — no URL scheme, no single-instance plugin, no socket, no
//! `notify` watcher; its dependency list is opener/dialog/updater/process/pty
//! and nothing else. So a request has to travel through the filesystem.
//!
//! **Why a drop directory in `~/.cache/worktrees/`, of all places.**
//!
//! Every other piece of app state lives beside `ui-state.json` in Tauri's
//! `app_config_dir()`. The CLI cannot go there: `app_config_dir()` is a Tauri
//! API, and reproducing it means hardcoding the bundle identifier — which would
//! then address the INSTALLED app and never `sandbox.sh --app`, whose identifier
//! is `net.casadelvalle.worktrees.sbx`. That is the one build this repo insists
//! on testing real timing against, so a channel it cannot reach is a channel
//! that cannot be tested. `~/.cache/worktrees/` is identifier-free, derivable
//! from `$HOME` on both sides, and already this repo's word for scratch —
//! `mcp.rs` puts its debug log there and says so.
//!
//! The cost of being identifier-free is that a sandbox build and an installed
//! build share one inbox, and whichever ticks first wins the request. Both are
//! "an app showing you this project", the sandbox only ever runs deliberately,
//! and the fix if it ever bites is a claim file rather than a different
//! directory.
//!
//! **One file per request, never one file rewritten.** Two sessions can ask at
//! once, and a single `inbox.json` would make that a read-modify-write race
//! against a reader that is also deleting it. A directory makes a request a
//! create, makes consumption a delete, and makes concurrency a non-question.
//! Each file is written to a temp name and renamed, so a reader can never see
//! half of one.
//!
//! **A request expires.** `MAX_AGE_SECS` is the whole difference between "show
//! me this" and a queue. If the app is not running, the ask simply does not
//! happen — it must not be replayed onto the screen an hour later when the app
//! next opens, which is what a durable queue would do. The reader deletes
//! stale entries as it finds them, so nothing accumulates either.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How long a request is worth acting on. Comfortably longer than the app's 3 s
/// poll (a request must survive a tick it just missed) and far shorter than the
/// time it takes to wonder why nothing happened.
pub const MAX_AGE_SECS: i64 = 30;

/// A hard cap on what one drain will look at, so a directory that somehow filled
/// up cannot turn the app's 3 s tick into a long read. Anything beyond it is
/// picked up by the next tick — or expires, which is the same thing.
const MAX_DRAIN: usize = 32;

/// One "show me this" — the whole protocol.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Request {
    /// Absolute, canonicalised by the writer. The READER still validates it
    /// against the registered projects before acting: this file is written by
    /// another process, and a path that arrives here has proved nothing.
    pub path: String,
    /// Unix seconds, for the expiry above.
    pub epoch: i64,
    /// The asking process. Carried for the app log — when two sessions ask at
    /// once, "which one" is the first question.
    pub pid: u32,
    /// Where in the file, 1-based. Optional on the wire in BOTH directions: a
    /// request from an older writer has neither key, and a request without a
    /// line omits them rather than writing `null`, so the format is unchanged
    /// for every ask that does not use them (`a_request_without_a_line_is_the_
    /// old_wire_format`). An older app ignores keys it does not know and opens
    /// the file at the top — the right degradation, not an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
}

/// A position in a file, both halves optional. `col` without `line` means
/// nothing and is refused rather than guessed at.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct At {
    pub line: Option<u32>,
    pub col: Option<u32>,
}

impl At {
    fn check(self) -> Result<Self, String> {
        if self.line == Some(0) {
            return Err("line is 1-based; 0 is not a line".to_string());
        }
        if self.col == Some(0) {
            return Err("col is 1-based; 0 is not a column".to_string());
        }
        if self.col.is_some() && self.line.is_none() {
            return Err("col needs a line".to_string());
        }
        Ok(self)
    }
}

/// Split `path:LINE[:COL]` — the form compilers, grep and agents print — into
/// the path and the position. Only when the LITERAL path does not exist: a
/// file may legitimately be called `notes:2`, and that file is what it names.
pub fn split_suffix(arg: &str) -> (PathBuf, At) {
    let whole = (PathBuf::from(arg), At::default());
    if Path::new(arg).exists() {
        return whole;
    }
    let num = |s: &str| (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<u32>().ok()).flatten();
    let mut parts = arg.rsplitn(3, ':');
    let (last, mid, head) = (parts.next(), parts.next(), parts.next());
    match (head, mid.and_then(num), last.and_then(num)) {
        (Some(p), Some(l), Some(c)) if !p.is_empty() => (PathBuf::from(p), At { line: Some(l), col: Some(c) }),
        _ => match arg.rsplit_once(':') {
            Some((p, l)) if !p.is_empty() => match num(l) {
                Some(l) => (PathBuf::from(p), At { line: Some(l), col: None }),
                None => whole,
            },
            _ => whole,
        },
    }
}

/// `~/.cache/worktrees/inbox`. `None` only when `$HOME` is unset.
pub fn dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(Path::new(&home).join(".cache/worktrees/inbox"))
}

/// Ask whoever is watching to open `path`, optionally at a line.
///
/// Canonicalises first, which also means the file must EXIST — asking for a
/// document that is not there is a failure the session can report, rather than
/// a silence the user has to interpret. Returns the file written, for the log.
pub fn request(path: &Path, at: At, now: i64) -> Result<PathBuf, String> {
    let at = at.check()?;
    let canon = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !canon.is_file() {
        return Err(format!("not a file: {}", canon.display()));
    }
    let dir = dir().ok_or_else(|| "HOME is not set".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let pid = std::process::id();
    let req = Request { path: canon.to_string_lossy().to_string(), epoch: now, pid, line: at.line, col: at.col };
    let body = serde_json::to_string(&req).map_err(|e| e.to_string())?;
    // Temp + rename, so a reader mid-`read_dir` never parses a partial file.
    // Same directory, so the rename cannot cross a filesystem boundary.
    let stem = format!("{now}-{pid}");
    let tmp = dir.join(format!(".{stem}.tmp"));
    let dst = dir.join(format!("{stem}.json"));
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", dst.display())
    })?;
    Ok(dst)
}

/// Take everything waiting, oldest first, and leave the directory empty.
///
/// Consumption is a DELETE and it happens whether or not the entry parses or is
/// still fresh — a file that cannot be read is a file that would otherwise be
/// re-read on every 3 s tick forever. Expired entries are dropped silently;
/// that is the design, not a lost message (see the module note).
///
/// Never errors: a missing directory is an empty inbox, which is the normal
/// state.
pub fn drain(now: i64) -> Vec<Request> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    // The name begins with the epoch, so lexical order is close enough to
    // chronological for a handful of entries — and a deterministic order beats
    // `read_dir`'s, which is the filesystem's business.
    names.sort();
    let mut out = Vec::new();
    for p in names.into_iter().take(MAX_DRAIN) {
        let body = std::fs::read_to_string(&p).ok();
        let _ = std::fs::remove_file(&p);
        let Some(req) = body.and_then(|b| serde_json::from_str::<Request>(&b).ok()) else {
            continue;
        };
        if now - req.epoch > MAX_AGE_SECS || req.epoch > now + MAX_AGE_SECS {
            continue; // expired, or a clock that disagrees enough to be noise
        }
        out.push(req);
    }
    out
}

/// `worktrees show <path> [--line N]` — ask the app to open a file, at a line.
///
/// Refuses a path outside the project it was run in, for the same reason no MCP
/// tool takes a repo path: the thing holding the request is a session that lives
/// in ONE checkout, and "show me a file" must not be a way to point the app at
/// somebody else's tree. The app re-validates anyway (it is another process's
/// word), but refusing at the asking end is what produces an error message
/// instead of a silence.
pub fn cmd_show(project: &crate::Project, ui: &mut dyn crate::ui::Ui, args: &[String]) -> i32 {
    const USAGE: &str = "usage: worktrees show <file>[:line[:col]] [--line N] [--col N]";
    let (arg, flags) = match parse_show_args(args) {
        Ok(v) => v,
        Err(e) => {
            ui.error(&format!("{e}\n{USAGE}"));
            return 1;
        }
    };
    let Some(arg) = arg else {
        ui.error(USAGE);
        return 1;
    };
    // An explicit flag wins over a `:42` suffix; the suffix is only split off
    // when the literal name does not exist (`split_suffix`).
    let (path, suffix) = split_suffix(&arg);
    let at = if flags.line.is_some() { flags } else { At { line: suffix.line, col: flags.col.or(suffix.col) } };
    let target = match std::fs::canonicalize(&path) {
        Ok(t) => t,
        Err(e) => {
            ui.error(&format!("{}: {e}", path.display()));
            return 1;
        }
    };
    let root = std::fs::canonicalize(&project.main_root).unwrap_or_else(|_| PathBuf::from(&project.main_root));
    if !target.starts_with(&root) {
        ui.error(&format!("{} is outside {}", target.display(), root.display()));
        return 1;
    }
    match request(&target, at, crate::sysclock::now_epoch()) {
        Ok(_) => {
            // Deliberately not "opened": nothing here knows whether the app is
            // running, and claiming a result this process cannot observe is how
            // a silent failure gets reported as a success.
            let pos = match (at.line, at.col) {
                (Some(l), Some(c)) => format!(" at line {l}, column {c}"),
                (Some(l), None) => format!(" at line {l}"),
                _ => String::new(),
            };
            ui.info(&format!("asked the app to show {}{pos}", target.display()));
            0
        }
        Err(e) => {
            ui.error(&e);
            1
        }
    }
}

/// `<file> [--line N] [--col N]`, flags in either `--line N` or `--line=N`
/// form and on either side of the file.
fn parse_show_args(args: &[String]) -> Result<(Option<String>, At), String> {
    let mut file = None;
    let mut at = At::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        let slot = match name {
            "--line" => &mut at.line,
            "--col" | "--column" => &mut at.col,
            _ if a.starts_with('-') => return Err(format!("unknown option {a}")),
            _ => {
                if file.replace(a.clone()).is_some() {
                    return Err("one file at a time".to_string());
                }
                continue;
            }
        };
        let v = inline.or_else(|| it.next().cloned()).ok_or_else(|| format!("{name} needs a number"))?;
        *slot = Some(v.parse::<u32>().map_err(|_| format!("{name}: not a number: {v}"))?);
    }
    Ok((file, at))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test gets its own `$HOME`, since `dir()` reads it. Serialised by the
    /// lock below: `set_var` is process-global, so two of these in parallel
    /// would each drain the other's inbox.
    struct Home(PathBuf);
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn home(tag: &str) -> Home {
        let d = std::env::temp_dir().join(format!("wtinbox-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::env::set_var("HOME", &d);
        Home(d)
    }

    fn doc(h: &Home, name: &str) -> PathBuf {
        let p = h.0.join(name);
        std::fs::write(&p, "# hi").unwrap();
        p
    }

    #[test]
    fn a_request_round_trips_and_the_inbox_is_left_empty() {
        let _g = HOME_LOCK.lock().unwrap();
        let h = home("roundtrip");
        let f = doc(&h, "CLAUDE.md");
        request(&f, At::default(), 1000).unwrap();
        let got = drain(1005);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, std::fs::canonicalize(&f).unwrap().to_string_lossy());
        assert_eq!(got[0].pid, std::process::id());
        // Consumed, not re-served: a second drain must be empty, or every tick
        // would re-open the same document forever.
        assert!(drain(1005).is_empty(), "drain must consume");
        assert!(std::fs::read_dir(dir().unwrap()).unwrap().next().is_none(), "no files left behind");
    }

    /// The difference between "show me this" and a queue. Open the app an hour
    /// after the ask and nothing should appear.
    #[test]
    fn a_stale_request_is_dropped_and_deleted() {
        let _g = HOME_LOCK.lock().unwrap();
        let h = home("stale");
        let f = doc(&h, "old.md");
        request(&f, At::default(), 1000).unwrap();
        assert!(drain(1000 + MAX_AGE_SECS + 1).is_empty(), "expired request must not be served");
        assert!(
            std::fs::read_dir(dir().unwrap()).unwrap().next().is_none(),
            "an expired request must still be deleted, or it is re-read every tick forever"
        );
    }

    #[test]
    fn two_requests_both_arrive_oldest_first() {
        let _g = HOME_LOCK.lock().unwrap();
        let h = home("two");
        let a = doc(&h, "a.md");
        let b = doc(&h, "b.md");
        request(&a, At::default(), 1000).unwrap();
        request(&b, At::default(), 1001).unwrap();
        let got = drain(1002);
        assert_eq!(got.len(), 2, "a drop directory must not make two asks collide");
        assert!(got[0].path.ends_with("a.md"), "oldest first, got {:?}", got[0].path);
    }

    /// A line (and column) rides along, so "open ops.rs at 1022" lands on the
    /// line rather than at the top of a 3000-line file.
    #[test]
    fn a_line_and_column_round_trip() {
        let _g = HOME_LOCK.lock().unwrap();
        let h = home("line");
        let f = doc(&h, "ops.rs");
        request(&f, At { line: Some(1022), col: Some(7) }, 1000).unwrap();
        let got = drain(1001);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].line, got[0].col), (Some(1022), Some(7)));
    }

    /// Both directions across a version skew. An OLD writer's request has no
    /// line and must still parse in a new app; a NEW request without one must
    /// not grow `"line":null`, so an old app's parser — which ignores unknown
    /// keys but would still be handed noise — sees exactly what it always did.
    #[test]
    fn a_request_without_a_line_is_the_old_wire_format() {
        let old = r#"{"path":"/x/CLAUDE.md","epoch":1000,"pid":7}"#;
        let r: Request = serde_json::from_str(old).expect("an old request must still parse");
        assert_eq!((r.line, r.col), (None, None));
        assert_eq!(serde_json::to_string(&r).unwrap(), old, "no line → byte-identical to the old format");
    }

    #[test]
    fn a_zero_line_is_refused() {
        let _g = HOME_LOCK.lock().unwrap();
        let h = home("zero");
        let f = doc(&h, "a.rs");
        let e = request(&f, At { line: Some(0), col: None }, 1000).expect_err("lines are 1-based");
        assert!(e.contains("line"), "{e}");
        let e = request(&f, At { line: None, col: Some(3) }, 1000).expect_err("a column needs a line");
        assert!(e.contains("col"), "{e}");
    }

    /// `worktrees show src/foo.ts:42:7` — the form every compiler and agent
    /// prints — is the same ask as `--line 42 --col 7`, but only when the
    /// literal name does not exist (a file CAN be called `x:2`).
    #[test]
    fn a_path_line_suffix_is_split_only_when_the_literal_path_is_missing() {
        let _g = HOME_LOCK.lock().unwrap();
        let h = home("suffix");
        let f = doc(&h, "foo.ts");
        let s = f.to_string_lossy().to_string();
        assert_eq!(split_suffix(&format!("{s}:42")), (PathBuf::from(&s), At { line: Some(42), col: None }));
        assert_eq!(split_suffix(&format!("{s}:42:7")), (PathBuf::from(&s), At { line: Some(42), col: Some(7) }));
        let odd = doc(&h, "x:2");
        let o = odd.to_string_lossy().to_string();
        assert_eq!(split_suffix(&o), (odd.clone(), At::default()), "an existing file named x:2 is that file");
        assert_eq!(split_suffix(&s), (f.clone(), At::default()));
    }

    #[test]
    fn show_flags_parse_in_either_form_and_either_order() {
        let v = |a: &[&str]| parse_show_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(v(&["f.rs", "--line", "4"]).unwrap(), (Some("f.rs".into()), At { line: Some(4), col: None }));
        assert_eq!(v(&["--line=4", "--col=2", "f.rs"]).unwrap(), (Some("f.rs".into()), At { line: Some(4), col: Some(2) }));
        assert!(v(&["f.rs", "--line"]).is_err(), "a flag with no value");
        assert!(v(&["f.rs", "--line", "x"]).is_err(), "not a number");
        assert!(v(&["f.rs", "--nope"]).is_err(), "unknown flag");
        assert!(v(&["a", "b"]).is_err(), "two files");
    }

    #[test]
    fn a_missing_file_is_refused_at_the_asking_end() {
        let _g = HOME_LOCK.lock().unwrap();
        let h = home("missing");
        let e = request(&h.0.join("nope.md"), At::default(), 1000).expect_err("a file that is not there cannot be shown");
        assert!(e.contains("nope.md"), "the error must name the path: {e}");
    }

    /// Junk must not wedge the tick. A file that cannot be parsed has to be
    /// deleted anyway, or it is re-read every 3 s for as long as the app runs.
    #[test]
    fn an_unparseable_entry_is_swallowed_and_removed() {
        let _g = HOME_LOCK.lock().unwrap();
        let _h = home("junk");
        let d = dir().unwrap();
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("1000-1.json"), "{ not json").unwrap();
        assert!(drain(1000).is_empty());
        assert!(std::fs::read_dir(&d).unwrap().next().is_none(), "junk must be consumed too");
    }
}

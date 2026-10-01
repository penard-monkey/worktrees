//! Claude plan usage — the reader, moved out of the app.
//!
//! It lived in `app/src-tauri` because the nav footer was its only consumer.
//! The quota gate in `ops::cmd_new` is the second: to choose between providers
//! by headroom, or to refuse a spawn onto a spent one, the SPAWN path has to
//! read this — and the CLI can reach nothing under `app/src-tauri`. Same move,
//! and the same reason, as `codex_usage`.
//!
//! Nothing about how it works changed: still `security` for the keychain token
//! and `curl` for the GET (no HTTP crate, no second TLS stack), still the same
//! TTL, backoff and statusline fallback, still "missing data is not an error".
//!
//! `applog` became `logsink::log()`, the engine's one installable sink — the
//! app hands its logger over at startup, the CLI leaves it unset, and unset
//! means "do not log", never a reason to fail.

use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

use crate::logsink::log;

// ── Claude plan usage (nav footer widget) ────────────────────────────────────
// Same bars Claude Code's /usage panel shows: the 5h session window, the weekly
// all-models window, and any model-scoped weekly bucket (e.g. "Fable").
//
// Primary source is the OAuth usage endpoint the TUI itself calls — free GET, no
// quota, but undocumented and unversioned, so EVERY step degrades instead of
// failing: the authoritative field is `limits[]` and the rest of the payload is
// full of experimental nulls we deliberately ignore. Token comes from the macOS
// Keychain via `security` (shelled out — house style, and it keeps the secret out
// of our address space longer than a lib would). Claude Code rotates the token
// ~hourly, so a 401 buys exactly one retry with a freshly re-read token.
//
// Fallback is the statusline widget's local snapshot (`~/.claude/widgets/
// rate_limits.json`, written by whatever statusline script the user runs) —
// session + weekly only, no model bucket, and only as fresh as the last Claude
// Code session, hence the file mtime as `fetched_at` and the dimmed UI.
//
// Missing data is NOT an error: `source: "unavailable"` with no limits just
// hides the widget. The reason still lands in app.log.
//
// And a failed fetch degrades before it hides — see `usage_degraded`. Hiding is
// the answer only when there is no reading left anywhere, because the widget
// disappearing reads as "something is broken in the app" when what happened is
// "one GET came back 429".

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Claude Code's own UA. Load-bearing: a foreign UA lands in a much more
/// aggressive rate-limit bucket on this endpoint.
const USAGE_UA: &str = "claude-code/2.1.220";
/// Minimum gap between real fetches. The frontend polls at 180s; this is the
/// floor that also covers window-focus pulls and a re-mount storm.
const USAGE_TTL_SECS: i64 = 120;
/// The same floor for a FAILED attempt. Without it the TTL only throttled the
/// happy path — nothing was cached on failure, so every focus pull went straight
/// back out to an endpoint that had just refused us. On 2026-09-19 that turned
/// one 429 into 15 real fetches inside a single minute (⌘-tabbing re-runs the
/// poll effect AND fires the `focus` listener), which is how a rate limit gets
/// held open by the thing complaining about it. Half the poll period, so a
/// recovered endpoint is still picked up by the next pull rather than the one
/// after it.
const USAGE_FAIL_TTL_SECS: i64 = 60;
/// How long the last good reading may stand in for a live one. The bars measure
/// 5h and 7d windows, so a reading of this age is still the right shape — but it
/// is bounded, because a percentage from before a window rolled over is not
/// stale data, it is wrong data.
///
/// The same bargain `STATUS_STALE_MAX_SECS` strikes for the service-status
/// probe, at the same 30 minutes and with the same inclusive bound — this
/// widget simply took longer to learn it. Deliberately a SECOND constant and
/// not a shared one: they answer for different data, and a future change to one
/// window must not silently move the other.
const USAGE_STALE_MAX_SECS: i64 = 1800;

#[derive(Serialize, Clone)]
pub struct UsageLimit {
    kind: String,     // session | weekly_all | weekly_scoped
    label: String,    // "Session" | "Weekly" | model display name ("Fable")
    percent: f64,
    severity: String, // normal | warning | … (rendered as a color tier)
    resets_at: Option<i64>, // unix seconds
}

#[derive(Serialize, Clone)]
pub struct UsageInfo {
    source: String, // oauth | cached | statusline | unavailable
    fetched_at: i64,
    limits: Vec<UsageLimit>,
}

/// Last SUCCESSFUL oauth answer, with the epoch it was fetched at (see TTL).
static USAGE_CACHE: Mutex<Option<UsageInfo>> = Mutex::new(None);
/// Epoch of the last FAILED real attempt — the negative half of the TTL below.
static USAGE_FAIL_AT: Mutex<Option<i64>> = Mutex::new(None);

/// The Claude Code OAuth access token, straight out of the login Keychain item.
/// None on any failure (not macOS, item absent, locked keychain, shape changed).
fn claude_oauth_token() -> Option<String> {
    let mut cmd = std::process::Command::new("/usr/bin/security");
    cmd.args(["find-generic-password", "-s", "Claude Code-credentials", "-w"]);
    let out = crate::proc::run_deadline(cmd, 6).ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("claudeAiOauth")?.get("accessToken")?.as_str().map(String::from)
}

/// GET the usage endpoint → (http status, body). curl, not a HTTP crate: the app
/// already shells out to curl for the release check and this keeps the dep tree
/// (and the TLS stack) exactly where it is.
fn usage_get(token: &str) -> Result<(u16, String), String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-s", "--max-time", "10", "-w", "\n%{http_code}"])
        .arg("-H")
        .arg(format!("Authorization: Bearer {token}"))
        .arg("-H")
        .arg("anthropic-beta: oauth-2025-04-20")
        .arg("-H")
        .arg(format!("User-Agent: {USAGE_UA}"))
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg(USAGE_URL);
    let out = crate::proc::run_deadline(cmd, 15).map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("curl exited {}", out.status.code().unwrap_or(-1)));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    // -w appended "\n<code>" AFTER the body; the body itself may contain newlines.
    let cut = text.rfind('\n').ok_or("curl produced no status line")?;
    let code: u16 = text[cut + 1..].trim().parse().map_err(|_| "curl produced no status code".to_string())?;
    Ok((code, text[..cut].to_string()))
}

/// `limits[]` → our rows. Unknown kinds and unparseable entries are SKIPPED, not
/// fatal — the endpoint ships experimental buckets we've never seen. `None` only
/// when there is no `limits` array at all (i.e. the shape moved under us).
fn parse_usage_limits(body: &str) -> Option<Vec<UsageLimit>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let arr = v.get("limits")?.as_array()?;
    let mut out = Vec::new();
    for e in arr {
        let kind = e.get("kind").and_then(|k| k.as_str()).unwrap_or_default();
        let label = match kind {
            "session" => "Session".to_string(),
            "weekly_all" => "Weekly".to_string(),
            // the model bar (e.g. "Fable") — no name, no row
            "weekly_scoped" => match e.pointer("/scope/model/display_name").and_then(|d| d.as_str()) {
                Some(n) => n.to_string(),
                None => continue,
            },
            _ => continue,
        };
        let percent = match e.get("percent").and_then(|p| p.as_f64()) {
            Some(p) => p,
            None => continue,
        };
        out.push(UsageLimit {
            kind: kind.to_string(),
            label,
            percent,
            severity: e.get("severity").and_then(|s| s.as_str()).unwrap_or("normal").to_string(),
            resets_at: e.get("resets_at").and_then(|r| r.as_str()).and_then(crate::sysclock::parse_iso8601),
        });
    }
    Some(out)
}

fn usage_from_oauth(now: i64) -> Result<UsageInfo, String> {
    let token = claude_oauth_token().ok_or("keychain: no Claude Code credentials")?;
    let (mut code, mut body) = usage_get(&token)?;
    if code == 401 {
        // token rotated under us (Claude Code refreshes ~hourly) — re-read once
        let fresh = claude_oauth_token().ok_or("keychain: re-read failed after 401")?;
        let retry = usage_get(&fresh)?;
        code = retry.0;
        body = retry.1;
    }
    if code != 200 {
        return Err(format!("usage endpoint http {code}"));
    }
    let limits = parse_usage_limits(&body).ok_or("usage response carries no `limits` array")?;
    Ok(UsageInfo { source: "oauth".into(), fetched_at: now, limits })
}

/// The statusline widget's local snapshot: `{"five_hour":{"used_percentage":46,
/// "resets_at":<epoch secs>}, "seven_day":{…}}`. No severity and no model bucket
/// — the frontend dims the whole widget for this source.
fn usage_from_statusline() -> Option<UsageInfo> {
    let home = std::env::var("HOME").ok()?;
    let path = Path::new(&home).join(".claude/widgets/rate_limits.json");
    let fetched_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_else(crate::sysclock::now_epoch);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    let mut limits = Vec::new();
    for (key, kind, label) in [("five_hour", "session", "Session"), ("seven_day", "weekly_all", "Weekly")] {
        let bucket = match v.get(key) {
            Some(b) => b,
            None => continue,
        };
        let percent = match bucket.get("used_percentage").and_then(|p| p.as_f64()) {
            Some(p) => p,
            None => continue,
        };
        limits.push(UsageLimit {
            kind: kind.into(),
            label: label.into(),
            percent,
            severity: "normal".into(),
            resets_at: bucket.get("resets_at").and_then(|r| r.as_i64()),
        });
    }
    if limits.is_empty() {
        return None;
    }
    Some(UsageInfo { source: "statusline".into(), fetched_at, limits })
}

/// Is a real fetch still being held off after a failure? Its own function so
/// the boundary is assertable — the whole defect was a floor that existed for
/// successes and not for failures, and a silently drifting constant here brings
/// it straight back.
fn usage_in_backoff(now: i64, failed_at: Option<i64>) -> bool {
    failed_at.is_some_and(|t| now - t < USAGE_FAIL_TTL_SECS)
}

/// The answer when there is no live one: the best reading we still hold, marked
/// as not-live so the frontend dims it.
///
/// This exists because "the endpoint said no" used to mean the widget VANISHED.
/// A 429 that lasted four minutes took the rail tile with it, and a reading 130
/// seconds old — indistinguishable from a fresh one at the resolution of a 4px
/// bar — was thrown away to show nothing at all. Degrading to it keeps the
/// answer on screen and keeps the layout still.
///
/// The cached reading wins while it is inside the ceiling, even against a
/// FRESHER statusline snapshot — the one place this does not simply prefer
/// newer data, and deliberately.
///
/// The snapshot is often the newer of the two: the statusline rewrites
/// `rate_limits.json` on every Claude Code prompt, while the cached reading is
/// by construction at least `USAGE_TTL_SECS` old by the time this runs. But it
/// is a POORER SHAPE — two rows instead of three, no severity tiers, no model
/// bucket. Preferring it would drop a bar and lose the amber tier for the
/// length of an outage and then put them back, which is the layout moving
/// exactly when this function exists to hold it still. The percentages here
/// measure 5h and 7d windows; between a reading two minutes old and one five
/// seconds old there is nothing to choose, and the ceiling already guarantees
/// the older one is not misleading. So: same shape as the live widget for as
/// long as that is honest, and the snapshot as the fallback for when it is not.
fn usage_degraded(now: i64, cached: Option<UsageInfo>, snapshot: Option<UsageInfo>) -> UsageInfo {
    match cached.filter(|c| now - c.fetched_at <= USAGE_STALE_MAX_SECS) {
        Some(c) => UsageInfo { source: "cached".into(), ..c },
        None => snapshot
            .unwrap_or(UsageInfo { source: "unavailable".into(), fetched_at: now, limits: Vec::new() }),
    }
}
/// Accessors for `quota`, which has to weigh these against Codex's. Fields stay
/// private: this type is serialised to the frontend and its shape is a contract.
impl UsageLimit {
    pub fn percent(&self) -> f64 { self.percent }
    pub fn severity(&self) -> &str { &self.severity }
    pub fn label(&self) -> &str { &self.label }
    pub fn resets_at(&self) -> Option<i64> { self.resets_at }

    /// The model this bucket is scoped to, when it is one.
    ///
    /// `weekly_scoped` is the only kind that names a model, and for that kind
    /// the LABEL is the model's display name — `parse_usage_limits` puts
    /// `/scope/model/display_name` there. Load-bearing for the launch gate: a
    /// weekly "Fable" bucket at 85% says nothing about a lane starting on
    /// Opus, and grading it against one would refuse a launch that shares none
    /// of the spent allowance (`quota::worst_window`).
    pub fn scoped_model(&self) -> Option<&str> {
        (self.kind == "weekly_scoped").then_some(self.label.as_str())
    }
}

/// Constructors for tests in sibling modules (`harness`'s window mappings).
#[cfg(test)]
impl UsageLimit {
    pub(crate) fn for_test(kind: &str, label: &str, percent: f64, severity: &str, resets_at: Option<i64>) -> Self {
        Self { kind: kind.into(), label: label.into(), percent, severity: severity.into(), resets_at }
    }
}

#[cfg(test)]
impl UsageInfo {
    pub(crate) fn for_test(limits: Vec<UsageLimit>) -> Self {
        Self { source: "oauth".into(), fetched_at: 0, limits }
    }
}

impl UsageInfo {
    pub fn limits(&self) -> &[UsageLimit] { &self.limits }
}

/// The public read: the tauri command is now a one-liner over this, and the
/// quota gate calls it directly.
pub fn read() -> UsageInfo {
    let now = crate::sysclock::now_epoch();
    let cached = USAGE_CACHE.lock().unwrap().clone();
    if let Some(c) = cached.as_ref() {
        if now - c.fetched_at < USAGE_TTL_SECS {
            return c.clone();
        }
    }
    // The negative half of the TTL. Note it is checked AFTER the positive one:
    // a failure never shortens the life of a good reading.
    if usage_in_backoff(now, *USAGE_FAIL_AT.lock().unwrap()) {
        // Silent on purpose: one log line per real attempt, not one per pull.
        // The suppressed pulls ARE the bug this is fixing, and logging them
        // would reproduce its shape in the file used to diagnose it.
        return usage_degraded(now, cached, usage_from_statusline());
    }
    match usage_from_oauth(now) {
        Ok(info) => {
            *USAGE_CACHE.lock().unwrap() = Some(info.clone());
            *USAGE_FAIL_AT.lock().unwrap() = None;
            info
        }
        Err(why) => {
            *USAGE_FAIL_AT.lock().unwrap() = Some(now);
            // Re-read the cache rather than degrading from the copy taken at
            // the top: curl just spent up to 15 seconds, and two pulls overlap
            // ROUTINELY here — the focus listener and the interval both reach
            // this reader, which is the doubled-pull shape the backoff above
            // exists to damp. If the other one succeeded while we were out, the
            // cache now holds a reading fresher than anything we could degrade
            // to, and the frontend takes whichever response lands LAST: serving
            // the pre-fetch copy would dim a live widget — or hide it outright
            // on a cold start — for up to a full poll period, purely because
            // the loser of a race answered second.
            let cached = USAGE_CACHE.lock().unwrap().clone();
            // …and if that reading is inside the positive TTL, it is not a
            // fallback at all. Answer exactly as the check at the top of this
            // function would have, had we arrived a moment later.
            if let Some(c) = cached.as_ref() {
                if now - c.fetched_at < USAGE_TTL_SECS {
                    log("warn", &format!("claude_usage: oauth unavailable: {why} — another pull succeeded, serving that"));
                    return c.clone();
                }
            }
            let out = usage_degraded(now, cached, usage_from_statusline());
            // What the user will SEE, next to why — the old pair of lines said
            // "widget hidden" unconditionally, which was about to stop being
            // true for most failures.
            let shown = match out.source.as_str() {
                "unavailable" => "widget hidden".to_string(),
                src => format!("showing {src} reading from {}s ago", now - out.fetched_at),
            };
            log("warn", &format!("claude_usage: oauth unavailable: {why} — {shown}"));
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Moved here with the code they test. They were written after an episode on
    // 2026-09-19 where the oauth endpoint 429'd for ~4m45s and the rail tile
    // vanished for all of it while the app spent 15 real fetches in one minute
    // keeping the rate limit warm — the degrade ladder is the answer to that,
    // and it is worth more than the module it happens to live in.
        // ── Claude plan usage: what a failed fetch answers ──────────────────────
        //
        // Written after an episode on 2026-09-19 where the oauth endpoint 429'd for
        // ~4m45s and the rail tile disappeared for the whole of it, while the app
        // spent 15 real fetches inside one minute keeping the rate limit warm.

        fn reading(source: &str, fetched_at: i64, pct: f64) -> UsageInfo {
            UsageInfo {
                source: source.into(),
                fetched_at,
                limits: vec![UsageLimit {
                    kind: "session".into(),
                    label: "Session".into(),
                    percent: pct,
                    severity: "normal".into(),
                    resets_at: None,
                }],
            }
        }

        /// The fix, stated: a failed poll keeps the numbers on screen. `source`
        /// changes so the frontend dims it and names what it is showing; the
        /// reading itself — including `fetched_at`, which the tooltip prints — is
        /// untouched, because inventing a fresh timestamp for old data would be the
        /// one thing worse than hiding it.
        #[test]
        fn a_failed_fetch_degrades_to_the_last_good_reading() {
            let out = usage_degraded(1_000, Some(reading("oauth", 870, 35.0)), None);
            assert_eq!(out.source, "cached");
            assert_eq!(out.fetched_at, 870, "the age is the reading's, not now's");
            assert_eq!(out.limits[0].percent, 35.0);
        }

        /// Bounded, though: the bars measure a 5h and a 7d window, so a reading
        /// from before one rolled over is not stale, it is wrong. Inclusive at the
        /// bound, like `stale_or_silence` — the sibling probe that already made
        /// this bargain, and whose boundary test this one is written to match.
        #[test]
        fn a_reading_past_the_stale_ceiling_is_dropped() {
            let at = |age: i64| Some(reading("oauth", 1_000 - age, 35.0));
            assert_eq!(usage_degraded(1_000, at(USAGE_STALE_MAX_SECS), None).source, "cached", "at the bound, inclusive");
            assert_eq!(usage_degraded(1_000, at(USAGE_STALE_MAX_SECS + 1), None).source, "unavailable");
        }

        /// The cached reading beats a statusline snapshot even when the snapshot is
        /// NEWER, which is the one place this does not prefer fresher data. The
        /// statusline rewrites its file on every Claude Code prompt, so on a machine
        /// that runs one the snapshot is almost always newer — and it is two rows
        /// with no severity and no model bucket. Preferring it would drop a bar for
        /// the length of an outage and put it back afterwards, which is the layout
        /// moving exactly when this function exists to hold it still.
        #[test]
        fn the_cached_reading_outranks_a_fresher_snapshot() {
            let out = usage_degraded(1_000, Some(reading("oauth", 800, 35.0)), Some(reading("statusline", 995, 61.0)));
            assert_eq!(out.source, "cached");
            assert_eq!(out.limits[0].percent, 35.0, "…and it is the cached NUMBERS, not just the label");
            // once it is past the ceiling the snapshot is all that is left, and then
            // it is served whatever its age
            let out = usage_degraded(1_000, Some(reading("oauth", 1_000 - USAGE_STALE_MAX_SECS - 1, 35.0)), Some(reading("statusline", 995, 61.0)));
            assert_eq!(out.source, "statusline");
            assert_eq!(out.limits[0].percent, 61.0);
        }

        /// Hiding is still the answer when there is nothing left to show — and a
        /// statusline snapshot on its own is still served, ceiling or no ceiling:
        /// it carries its own honest mtime and has always been allowed to be old.
        #[test]
        fn with_nothing_held_the_widget_still_hides() {
            assert_eq!(usage_degraded(1_000, None, None).source, "unavailable");
            assert!(usage_degraded(1_000, None, None).limits.is_empty());
            let ancient = usage_degraded(1_000, None, Some(reading("statusline", 1, 61.0)));
            assert_eq!(ancient.source, "statusline");
        }

        /// The other half: a failure now buys the same silence a success does.
        /// Before this, nothing was cached on failure, so the 120s floor never
        /// applied and every window-focus pull went straight back to the endpoint.
        #[test]
        fn a_failure_holds_off_the_next_fetch() {
            assert!(!usage_in_backoff(1_000, None), "no failure recorded, no backoff");
            assert!(usage_in_backoff(1_000, Some(1_000 - USAGE_FAIL_TTL_SECS + 1)));
            assert!(!usage_in_backoff(1_000, Some(1_000 - USAGE_FAIL_TTL_SECS)), "the floor is a floor, not a ceiling");
            // the shape that broke: a burst of pulls seconds after a 429
            assert!(usage_in_backoff(1_000, Some(999)));
        }

}

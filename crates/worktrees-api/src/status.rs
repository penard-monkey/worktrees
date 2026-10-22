//! What the app reports about itself and its world: Claude's plan usage,
//! Claude's service status, and the published releases it could move to.
//!
//! `install_app_version` is NOT here. It drives `tauri_plugin_updater`
//! against a specific release's manifest, which is the app updating its own
//! bundle — client-local by §III.4.B, and meaningless to a server whose
//! clients are browsers.

use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use worktrees_core::{codex_usage, sysclock};

use crate::log::log as applog;
// `check_update`/`update_cli` report on the same release list the update-check
// helpers resolve, which live beside the verbs that use them.
use crate::ops::{latest_release_tag, CmdResult, UpdateInfo, REPO_SLUG};
use worktrees_core::proc::run_deadline;

use worktrees_core::claude_usage::{self, UsageInfo};

/// Plan-usage bars for the nav footer. Never Errs on "no data" — the widget is
/// ambient, and a banner for a background poll would be worse than a blank
/// corner; the reason goes to app.log instead.
pub fn claude_usage() -> Result<UsageInfo, String> {
    Ok(claude_usage::read())
}

/// kimi's plan windows (core `kimi_usage`). Blocking: it spawns `kimi web`,
/// asks once and ends it, and core caches the answer for 120s, so focus pulls
/// cannot start more than one server in that window. Never Errs, like the
/// other two readers.
pub fn kimi_usage() -> Result<worktrees_core::kimi_usage::Info, String> {
    Ok(worktrees_core::kimi_usage::read())
}

pub fn codex_usage() -> Result<codex_usage::Info, String> {
    // The neutral probe dir is core's now, and shared: the CLI and MCP read
    // usage too, and two definitions of "somewhere that is not the repo" is one
    // too many. See `codex_usage::probe_cwd` for why it is not `app_cache_dir`.
    let cwd = codex_usage::probe_cwd().ok_or("usage cache directory unavailable")?;
    Ok(codex_usage::read(cwd))
}


pub const STATUS_URL: &str = "https://status.claude.com/api/v2/summary.json";
/// The status page's URL for a human, carried in the payload so the frontend
/// never hardcodes a second copy of it.
pub const STATUS_PAGE: &str = "https://status.claude.com";
/// (component id, name prefix). Order is the display order.
pub const STATUS_WATCHED: [(&str, &str); 2] = [
    ("yyzkbfz2thpt", "Claude Code"),
    ("k8w3r06qmzrp", "Claude API"),
];
/// Minimum gap between real fetches. Longer than usage's 120s: a status page
/// changes on human timescales (an incident is minutes old before it is
/// posted), and this is a poll nobody asked for.
pub const STATUS_TTL_SECS: i64 = 240;
/// How long an EXPIRED answer may still be served when a fetch fails. A pull
/// that lands before the network is back — waking from a closed lid is the
/// common one, and the window-focus pull makes it likely — would otherwise turn
/// a live incident into silence and the indicator would blink out and back.
/// Bounded, because the opposite failure is worse: a cached "Claude Code is
/// down" held over a long disconnection would still be on screen hours after
/// the outage ended. Past this, silence is the honest answer.
pub const STATUS_STALE_MAX_SECS: i64 = 1800;

#[derive(Serialize, Clone, PartialEq)]
pub struct StatusComponent {
    pub name: String,
    /// Statuspage's own vocabulary, verbatim: operational | degraded_performance
    /// | partial_outage | major_outage | under_maintenance.
    pub status: String,
}

#[derive(Serialize, Clone, PartialEq)]
pub struct StatusIncident {
    pub name: String,
    /// investigating | identified | monitoring | resolved
    pub status: String,
    /// The latest update's body, which is the one sentence worth showing.
    pub body: Option<String>,
    pub updated_at: Option<i64>,
    pub url: Option<String>,
}

#[derive(Serialize, Clone, PartialEq)]
pub struct ClaudeStatus {
    pub source: String, // live | unavailable
    pub fetched_at: i64,
    /// none | degraded | down — the only field the indicator's visibility reads.
    pub severity: String,
    pub components: Vec<StatusComponent>,
    /// How many components the page carries in total. Only the footer line
    /// "watching 2 of 6" reads it — but that line is the one place the UI
    /// admits its own scope, and a hardcoded 6 would quietly start lying.
    pub total: usize,
    pub incident: Option<StatusIncident>,
    pub page_url: String,
}

pub static STATUS_CACHE: Mutex<Option<ClaudeStatus>> = Mutex::new(None);

/// A component's status → 0 none / 1 degraded / 2 down. An UNKNOWN value counts
/// as degraded rather than as operational: if Statuspage adds a state, the
/// honest default is "something is off", not silence.
pub fn status_rank(s: &str) -> u8 {
    match s {
        "operational" => 0,
        "major_outage" => 2,
        _ => 1,
    }
}

pub fn status_word(rank: u8) -> &'static str {
    match rank {
        0 => "none",
        2 => "down",
        _ => "degraded",
    }
}

/// `summary.json` → our payload. Pure, so the mapping is unit-tested against a
/// captured body rather than against the live page.
pub fn status_parse(body: &str, now: i64) -> Result<ClaudeStatus, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("bad json: {e}"))?;
    let comps = v
        .get("components")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "no components array".to_string())?;

    let mut watched_ids: Vec<String> = Vec::new();
    let mut out: Vec<StatusComponent> = Vec::new();
    for (id, prefix) in STATUS_WATCHED {
        let hit = comps
            .iter()
            .find(|c| c.get("id").and_then(|x| x.as_str()) == Some(id))
            .or_else(|| {
                comps.iter().find(|c| {
                    c.get("name")
                        .and_then(|x| x.as_str())
                        .is_some_and(|n| n.starts_with(prefix))
                })
            });
        let Some(c) = hit else { continue };
        watched_ids.push(c.get("id").and_then(|x| x.as_str()).unwrap_or(id).to_string());
        out.push(StatusComponent {
            name: c.get("name").and_then(|x| x.as_str()).unwrap_or(prefix).to_string(),
            status: c
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("operational")
                .to_string(),
        });
    }
    // Every watched component gone from the page is the shape moving under us,
    // not an all-clear: say so rather than reporting calm.
    if out.is_empty() {
        return Err("no watched component on the page".into());
    }

    let rank = out.iter().map(|c| status_rank(&c.status)).max().unwrap_or(0);

    // `incidents` in summary.json is the UNRESOLVED set. We take the first one
    // that names a watched component — the page lists newest first, and a second
    // simultaneous incident is detail the link covers.
    let incident = v
        .get("incidents")
        .and_then(|i| i.as_array())
        .and_then(|arr| {
            arr.iter().find(|inc| {
                inc.get("components")
                    .and_then(|c| c.as_array())
                    .is_some_and(|cs| {
                        cs.iter().any(|c| {
                            c.get("id")
                                .and_then(|x| x.as_str())
                                .is_some_and(|id| watched_ids.iter().any(|w| w == id))
                        })
                    })
            })
        })
        .map(|inc| {
            let latest = inc
                .get("incident_updates")
                .and_then(|u| u.as_array())
                .and_then(|a| a.first());
            StatusIncident {
                name: inc
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("Incident")
                    .to_string(),
                status: inc
                    .get("status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("investigating")
                    .to_string(),
                body: latest
                    .and_then(|u| u.get("body"))
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
                updated_at: latest
                    .and_then(|u| u.get("updated_at").or_else(|| u.get("created_at")))
                    .and_then(|x| x.as_str())
                    .and_then(sysclock::parse_iso8601)
                    .or_else(|| {
                        inc.get("updated_at")
                            .and_then(|x| x.as_str())
                            .and_then(sysclock::parse_iso8601)
                    }),
                url: inc
                    .get("shortlink")
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
            }
        });

    Ok(ClaudeStatus {
        source: "live".into(),
        fetched_at: now,
        severity: status_word(rank).into(),
        components: out,
        total: comps.len(),
        incident,
        page_url: STATUS_PAGE.into(),
    })
}

/// GET the status summary → (http status, body). curl, same reasoning as
/// `usage_get`: the dep tree and the TLS stack stay where they are.
pub fn status_get() -> Result<(u16, String), String> {
    let mut cmd = worktrees_core::childenv::command("curl");
    cmd.args(["-s", "--max-time", "10", "-w", "\n%{http_code}", STATUS_URL]);
    let out = run_deadline(cmd, 15).map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("curl exited {}", out.status.code().unwrap_or(-1)));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let cut = text.rfind('\n').ok_or("curl produced no status line")?;
    let code: u16 = text[cut + 1..]
        .trim()
        .parse()
        .map_err(|_| "curl produced no status code".to_string())?;
    Ok((code, text[..cut].to_string()))
}

/// Is Claude itself broken right now. Never Errs — see the block comment above.
pub fn claude_status() -> Result<ClaudeStatus, String> {
    let now = sysclock::now_epoch();
    let cached = STATUS_CACHE.lock().unwrap().clone();
    if let Some(c) = cached.as_ref() {
        if now - c.fetched_at < STATUS_TTL_SECS {
            return Ok(c.clone());
        }
    }
    // A failed fetch falls back to the expired entry while it is young enough
    // to still be true (see STATUS_STALE_MAX_SECS), and to silence after that.
    let fallback = |why: String| match stale_or_silence(cached, now) {
        Some(c) => {
            applog("warn", &format!("claude_status: {why} — serving the last answer"));
            c
        }
        None => fallback_hard(now, why),
    };
    match status_get() {
        Ok((200, body)) => match status_parse(&body, now) {
            Ok(info) => {
                *STATUS_CACHE.lock().unwrap() = Some(info.clone());
                Ok(info)
            }
            // A body we cannot read is NOT a transient failure — the page's
            // shape moved — so this one does not fall back to a cached answer
            // that the next fetch will never be able to refresh.
            Err(why) => Ok(fallback_hard(now, why)),
        },
        Ok((code, _)) => Ok(fallback(format!("http {code}"))),
        Err(why) => Ok(fallback(why)),
    }
}

/// What a FAILED fetch should serve: the expired cache while it is young enough
/// to still be true, or `None` for silence. Pure, so the boundary is a test and
/// not a claim.
pub fn stale_or_silence(cached: Option<ClaudeStatus>, now: i64) -> Option<ClaudeStatus> {
    match cached {
        Some(c) if now - c.fetched_at <= STATUS_STALE_MAX_SECS => Some(c),
        _ => None,
    }
}

/// Silence, with the reason logged. Used when the page answered but we could not
/// read it: serving a stale entry there would keep a possibly-resolved incident
/// on screen with no prospect of it ever being corrected.
pub fn fallback_hard(now: i64, why: String) -> ClaudeStatus {
    applog("warn", &format!("claude_status: {why}"));
    ClaudeStatus {
        source: "unavailable".into(),
        fetched_at: now,
        severity: "none".into(),
        components: Vec::new(),
        total: 0,
        incident: None,
        page_url: STATUS_PAGE.into(),
    }
}

/// Find the installed CLI: whatever `command -v` resolves in the USER'S login
/// shell (zsh reads ~/.zprofile; plain sh would not — the app may be launched
/// from Finder with a bare PATH), then the common install dirs. Chatty profiles
/// are filtered by taking the last absolute-path line. Returns (path, version).
pub fn cli_binary() -> Option<(String, String)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut candidates: Vec<String> = Vec::new();
    let mut probe = worktrees_core::childenv::command(&shell);
    probe.args(["-lc", "command -v -- worktrees"]);
    if let Ok(out) = run_deadline(probe, 5) {
        if out.status.success() {
            if let Some(p) = String::from_utf8_lossy(&out.stdout)
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| l.starts_with('/'))
            {
                candidates.push(p.to_string());
            }
        }
    }
    candidates.push(format!("{home}/.local/bin/worktrees"));
    candidates.push(format!("{home}/bin/worktrees"));
    candidates.push("/opt/homebrew/bin/worktrees".into());
    candidates.push("/usr/local/bin/worktrees".into());
    for path in candidates {
        let mut c = worktrees_core::childenv::command(&path);
        c.arg("--version");
        if let Ok(out) = run_deadline(c, 5) {
            if out.status.success() {
                let full = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // "worktrees 0.2.0" → "0.2.0"
                let v = full.rsplit(' ').next().unwrap_or(&full).to_string();
                return Some((path, v));
            }
        }
    }
    None
}

pub fn check_update() -> Result<UpdateInfo, String> {
    let (cli_path, cli_version) = match cli_binary() {
        Some((p, v)) => (Some(p), Some(v)),
        None => (None, None),
    };
    Ok(UpdateInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        cli_version,
        cli_path,
        latest: latest_release_tag(),
    })
}

/// Update the installed CLI via the PINNED-TAG installer — install.sh stays the
/// single source of truth for download/checksum/replace. Hardening (review):
/// the webview PROPOSES a tag but this side re-resolves it — latest, or (for a
/// user-chosen rollback) a tag in the published stable list, never an arbitrary
/// string; the script is fetched with a CHECKED curl (a piped `curl | bash`
/// masks download failure as success — pipeline status is the last command's)
/// and run from a private directory ([`run_installer`]); and success is
/// declared only when the re-probed CLI actually reports the new version.
pub fn update_cli(tag: String) -> Result<CmdResult, String> {
    if !valid_tag(&tag) {
        return Err(format!("suspicious tag '{tag}'"));
    }
    // Latest via the cheap redirect; anything else (a rollback) must be a
    // published stable release — the webview still cannot name an arbitrary tag.
    match latest_release_tag() {
        Some(latest) if latest == tag => {}
        Some(_) => {
            let known = list_published_releases()?;
            if !known.iter().any(|r| r.tag == tag) {
                return Err(format!("{tag} is not a published release — re-check the version list"));
            }
        }
        None => return Err("could not re-verify the release list (offline?)".into()),
    }

    let url = format!("https://raw.githubusercontent.com/{REPO_SLUG}/{tag}/install.sh");
    // replace the binary that's actually resolved, not blindly ~/.local/bin
    let install_dir = cli_binary().and_then(|(path, _)| Path::new(&path).parent().map(Path::to_path_buf));
    let base = installer_base(std::env::var_os("HOME"));
    let out = match run_installer(&base, &url, &tag, install_dir.as_deref())? {
        Installer::Ran(out) => out,
        Installer::NotFetched(f) => {
            return Ok(CmdResult {
                ok: false,
                code: f.status.code().unwrap_or(-1),
                output: format!("installer download failed\n{}", String::from_utf8_lossy(&f.stderr)),
                slug: None,
                needs_confirm: None,
                warnings: Vec::new(),
            });
        }
    };
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));

    // the real success signal: the resolved CLI now reports the new version
    let verified = cli_binary().map(|(_, v)| format!("v{v}") == tag).unwrap_or(false);
    if !out.status.success() || !verified {
        applog("warn", &format!("update_cli {tag}: rc={} verified={verified}", out.status.code().unwrap_or(-1)));
    }
    if out.status.success() && !verified {
        text.push_str("\n! installer exited 0 but the resolved CLI does not report the new version — check PATH shadowing.");
    }
    Ok(CmdResult {
        ok: out.status.success() && verified,
        code: out.status.code().unwrap_or(-1),
        output: text,
        slug: None,
        needs_confirm: None,
        warnings: Vec::new(),
    })
}

/// How far [`run_installer`] got: the installer ran (its exit status and
/// output), or curl could not fetch it (curl's).
enum Installer {
    Ran(std::process::Output),
    NotFetched(std::process::Output),
}

/// Where the installer is staged: worktrees' own cache, `~/.cache/worktrees`,
/// as for everything else it keeps. Not `temp_dir()`, the shared `/tmp` on
/// Linux, except for a process with no HOME, where the private directory
/// [`run_installer`] makes is what holds.
fn installer_base(home: Option<std::ffi::OsString>) -> std::path::PathBuf {
    match home.filter(|h| !h.is_empty()) {
        Some(h) => Path::new(&h).join(".cache/worktrees"),
        None => std::env::temp_dir(),
    }
}

/// Fetch `url` and run it with bash, with `tag` and `install_dir` passed to
/// install.sh the way it reads them.
///
/// It used to go `curl -o temp_dir()/worktrees-install-<pid>.sh`, then
/// `bash` that path. On Linux that name is in the shared `/tmp`, and another
/// local user who took it first got their own script run: a 0666 file of
/// theirs is written into by curl and rewritten by them before bash reads it
/// (measured), and a link is stopped only by `fs.protected_symlinks`. Now
/// curl hands the bytes back, and they are written as a new 0700 file in a
/// fresh 0700 directory under `base` (`fsx::PrivateDir`), which is run by
/// that path and removed with the directory when this returns.
fn run_installer(base: &Path, url: &str, tag: &str, install_dir: Option<&Path>) -> Result<Installer, String> {
    let mut fetch = worktrees_core::childenv::command("curl");
    fetch.args(["-fsSL", "--connect-timeout", "10", "--max-time", "120"]).arg(url);
    let f = run_deadline(fetch, 150).map_err(|e| {
        applog("error", &format!("update_cli {tag}: installer download failed: {e}"));
        format!("installer download failed: {e}")
    })?;
    if !f.status.success() {
        applog("error", &format!("update_cli {tag}: installer download rc={}", f.status.code().unwrap_or(-1)));
        return Ok(Installer::NotFetched(f));
    }

    let staged = worktrees_core::fsx::PrivateDir::new(base, "update-cli")
        .and_then(|dir| dir.create_file("install.sh", &f.stdout, 0o700).map(|script| (dir, script)));
    let (_dir, script) = staged.map_err(|e| {
        applog("error", &format!("update_cli {tag}: staging the installer under {} failed: {e}", base.display()));
        format!("could not stage the installer under {}: {e}", base.display())
    })?;
    let mut run = worktrees_core::childenv::command("bash");
    run.arg(&script).env("WORKTREES_INSTALL_VERSION", tag);
    if let Some(dir) = install_dir {
        run.env("WORKTREES_INSTALL_DIR", dir);
    }
    let out = run_deadline(run, 600).map_err(|e| {
        applog("error", &format!("update_cli {tag}: installer run failed: {e}"));
        format!("installer run failed: {e}")
    })?;
    Ok(Installer::Ran(out))
}

pub fn valid_tag(tag: &str) -> bool {
    tag.starts_with('v')
        && tag.len() > 1
        && tag[1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReleaseRow {
    pub tag: String,
    pub published: String, // YYYY-MM-DD
}

/// GitHub's releases list → installable rows, newest first. Drafts and
/// prereleases are out, and so is anything without a `latest.json` — the app
/// half of the install cannot run without it.
pub fn parse_releases(body: &str) -> Result<Vec<ReleaseRow>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("release list: {e}"))?;
    let arr = v.as_array().ok_or("release list: not an array (rate-limited?)")?;
    Ok(arr
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(true) && !r["prerelease"].as_bool().unwrap_or(true))
        .filter(|r| {
            r["assets"].as_array().is_some_and(|a| a.iter().any(|x| x["name"] == "latest.json"))
        })
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            valid_tag(tag).then(|| ReleaseRow {
                tag: tag.to_string(),
                published: r["published_at"].as_str().unwrap_or("").chars().take(10).collect(),
            })
        })
        .collect())
}

pub fn list_published_releases() -> Result<Vec<ReleaseRow>, String> {
    let mut cmd = worktrees_core::childenv::command("curl");
    cmd.args(["-fsSL", "--max-time", "10", "-H", "Accept: application/vnd.github+json"])
        .arg(format!("https://api.github.com/repos/{REPO_SLUG}/releases?per_page=30"));
    let out = run_deadline(cmd, 15).map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("couldn't fetch the release list (curl exited {})", out.status.code().unwrap_or(-1)));
    }
    parse_releases(&String::from_utf8_lossy(&out.stdout))
}

pub fn list_releases() -> Result<Vec<ReleaseRow>, String> {
    list_published_releases()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_list_keeps_only_installable_stable_tags() {
        let body = r#"[
          {"tag_name":"v0.29.0-rc1","draft":false,"prerelease":true,"published_at":"2026-09-22T00:00:00Z","assets":[{"name":"latest.json"}]},
          {"tag_name":"v0.28.0","draft":false,"prerelease":false,"published_at":"2026-09-21T22:24:30Z","assets":[{"name":"latest.json"},{"name":"checksums.txt"}]},
          {"tag_name":"v0.27.9","draft":true,"prerelease":false,"published_at":null,"assets":[{"name":"latest.json"}]},
          {"tag_name":"v0.27.0","draft":false,"prerelease":false,"published_at":"2026-09-20T19:02:45Z","assets":[{"name":"checksums.txt"}]},
          {"tag_name":"v0.26.0; rm -rf","draft":false,"prerelease":false,"published_at":"x","assets":[{"name":"latest.json"}]},
          {"tag_name":"v0.3.0","draft":false,"prerelease":false,"published_at":"2026-07-27T16:24:43Z","assets":[{"name":"latest.json"}]}
        ]"#;
        let rows = parse_releases(body).unwrap();
        assert_eq!(
            rows,
            vec![
                ReleaseRow { tag: "v0.28.0".into(), published: "2026-09-21".into() },
                ReleaseRow { tag: "v0.3.0".into(), published: "2026-07-27".into() },
            ]
        );
        // a rate-limit answer is an object, not a list — an error, not "no releases"
        assert!(parse_releases(r#"{"message":"API rate limit exceeded"}"#).is_err());
    }

    /// A pull that lands before the network is back must not turn a live
    /// incident into silence — but a cached outage may not outlive the outage
    /// either. Both halves of that bargain, at the boundary.
    #[test]
    fn a_failed_fetch_serves_the_last_answer_only_while_it_can_still_be_true() {
        let at = |t: i64| Some(ClaudeStatus {
            source: "live".into(), fetched_at: t, severity: "down".into(),
            components: vec![StatusComponent { name: "Claude Code".into(), status: "major_outage".into() }],
            total: 6, incident: None, page_url: STATUS_PAGE.into(),
        });
        let now = 1_000_000;
        // fresh-ish and expired: still the truest thing we have
        assert_eq!(stale_or_silence(at(now - 60), now).unwrap().severity, "down");
        assert!(stale_or_silence(at(now - STATUS_STALE_MAX_SECS), now).is_some(), "at the bound, inclusive");
        // past the bound the outage may well be over; silence beats a stale alarm
        assert!(stale_or_silence(at(now - STATUS_STALE_MAX_SECS - 1), now).is_none());
        // nothing cached at all
        assert!(stale_or_silence(None, now).is_none());
    }

    /// A trimmed `summary.json`: six components as the real page carries them,
    /// plus whatever incidents the caller wants.
    fn status_body(code: &str, api: &str, incidents: &str) -> String {
        format!(
            r#"{{
              "page": {{"id":"tymt9n04zgry","name":"Claude","url":"https://status.claude.com"}},
              "components": [
                {{"id":"rwppv331jlwc","name":"claude.ai","status":"major_outage"}},
                {{"id":"0qbwn08sd68x","name":"Claude Console (platform.claude.com)","status":"operational"}},
                {{"id":"k8w3r06qmzrp","name":"Claude API (api.anthropic.com)","status":"{api}"}},
                {{"id":"yyzkbfz2thpt","name":"Claude Code","status":"{code}"}},
                {{"id":"bpp5gb3hpjcl","name":"Claude Cowork","status":"operational"}},
                {{"id":"0scnb50nvy53","name":"Claude for Government","status":"operational"}}
              ],
              "incidents": [{incidents}],
              "scheduled_maintenances": [],
              "status": {{"indicator":"critical","description":"Major outage"}}
            }}"#
        )
    }

    /// The whole reason this does not read `status.indicator`: the page's own
    /// verdict covers six components and this app feels two. claude.ai is in a
    /// major outage in every body above and the page calls itself critical —
    /// and a worktree does not care.
    #[test]
    fn severity_ignores_components_this_app_does_not_use() {
        let s = status_parse(&status_body("operational", "operational", ""), 100).unwrap();
        assert_eq!(s.severity, "none");
        assert_eq!(s.components.len(), 2, "only the watched two are reported");
        assert_eq!(s.components[0].name, "Claude Code");
        assert_eq!(s.total, 6, "…out of the six the page carries");
    }

    #[test]
    fn severity_is_the_worst_watched_component() {
        let deg = status_parse(&status_body("operational", "degraded_performance", ""), 100).unwrap();
        assert_eq!(deg.severity, "degraded");
        let part = status_parse(&status_body("partial_outage", "operational", ""), 100).unwrap();
        assert_eq!(part.severity, "degraded");
        let down = status_parse(&status_body("major_outage", "degraded_performance", ""), 100).unwrap();
        assert_eq!(down.severity, "down", "down wins over degraded");
    }

    /// A state Statuspage has not shipped yet must not read as "fine".
    #[test]
    fn an_unknown_component_state_counts_as_degraded() {
        let s = status_parse(&status_body("operational", "on_fire", ""), 100).unwrap();
        assert_eq!(s.severity, "degraded");
    }

    /// Ids are the match, so a retitled component keeps being watched — and the
    /// name we SHOW is the page's current one, not our stale prefix.
    #[test]
    fn a_renamed_component_is_still_matched_by_id() {
        let body = status_body("operational", "operational", "")
            .replace("Claude Code", "Claude Code (CLI + app)");
        let s = status_parse(&body, 100).unwrap();
        assert_eq!(s.components[0].name, "Claude Code (CLI + app)");
    }

    /// And a re-ISSUED component — new id, same name — is matched by the name
    /// prefix, which is the only reason the prefix is carried at all.
    #[test]
    fn a_reissued_component_is_still_matched_by_name() {
        let body = status_body("major_outage", "operational", "").replace("yyzkbfz2thpt", "newidnewid00");
        let s = status_parse(&body, 100).unwrap();
        assert_eq!(s.severity, "down");
    }

    /// Both watched components gone is the page's shape moving under us. An
    /// all-clear would be a lie told exactly when we can no longer tell.
    #[test]
    fn losing_every_watched_component_is_an_error_not_an_all_clear() {
        let body = status_body("operational", "operational", "")
            .replace("yyzkbfz2thpt", "gone1")
            .replace("Claude Code", "Something Else")
            .replace("k8w3r06qmzrp", "gone2")
            .replace("Claude API (api.anthropic.com)", "Other API");
        assert!(status_parse(&body, 100).is_err());
    }

    #[test]
    fn the_incident_shown_is_one_that_names_a_watched_component() {
        let body = status_body("operational", "degraded_performance", &format!("{INC_OTHER},{INC_API}"));
        let inc = status_parse(&body, 100).unwrap().incident.expect("an incident");
        assert_eq!(inc.name, "Elevated errors on the Messages API");
        // the LATEST update's body, which is the sentence worth showing
        assert!(inc.body.unwrap().starts_with("A fix has been applied"));
        assert_eq!(inc.updated_at, Some(1789743840));
        assert_eq!(inc.url.as_deref(), Some("https://stspg.io/abc"));
    }

    #[test]
    fn an_incident_on_an_unwatched_component_is_not_reported() {
        let body = status_body("operational", "operational", INC_OTHER);
        let s = status_parse(&body, 100).unwrap();
        assert!(s.incident.is_none());
        assert_eq!(s.severity, "none");
    }

    #[test]
    fn a_body_that_is_not_the_status_page_is_an_error() {
        assert!(status_parse("not json", 100).is_err());
        assert!(status_parse(r#"{"page":{}}"#, 100).is_err());
    }

    const INC_API: &str = r#"{
        "name":"Elevated errors on the Messages API","status":"monitoring",
        "shortlink":"https://stspg.io/abc","updated_at":"2026-09-18T15:00:00Z",
        "components":[{"id":"k8w3r06qmzrp"}],
        "incident_updates":[
          {"body":"A fix has been applied and error rates are recovering.","updated_at":"2026-09-18T15:04:00Z"},
          {"body":"We are investigating.","updated_at":"2026-09-18T14:30:00Z"}
        ]
    }"#;
    const INC_OTHER: &str = r#"{
        "name":"Issues with Google Play subscriptions","status":"investigating",
        "components":[{"id":"rwppv331jlwc"}],
        "incident_updates":[{"body":"Unrelated.","updated_at":"2026-09-18T15:04:00Z"}]
    }"#;

    // ── staging the installer ────────────────────────────────────────────────

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn scratch(tag: &str) -> Scratch {
        let t = std::env::temp_dir().join(format!("wtupd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        Scratch(t)
    }

    /// Whatever a test planted at a fixed path outside its scratch dir.
    struct Planted(std::path::PathBuf);
    impl Drop for Planted {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// An install.sh, served by real curl from a `file://` URL, that writes
    /// down where it ran from — `$0`, its own mode, its directory's — and
    /// what it was handed.
    fn installer(s: &Scratch) -> String {
        let src = s.0.join("install.sh");
        let at = |n: &str| s.0.join(n).display().to_string();
        std::fs::write(
            &src,
            format!(
                "printf '%s' \"$0\" > '{}'\nls -l \"$0\" > '{}'\nls -ld \"$(dirname \"$0\")\" > '{}'\n\
                 printf '%s %s' \"$WORKTREES_INSTALL_VERSION\" \"$WORKTREES_INSTALL_DIR\" > '{}'\necho installed\n",
                at("argv0"),
                at("mode"),
                at("dirmode"),
                at("env")
            ),
        )
        .unwrap();
        format!("file://{}", src.display())
    }

    /// Run the installer; [`ran_privately`] then checks where it ran.
    fn install(base: &Path, url: &str) {
        let out = match run_installer(base, url, "v9.9.9", Some(Path::new("/opt/wt"))).unwrap() {
            Installer::Ran(out) => out,
            Installer::NotFetched(f) => panic!("not fetched: {}", String::from_utf8_lossy(&f.stderr)),
        };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "installed\n");
    }

    /// The fetched script ran, handed the tag and the install dir, from a 0700
    /// directory of its own under `base`, as a 0700 file, and that directory
    /// is gone. Returns the directory.
    fn ran_privately(s: &Scratch, base: &Path) -> std::path::PathBuf {
        let read = |n: &str| std::fs::read_to_string(s.0.join(n)).unwrap();
        assert_eq!(read("env"), "v9.9.9 /opt/wt");
        let argv0 = std::path::PathBuf::from(read("argv0"));
        let dir = argv0.parent().unwrap().to_path_buf();
        assert_eq!(dir.parent(), Some(base), "ran from {}, not a directory of its own under the cache", argv0.display());
        // The first ten columns: macOS appends `@` when there are xattrs.
        assert_eq!(&read("dirmode")[..10], "drwx------", "the directory was {}", read("dirmode"));
        assert_eq!(&read("mode")[..10], "-rwx------", "the script was {}", read("mode"));
        assert!(!dir.exists(), "{} outlived the run", dir.display());
        dir
    }

    /// The old staging name, `temp_dir()/worktrees-install-<pid>.sh`, is in
    /// the shared `/tmp` on Linux, where another local user could plant it
    /// first. Whatever is there now — a link to someone's file, or a script of
    /// the planter's own — is never written through, followed or run: the
    /// installer that runs is the one fetched, from a fresh 0700 directory of
    /// its own under the cache, which is gone again afterwards.
    #[test]
    fn a_planted_shared_name_is_never_written_or_run() {
        let s = scratch("plant");
        let base = s.0.join("cache");
        let url = installer(&s);
        let old = Planted(std::env::temp_dir().join(format!("worktrees-install-{}.sh", std::process::id())));
        let _ = std::fs::remove_file(&old.0);

        let victim = s.0.join("victim");
        std::fs::write(&victim, "precious").unwrap();
        std::os::unix::fs::symlink(&victim, &old.0).unwrap();
        install(&base, &url);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious", "the download went through the link");
        assert_eq!(std::fs::read_link(&old.0).unwrap(), victim, "the planted link was touched");
        let first = ran_privately(&s, &base);

        std::fs::remove_file(&old.0).unwrap();
        let pwned = s.0.join("pwned");
        let plant = format!("touch '{}'\n", pwned.display());
        std::fs::write(&old.0, &plant).unwrap();
        install(&base, &url);
        assert!(!pwned.exists(), "the planted script ran");
        assert_eq!(std::fs::read_to_string(&old.0).unwrap(), plant, "the planted file was written");
        let second = ran_privately(&s, &base);

        assert_ne!(first, second, "each run gets a fresh directory");
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0, "nothing is left in the cache");
    }

    /// A download that fails stages nothing at all.
    #[test]
    fn a_failed_download_stages_nothing() {
        let s = scratch("nofetch");
        let base = s.0.join("cache");
        let url = format!("file://{}", s.0.join("absent.sh").display());
        let Ok(Installer::NotFetched(f)) = run_installer(&base, &url, "v9.9.9", None) else {
            panic!("a missing installer must come back as not fetched");
        };
        assert!(!f.status.success());
        assert!(!base.exists() || std::fs::read_dir(&base).unwrap().count() == 0);
    }

    /// worktrees' own cache, as everywhere else: only a process with no HOME
    /// falls back to `temp_dir()`, and the private directory holds there too.
    #[test]
    fn the_installer_is_staged_in_the_cache_not_the_shared_temp_dir() {
        assert_eq!(installer_base(Some("/home/u".into())), Path::new("/home/u/.cache/worktrees"));
        assert_eq!(installer_base(Some("".into())), std::env::temp_dir());
        assert_eq!(installer_base(None), std::env::temp_dir());
    }
}

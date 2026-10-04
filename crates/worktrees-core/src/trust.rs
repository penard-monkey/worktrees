//! Whether a harness may load a repo's OWN executable resources — pi's
//! project trust (pi-harness §8, decided 2026-09-29), and the `[trust]` table
//! the opencode gate will share.
//!
//! pi loads `.pi/extensions` (TypeScript, in pi's process), `.pi/skills`,
//! `.pi/settings.json` and `.agents/skills` from any ancestor once a project
//! is trusted, and its trust modal's highlighted choice is **Trust**. So:
//!
//! - pi launches with `--no-approve` by default: project resources are skipped
//!   for the run, while AGENTS.md/CLAUDE.md still load.
//! - **pi's own trust counts** (decided 2026-09-29): a place pi's `trust.json`
//!   trusts — its nearest entry, exactly as pi resolves it — launches with
//!   `--approve`; one it explicitly distrusts gets `--no-approve`. READ only:
//!   worktrees never writes that file (`pi_own_trust`).
//! - `pi_project_trust = "ask"` (a user setting) passes neither flag, and pi
//!   asks — the modal the activity reader reports as waiting.
//! - A repo in the user's allowance, `[trust] pi = ["<repo root>", …]` in
//!   `~/.config/worktrees/config.toml`, launches with `--approve` — even over
//!   an explicit distrust in pi's file.
//! - Over everything: a place whose `.pi/mcp.json` defines `worktrees` never
//!   gets `--approve` (`pi_mcp_shadowed`).
//!
//! Only the user grants the allowance — `worktrees trust pi`, or Settings → pi.
//! MCP and agents may READ it. `trust` joins `USER_ONLY_KEYS`, so a
//! `.worktrees.toml` naming it is a hard parse error. And worktrees never writes
//! pi's own `trust.json`: the allowance does not leak into a `pi` run outside
//! worktrees, and revoking it takes effect at the next launch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// `pi_project_trust`: what a repo that is NOT in the allowance gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PiTrust {
    /// `--no-approve` (the default).
    Never,
    /// Neither flag: pi's own modal decides.
    Ask,
}

impl PiTrust {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "never" => Some(Self::Never),
            "ask" => Some(Self::Ask),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Ask => "ask",
        }
    }
}

static PI_TRUST_OVERRIDE: Mutex<Option<PiTrust>> = Mutex::new(None);

/// The app's Settings → pi value, pushed in-process (same shape as
/// `codex::set_permissions_override`).
pub fn set_pi_trust_override(t: Option<PiTrust>) {
    *PI_TRUST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = t;
}

/// app setting > `$WORKTREES_PI_PROJECT_TRUST` > `pi_project_trust` user
/// config > `never`. An unknown value falls through to the next tier — and in
/// the end to `never`, the narrow choice.
pub fn pi_trust() -> PiTrust {
    if let Some(t) = *PI_TRUST_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
        return t;
    }
    let env = std::env::var("WORKTREES_PI_PROJECT_TRUST").ok();
    let cfg = crate::config::user_cfg("pi_project_trust");
    env.as_deref().and_then(PiTrust::parse).or_else(|| cfg.as_deref().and_then(PiTrust::parse)).unwrap_or(PiTrust::Never)
}

/// The repo a place belongs to, as the allowance names it: the parent of its
/// git COMMON dir, canonicalised — so every worktree of a repo, including ones
/// created later, shares one entry, and a place directory a repo could fake is
/// never the key.
pub fn repo_root(wt: &str) -> Option<String> {
    let common = crate::codex::git_common_dir(wt)?;
    Some(Path::new(&common).parent()?.to_string_lossy().into_owned())
}

/// The allowance for `harness` in a `config.toml` text. Lenient, like every
/// user-config read: a parse error or a wrong type is an empty list.
pub fn allowed_in(text: &str, harness: &str) -> Vec<String> {
    let Ok(root) = toml::from_str::<BTreeMap<String, toml::Value>>(text) else { return Vec::new() };
    root.get("trust")
        .and_then(|t| t.get(harness))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn allowed(harness: &str) -> Vec<String> {
    std::fs::read_to_string(crate::config::config_toml_path()).map(|t| allowed_in(&t, harness)).unwrap_or_default()
}

/// The root a grant or revoke for `repo` names: on a revoke, a string already
/// in the list is taken literally (the repo may be gone); otherwise the repo's
/// root via git.
pub fn root_for(harness: &str, repo: &str, allow: bool) -> Option<String> {
    if !allow {
        if let Some(r) = allowed(harness).into_iter().find(|r| r == repo) {
            return Some(r);
        }
    }
    repo_root(repo)
}

pub fn is_allowed(harness: &str, repo_root: &str) -> bool {
    allowed(harness).iter().any(|r| r == repo_root)
}

/// A trust decision in pi's own `trust.json`: the key that matched, and its
/// value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PiOwnTrust {
    pub path: String,
    pub trusted: bool,
}

/// How a pi launch in a place is trusted, and why — one answer for the launch,
/// `doctor --pi` and Settings → pi.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PiLaunchTrust {
    /// The shell word, or none (`ask`: pi's own modal decides).
    pub flag: Option<&'static str>,
    /// `shadowed` · `allowance` · `pi-trusted` · `pi-untrusted` · `never` · `ask`.
    pub source: &'static str,
    /// pi's own matching entry, whether or not it decided.
    pub pi_entry: Option<PiOwnTrust>,
    pub shadowed: bool,
    /// Something would have granted `--approve` (the allowance or pi's trust).
    /// With `shadowed`, that is the case to say out loud.
    pub would_approve: bool,
}

/// The precedence, without the reads: protect-ours over everything, then the
/// worktrees allowance, then pi's own nearest entry, then `pi_project_trust`.
/// A shadowing place gets an explicit `--no-approve`, never "ask": pi's modal
/// highlights Trust, and one Enter there would load exactly the server this
/// refuses.
pub fn pi_launch_trust_for(allowed: bool, mode: PiTrust, shadowed: bool, pi_entry: Option<PiOwnTrust>) -> PiLaunchTrust {
    let pi_says = pi_entry.as_ref().map(|e| e.trusted);
    let would_approve = allowed || pi_says == Some(true);
    let (flag, source) = if shadowed {
        (Some("--no-approve"), "shadowed")
    } else if allowed {
        (Some("--approve"), "allowance")
    } else if pi_says == Some(true) {
        (Some("--approve"), "pi-trusted")
    } else if pi_says == Some(false) {
        (Some("--no-approve"), "pi-untrusted")
    } else {
        match mode {
            PiTrust::Never => (Some("--no-approve"), "never"),
            PiTrust::Ask => (None, "ask"),
        }
    };
    PiLaunchTrust { flag, source, pi_entry, shadowed, would_approve }
}

pub fn pi_launch_trust(wt: &str) -> PiLaunchTrust {
    let allowed = repo_root(wt).is_some_and(|r| is_allowed("pi", &r));
    pi_launch_trust_for(allowed, pi_trust(), pi_mcp_shadowed(wt), pi_own_trust(wt))
}

/// pi's trust flag for a launch in `wt`, already a shell word (`pi_launch_trust`).
pub fn pi_flag(wt: &str) -> Option<&'static str> {
    pi_launch_trust(wt).flag
}

/// pi's `trust.json`, in its agent dir. worktrees READS it and never writes it.
pub fn pi_trust_path() -> PathBuf {
    crate::pi::agent_dir().join("trust.json")
}

/// pi 0.99.1's `readTrustFile`: a BOM is stripped, the root must be an object,
/// and every value must be `true`, `false` or `null` — anything else and pi
/// refuses the whole file, so this does too.
pub fn parse_pi_trust(text: &str) -> Result<BTreeMap<String, Option<bool>>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let obj = v.as_object().ok_or("expected an object")?;
    obj.iter()
        .map(|(k, v)| match v {
            serde_json::Value::Bool(b) => Ok((k.clone(), Some(*b))),
            serde_json::Value::Null => Ok((k.clone(), None)),
            _ => Err(format!("value for {k:?} must be true, false or null")),
        })
        .collect()
}

/// pi 0.99.1's `findNearestTrustEntry`: from the canonical cwd up to `/`, the
/// first key whose value is `true` or `false`. Keys match as exact strings (pi
/// writes realpaths); a `null` is no decision and the walk goes on.
pub fn nearest_pi_trust(data: &BTreeMap<String, Option<bool>>, canonical_cwd: &Path) -> Option<PiOwnTrust> {
    let mut dir = Some(canonical_cwd);
    while let Some(d) = dir {
        if let Some(Some(b)) = data.get(d.to_string_lossy().as_ref()) {
            return Some(PiOwnTrust { path: d.to_string_lossy().into_owned(), trusted: *b });
        }
        dir = d.parent();
    }
    None
}

/// pi's own decision for `wt`, as pi would resolve it (`realpath`, falling
/// back to the path as given, like its `canonicalizePath`). `None` when the
/// file is missing, unreadable, or has no entry on the way up.
pub fn pi_own_trust(wt: &str) -> Option<PiOwnTrust> {
    let data = parse_pi_trust(&std::fs::read_to_string(pi_trust_path()).ok()?).ok()?;
    let cwd = std::fs::canonicalize(wt).unwrap_or_else(|_| PathBuf::from(wt));
    nearest_pi_trust(&data, &cwd)
}

/// The place's own `.pi/mcp.json`, which pi reads (for a trusted project)
/// from the session cwd itself — per BRANCH, where the allowance is per repo.
pub fn pi_project_mcp(wt: &str) -> PathBuf {
    Path::new(wt).join(".pi").join("mcp.json")
}

/// Whether `wt`'s `.pi/mcp.json` would replace the user's `worktrees` MCP
/// server once trusted (pi-harness §4.4): a project entry REPLACES a user
/// entry of the same name, disabled or not, so the lane would talk to repo
/// code instead of the bus. A file that exists but does not parse counts as
/// shadowing — refusing `--approve` for one launch is cheap, and a parser
/// disagreement with pi must not be the thing that lets it through.
pub fn pi_mcp_shadowed(wt: &str) -> bool {
    match std::fs::read_to_string(pi_project_mcp(wt)) {
        Ok(text) => mcp_json_defines(&text, crate::pimcp::SERVER_KEY),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// `mcpServers.<name>` exists in an `mcp.json` text; unparsable text counts.
pub fn mcp_json_defines(text: &str, name: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => v.get("mcpServers").and_then(|m| m.get(name)).is_some(),
        Err(_) => true,
    }
}

/// The sentence a launch and `doctor --pi` say when `pi_mcp_shadowed` holds.
pub fn pi_shadow_warning(wt: &str) -> String {
    format!(
        "{} defines a `{}` MCP server, which would replace the worktrees tools in a trusted pi \
         lane — so pi launches there WITHOUT --approve, and this branch's .pi/ resources are \
         skipped. .pi/mcp.json is per branch; the allowance is per repo.",
        pi_project_mcp(wt).display(),
        crate::pimcp::SERVER_KEY
    )
}

fn toml_string(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn is_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// `text` with `[trust] <harness>` set to `list`, changing nothing else.
///
/// A surgical edit rather than a re-serialisation: this is the user's own
/// file, with their comments and layout, and the `toml` build here is
/// parse-only. The edit replaces the key's lines inside the `[trust]` table
/// (or adds the table), then the RESULT is parsed back and must hold exactly
/// `list` — any layout this does not understand (a dotted `trust.pi = …` at the
/// top level, an inline table) is refused with no write, rather than guessed
/// at.
pub fn with_allowed(text: &str, harness: &str, list: &[String]) -> Result<String, String> {
    if !text.is_empty() && toml::from_str::<BTreeMap<String, toml::Value>>(text).is_err() {
        return Err("your config.toml does not parse; fix it first".into());
    }
    let rendered = format!("{harness} = [{}]", list.iter().map(|s| toml_string(s)).collect::<Vec<_>>().join(", "));
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let mut placed = false;
    let mut in_trust = false;
    while i < lines.len() {
        let l = lines[i];
        if is_header(l) {
            if in_trust && !placed {
                out.push(rendered.clone());
                placed = true;
            }
            in_trust = l.trim() == "[trust]";
            out.push(l.to_string());
            i += 1;
            continue;
        }
        let key = l.split('=').next().unwrap_or("").trim();
        if in_trust && (key == harness || key == format!("\"{harness}\"")) {
            // Drop the key's lines through the one that closes its array.
            let mut depth = 0i32;
            loop {
                let cur = lines[i];
                depth += cur.matches('[').count() as i32 - cur.matches(']').count() as i32;
                i += 1;
                if depth <= 0 || i >= lines.len() {
                    break;
                }
            }
            if !placed {
                out.push(rendered.clone());
                placed = true;
            }
            continue;
        }
        out.push(l.to_string());
        i += 1;
    }
    if in_trust && !placed {
        out.push(rendered.clone());
        placed = true;
    }
    if !placed {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("[trust]".into());
        out.push(rendered);
    }
    let mut result = out.join("\n");
    result.push('\n');
    if toml::from_str::<BTreeMap<String, toml::Value>>(&result).is_err() || allowed_in(&result, harness) != list {
        return Err("could not edit [trust] in your config.toml safely (an unusual layout?) — edit it by hand".into());
    }
    Ok(result)
}

/// Grant (or revoke) `harness`'s allowance for `repo_root` in the user's
/// `config.toml`. The user's act only: the CLI verb and the app call this; the
/// MCP server never does. Returns whether anything changed.
pub fn set_allowed(harness: &str, repo_root: &str, allow: bool) -> Result<bool, String> {
    let path = crate::config::config_toml_path();
    set_allowed_at(&path, harness, repo_root, allow)
}

pub fn set_allowed_at(path: &Path, harness: &str, repo_root: &str, allow: bool) -> Result<bool, String> {
    crate::config::edit_user_config(path, |text| {
        let mut list = allowed_in(text, harness);
        let had = list.iter().any(|r| r == repo_root);
        if had == allow {
            return Ok(None);
        }
        if allow {
            list.push(repo_root.to_string());
        } else {
            list.retain(|r| r != repo_root);
        }
        with_allowed(text, harness, &list).map(Some)
    })
}

/// `worktrees trust pi [<repo>] [--revoke]` — and bare `worktrees trust` lists.
pub fn cmd_trust(p: &crate::Project, ui: &mut dyn crate::Ui, args: &[String]) -> i32 {
    let mut revoke = false;
    let mut pos: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--revoke" => revoke = true,
            s if s.starts_with('-') => {
                ui.error(&format!("Unknown flag: {s}"));
                return 1;
            }
            s => pos.push(s),
        }
    }
    let Some(harness) = pos.first().copied() else {
        let list = allowed("pi");
        ui.info(&format!("pi project trust for repos not listed: {}", pi_trust().as_str()));
        if list.is_empty() {
            ui.info("No repo is allowed to load its own pi resources (.pi/, .agents/skills).");
        }
        for r in list {
            ui.info(&format!("pi allowed: {r}"));
        }
        return 0;
    };
    if harness != crate::provider::PI.id {
        ui.error(&format!("trust applies to pi only (got '{harness}')"));
        return 1;
    }
    if pos.len() > 2 {
        ui.error("trust takes at most one repo");
        return 1;
    }
    let wt = pos.get(1).map(|s| s.to_string()).unwrap_or_else(|| p.main_root.clone());
    // A revoke names what is LISTED, which may be a repo that has since moved
    // or been deleted — git cannot resolve it any more, and that must not make
    // an allowance permanent. So the listed string is matched first, as is.
    let listed = if revoke { allowed(harness).into_iter().find(|r| r == &wt) } else { None };
    let Some(root) = listed.or_else(|| repo_root(&wt)) else {
        ui.error(&format!("{wt} is not inside a git repository"));
        return 1;
    };
    match set_allowed(harness, &root, !revoke) {
        Ok(changed) => {
            let state = match (revoke, changed) {
                (false, true) => "now allowed: pi lanes in this repo launch with --approve and load its .pi/ and .agents/skills, and start its .pi/mcp.json servers — repo code runs with no prompt (a branch whose .pi/mcp.json defines `worktrees` is still refused).",
                (false, false) => "already allowed.",
                (true, true) => "no longer allowed: its next pi launch skips the repo's own resources.",
                (true, false) => "was not allowed.",
            };
            ui.info(&format!("{root}: {state}"));
            0
        }
        Err(e) => {
            ui.error(&e);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowance_edit_changes_only_the_trust_key() {
        let base = "# my config\nai_cmd = \"claude\" # inline\n\n[sync]\nhub = \"/Volumes/x\"\n";
        let one = with_allowed(base, "pi", &["/r/a".into()]).unwrap();
        assert!(one.starts_with(base), "everything before is untouched:\n{one}");
        assert!(one.ends_with("[trust]\npi = [\"/r/a\"]\n"), "{one}");
        assert_eq!(allowed_in(&one, "pi"), vec!["/r/a".to_string()]);
        // Replacing a multi-line array inside an existing table, beside another key.
        let multi = "[trust]\nopencode = [\"/o\"]\npi = [\n  \"/r/a\",\n  \"/r/b\",\n]\n\n[sync]\nhub = \"h\"\n";
        let two = with_allowed(multi, "pi", &["/r/b".into()]).unwrap();
        assert_eq!(two, "[trust]\nopencode = [\"/o\"]\npi = [\"/r/b\"]\n\n[sync]\nhub = \"h\"\n");
        assert_eq!(allowed_in(&two, "opencode"), vec!["/o".to_string()], "the other harness keeps its list");
        // A [trust] table without the key gets it.
        let t = with_allowed("[trust]\nopencode = []\n", "pi", &["/x".into()]).unwrap();
        assert_eq!(allowed_in(&t, "pi"), vec!["/x".to_string()]);
        // Quotes and backslashes in a path survive.
        let q = with_allowed("", "pi", &["/a \"b\"\\c".into()]).unwrap();
        assert_eq!(allowed_in(&q, "pi"), vec!["/a \"b\"\\c".to_string()]);
        // A layout the edit does not understand is refused, not guessed at.
        assert!(with_allowed("trust.pi = [\"/r\"]\n", "pi", &[]).is_err());
        assert!(with_allowed("[broken\n", "pi", &[]).is_err());
    }

    #[test]
    fn set_allowed_edits_a_symlinked_config_through_the_link() {
        crate::config::assert_writes_through_a_link(
            "trust",
            |p| assert_eq!(set_allowed_at(p, "pi", "/r/a", true), Ok(true)),
            |t| allowed_in(t, "pi") == vec!["/r/a".to_string()],
        );
    }

    #[test]
    fn set_allowed_round_trips_and_reports_no_op() {
        let d = std::env::temp_dir().join(format!("wttrust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let f = d.join("config.toml");
        assert_eq!(set_allowed_at(&f, "pi", "/r/a", true), Ok(true), "creates the file");
        assert_eq!(set_allowed_at(&f, "pi", "/r/a", true), Ok(false));
        assert_eq!(set_allowed_at(&f, "pi", "/r/b", true), Ok(true));
        assert_eq!(allowed_in(&std::fs::read_to_string(&f).unwrap(), "pi"), vec!["/r/a".to_string(), "/r/b".to_string()]);
        assert_eq!(set_allowed_at(&f, "pi", "/r/a", false), Ok(true));
        assert_eq!(allowed_in(&std::fs::read_to_string(&f).unwrap(), "pi"), vec!["/r/b".to_string()]);
        assert_eq!(set_allowed_at(&f, "pi", "/nope", false), Ok(false));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Precedence (decided 2026-09-29): protect-ours over everything, then the
    /// worktrees allowance, then pi's own entry, then `pi_project_trust`.
    #[test]
    fn launch_trust_precedence() {
        let t = |b| Some(PiOwnTrust { path: "/w".into(), trusted: b });
        let f = |allowed, mode, shadowed, e: Option<PiOwnTrust>| {
            let d = pi_launch_trust_for(allowed, mode, shadowed, e);
            (d.flag, d.source)
        };
        use PiTrust::{Ask, Never};
        assert_eq!(f(false, Never, false, None), (Some("--no-approve"), "never"));
        assert_eq!(f(false, Ask, false, None), (None, "ask"));
        assert_eq!(f(false, Never, false, t(true)), (Some("--approve"), "pi-trusted"));
        assert_eq!(f(false, Ask, false, t(true)), (Some("--approve"), "pi-trusted"));
        assert_eq!(f(false, Ask, false, t(false)), (Some("--no-approve"), "pi-untrusted"));
        assert_eq!(f(true, Never, false, t(false)), (Some("--approve"), "allowance"), "the allowance still grants");
        assert_eq!(f(true, Never, false, None), (Some("--approve"), "allowance"));
        // Shadowing wins over every grant, and is never "ask".
        for (allowed, e) in [(true, None), (false, t(true)), (true, t(true)), (false, None)] {
            for mode in [Never, Ask] {
                assert_eq!(f(allowed, mode, true, e.clone()), (Some("--no-approve"), "shadowed"));
            }
        }
        assert!(pi_launch_trust_for(false, Never, true, t(true)).would_approve);
        assert!(!pi_launch_trust_for(false, Never, true, t(false)).would_approve);
    }

    /// pi 0.99.1's `findNearestTrustEntry` and `readTrustFile`, case by case.
    #[test]
    fn pi_trust_resolves_as_pi_does() {
        let data = parse_pi_trust("\u{feff}{\"/Users/me/workspace\": true, \"/Users/me/workspace/repo/.worktrees/x\": false, \"/Users/me/workspace/other\": null}").unwrap();
        let at = |p: &str| nearest_pi_trust(&data, Path::new(p));
        // An ancestor entry covers everything below it.
        assert_eq!(at("/Users/me/workspace/repo/.worktrees/lane"), Some(PiOwnTrust { path: "/Users/me/workspace".into(), trusted: true }));
        assert_eq!(at("/Users/me/workspace").map(|e| e.trusted), Some(true));
        // The NEAREST decision wins, including a closer `false`.
        assert_eq!(at("/Users/me/workspace/repo/.worktrees/x/sub"), Some(PiOwnTrust { path: "/Users/me/workspace/repo/.worktrees/x".into(), trusted: false }));
        // `null` is no decision: the walk continues to the ancestor.
        assert_eq!(at("/Users/me/workspace/other").map(|e| e.path), Some("/Users/me/workspace".into()));
        // Outside every entry; and keys are exact strings, never prefixes.
        assert_eq!(at("/Users/me/work"), None);
        assert_eq!(at("/Users/me/workspace2/r"), None);
        // `/` itself is a key pi would check.
        let root = parse_pi_trust(r#"{"/": false}"#).unwrap();
        assert_eq!(nearest_pi_trust(&root, Path::new("/a/b")).map(|e| e.trusted), Some(false));
        // pi refuses the whole file on a bad value or shape; so do we.
        assert!(parse_pi_trust(r#"{"/a": "yes"}"#).is_err());
        assert!(parse_pi_trust("[]").is_err());
        assert!(parse_pi_trust("{").is_err());
        assert!(parse_pi_trust("{}").unwrap().is_empty());
    }

    #[test]
    fn shadow_detection_reads_the_server_name_and_fails_closed() {
        assert!(mcp_json_defines(r#"{"mcpServers":{"worktrees":{"command":"sh"}}}"#, "worktrees"));
        assert!(mcp_json_defines(r#"{"mcpServers":{"worktrees":{"command":"sh","enabled":false}}}"#, "worktrees"));
        assert!(!mcp_json_defines(r#"{"mcpServers":{"other":{"command":"sh"}}}"#, "worktrees"));
        assert!(!mcp_json_defines(r#"{"autoEnableCodemode":false}"#, "worktrees"));
        assert!(mcp_json_defines("{ not json", "worktrees"), "unparsable counts as shadowing");

        let dir = std::env::temp_dir().join(format!("wt-shadow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".pi")).unwrap();
        let wt = dir.to_string_lossy().to_string();
        std::fs::remove_dir(dir.join(".pi")).unwrap();
        assert!(!pi_mcp_shadowed(&wt), "no .pi/mcp.json: nothing to shadow");
        std::fs::create_dir_all(dir.join(".pi")).unwrap();
        std::fs::write(dir.join(".pi/mcp.json"), r#"{"mcpServers":{"worktrees":{"command":"sh"}}}"#).unwrap();
        assert!(pi_mcp_shadowed(&wt));
        std::fs::write(dir.join(".pi/mcp.json"), r#"{"mcpServers":{"lint":{"command":"sh"}}}"#).unwrap();
        assert!(!pi_mcp_shadowed(&wt));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trust_modes_parse_narrowly() {
        assert_eq!(PiTrust::parse("ask"), Some(PiTrust::Ask));
        assert_eq!(PiTrust::parse("never"), Some(PiTrust::Never));
        assert_eq!(PiTrust::parse("always"), None, "there is no global yes");
    }
}

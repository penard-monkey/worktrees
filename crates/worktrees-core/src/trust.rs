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
//! - `pi_project_trust = "ask"` (a user setting) passes neither flag, and pi
//!   asks — the modal the activity reader reports as waiting.
//! - A repo in the user's allowance, `[trust] pi = ["<repo root>", …]` in
//!   `~/.config/worktrees/config.toml`, launches with `--approve`. That is the
//!   ONLY case worktrees passes it.
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

pub fn is_allowed(harness: &str, repo_root: &str) -> bool {
    allowed(harness).iter().any(|r| r == repo_root)
}

/// pi's trust flag for a launch in `wt`, already a shell word: `--approve` for
/// an allowed repo, `--no-approve` under `never`, nothing under `ask`.
pub fn pi_flag(wt: &str) -> Option<&'static str> {
    if repo_root(wt).is_some_and(|r| is_allowed("pi", &r)) {
        return Some("--approve");
    }
    match pi_trust() {
        PiTrust::Never => Some("--no-approve"),
        PiTrust::Ask => None,
    }
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
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let mut list = allowed_in(&text, harness);
    let had = list.iter().any(|r| r == repo_root);
    if had == allow {
        return Ok(false);
    }
    if allow {
        list.push(repo_root.to_string());
    } else {
        list.retain(|r| r != repo_root);
    }
    let new = with_allowed(&text, harness, &list)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let tmp: PathBuf = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, new).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))?;
    Ok(true)
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
    let Some(root) = repo_root(&wt) else {
        ui.error(&format!("{wt} is not inside a git repository"));
        return 1;
    };
    match set_allowed(harness, &root, !revoke) {
        Ok(changed) => {
            let state = match (revoke, changed) {
                (false, true) => "now allowed: pi lanes in this repo launch with --approve and load its .pi/ and .agents/skills (repo code runs inside pi).",
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

    #[test]
    fn trust_modes_parse_narrowly() {
        assert_eq!(PiTrust::parse("ask"), Some(PiTrust::Ask));
        assert_eq!(PiTrust::parse("never"), Some(PiTrust::Never));
        assert_eq!(PiTrust::parse("always"), None, "there is no global yes");
    }
}

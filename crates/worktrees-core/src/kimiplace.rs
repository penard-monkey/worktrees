//! The files worktrees writes INTO a place for kimi, and owns.
//!
//! kimi has no per-launch flag for an MCP server, a skill or a system prompt
//! (`--skills-dir` REPLACES discovery rather than adding to it), and its user
//! config is kimi's to write, not ours. What it does read, per directory, is
//! `<cwd>/.kimi-code/`: `mcp.json` (layered last, so it wins by name over the
//! repo's `.mcp.json` and the user's own), `AGENTS.md` (read beside the
//! directory's plain AGENTS.md) and `skills/<name>/SKILL.md`. Measured on
//! kimi 2.2.0. So the per-launch delivery is three files in the place:
//!
//! - **owned**: each carries a mark (`"//"` in the JSON, a first-line comment
//!   in AGENTS.md, a `.worktrees` file beside the skill), and a file without
//!   it — or a path the repo TRACKS — is never written or removed; the launch
//!   says why instead;
//! - **excluded** in the repo's COMMON `info/exclude`, anchored, so the lines
//!   cover every place of the repo and nothing the user keeps in `.kimi-code/`;
//! - **gone with the place** (they live in its directory), and removed when
//!   their setting is turned off, so "off" means off at the next launch.
//!
//! kimi loads them only for a trusted folder — its own prompt, or the user's
//! allowance (`trust::kimi_launch_trust`).

use std::path::{Path, PathBuf};

/// The server's name in `mcpServers`; kimi names its tools `mcp__worktrees__*`.
pub const SERVER_KEY: &str = "worktrees";
/// The mark in our `mcp.json` (its `"//"` key) and the start of AGENTS.md's.
pub const MARK: &str = "written by worktrees for this place";
/// What the server is told about its client.
pub const PROVIDER_ENV: (&str, &str) = ("WORKTREES_MCP_PROVIDER", "kimi");

const DIR: &str = ".kimi-code";
const SKILL_DIR: &str = "skills/worktrees";
const SKILL_MARK: &str = ".worktrees";

/// The lines in `info/exclude`. Anchored (`/`): relative to each worktree's
/// root, which is every place's own `.kimi-code/`.
pub const EXCLUDES: &[&str] = &["/.kimi-code/mcp.json", "/.kimi-code/AGENTS.md", "/.kimi-code/skills/worktrees/"];
/// The comment above them, so whoever reads the file knows whose they are.
pub const EXCLUDE_HEADER: &str = "# worktrees: kimi's per-place files (removed once no place has any)";

pub fn mcp_path(wt: &str) -> PathBuf {
    Path::new(wt).join(DIR).join("mcp.json")
}
pub fn agents_path(wt: &str) -> PathBuf {
    Path::new(wt).join(DIR).join("AGENTS.md")
}
pub fn skill_dir(wt: &str) -> PathBuf {
    Path::new(wt).join(DIR).join(SKILL_DIR)
}

/// The `kimi_mcp` user setting: whether kimi lanes get the worktrees server,
/// and with which tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpMode {
    Off,
    ReadOnly,
    Mutations,
}

impl McpMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "off" => Some(Self::Off),
            "read-only" => Some(Self::ReadOnly),
            "mutations" => Some(Self::Mutations),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReadOnly => "read-only",
            Self::Mutations => "mutations",
        }
    }
}

/// `$WORKTREES_KIMI_MCP` > `kimi_mcp` in the user config > off. Off by
/// default, like every other harness's server: `worktrees mcp --install --ai
/// kimi` is the act that turns it on (mutating unless `--read-only`).
pub fn mcp_mode() -> McpMode {
    env_mode()
        .or_else(|| crate::config::user_cfg("kimi_mcp").as_deref().and_then(McpMode::parse))
        .unwrap_or(McpMode::Off)
}

/// `$WORKTREES_KIMI_MCP`, when it is set to one of the three words — it then
/// decides, whatever the config says.
fn env_mode() -> Option<McpMode> {
    std::env::var("WORKTREES_KIMI_MCP").ok().as_deref().and_then(McpMode::parse)
}

/// Our `mcp.json` for a place, pointing at `worktrees_bin`.
pub fn render_mcp(worktrees_bin: &str, mutations: bool) -> String {
    let mut args = vec!["mcp"];
    if mutations {
        args.push("--mutations");
    }
    let v = serde_json::json!({
        "//": format!("{MARK}; rewritten at each kimi launch, removed when kimi_mcp is off. Do not edit."),
        "mcpServers": {
            SERVER_KEY: {
                "command": worktrees_bin,
                "args": args,
                "env": { PROVIDER_ENV.0: PROVIDER_ENV.1 },
            }
        }
    });
    let mut s = serde_json::to_string_pretty(&v).unwrap_or_default();
    s.push('\n');
    s
}

/// Whether an `mcp.json` text is one worktrees wrote.
pub fn is_ours(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("//").and_then(|m| m.as_str()).map(|m| m.starts_with(MARK)))
        .unwrap_or(false)
}

/// Our AGENTS.md: the mark, then the rule.
pub fn render_agents(rules: &str) -> String {
    format!("<!-- {MARK}; rewritten at each kimi launch. Do not edit. -->\n\n{rules}\n")
}

fn agents_is_ours(text: &str) -> bool {
    text.starts_with(&format!("<!-- {MARK}"))
}

/// What one launch wants in the place. `None` for a file means "not wanted":
/// ours is removed if it is there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Want {
    pub mcp: Option<String>,
    pub agents: Option<String>,
    pub skill: Option<String>,
}

/// Which of the three paths the repo tracks, in one `git ls-files`.
fn tracked(wt: &str) -> Vec<String> {
    let out = std::process::Command::new("git")
        .args(["-C", wt, "ls-files", "--", ".kimi-code/mcp.json", ".kimi-code/AGENTS.md", ".kimi-code/skills/worktrees"])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// One file's rule: write `want` when the path is absent or ours, remove ours
/// when nothing is wanted, and leave anything else alone with a reason.
fn sync_file(path: &Path, want: Option<&str>, ours: impl Fn(&str) -> bool, tracked: bool, what: &str) -> Option<String> {
    let current = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Some(format!("{what}: could not read {}: {e}", path.display())),
    };
    let foreign = tracked || current.as_deref().is_some_and(|t| !ours(t));
    if foreign {
        return want.map(|_| {
            format!(
                "{what}: {} is the repo's own{} — worktrees does not overwrite it, so kimi launches without it",
                path.display(),
                if tracked { " (tracked)" } else { "" }
            )
        });
    }
    match (want, current.as_deref()) {
        (Some(w), Some(c)) if w == c => None,
        (Some(w), _) => {
            if let Some(dir) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    return Some(format!("{what}: could not create {}: {e}", dir.display()));
                }
            }
            std::fs::write(path, w).err().map(|e| format!("{what}: could not write {}: {e}", path.display()))
        }
        (None, Some(_)) => std::fs::remove_file(path).err().map(|e| format!("{what}: could not remove {}: {e}", path.display())),
        (None, None) => None,
    }
}

/// Bring a place's kimi files to `want`. Returns one warning per file it could
/// not deliver; an empty list is the normal answer. Excludes are added to the
/// repo's common `info/exclude` before anything is written.
pub fn sync(wt: &str, git_common: &str, want: &Want) -> Vec<String> {
    let mut warn = Vec::new();
    if want.mcp.is_some() || want.agents.is_some() || want.skill.is_some() {
        if let Err(e) = ensure_excluded(git_common) {
            warn.push(format!("could not exclude the kimi files from git: {e}"));
        }
    }
    let tracked = tracked(wt);
    let is_tracked = |rel: &str| tracked.iter().any(|t| t == rel || t.starts_with(&format!("{rel}/")));
    warn.extend(sync_file(&mcp_path(wt), want.mcp.as_deref(), is_ours, is_tracked(".kimi-code/mcp.json"), "kimi MCP"));
    warn.extend(sync_file(&agents_path(wt), want.agents.as_deref(), agents_is_ours, is_tracked(".kimi-code/AGENTS.md"), "kimi rule"));
    // The skill: SKILL.md must start with its frontmatter, so its mark is a
    // sibling file — and a skill dir without that mark is not ours.
    let dir = skill_dir(wt);
    let mark = dir.join(SKILL_MARK);
    let ours = !dir.exists() || mark.is_file();
    let skill_tracked = is_tracked(".kimi-code/skills/worktrees");
    if !ours || skill_tracked {
        if want.skill.is_some() {
            warn.push(format!(
                "kimi skill: {} is the repo's own — worktrees does not overwrite it, so kimi launches without the worktrees skill",
                dir.display()
            ));
        }
    } else if let Some(text) = &want.skill {
        warn.extend(sync_file(&dir.join("SKILL.md"), Some(text), |_| true, false, "kimi skill"));
        warn.extend(sync_file(&mark, Some(MARK), |_| true, false, "kimi skill"));
    } else if dir.exists() {
        // Only what we wrote: the skill and its mark. Anything else the user
        // put beside them stays, and so does the directory then.
        for f in [dir.join("SKILL.md"), mark.clone()] {
            match std::fs::remove_file(&f) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn.push(format!("kimi skill: could not remove {}: {e}", f.display())),
            }
        }
        let _ = std::fs::remove_dir(&dir);
    }
    // Nothing of ours left here: if no place of the repo has any either, our
    // exclude lines go too, so they never outlive the files they are for.
    if !ours_present(wt) {
        let places = worktree_dirs(wt);
        if !places.is_empty() && places.iter().all(|w| !ours_present(w)) {
            if let Err(e) = prune_excluded(git_common) {
                warn.push(format!("could not remove worktrees' kimi lines from info/exclude: {e}"));
            }
        }
    }
    // A file of the user's under one of OUR exclude lines is invisible to
    // `git status` and `git add -A` in every place — say so wherever one is.
    for rel in [".kimi-code/mcp.json", ".kimi-code/AGENTS.md", ".kimi-code/skills/worktrees/SKILL.md"] {
        let path = Path::new(wt).join(rel);
        let mine = match rel {
            ".kimi-code/mcp.json" => std::fs::read_to_string(&path).is_ok_and(|t| is_ours(&t)),
            ".kimi-code/AGENTS.md" => std::fs::read_to_string(&path).is_ok_and(|t| agents_is_ours(&t)),
            _ => mark.is_file(),
        };
        if path.is_file() && !mine && !is_tracked(rel) && hidden_by_us(wt, rel) {
            warn.push(format!(
                "{} is yours, but worktrees' line in {}/info/exclude hides it from git status and git add — \
                 `git add -f` it to keep it",
                path.display(),
                git_common
            ));
        }
    }
    warn
}

/// Whether `wt` holds any kimi file worktrees wrote.
pub fn ours_present(wt: &str) -> bool {
    std::fs::read_to_string(mcp_path(wt)).is_ok_and(|t| is_ours(&t))
        || std::fs::read_to_string(agents_path(wt)).is_ok_and(|t| agents_is_ours(&t))
        || skill_dir(wt).join(SKILL_MARK).is_file()
}

/// Every worktree of `wt`'s repo, the main checkout included.
fn worktree_dirs(wt: &str) -> Vec<String> {
    let out = std::process::Command::new("git").args(["-C", wt, "worktree", "list", "--porcelain"]).output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("worktree "))
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether `rel` in `wt` is ignored by one of OUR exclude lines.
fn hidden_by_us(wt: &str, rel: &str) -> bool {
    let out = std::process::Command::new("git").args(["-C", wt, "check-ignore", "-v", "--no-index", "--", rel]).output();
    match out {
        Ok(o) if o.status.success() => {
            let line = String::from_utf8_lossy(&o.stdout);
            line.contains("info/exclude:") && EXCLUDES.iter().any(|e| line.contains(&format!(":{e}\t")))
        }
        _ => false,
    }
}

/// Remove our lines (and their header) from `<git_common>/info/exclude`.
pub fn prune_excluded(git_common: &str) -> Result<(), String> {
    let excl = Path::new(git_common).join("info").join("exclude");
    let existing = match std::fs::read_to_string(&excl) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", excl.display())),
    };
    let kept: Vec<&str> = existing.lines().filter(|l| l.trim() != EXCLUDE_HEADER && !EXCLUDES.contains(&l.trim())).collect();
    if kept.len() == existing.lines().count() {
        return Ok(());
    }
    let mut body = kept.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    std::fs::write(&excl, body).map_err(|e| format!("{}: {e}", excl.display()))
}

/// Append any missing `EXCLUDES` line to `<git_common>/info/exclude`.
pub fn ensure_excluded(git_common: &str) -> Result<(), String> {
    let excl = Path::new(git_common).join("info").join("exclude");
    let existing = match std::fs::read_to_string(&excl) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", excl.display())),
    };
    let missing: Vec<&str> = EXCLUDES.iter().copied().filter(|l| !existing.lines().any(|x| x.trim() == *l)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    if let Some(dir) = excl.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut body = existing;
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    if !body.lines().any(|l| l.trim() == EXCLUDE_HEADER) {
        body.push_str(EXCLUDE_HEADER);
        body.push('\n');
    }
    for l in missing {
        body.push_str(l);
        body.push('\n');
    }
    std::fs::write(&excl, body).map_err(|e| format!("{}: {e}", excl.display()))
}

// ── the setting: `worktrees mcp --install|--uninstall|--status --ai kimi` ──

/// What `--status --ai kimi` reports.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    /// `mutations` · `read-only` · `off` · `cli-missing` · `kimi-missing`.
    pub state: &'static str,
    pub mode: McpMode,
    pub kimi_bin: Option<String>,
    pub worktrees_bin: Option<String>,
    /// Where it lands: each place's own file, written at launch.
    pub file: &'static str,
    /// `env` while `$WORKTREES_KIMI_MCP` decides (a change from Settings
    /// cannot take effect then), else `config`.
    pub source: &'static str,
    pub config_path: String,
}

pub fn status() -> Status {
    let mode = mcp_mode();
    let kimi_bin = crate::kimi::kimi_bin().map(|p| p.to_string_lossy().into_owned());
    let worktrees_bin = crate::profile::worktrees_bin().map(|p| p.to_string_lossy().into_owned());
    let state = if kimi_bin.is_none() {
        "kimi-missing"
    } else if mode != McpMode::Off && worktrees_bin.is_none() {
        "cli-missing"
    } else {
        mode.as_str()
    };
    Status {
        state,
        mode,
        kimi_bin,
        worktrees_bin,
        file: "<place>/.kimi-code/mcp.json",
        source: if env_mode().is_some() { "env" } else { "config" },
        config_path: crate::config::config_toml_path().to_string_lossy().into_owned(),
    }
}

/// `text` with the top-level `kimi_mcp` set to `mode` (removed for `Off`),
/// nothing else changed — a surgical edit of the user's own file, checked by
/// parsing the result back, the shape of `trust::with_allowed`.
pub fn with_mode(text: &str, mode: McpMode) -> Result<String, String> {
    use std::collections::BTreeMap;
    let parses = |t: &str| toml::from_str::<BTreeMap<String, toml::Value>>(t).ok();
    if !text.is_empty() && parses(text).is_none() {
        return Err("your config.toml does not parse; fix it first".into());
    }
    let mut out: Vec<String> = Vec::new();
    let mut at_root = true;
    let mut placed = false;
    let line = format!("kimi_mcp = \"{}\"", mode.as_str());
    for l in text.lines() {
        if l.trim_start().starts_with('[') {
            if at_root && !placed && mode != McpMode::Off {
                out.push(line.clone());
                placed = true;
            }
            at_root = false;
        } else if at_root && l.split('=').next().map(str::trim) == Some("kimi_mcp") {
            if mode != McpMode::Off && !placed {
                out.push(line.clone());
                placed = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !placed && mode != McpMode::Off {
        out.push(line);
    }
    let mut result = out.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    let back = parses(&result).ok_or("could not edit kimi_mcp in your config.toml safely — edit it by hand")?;
    let got = back.get("kimi_mcp").and_then(|v| v.as_str()).and_then(McpMode::parse).unwrap_or(McpMode::Off);
    if got != mode {
        return Err("could not edit kimi_mcp in your config.toml safely — edit it by hand".into());
    }
    Ok(result)
}

/// Set the user's `kimi_mcp`. The user's act (the CLI verb, the app); the MCP
/// server never calls it. Returns whether anything changed.
pub fn set_mode(mode: McpMode) -> Result<bool, String> {
    set_mode_guarded(env_mode(), &crate::config::config_toml_path(), mode)
}

/// `set_mode` over an explicit env reading, so a test proves the refusal
/// without touching the process environment.
fn set_mode_guarded(env: Option<McpMode>, path: &Path, mode: McpMode) -> Result<bool, String> {
    // Writing the config while the env decides would "work" and change
    // nothing — the setting reads the env first. Refuse, and say why.
    if let Some(e) = env {
        return Err(format!(
            "WORKTREES_KIMI_MCP={} is set in this process's environment and decides kimi's tools; unset it to change them here",
            e.as_str()
        ));
    }
    set_mode_at(path, mode)
}

pub fn set_mode_at(path: &Path, mode: McpMode) -> Result<bool, String> {
    crate::config::edit_user_config(path, |text| {
        let cur = toml::from_str::<std::collections::BTreeMap<String, toml::Value>>(text)
            .ok()
            .and_then(|t| t.get("kimi_mcp").and_then(|v| v.as_str()).and_then(McpMode::parse))
            .unwrap_or(McpMode::Off);
        if cur == mode {
            return Ok(None);
        }
        with_mode(text, mode).map(Some)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str) -> (PathBuf, String) {
        let t = std::env::temp_dir().join(format!("kimiplace-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let ok = std::process::Command::new("git").args(["init", "-q"]).current_dir(&t).status().unwrap().success();
        assert!(ok);
        let common = t.join(".git").to_string_lossy().into_owned();
        (t, common)
    }

    fn want_all() -> Want {
        Want {
            mcp: Some(render_mcp("/bin/worktrees", true)),
            agents: Some(render_agents("the rule")),
            skill: Some("---\nname: worktrees\ndescription: d\n---\n".into()),
        }
    }

    #[test]
    fn our_mcp_json_is_marked_and_names_the_client() {
        let s = render_mcp("/x/worktrees", true);
        assert!(is_ours(&s));
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        let e = &v["mcpServers"]["worktrees"];
        assert_eq!(e["command"], "/x/worktrees");
        assert_eq!(e["args"], serde_json::json!(["mcp", "--mutations"]));
        assert_eq!(e["env"]["WORKTREES_MCP_PROVIDER"], "kimi");
        let ro: serde_json::Value = serde_json::from_str(&render_mcp("/x/worktrees", false)).unwrap();
        assert_eq!(ro["mcpServers"]["worktrees"]["args"], serde_json::json!(["mcp"]));
        assert!(!is_ours(r#"{"mcpServers":{"worktrees":{"command":"/x/worktrees"}}}"#), "no mark, not ours");
        assert!(!is_ours("{not json"));
    }

    #[test]
    fn the_files_are_written_excluded_rewritten_and_removed_when_turned_off() {
        let (t, common) = repo("cycle");
        let wt = t.to_string_lossy().into_owned();
        assert_eq!(sync(&wt, &common, &want_all()), Vec::<String>::new());
        assert!(is_ours(&std::fs::read_to_string(mcp_path(&wt)).unwrap()));
        assert!(std::fs::read_to_string(agents_path(&wt)).unwrap().contains("the rule"));
        assert!(skill_dir(&wt).join("SKILL.md").is_file());
        let excl = std::fs::read_to_string(t.join(".git/info/exclude")).unwrap();
        for l in EXCLUDES.iter().chain([&EXCLUDE_HEADER]) {
            assert_eq!(excl.lines().filter(|x| x == l).count(), 1, "{l}");
        }
        // git sees none of it.
        let st = std::process::Command::new("git").args(["status", "--porcelain"]).current_dir(&t).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&st.stdout), "", "excluded");
        // Idempotent, and the exclude is not appended twice.
        assert!(sync(&wt, &common, &want_all()).is_empty());
        let again = std::fs::read_to_string(t.join(".git/info/exclude")).unwrap();
        assert_eq!(excl, again);
        // Off: ours go, and a user's own file beside them stays.
        std::fs::write(t.join(".kimi-code/tui-notes.md"), "mine").unwrap();
        assert!(sync(&wt, &common, &Want { mcp: None, agents: None, skill: None }).is_empty());
        assert!(!mcp_path(&wt).exists() && !agents_path(&wt).exists() && !skill_dir(&wt).exists());
        assert!(t.join(".kimi-code/tui-notes.md").is_file());
        // No place has any of ours now: the lines and their header go too.
        let after = std::fs::read_to_string(t.join(".git/info/exclude")).unwrap();
        assert!(EXCLUDES.iter().chain([&EXCLUDE_HEADER]).all(|l| !after.lines().any(|x| x == *l)), "{after}");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_file_we_did_not_write_is_never_touched() {
        let (t, common) = repo("foreign");
        let wt = t.to_string_lossy().into_owned();
        std::fs::create_dir_all(t.join(".kimi-code/skills/worktrees")).unwrap();
        let theirs = r#"{"mcpServers":{"db":{"command":"psql"}}}"#;
        std::fs::write(mcp_path(&wt), theirs).unwrap();
        std::fs::write(agents_path(&wt), "# their own kimi notes\n").unwrap();
        std::fs::write(skill_dir(&wt).join("SKILL.md"), "theirs").unwrap();
        let warn = sync(&wt, &common, &want_all());
        assert_eq!(warn.iter().filter(|w| w.contains("does not overwrite")).count(), 3, "{warn:?}");
        // Nothing of ours was written anywhere, so the lines this sync added
        // came straight back out: none of the user's files is hidden by us.
        assert_eq!(warn.len(), 3, "{warn:?}");
        let excl = std::fs::read_to_string(t.join(".git/info/exclude")).unwrap_or_default();
        assert!(EXCLUDES.iter().all(|l| !excl.lines().any(|x| x == *l)), "{excl}");
        assert_eq!(std::fs::read_to_string(mcp_path(&wt)).unwrap(), theirs);
        assert_eq!(std::fs::read_to_string(agents_path(&wt)).unwrap(), "# their own kimi notes\n");
        assert_eq!(std::fs::read_to_string(skill_dir(&wt).join("SKILL.md")).unwrap(), "theirs");
        // Turning it off removes nothing of theirs, and says nothing.
        assert!(sync(&wt, &common, &Want { mcp: None, agents: None, skill: None }).is_empty());
        assert!(mcp_path(&wt).is_file() && agents_path(&wt).is_file());
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Review of #509: our lines outlive the moment they were written for. A
    /// user who LATER writes their own `.kimi-code/AGENTS.md` must be told it
    /// is invisible to git, not discover it after a push without it.
    #[test]
    fn a_users_own_file_under_our_exclude_line_is_named_at_every_launch() {
        let (t, common) = repo("hidden");
        let wt = t.to_string_lossy().into_owned();
        let only_mcp = Want { mcp: Some(render_mcp("/bin/worktrees", true)), agents: None, skill: None };
        assert!(sync(&wt, &common, &only_mcp).is_empty());
        std::fs::write(agents_path(&wt), "# my own kimi notes\n").unwrap();
        let warn = sync(&wt, &common, &only_mcp);
        assert_eq!(warn.len(), 1, "{warn:?}");
        assert!(warn[0].contains("AGENTS.md is yours") && warn[0].contains("git add -f"), "{}", warn[0]);
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn turning_the_skill_off_leaves_what_the_user_put_beside_it() {
        let (t, common) = repo("skillkeep");
        let wt = t.to_string_lossy().into_owned();
        assert!(sync(&wt, &common, &want_all()).is_empty());
        std::fs::write(skill_dir(&wt).join("notes.md"), "mine").unwrap();
        sync(&wt, &common, &Want { mcp: None, agents: None, skill: None });
        assert!(!skill_dir(&wt).join("SKILL.md").exists() && !skill_dir(&wt).join(SKILL_MARK).exists());
        assert_eq!(std::fs::read_to_string(skill_dir(&wt).join("notes.md")).unwrap(), "mine");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn the_lines_stay_while_another_place_still_has_our_files() {
        let (t, common) = repo("stay");
        let wt = t.to_string_lossy().into_owned();
        let git = |a: &[&str]| assert!(std::process::Command::new("git").args(a).current_dir(&t).status().unwrap().success());
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"]);
        let other = t.join("other");
        git(&["worktree", "add", "-q", "-b", "other", other.to_str().unwrap()]);
        let ow = other.to_string_lossy().into_owned();
        assert!(sync(&ow, &common, &want_all()).is_empty());
        // This place turns it all off; the other still has ours.
        assert!(sync(&wt, &common, &Want { mcp: None, agents: None, skill: None }).is_empty());
        let excl = std::fs::read_to_string(t.join(".git/info/exclude")).unwrap();
        assert!(EXCLUDES.iter().all(|l| excl.lines().any(|x| x == *l)), "{excl}");
        // The other turns off too: now they go.
        assert!(sync(&ow, &common, &Want { mcp: None, agents: None, skill: None }).is_empty());
        let excl = std::fs::read_to_string(t.join(".git/info/exclude")).unwrap();
        assert!(EXCLUDES.iter().all(|l| !excl.lines().any(|x| x == *l)), "{excl}");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_tracked_path_is_refused_even_when_it_carries_our_mark() {
        let (t, common) = repo("tracked");
        let wt = t.to_string_lossy().into_owned();
        std::fs::create_dir_all(t.join(".kimi-code")).unwrap();
        // A committed copy of our own file: still the repo's now.
        let committed = render_mcp("/bin/worktrees", false);
        std::fs::write(mcp_path(&wt), &committed).unwrap();
        let git = |a: &[&str]| std::process::Command::new("git").args(a).current_dir(&t).output().unwrap();
        git(&["add", "-f", ".kimi-code/mcp.json"]);
        let warn = sync(&wt, &common, &want_all());
        assert!(warn.iter().any(|w| w.contains("(tracked)")), "{warn:?}");
        assert_eq!(std::fs::read_to_string(mcp_path(&wt)).unwrap(), committed, "left as committed");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn the_setting_edit_touches_one_key() {
        let base = "# mine\nai_cmd = \"claude\"\n\n[trust]\npi = [\"/r\"]\n";
        let on = with_mode(base, McpMode::Mutations).unwrap();
        assert_eq!(on, "# mine\nai_cmd = \"claude\"\n\nkimi_mcp = \"mutations\"\n[trust]\npi = [\"/r\"]\n");
        let ro = with_mode(&on, McpMode::ReadOnly).unwrap();
        assert!(ro.contains("kimi_mcp = \"read-only\"") && !ro.contains("mutations"), "{ro}");
        let off = with_mode(&ro, McpMode::Off).unwrap();
        assert!(!off.contains("kimi_mcp"), "{off}");
        assert!(off.contains("[trust]\npi = [\"/r\"]"), "{off}");
        assert_eq!(with_mode("", McpMode::Mutations).unwrap(), "kimi_mcp = \"mutations\"\n");
        assert!(with_mode("[broken", McpMode::Mutations).is_err());
    }

    /// #510 review: with the env knob set, a write from Settings changed the
    /// file and nothing else — the button "worked" and the verdict stayed.
    #[test]
    fn the_env_knob_refuses_a_write_it_would_silently_override() {
        let d = std::env::temp_dir().join(format!("kimi-mode-env-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("config.toml");
        let e = set_mode_guarded(Some(McpMode::Off), &p, McpMode::Mutations).unwrap_err();
        assert!(e.contains("WORKTREES_KIMI_MCP=off"), "{e}");
        assert!(!p.exists(), "nothing was written");
        assert_eq!(set_mode_guarded(None, &p, McpMode::Mutations), Ok(true));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn set_mode_round_trips_through_a_link() {
        crate::config::assert_writes_through_a_link(
            "kimi_mcp",
            |p| assert_eq!(set_mode_at(p, McpMode::ReadOnly), Ok(true)),
            |t| t.contains("kimi_mcp = \"read-only\""),
        );
    }

    #[test]
    fn the_mode_reads_its_three_words_and_nothing_else() {
        assert_eq!(McpMode::parse("mutations"), Some(McpMode::Mutations));
        assert_eq!(McpMode::parse(" read-only "), Some(McpMode::ReadOnly));
        assert_eq!(McpMode::parse("off"), Some(McpMode::Off));
        assert_eq!(McpMode::parse("on"), None);
    }
}

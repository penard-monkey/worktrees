//! Config + naming resolution, faithful to the bash CLI.
//! Precedence: flag > env > user config > default. The config is parsed as
//! data, never executed.
//!
//! The user config is TWO files in `~/.config/worktrees` (respecting
//! `$XDG_CONFIG_HOME`): `config.toml` first, then the original kv `config` as a
//! permanent silent fallback — every install predating TOML keeps working, and
//! nobody is asked to migrate. Proposal §8 splits the formats on *who writes
//! them*: human files are TOML, machine files are JSON.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// tmux/name-safe prefix: lowercase, then every byte not in `[a-z0-9_-]` → `-`
/// (per-byte, NOT run-collapsing — matches `tr -c 'a-z0-9_-' '-'`).
pub fn sanitize_prefix(s: &str) -> String {
    s.chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Last matching `key = value` line (inline ` #…` stripped, trailing ws trimmed),
/// or `None`. Mirrors the bash `cfg_get` sed pipeline.
pub fn cfg_get(cfg_path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(cfg_path).ok()?;
    let mut found = None;
    for line in text.lines() {
        if let Some(v) = parse_kv_line(line.trim_start(), key) {
            found = Some(v);
        }
    }
    found
}

/// `key <ws>* = <ws>* value` (line already left-trimmed) → the value, with an
/// inline ` #…` comment and trailing whitespace removed.
fn parse_kv_line(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?;
    let mut val = rest.trim_start().to_string();
    if let Some(idx) = find_inline_comment(&val) {
        val.truncate(idx);
    }
    Some(val.trim_end().to_string())
}

/// Index of a ` #` / `\t#` inline comment (whitespace immediately before `#`).
fn find_inline_comment(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    (1..b.len()).find(|&i| b[i] == b'#' && (b[i - 1] == b' ' || b[i - 1] == b'\t')).map(|i| i - 1)
}

/// `$XDG_CONFIG_HOME/worktrees` (or `~/.config/worktrees`).
fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{home}/.config")
        });
    PathBuf::from(base).join("worktrees")
}

/// The original kv config. Still read, forever — see the module docstring.
pub fn config_path() -> PathBuf {
    config_dir().join("config")
}

/// The TOML user config, which wins over `config_path()` key by key.
pub fn config_toml_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// One top-level string key from `config.toml`, or `None`.
///
/// Lenient on purpose, unlike `.worktrees.toml`: this file is the USER's, the
/// resolvers below have no error channel (they return a `String`), and a typo
/// here must never lock someone out of their own tool — the kv `config` still
/// answers. Non-string values are `None` (the three keys are all strings).
pub fn cfg_toml_get(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let table: BTreeMap<String, toml::Value> = toml::from_str(&text).ok()?;
    table.get(key)?.as_str().map(|s| s.to_string())
}

/// The user tier of the precedence chain: `config.toml` wins over the kv
/// `config` when both define a key. Pure form for testing (`*_from` convention).
pub fn user_cfg_from(toml_val: Option<&str>, kv_val: Option<&str>) -> Option<String> {
    toml_val
        .filter(|s| !s.is_empty())
        .or(kv_val.filter(|s| !s.is_empty()))
        .map(|s| s.to_string())
}

/// Live user-config lookup for one key (`ai_cmd`, `ai_resume_arg`, `prefix`).
pub fn user_cfg(key: &str) -> Option<String> {
    user_cfg_from(
        cfg_toml_get(&config_toml_path(), key).as_deref(),
        cfg_get(&config_path(), key).as_deref(),
    )
}

/// AI pane command: flag > `$WORKTREES_AI_CMD` > `$WORKTREES_CLAUDE_CMD` (deprecated)
/// > `ai_cmd` config > default `claude`. `none` → empty (plain shell). Pure form
/// for testing.
pub fn resolve_ai_cmd_from(
    flag: Option<&str>,
    env_ai: Option<&str>,
    env_claude: Option<&str>,
    cfg: Option<&str>,
) -> String {
    let v = flag
        .filter(|s| !s.is_empty())
        .or(env_ai.filter(|s| !s.is_empty()))
        .or(env_claude.filter(|s| !s.is_empty()))
        .or(cfg.filter(|s| !s.is_empty()))
        .unwrap_or("claude");
    if v == "none" {
        String::new()
    } else {
        v.to_string()
    }
}

/// The prefix chain (§5), as a pure function:
///
/// ```text
/// $WORKTREES_PREFIX > .worktree-prefix > [project] prefix > user config > basename(main_root)
/// ```
///
/// The legacy `.worktree-prefix` file comes FIRST among the two project-scoped
/// sources on purpose: every repo that already ships one keeps the name it has,
/// so adding `[project] prefix` alongside it is a no-op rather than a rename.
/// `doctor` reports the disagreement instead of the tool silently picking.
///
/// ⚠ Everything leaves through `sanitize_prefix`, and that is what makes a
/// PROJECT-settable prefix safe at all: a cloned repo's string is reduced to
/// `[a-z0-9_-]`, so it can carry no shell metacharacter, no path separator and
/// no whitespace into a tmux session name or a `docker -p` argument. The
/// allow-list in §5 rests on this one call — do not add a path that skips it.
///
/// It does NOT strip a leading `-` (that character is inside the allow-list, and
/// this function is a faithful port of bash's `tr -c 'a-z0-9_-' '-'` — the
/// behaviour is long-standing and callers depend on it). `profile::sanitize_id`
/// does strip one, because a profile id becomes a bare path component.
pub fn resolve_prefix_from(
    env: Option<&str>,
    prefix_file: Option<&str>,
    project: Option<&str>,
    cfg: Option<&str>,
    fallback: &str,
) -> String {
    // Whitespace-only is ABSENT, in every tier — the same rule the legacy file
    // tier has always applied (`project::prefix_file` strips whitespace and then
    // filters empty). Only checking `is_empty` let `[project] prefix = "   "`
    // win its rung and sanitize to `---`, so a repo could rename every session
    // to `----<slug>` with three spaces nobody can see in a diff.
    let present = |s: &&str| !s.trim().is_empty();
    let raw = env
        .filter(present)
        .or(prefix_file.filter(present))
        .or(project.filter(present))
        .or(cfg.filter(present))
        .unwrap_or(fallback);
    sanitize_prefix(raw)
}

/// Resume arg (`-r` appends it): `$WORKTREES_AI_RESUME_ARG` > `ai_resume_arg` > `-r`.
pub fn resolve_ai_resume_arg_from(env: Option<&str>, cfg: Option<&str>) -> String {
    env.filter(|s| !s.is_empty())
        .or(cfg.filter(|s| !s.is_empty()))
        .unwrap_or("-r")
        .to_string()
}

/// Live resolution reading env + user config.
pub fn resolve_ai_cmd(flag: Option<&str>) -> String {
    let env_ai = std::env::var("WORKTREES_AI_CMD").ok();
    let env_claude = std::env::var("WORKTREES_CLAUDE_CMD").ok();
    let cfg = user_cfg("ai_cmd");
    resolve_ai_cmd_from(flag, env_ai.as_deref(), env_claude.as_deref(), cfg.as_deref())
}

pub fn resolve_ai_resume_arg() -> String {
    let env = std::env::var("WORKTREES_AI_RESUME_ARG").ok();
    let cfg = user_cfg("ai_resume_arg");
    resolve_ai_resume_arg_from(env.as_deref(), cfg.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sanitize_matches_bash_tr() {
        assert_eq!(sanitize_prefix("My.Repo!"), "my-repo-");
        assert_eq!(sanitize_prefix("feat/Foo"), "feat-foo");
        assert_eq!(sanitize_prefix("a b"), "a-b");
        assert_eq!(sanitize_prefix("ok-name_1"), "ok-name_1");
    }

    #[test]
    fn cfg_last_match_wins_and_strips_inline_comment() {
        let dir = std::env::temp_dir().join(format!("wtcfgtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f, "ai_cmd = first").unwrap();
        writeln!(f, "ai_cmd = codex   # inline note").unwrap();
        writeln!(f, "prefix=teamx").unwrap();
        drop(f);
        assert_eq!(cfg_get(&p, "ai_cmd").as_deref(), Some("codex"));
        assert_eq!(cfg_get(&p, "prefix").as_deref(), Some("teamx"));
        assert_eq!(cfg_get(&p, "missing"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ai_cmd_precedence() {
        assert_eq!(resolve_ai_cmd_from(Some("flagcmd"), Some("envai"), None, Some("cfg")), "flagcmd");
        assert_eq!(resolve_ai_cmd_from(None, Some("envai"), Some("claudeenv"), Some("cfg")), "envai");
        assert_eq!(resolve_ai_cmd_from(None, None, Some("claudeenv"), Some("cfg")), "claudeenv");
        assert_eq!(resolve_ai_cmd_from(None, None, None, Some("cfg")), "cfg");
        assert_eq!(resolve_ai_cmd_from(None, None, None, None), "claude");
        assert_eq!(resolve_ai_cmd_from(Some("none"), None, None, None), "");
    }

    #[test]
    fn prefix_precedence_puts_the_legacy_file_ahead_of_the_project_config() {
        let r = |e, f, p, c| resolve_prefix_from(e, f, p, c, "reponame");
        // the full chain, one rung at a time
        assert_eq!(r(Some("envp"), Some("filep"), Some("projp"), Some("cfgp")), "envp");
        assert_eq!(r(None, Some("filep"), Some("projp"), Some("cfgp")), "filep");
        assert_eq!(r(None, None, Some("projp"), Some("cfgp")), "projp");
        assert_eq!(r(None, None, None, Some("cfgp")), "cfgp");
        assert_eq!(r(None, None, None, None), "reponame");
        // §5's migration rule, stated as an assertion: adding [project] prefix to
        // a repo that already ships .worktree-prefix renames NOTHING.
        assert_eq!(r(None, Some("legacy"), Some("brandnew"), None), "legacy");
        // an empty value is not a value, in every tier
        assert_eq!(r(Some(""), Some(""), Some(""), Some("")), "reponame");
        assert_eq!(r(Some(""), None, Some("projp"), None), "projp");
    }

    #[test]
    fn a_whitespace_only_prefix_is_absent_in_every_tier() {
        // The file tier has always read it this way (`project::prefix_file`
        // strips whitespace, then filters empty). The other tiers filtered only
        // `is_empty`, so `[project] prefix = "   "` won its rung and sanitized to
        // `---` — sessions named `----feat-x` from three invisible characters.
        let r = |e, f, p, c| resolve_prefix_from(e, f, p, c, "reponame");
        assert_eq!(r(None, None, Some("   "), None), "reponame");
        assert_eq!(r(Some(" \t "), None, None, None), "reponame");
        assert_eq!(r(None, Some("\t"), None, Some("cfgp")), "cfgp");
        // …and a blank rung is SKIPPED, not fatal: the next one still answers
        assert_eq!(r(Some("  "), None, Some("projp"), None), "projp");
    }

    #[test]
    fn a_hostile_project_prefix_cannot_survive_sanitization() {
        // THE reason §5 lets a project set this at all: whatever a cloned repo
        // writes is reduced to [a-z0-9_-] before it can reach a tmux session
        // name or a `docker -p` argument.
        for hostile in [
            "; rm -rf /",
            "$(whoami)",
            "../../etc",
            "--force",
            "a b\tc",
            "Team.X!",
            "sess'name",
            "\u{1b}[31m",
        ] {
            let got = resolve_prefix_from(None, None, Some(hostile), None, "reponame");
            assert!(
                got.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'),
                "{hostile} → {got}"
            );
        }
        assert_eq!(resolve_prefix_from(None, None, Some("Team.X!"), None, "r"), "team-x-");
        // and the fallback is sanitized too — a repo directory can be named anything
        assert_eq!(resolve_prefix_from(None, None, None, None, "My Repo"), "my-repo");
    }

    #[test]
    fn resume_arg_precedence() {
        assert_eq!(resolve_ai_resume_arg_from(Some("resume"), Some("--cont")), "resume");
        assert_eq!(resolve_ai_resume_arg_from(None, Some("--cont")), "--cont");
        assert_eq!(resolve_ai_resume_arg_from(None, None), "-r");
    }

    #[test]
    fn config_toml_wins_but_the_kv_file_stays_a_fallback() {
        assert_eq!(user_cfg_from(Some("codex"), Some("claude")).as_deref(), Some("codex"));
        assert_eq!(user_cfg_from(None, Some("claude")).as_deref(), Some("claude"));
        assert_eq!(user_cfg_from(Some("codex"), None).as_deref(), Some("codex"));
        assert_eq!(user_cfg_from(None, None), None);
        // an empty value is not a value, in either tier
        assert_eq!(user_cfg_from(Some(""), Some("claude")).as_deref(), Some("claude"));
        assert_eq!(user_cfg_from(Some(""), Some("")), None);
    }

    #[test]
    fn cfg_toml_reads_top_level_strings_and_shrugs_at_anything_else() {
        let dir = std::env::temp_dir().join(format!("wtcfgtoml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");

        std::fs::write(&p, "# note\nai_cmd = \"codex\"\nprefix = \"teamx\"\nai_auto = true\n").unwrap();
        assert_eq!(cfg_toml_get(&p, "ai_cmd").as_deref(), Some("codex"));
        assert_eq!(cfg_toml_get(&p, "prefix").as_deref(), Some("teamx"));
        assert_eq!(cfg_toml_get(&p, "ai_auto"), None, "non-string values are not config values");
        assert_eq!(cfg_toml_get(&p, "missing"), None);

        // a broken file falls through to the kv config rather than erroring out
        std::fs::write(&p, "ai_cmd = \n").unwrap();
        assert_eq!(cfg_toml_get(&p, "ai_cmd"), None);
        assert_eq!(cfg_toml_get(&dir.join("absent.toml"), "ai_cmd"), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

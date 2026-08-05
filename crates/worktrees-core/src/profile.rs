//! AI profiles — the user's own rules/skills/MCP/settings, applied to the AI pane
//! that worktrees launches, instead of their global `~/.claude` setup.
//!
//! Two locations, deliberately split (§8's rule: human files TOML, machine files
//! JSON — and *config* is not *data*):
//!
//! - `~/.config/worktrees/profiles.json` — the DECLARATIONS. Small, JSON,
//!   machine-written by the app, hand-editable in a pinch. Core reads it, so the
//!   CLI (`worktrees open`) and the app resolve identically; the app must never
//!   own this schema privately or the two binaries drift.
//! - `$XDG_DATA_HOME/worktrees/profiles/<id>/` — the MATERIALIZED config dir,
//!   handed to claude as `CLAUDE_CONFIG_DIR`. It is *data*, not config: claude
//!   writes transcripts, caches and shell snapshots in there and it grows without
//!   bound. Keeping that out of `~/.config` matters to anyone who backs up their
//!   dotfiles.
//!
//! ⚠ THE PATH IS AN IDENTITY, NOT JUST A LOCATION. Claude derives its macOS
//! keychain service name from the config-dir path (`Claude Code-credentials-<8
//! hex>`), so a profile's credential is bound to `profile_dir(id)`. Changing the
//! path scheme — or renaming a profile's `id` — silently invalidates the login
//! and the user is dropped back to `Not logged in` with no explanation. Hence:
//! `id` is assigned once at creation and is immutable; `name` is what the user
//! renames. See findings.md "AUTH SPIKE — RESOLVED".
//!
//! Worktrees never handles a credential. There is no code here that reads,
//! writes, copies or inspects a token — a profile's first launch shows
//! `Not logged in · Run /login` in the pane and the user signs in once. That is
//! the whole auth story, and it is why this module has no `security(1)` calls.

use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::sysclock::now_epoch;

const PROFILES_FILE: &str = "profiles.json";

/// The spelling that means "explicitly unprofiled" in every tier of the
/// resolution chain. Named once so the sentinel and the reservation that keeps
/// a real profile from claiming it cannot drift apart.
pub const RESERVED_ID: &str = "none";

/// Whether the LAUNCH side actually applies profiles yet.
///
/// This exists to keep the two halves of the seam from disagreeing. The probes
/// (`claude_config_dir_for_repo`) and the launch (`ops::ai_launch_for`) must
/// agree about where claude keeps its state, and they are implemented in
/// different phases. If the probes honoured a profile binding while the launch
/// still ran unprofiled, claude would write to `~/.claude` while the app looked
/// in the profile dir — auto-resume would silently stop passing `-r` and the
/// user's conversation would appear to vanish, with nothing in the log.
///
/// Both sides read this flag, so it flips in exactly one commit.
const LAUNCH_HONORS_PROFILES: bool = true;

/// Read by BOTH sides of the seam (`ops::ai_launch_for` and
/// `claude_config_dir_for_repo`) so neither can honour profiles without the
/// other.
pub(crate) fn launch_honors_profiles() -> bool {
    LAUNCH_HONORS_PROFILES
}

/// Serialize in-process writes (Tauri multi-window = one process), mirroring
/// `store::WRITE_LOCK`.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

// ── paths ────────────────────────────────────────────────────────────────────

/// `$XDG_CONFIG_HOME/worktrees` (or `~/.config/worktrees`) — same root the user
/// config lives in. Kept private and duplicated from `config.rs` rather than
/// exported from it, because that module's copy is about the *user config* and
/// this one is about *profile declarations*; a future move of either must not
/// silently drag the other along.
fn config_root() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.config", home()));
    PathBuf::from(base).join("worktrees")
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// The shared config root, for the skill store's manifest — it belongs beside
/// `profiles.json` for the same reason (declarations are config, content is data).
pub fn config_root_pub() -> PathBuf {
    config_root()
}

/// `~/.config/worktrees/profiles.json`.
pub fn profiles_path() -> PathBuf {
    config_root().join(PROFILES_FILE)
}

/// `$XDG_DATA_HOME/worktrees/profiles` (or `~/.local/share/worktrees/profiles`).
pub fn profiles_data_root() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.local/share", home()));
    PathBuf::from(base).join("worktrees").join("profiles")
}

/// The materialized `CLAUDE_CONFIG_DIR` for one profile. STABLE for the life of
/// the profile — see the module docstring on why this is an identity.
///
/// Returns `None` for an id that does not survive `sanitize_id`, so a hand-edited
/// `profiles.json` cannot aim this at `../../..`.
pub fn profile_dir(id: &str) -> Option<PathBuf> {
    let clean = sanitize_id(id);
    // Capped because this is a PERMANENT directory name and keychain identity:
    // a pasted 10k-character display name would otherwise mint an id that only
    // fails at create_dir_all with ENAMETOOLONG, long after it was recorded.
    if clean.is_empty() || clean != id || clean.len() > MAX_ID_LEN {
        return None;
    }
    Some(profiles_data_root().join(clean))
}

/// Comfortably under every filesystem's NAME_MAX, and far longer than any name
/// a human types.
pub const MAX_ID_LEN: usize = 64;

// ── ids ──────────────────────────────────────────────────────────────────────

/// Profile ids are `[a-z0-9_-]`, non-empty, and never `.`/`..`.
///
/// This is the same allow-list `config::sanitize_prefix` applies, and for the
/// same reason: the id becomes a DIRECTORY NAME under `profiles_data_root()` and
/// is interpolated into a shell string in the launch line. Anything that escapes
/// this set could carry a path separator (`../`), a shell metacharacter, or a
/// leading `-` that a later flag parser would read as an option.
pub fn sanitize_id(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Strip LEADING dashes. Without this the paragraph above is a lie: `-` is in
    // the allow-list, so a benign display name like "@Work" maps to `-work` — a
    // path component that a later argument parser reads as an option. It has to
    // happen here rather than at the call sites because `profile_dir` only
    // accepts an id that round-trips through this function, so an id that keeps
    // its dash would harden into a permanent directory + keychain identity.
    let mapped = mapped.trim_start_matches('-').to_string();
    // `.` and `..` cannot survive the map above (both become dashes), but a
    // dash-only string is still a useless directory name — reject it outright so
    // callers get an empty string to test rather than a `--` dir.
    if mapped.chars().all(|c| c == '-') {
        String::new()
    } else {
        mapped
    }
}

/// A fresh id derived from a display name, made unique against what already
/// exists. Pure so the collision rule is testable without touching disk.
pub fn new_id_from(name: &str, taken: &[String]) -> String {
    let base = {
        let mut s = sanitize_id(name);
        // Truncate BEFORE the collision suffix, so `-2` is never what pushes an
        // id over the cap. Only here, never in sanitize_id: that function is a
        // round-trip predicate for ids that already exist.
        if s.len() > MAX_ID_LEN {
            s.truncate(MAX_ID_LEN);
            s = s.trim_end_matches('-').to_string();
        }
        if s.is_empty() {
            "profile".to_string()
        } else {
            s
        }
    };
    // `none` is the unprofiled sentinel, so a profile can never be allowed to
    // own it — a profile named "None" would save and assign successfully and
    // then resolve as "no profile" at every launch, with no error anywhere.
    // Reserved HERE rather than by seeding callers' `taken` lists: this function
    // is pure and every future caller would otherwise have to remember to do it.
    if base != RESERVED_ID && !taken.iter().any(|t| t == &base) {
        return base;
    }
    // `-2`, `-3`, … rather than a hash: the id shows up as a directory name the
    // user may well look at, and `work-2` reads better than `work-3f9a1c`.
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|cand| !taken.iter().any(|t| t == cand))
        .unwrap_or(base)
}

// ── model ────────────────────────────────────────────────────────────────────

/// One profile. Unknown keys round-trip through `extra` so a newer app version
/// (or a hand edit) is not clobbered by an older one — same contract as
/// `store::Declared`.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Profile {
    /// Immutable. Directory name + keychain identity. Never renamed.
    pub id: String,
    /// What the user sees and can rename freely.
    pub name: String,

    /// Free markdown, materialized to `<dir>/rules.md` and delivered with
    /// `--append-system-prompt-file`. NOT a CLAUDE.md: the user's global
    /// `~/.claude/CLAUDE.md` loads regardless of the config-dir swap (verified,
    /// see findings.md), so a profile ADDS rules and cannot suppress theirs.
    #[serde(default)]
    pub rules: String,

    /// Skill-store entry names enabled for this profile.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Also expose the user's own `~/.claude/skills` in this profile.
    #[serde(default)]
    pub inherit_global_skills: bool,

    /// Raw `mcpServers` stanzas, written verbatim into `<dir>/mcp.json`. Kept as
    /// opaque JSON on purpose — the MCP config shape is claude's, not ours, and
    /// re-modelling it here would date badly.
    #[serde(default)]
    pub mcp_servers: Map<String, serde_json::Value>,
    /// Merge the user's global `mcpServers` in as well. When false the launch
    /// line adds `--strict-mcp-config`, which is what actually makes a profile
    /// able to REMOVE a noisy global server.
    #[serde(default)]
    pub inherit_global_mcp: bool,

    /// Expose worktrees' own MCP server (`worktrees mcp`) to this profile.
    #[serde(default)]
    pub worktrees_mcp: bool,

    /// `--model`, when the profile pins one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Raw `settings.json` body, materialized verbatim. Opaque for the same
    /// reason as `mcp_servers`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,

    /// Bumped on every save. The stale badge compares this against the value
    /// recorded when a live session launched — no polling, no hashing of the
    /// materialized dir (claude writes into that dir at runtime, so a content
    /// hash would never settle).
    #[serde(default)]
    pub updated_epoch: i64,

    /// The keychain service name claude derived for this profile's dir, recorded
    /// the first time we observe it. Deleting a profile must delete this item or
    /// a live OAuth token is orphaned in the user's keychain.
    ///
    /// Recorded, never computed: the `<8 hex>` suffix is undocumented and may
    /// change between claude versions. Reimplementing the hash would rot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keychain_service: Option<String>,

    #[serde(flatten)]
    pub extra: Map<String, serde_json::Value>,
}

/// The whole declaration file.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Profiles {
    #[serde(default)]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_epoch: Option<i64>,
    /// Applied to every repo without its own assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_id: Option<String>,
    /// By id.
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
    /// Repo main-root path → profile id. A project REPLACES the global default
    /// outright (the user's call): no stacking, no merge engine.
    #[serde(default)]
    pub assignments: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: Map<String, serde_json::Value>,
}

// ── resolution ───────────────────────────────────────────────────────────────

/// Which profile applies, as a pure function:
///
/// ```text
/// $WORKTREES_PROFILE > assignments[repo] > default_id > none
/// ```
///
/// `none` in any tier means "explicitly unprofiled" and stops the chain — the
/// same escape-hatch spelling `ai_cmd` already uses, so `WORKTREES_PROFILE=none
/// worktrees open foo` gives you a plain session without editing config.
///
/// An id that no longer exists resolves to `None` rather than erroring: a profile
/// deleted while a repo still points at it must degrade to an unprofiled launch,
/// not a broken one.
pub fn resolve_profile_id_from(
    env: Option<&str>,
    assigned: Option<&str>,
    default_id: Option<&str>,
    known: &[String],
) -> Option<String> {
    let present = |s: &&str| !s.trim().is_empty();
    let picked = env.filter(present).or(assigned.filter(present)).or(default_id.filter(present))?;
    if picked == "none" {
        return None;
    }
    known.iter().find(|k| k.as_str() == picked).cloned()
}

/// The profile in effect for a repo, if any.
pub fn resolve_profile(repo_root: &str) -> Option<Profile> {
    let ps = read_lenient();
    let env = std::env::var("WORKTREES_PROFILE").ok();
    let known: Vec<String> = ps.profiles.keys().cloned().collect();
    let id = resolve_profile_id_from(
        env.as_deref(),
        ps.assignments.get(repo_root).map(|s| s.as_str()),
        ps.default_id.as_deref(),
        &known,
    )?;
    ps.profiles.get(&id).cloned()
}

/// Live resolution for one repo root.
pub fn resolve_profile_id(repo_root: &str) -> Option<String> {
    let ps = read_lenient();
    let env = std::env::var("WORKTREES_PROFILE").ok();
    let known: Vec<String> = ps.profiles.keys().cloned().collect();
    resolve_profile_id_from(
        env.as_deref(),
        ps.assignments.get(repo_root).map(|s| s.as_str()),
        ps.default_id.as_deref(),
        &known,
    )
}

// ── where claude keeps its state ─────────────────────────────────────────────

/// The claude config root in effect for a repo: its bound profile's directory,
/// or the user's own `~/.claude`.
///
/// Everything that PROBES claude's on-disk state must go through this. Two such
/// probes are `$HOME`-anchored today and would silently stop working the moment
/// a profile swaps the config dir, with no error surfaced anywhere:
///
/// - `project::claude_session_present` — drives the app's auto-resume (`-r`).
///   Wrong root → the app decides there is no history → every profiled session
///   starts cold and the user loses their conversation.
/// - the app's `claude_activity` scan — drives the busy/waiting dots.
///   Wrong root → the dots simply never light for profiled places.
pub fn claude_config_dir_for_repo(repo_root: &str) -> PathBuf {
    // Gated so the probe side cannot get ahead of the launch side — see
    // LAUNCH_HONORS_PROFILES. Until the adapter emits CLAUDE_CONFIG_DIR, claude
    // is still writing to ~/.claude and that is where we must look, whatever
    // profiles.json says.
    if !LAUNCH_HONORS_PROFILES {
        return default_claude_dir();
    }
    resolve_profile_id(repo_root)
        .and_then(|id| profile_dir(&id))
        .unwrap_or_else(default_claude_dir)
}

/// `~/.claude` — the unprofiled default.
pub fn default_claude_dir() -> PathBuf {
    PathBuf::from(home()).join(".claude")
}

/// Every config root that might hold a live session probe: the user's own, plus
/// one per declared profile.
///
/// The activity scan reads probe files that each carry their own `cwd` and `pid`,
/// so the union is self-describing — we do not need to know which profile a given
/// place uses, and a place whose profile changed mid-session still lights up.
/// Directories that do not exist are harmless; the caller skips them.
pub fn claude_config_dirs_all() -> Vec<PathBuf> {
    let mut out = claude_config_dirs_from(&read_lenient());
    // Also every directory that EXISTS but is no longer declared. remove()
    // deliberately leaves the materialized dir on disk (it holds the user's
    // transcripts), so deleting a profile whose session is still running would
    // otherwise drop that root from the union and put the busy/waiting dot out
    // while the session is very much alive.
    if let Ok(rd) = fs::read_dir(profiles_data_root()) {
        for e in rd.flatten().filter(|e| e.path().is_dir()) {
            let p = e.path();
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// The declared half of the union, pure so it can be tested without touching the
/// developer's real `~/.config`.
fn claude_config_dirs_from(ps: &Profiles) -> Vec<PathBuf> {
    let mut out = vec![default_claude_dir()];
    for id in ps.profiles.keys() {
        // A hand-edited id that does not round-trip yields None and is skipped
        // rather than silently resolving to some other directory.
        if let Some(d) = profile_dir(id) {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

// ── the launch shape ─────────────────────────────────────────────────────────

/// What the AI pane actually launches, resolved once and carried as a struct.
///
/// THE POINT of this type is `match_word`. `ai_word` used to be re-derived in
/// three places (`ops::launch`, `project::adopt_ai_word`, the app's
/// `ai_is_claude`) as *the first whitespace-separated word of the command
/// string*, and `tmux::session_in` SUBSTRING-matches it against
/// `pane_current_command`. Prefix the command with `CLAUDE_CONFIG_DIR=… ` and
/// that derivation yields `CLAUDE_CONFIG_DIR=…` — session adoption silently
/// degrades to "first pane in the worktree" and auto-resume switches itself off,
/// with no error anywhere. So: env travels in `env`, never glued onto `cmd`, and
/// everything that needs to recognise the process reads `match_word`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiLaunch {
    /// `KEY=VALUE` pairs, composed INSIDE the `sh -ic` string at launch. Not
    /// `tmux new-session -e`: env baked into the process survives detach,
    /// reattach and a tmux server restart by definition, and the bats fake-tmux
    /// shim parses argv positionally with no `-e` case.
    pub env: Vec<(String, String)>,
    /// The command with its flags. Empty means "plain shell" (`ai_cmd = none`).
    pub cmd: String,
    /// Basename of the real program, for adoption/session matching. `claude`.
    pub match_word: String,
}

/// Compose the profiled launch for claude: the config-dir swap plus the flags
/// that carry the parts a swapped dir does not.
///
/// Flag ORDER matters. Everything the profile adds goes AFTER whatever `base`
/// already holds, because `base.cmd` may already end in the resume arg (`-r`)
/// and three bats assertions pin the ai word and resume arg as adjacent. Adding
/// on the end keeps those true.
///
/// Every value interpolated here goes through `shell_quote`: these end up inside
/// the `sh -ic` string, and `model` in particular is user-typed text.
pub fn claude_launch(base: &AiLaunch, p: &Profile, m: &Materialized) -> AiLaunch {
    let mut cmd = base.cmd.clone();
    let mut push = |flag: &str, val: &Path| {
        cmd.push(' ');
        cmd.push_str(flag);
        cmd.push(' ');
        cmd.push_str(&shell_quote(&val.to_string_lossy()));
    };

    if let Some(rules) = &m.rules {
        push("--append-system-prompt-file", rules);
    }
    if let Some(settings) = &m.settings {
        // Passed explicitly rather than relying on `<config dir>/settings.json`
        // being picked up: the flag is verified, the implicit path is inferred.
        push("--settings", settings);
    }
    if let Some(mcp) = &m.mcp {
        push("--mcp-config", mcp);
        // THE flag that makes a profile able to REMOVE a noisy global server
        // rather than only add to the set.
        if !p.inherit_global_mcp {
            cmd.push_str(" --strict-mcp-config");
        }
    }
    if let Some(model) = p.model.as_deref().filter(|s| !s.trim().is_empty()) {
        cmd.push_str(" --model ");
        cmd.push_str(&shell_quote(model));
    }

    AiLaunch {
        env: vec![("CLAUDE_CONFIG_DIR".to_string(), m.dir.to_string_lossy().into_owned())],
        cmd,
        // Unchanged, and that is the whole point of the type: tmux adoption and
        // the app's auto-resume gate match the PROGRAM, never this string.
        match_word: base.match_word.clone(),
    }
}

/// The one true `ai_word` derivation: first whitespace-separated word of the
/// resolved command, basename, defaulting to `claude`.
///
/// Previously copy-pasted in three files. Anything that matches a running pane
/// must call THIS, so the copies cannot drift apart again.
pub fn ai_word_of(ai_cmd: &str) -> String {
    let full = ai_cmd.split_whitespace().next().unwrap_or("");
    let word = if full.is_empty() { "claude" } else { full };
    basename(word)
}

fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

impl AiLaunch {
    /// An unprofiled launch — exactly today's behaviour.
    pub fn plain(ai_cmd: &str) -> Self {
        AiLaunch { env: Vec::new(), cmd: ai_cmd.to_string(), match_word: ai_word_of(ai_cmd) }
    }

    /// The env assignments as a shell prefix: `K='V' `, one per pair, in order.
    ///
    /// Values are single-quoted HERE by `shell_quote`, for the INNER shell that
    /// `sh -ic` runs. The caller separately wraps the whole body in `tmux::sq`
    /// for tmux's own parse — two nested layers, each quoting for a different
    /// reader.
    pub fn shell_prefix(&self) -> String {
        self.env
            .iter()
            // Values are quoted; a KEY cannot be, since `K=v` is shell syntax.
            // `env` is a pub field, so the day a phase adds profile-defined env
            // vars a key of `X; evil #` would inject straight into the inner
            // shell. Skipping is right: an unusable name is not a variable.
            .filter(|(k, _)| is_env_name(k))
            .map(|(k, v)| format!("{k}={} ", shell_quote(v)))
            .collect()
    }

    /// The body of pane0's `sh -ic` string: assignments, then the command, then
    /// the keep-alive.
    ///
    /// This lives here, called by `ops::launch`, rather than being spelled out
    /// inline at the call site — the assignments-before-command ORDER is the
    /// entire point of this type, and inline it was pinned by no test at any
    /// level (bats runs with an empty env, so reversing the order passed the
    /// whole suite).
    pub fn pane0_body(&self, keep: &str) -> String {
        format!("{}{}; {}", self.shell_prefix(), self.cmd, keep)
    }
}

/// A shell-safe environment variable NAME.
fn is_env_name(k: &str) -> bool {
    !k.is_empty()
        && k.chars().next().map(|c| c.is_ascii_uppercase() || c == '_').unwrap_or(false)
        && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Single-quote for POSIX sh: wrap in `'…'`, and end/re-open the quote around any
/// embedded `'`. Same trick as `tmux::sq`.
///
/// `pub(crate)` because the launch adapter appends profile-derived ARGUMENTS
/// (`--model <model>`, `--append-system-prompt-file <path>`) to `AiLaunch.cmd`,
/// and every one of them has to come through here — otherwise `model` is a
/// plain injection into the inner shell.
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── materialization ──────────────────────────────────────────────────────────
//
// Rendering a Profile into the directory claude is handed as CLAUDE_CONFIG_DIR.
//
// (Not to be confused with `materialize.rs`, which is about a worktree's
// `[[file]]` links. This is about a profile's claude config.)
//
// The directory is STABLE for the life of the profile, never per-launch: claude
// derives its keychain identity from this path, so a fresh directory per launch
// would demand a fresh `/login` every time. Only the files inside are rewritten.
//
// Every file is written temp-then-rename, because claude HOT-WATCHES its config
// dir: a plain truncating write would expose a half-file to a running session.

/// `$XDG_DATA_HOME/worktrees/skills` — the worktrees-owned skill store. Phase 7
/// owns installing into it; materialization only links out of it.
pub fn skills_store_root() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.local/share", home()));
    PathBuf::from(base).join("worktrees").join("skills")
}

/// A skill name is a PATH COMPONENT — `<dir>/skills/<name>` and
/// `<store>/<name>` — so it needs the same scrutiny an id gets. It comes from
/// the same hand-editable `profiles.json`, and without this a name of
/// `"../victim"` plants a symlink outside the profile dir entirely (the stale
/// sweep only reads `skills/`, so an escaped link is then orphaned forever).
///
/// Deliberately a REJECT, not a sanitize: a skill name must match a real store
/// directory, so silently rewriting it would just fail to find the skill in a
/// more confusing way.
fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(|c| std::path::is_separator(c))
        && !name.contains('\0')
}

/// What a materialization produced, so the launch adapter knows which flags are
/// worth passing (there is no point pointing `--mcp-config` at a file we did not
/// write).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Materialized {
    pub dir: PathBuf,
    pub rules: Option<PathBuf>,
    pub mcp: Option<PathBuf>,
    pub settings: Option<PathBuf>,
    /// Non-fatal problems worth surfacing in the UI/app log rather than failing
    /// the launch over — a skill that vanished from the store, say.
    pub warnings: Vec<String>,
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("no parent dir")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Temp in the SAME directory so rename(2) is atomic (same filesystem).
    // The name carries pid + a counter as well as the target's name: two
    // materializations of the SAME profile would otherwise pick the identical
    // temp path, and one could rename the other's half-written file into place —
    // exposing exactly the partial file this dance exists to prevent.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("f");
    let uniq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{uniq}.tmp", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Every filesystem location materialization touches, in one struct.
///
/// This exists so the whole thing is testable. The alternative — reading
/// `$XDG_DATA_HOME`/`$HOME` inside each function — would force tests to mutate
/// process-global env, which races under cargo's parallel test threads.
#[derive(Debug, Clone)]
pub struct MatPaths {
    /// The profile's config dir (becomes `CLAUDE_CONFIG_DIR`).
    pub dir: PathBuf,
    /// The worktrees-owned skill store.
    pub store: PathBuf,
    /// The user's own `~/.claude` (source for inherited skills).
    pub user_claude: PathBuf,
    /// The user's own `~/.claude.json` (seed for onboarding flags + global MCP).
    pub user_claude_json: PathBuf,
    /// The `worktrees` binary the MCP stanza should point at, resolved by the
    /// caller. Injected rather than probed inside, so the stanza is testable —
    /// `worktrees_bin()` reads `current_exe`/`PATH`, which is exactly the
    /// process-global state this struct exists to keep out of the tests.
    pub worktrees_bin: Option<PathBuf>,
}

impl MatPaths {
    /// The real locations for a profile id, or `None` if the id is not one we
    /// would ever have minted.
    pub fn for_profile(id: &str) -> Option<Self> {
        Some(MatPaths {
            dir: profile_dir(id)?,
            store: skills_store_root(),
            user_claude: default_claude_dir(),
            user_claude_json: PathBuf::from(home()).join(".claude.json"),
            worktrees_bin: worktrees_bin(),
        })
    }
}

/// Render `p` into its real directory, ready for a launch cwd'd in `worktree`.
pub fn materialize(p: &Profile, worktree: &str, repo_root: &str) -> Result<Materialized, String> {
    let paths = MatPaths::for_profile(&p.id).ok_or_else(|| format!("invalid profile id {:?}", p.id))?;
    materialize_with(&paths, p, worktree, repo_root)
}

/// The testable form: every location comes from `paths`.
///
/// `worktree` is needed for one thing only: the per-WORKTREE trust entry in
/// `.claude.json`. Trust is keyed by absolute path, so it has to be added at
/// launch time for the place being opened — a profile cannot pre-trust a
/// worktree that does not exist yet.
pub fn materialize_with(paths: &MatPaths, p: &Profile, worktree: &str, repo_root: &str) -> Result<Materialized, String> {
    let dir = paths.dir.clone();
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    // Same discipline the declaration store uses, for the same reason. Two
    // launches into the same profile (an app window and a `worktrees open`, or
    // two app windows) both read-modify-write `.claude.json`; unlocked, one
    // worktree's trust entry is silently dropped and that pane opens on a trust
    // dialog. Separate mutex from the store's — materialization never calls
    // edit(), and sharing one would invite a deadlock the first time it did.
    static MAT_LOCK: Mutex<()> = Mutex::new(());
    // Recover from poisoning rather than propagating it. The app is a long-lived
    // process: one panic anywhere under this lock would otherwise turn EVERY
    // later launch into a failed materialization until restart. The guarded
    // state is on disk and already protected by the dir lock + atomic renames,
    // so a poisoned flag tells us nothing useful about it.
    let _serial = MAT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _flock = DirLock::at(dir.with_extension("lock"))?;
    let mut out = Materialized { dir: dir.clone(), ..Default::default() };
    let user = read_user_claude_json(&paths.user_claude_json);

    // rules.md — delivered with --append-system-prompt-file. A CLAUDE.md here
    // would NOT do: the user's own ~/.claude/CLAUDE.md loads regardless of the
    // swap (verified), so profile rules ADD to theirs and cannot replace them.
    if !p.rules.trim().is_empty() {
        let f = dir.join("rules.md");
        write_atomic_bytes(&f, p.rules.as_bytes())?;
        out.rules = Some(f);
    } else {
        let _ = fs::remove_file(dir.join("rules.md"));
    }

    // settings.json — this IS the user-scope settings file under the swap, since
    // the whole config root relocated.
    if let Some(s) = &p.settings {
        let f = dir.join("settings.json");
        write_atomic_bytes(&f, serde_json::to_string_pretty(s).map_err(|e| e.to_string())?.as_bytes())?;
        out.settings = Some(f);
    }

    // mcp.json — passed with --mcp-config (+ --strict-mcp-config when not
    // inheriting). Never written into .claude.json: user-scope mcpServers are
    // exactly what a profile needs to be able to REMOVE.
    let mut servers = p.mcp_servers.clone();
    if p.inherit_global_mcp {
        for (k, v) in global_mcp_servers(&user) {
            // The profile's own definition wins a name collision — inheriting is
            // a convenience, not an override.
            servers.entry(k).or_insert(v);
        }
    }
    if p.worktrees_mcp {
        match paths.worktrees_bin.clone().filter(|b| b.is_absolute()) {
            Some(bin) => {
                servers.insert(
                    "worktrees".to_string(),
                    serde_json::json!({
                        "type": "stdio",
                        "command": bin.to_string_lossy(),
                        "args": ["mcp"],
                    }),
                );
            }
            // Resolved at materialize time, not baked in: the CLI may be
            // installed, moved or absent independently of the app bundle.
            None => out.warnings.push(
                "profile asks for the worktrees MCP server but no `worktrees` binary was found — skipping it".into(),
            ),
        }
    }
    let f = dir.join("mcp.json");
    write_atomic_bytes(&f, serde_json::to_string_pretty(&serde_json::json!({ "mcpServers": servers })).map_err(|e| e.to_string())?.as_bytes())?;
    out.mcp = Some(f);

    materialize_skills(paths, p, &mut out.warnings)?;
    materialize_claude_json(paths, &user, worktree, repo_root, &mut out.warnings)?;
    Ok(out)
}

/// `<dir>/skills/<name>` symlinks into the store (and optionally the user's own
/// skills).
///
/// SYMLINKS, not copies, so editing a skill in the store reaches a RUNNING
/// session — claude hot-watches the skills dir. The trade is that the app's
/// "profile changed, restart" badge cannot speak for skills; it covers rules,
/// settings, MCP and model only, and the UI must say so.
fn materialize_skills(paths: &MatPaths, p: &Profile, warnings: &mut Vec<String>) -> Result<(), String> {
    let skills = paths.dir.join("skills");
    fs::create_dir_all(&skills).map_err(|e| e.to_string())?;

    let mut wanted: BTreeMap<String, PathBuf> = BTreeMap::new();
    for name in &p.skills {
        if !valid_skill_name(name) {
            warnings.push(format!("skill name {name:?} is not a plain directory name — refusing it"));
            continue;
        }
        let src = paths.store.join(name);
        if src.is_dir() {
            wanted.insert(name.clone(), src);
        } else {
            warnings.push(format!("skill {name:?} is enabled but not in the store — skipping it"));
        }
    }
    if p.inherit_global_skills {
        if let Ok(rd) = fs::read_dir(paths.user_claude.join("skills")) {
            for e in rd.flatten().filter(|e| e.path().is_dir()) {
                let name = e.file_name().to_string_lossy().into_owned();
                // read_dir names cannot contain a separator, but the check is
                // cheap and keeps the invariant local to where it is relied on.
                if !valid_skill_name(&name) {
                    continue;
                }
                // A profile's own skill wins a name collision — inheriting must
                // never silently shadow something the user enabled explicitly.
                wanted.entry(name).or_insert_with(|| e.path());
            }
        }
    }

    // Drop links we no longer want. ONLY symlinks: if a real directory is
    // sitting here (a user poking around, or a claude-written file) we leave it
    // alone rather than deleting data we did not create.
    if let Ok(rd) = fs::read_dir(&skills) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_link = fs::symlink_metadata(e.path()).map(|m| m.file_type().is_symlink()).unwrap_or(false);
            if is_link && !wanted.contains_key(&name) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    for (name, src) in &wanted {
        let dst = skills.join(name);
        // Re-point an existing link rather than assuming it is current; the
        // store entry may have moved between launches.
        if fs::symlink_metadata(&dst).is_ok() {
            if fs::symlink_metadata(&dst).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
                let _ = fs::remove_file(&dst);
            } else {
                warnings.push(format!("{} exists and is not a symlink — leaving it", dst.display()));
                continue;
            }
        }
        // Belt and braces over valid_skill_name: whatever the name was, the
        // link must land directly inside this profile's skills dir.
        if dst.parent() != Some(skills.as_path()) {
            warnings.push(format!("refusing to link {name:?} outside {}", skills.display()));
            continue;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(src, &dst).map_err(|e| format!("link {name}: {e}"))?;
        #[cfg(not(unix))]
        {
            let _ = src;
            warnings.push(format!("skill {name:?} not linked — symlinks unsupported on this platform"));
        }
    }
    Ok(())
}

/// Seed `<dir>/.claude.json` from the user's real one.
///
/// COPIED WHOLESALE, then stripped — not assembled key by key. The real file has
/// ~80 top-level keys and claude gates interactive dialogs on several of them;
/// an early spike tried cherry-picking `hasCompletedOnboarding` + `theme` +
/// trust and still hit an "Allow external CLAUDE.md file imports?" prompt nobody
/// knew to seed, which parks a tmux pane on a dialog with nothing in the log.
/// Copying inherits every such flag, including ones future versions add.
///
/// Two keys are removed:
/// - `mcpServers` — the profile owns MCP via `--mcp-config`; leaving the user's
///   here would make "don't inherit global MCPs" impossible.
/// - `projects` — 40-odd other repos' history, and their `allowedTools`, which
///   would leak permission grants from unrelated repos into this profile.
fn materialize_claude_json(paths: &MatPaths, user: &serde_json::Value, worktree: &str, repo_root: &str, warnings: &mut Vec<String>) -> Result<(), String> {
    let target = paths.dir.join(".claude.json");
    // Start from what is already there, so a profile's own accumulated state
    // (its `projects` entries, its own flags) survives re-materialization.
    let mut root: serde_json::Value = fs::read(&target)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null);

    if !root.is_object() {
        if target.exists() {
            // It existed but would not parse. Re-seeding is the right recovery,
            // but the profile's accumulated state (trust entries, claude's own
            // counters) is being discarded — say so rather than doing it mutely.
            warnings.push(format!(
                "{} was unreadable and has been re-seeded — this profile's trusted worktrees were reset",
                target.display()
            ));
        }
        // First materialization: seed from the user's real file.
        root = user.clone();
        if let Some(o) = root.as_object_mut() {
            o.remove("mcpServers");
            o.remove("projects");
            // The module header claims worktrees never copies credential
            // material. A wholesale copy of a file whose key set another vendor
            // owns can only ASSERT that; this sweep ENFORCES it, and names what
            // it dropped (never the value) so a new key shape gets noticed
            // rather than silently carried.
            let dropped: Vec<String> = o
                .keys()
                .filter(|k| is_sensitive_key(k))
                .cloned()
                .collect();
            for k in &dropped {
                o.remove(k);
            }
            if !dropped.is_empty() {
                warnings.push(format!("not copied into this profile: {}", dropped.join(", ")));
            }
        }
    }
    // A user file that parses but is not an object (`[]`, `"x"`, `42`) must not
    // fail the launch — it is a file we do not own, and the doctrine everywhere
    // else here is degrade-and-warn rather than strand the user with no session.
    if !root.is_object() {
        warnings.push(format!(
            "{} is not a JSON object — starting this profile's config from scratch",
            paths.user_claude_json.display()
        ));
        root = serde_json::json!({});
    }
    let obj = root.as_object_mut().ok_or("claude config is not an object")?;

    // Trust: MIRROR, never invent.
    //
    // The trust dialog is the only gate in front of a repo's own
    // `.claude/settings.json` hooks, its `.mcp.json`, and its `CLAUDE.md`.
    // Pre-accepting it for a worktree of a freshly cloned repo would make
    // *enabling a profile* strictly weaker than running claude normally — the
    // opposite of what a profile is for. So we only carry across a decision the
    // user has already made in their own claude, for this repo.
    //
    // `hasClaudeMdExternalIncludesApproved` is NOT set under any circumstance:
    // it pre-approves `@`-imports that reach OUTSIDE the repo, so a hostile
    // CLAUDE.md could pull `@~/.ssh/config` into the model's context with no
    // prompt. That question is claude's to ask, every time.
    let already_trusted = user_trusts_in(user, repo_root) || user_trusts_in(user, worktree);
    if already_trusted {
        let projects = obj.entry("projects").or_insert_with(|| serde_json::json!({}));
        if let Some(pm) = projects.as_object_mut() {
            let entry = pm.entry(worktree.to_string()).or_insert_with(|| serde_json::json!({}));
            if let Some(em) = entry.as_object_mut() {
                em.insert("hasTrustDialogAccepted".into(), serde_json::Value::Bool(true));
                em.insert("hasCompletedProjectOnboarding".into(), serde_json::Value::Bool(true));
            }
        }
    }
    let bytes = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    write_atomic_bytes(&target, bytes.as_bytes())?;
    // Tightened even though the sweep above should have removed anything
    // sensitive: this is the one file seeded from a source we do not control,
    // and `fs::write` would otherwise leave it at 0644 regardless of how
    // restrictive the original was.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Keys never copied out of the user's `~/.claude.json`.
///
/// Two families: anything credential-shaped (the copy must not be able to
/// smuggle a token into a profile dir, or later into an export), and the
/// one-time acceptances of dangerous modes — inheriting those would let a
/// profiled session skip a confirmation the user granted in a different context.
fn is_sensitive_key(k: &str) -> bool {
    let lk = k.to_ascii_lowercase();
    // Substring match, so a key shape a future claude version introduces
    // (`someNewApiToken`) is caught without anyone updating this list.
    if ["key", "token", "secret", "credential", "oauth", "account", "bypasspermissions"]
        .iter()
        .any(|needle| lk.contains(needle))
    {
        return true;
    }
    // Stable identifiers. Not credentials, and harmless on this machine — but
    // they are identity, they regenerate on their own, and they must not ride
    // along into a profile someone later exports or shares.
    matches!(lk.as_str(), "userid" | "machineid" | "anonymousid")
}

/// True if the user's OWN claude already trusts `dir`, given their parsed file.
fn user_trusts_in(user: &serde_json::Value, dir: &str) -> bool {
    user.get("projects")
        .and_then(|p| p.get(dir))
        .and_then(|e| e.get("hasTrustDialogAccepted"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// The user's `~/.claude.json`, parsed once. It is ~135KB in practice and this
/// runs on every launch, so the three separate read+parse passes it used to take
/// (two trust checks + the MCP inherit) are collapsed into one.
fn read_user_claude_json(path: &Path) -> serde_json::Value {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// The user's global `mcpServers`, for the inherit toggle.
fn global_mcp_servers(user: &serde_json::Value) -> Map<String, serde_json::Value> {
    user.get("mcpServers").and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// Absolute path to a `worktrees` binary for the MCP stanza. Resolved fresh on
/// every materialization — an absolute path baked into a profile goes stale the
/// moment the binary moves or the profile is carried to another machine.
fn worktrees_bin() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if exe.file_name().and_then(|s| s.to_str()) == Some("worktrees") {
            return Some(exe);
        }
    }
    // The app links core in-process, so current_exe() is the app bundle — fall
    // back to whatever `worktrees` is on PATH.
    std::env::var("PATH").ok().and_then(|path| {
        path.split(':')
            // ABSOLUTE entries only. An empty PATH component (a trailing `:`,
            // which fixup_gui_path can produce) makes `PathBuf::join` yield the
            // RELATIVE path `worktrees` — resolved against the process cwd,
            // which for `worktrees open` is the cloned repo. A repo shipping a
            // file named `worktrees` would otherwise be written into mcp.json as
            // a command for claude to spawn.
            .filter(|d| d.starts_with('/'))
            .map(|d| PathBuf::from(d).join("worktrees"))
            .find(|c| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    c.metadata()
                        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                }
                #[cfg(not(unix))]
                {
                    c.is_file()
                }
            })
    })
}

// ── storage ──────────────────────────────────────────────────────────────────

/// For display and resolution: a missing or unparseable file is an EMPTY set of
/// profiles, never an error. A typo in `profiles.json` must degrade to
/// "unprofiled launches", not lock the user out of their own tool — the same
/// leniency `config::cfg_toml_get` applies for the same reason.
pub fn read_lenient() -> Profiles {
    let mut ps: Profiles = match fs::read(profiles_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Profiles::default(),
    };
    drop_incoherent(&mut ps);
    ps
}

/// Drop declarations whose map key disagrees with their own `id`.
///
/// `save()` keeps the two in step, but a hand-edited or imported file need not:
/// `profiles["safe"] = { "id": "work", … }` resolves as `safe` and then
/// materializes into `profiles/work/` — which IS the `work` profile's keychain
/// identity. One declaration would silently overwrite another logged-in
/// profile's rules, MCP config and settings.
fn drop_incoherent(ps: &mut Profiles) {
    ps.profiles.retain(|key, p| key == &p.id && sanitize_id(key) == *key);
    let known: Vec<String> = ps.profiles.keys().cloned().collect();
    ps.assignments.retain(|_, id| id == RESERVED_ID || known.contains(id));
}

/// For writes: refuse to clobber a file we cannot parse, so a hand-edit typo
/// stays human-repairable.
fn read_strict(path: &Path) -> Result<Profiles, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{PROFILES_FILE} is not valid JSON ({e}) — not overwriting")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Profiles::default()),
        Err(e) => Err(e.to_string()),
    }
}

struct DirLock(PathBuf);
impl DirLock {
    fn acquire(target: &Path) -> Result<Self, String> {
        Self::at(target.with_extension("json.lock"))
    }

    /// Lock at an explicit path — materialization locks a DIRECTORY, so it
    /// cannot use the `with_extension` convention the JSON files use.
    fn at(lock: PathBuf) -> Result<Self, String> {
        if let Some(parent) = lock.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // The retry horizon MUST exceed the staleness horizon below, or a lock
        // orphaned by a crash is unreclaimable for the difference: every launch
        // in that window fails. 100 × 15ms was 1.5s against a 15s staleness
        // threshold, so a SIGKILL mid-materialize broke launches for ~13.5s.
        for _ in 0..400 {
            match fs::create_dir(&lock) {
                Ok(_) => return Ok(DirLock(lock)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock)
                        .and_then(|m| m.modified())
                        // Short, because the critical section is a handful of
                        // file writes — not something that legitimately takes
                        // seconds. Paired with the retry budget above.
                        .map(|t| t.elapsed().map(|e| e.as_secs() > 3).unwrap_or(false))
                        .unwrap_or(false);
                    if stale {
                        let _ = fs::remove_dir_all(&lock);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                Err(e) => return Err(format!("lock error: {e}")),
            }
        }
        Err("could not acquire profiles lock".into())
    }
}
impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_atomic(path: &Path, ps: &Profiles) -> Result<(), String> {
    let dir = path.parent().ok_or("no parent dir for profiles")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(ps).map_err(|e| e.to_string())?;
    let tmp = dir.join(".profiles.json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Read-under-lock → mutate → atomic write. Preserves unknown keys and every
/// profile the closure does not touch.
pub fn edit<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce(&mut Profiles) -> Result<T, String>,
{
    let _serial = WRITE_LOCK.lock().map_err(|_| "profiles lock poisoned")?;
    let path = profiles_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let _flock = DirLock::acquire(&path)?;
    let mut ps = read_strict(&path)?;
    let out = f(&mut ps)?;
    // Never DOWNGRADE: the flattened `extra` maps let an older build rewrite a
    // newer file losslessly, but `version` is the one field it would clobber —
    // and a future v2 migration reading `version: 1` off a v2-shaped file would
    // mis-migrate it.
    ps.version = ps.version.max(1);
    ps.updated_epoch = Some(now_epoch());
    write_atomic(&path, &ps)?;
    Ok(out)
}

/// Create or replace one profile, stamping `updated_epoch` so the stale badge
/// can see the change. Returns the id.
pub fn save(mut p: Profile) -> Result<String, String> {
    let clean = sanitize_id(&p.id);
    if clean.is_empty() {
        return Err(format!("invalid profile id {:?}", p.id));
    }
    if clean != p.id {
        return Err(format!("profile id {:?} must be [a-z0-9_-]", p.id));
    }
    if p.id == RESERVED_ID {
        return Err(format!("{RESERVED_ID:?} is reserved — it means \"no profile\""));
    }
    if p.name.trim().is_empty() {
        p.name = p.id.clone();
    }
    p.updated_epoch = now_epoch();
    let id = p.id.clone();
    edit(|ps| {
        // Preserve the recorded keychain service across saves — the caller is a
        // UI form that has no reason to know about it, and losing it orphans a
        // token at delete time.
        if let Some(prev) = ps.profiles.get(&id) {
            if p.keychain_service.is_none() {
                p.keychain_service = prev.keychain_service.clone();
            }
        }
        ps.profiles.insert(id.clone(), p);
        Ok(())
    })?;
    Ok(id)
}

/// Bind a repo to a profile, or clear the binding with `None` (falling back to
/// the global default).
pub fn assign(repo_root: &str, id: Option<&str>) -> Result<(), String> {
    if repo_root.trim().is_empty() {
        return Err("empty repo root".into());
    }
    edit(|ps| {
        match id {
            Some(id) if !id.is_empty() => {
                if id != "none" && !ps.profiles.contains_key(id) {
                    return Err(format!("no such profile: {id}"));
                }
                ps.assignments.insert(repo_root.to_string(), id.to_string());
            }
            _ => {
                ps.assignments.remove(repo_root);
            }
        }
        Ok(())
    })
}

/// Set (or clear) the global default profile.
pub fn set_default(id: Option<&str>) -> Result<(), String> {
    edit(|ps| {
        match id {
            Some(id) if !id.is_empty() => {
                if !ps.profiles.contains_key(id) {
                    return Err(format!("no such profile: {id}"));
                }
                ps.default_id = Some(id.to_string());
            }
            _ => ps.default_id = None,
        }
        Ok(())
    })
}

/// Forget a profile: drop the declaration, every assignment pointing at it, and
/// the default if it was the default.
///
/// Returns the recorded keychain service name (if any) so the CALLER can delete
/// the keychain item. This module deliberately does not run `security(1)` —
/// keeping every credential-adjacent action out of core means the engine has no
/// code path that can touch a token, which is the property that made the whole
/// credential-copying design unnecessary in the first place.
///
/// The materialized directory is likewise left for the caller: it holds the
/// user's conversation transcripts, and silently `rm -rf`-ing those from a core
/// helper is not a decision this layer gets to make.
pub fn remove(id: &str) -> Result<Option<String>, String> {
    edit(|ps| {
        let gone = ps.profiles.remove(id).ok_or_else(|| format!("no such profile: {id}"))?;
        ps.assignments.retain(|_, v| v != id);
        if ps.default_id.as_deref() == Some(id) {
            ps.default_id = None;
        }
        Ok(gone.keychain_service)
    })
}

/// Record the keychain service name observed for a profile, once. Idempotent.
pub fn record_keychain_service(id: &str, service: &str) -> Result<(), String> {
    edit(|ps| {
        let p = ps.profiles.get_mut(id).ok_or_else(|| format!("no such profile: {id}"))?;
        if p.keychain_service.as_deref() != Some(service) {
            p.keychain_service = Some(service.to_string());
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_reduced_to_the_path_safe_alphabet() {
        assert_eq!(sanitize_id("Work Rules"), "work-rules");
        assert_eq!(sanitize_id("ok-name_1"), "ok-name_1");
        // the whole point: nothing here can escape a directory or a shell word
        for hostile in ["../../etc", "..", ".", "; rm -rf /", "$(whoami)", "a b\tc", "sess'name"] {
            let got = sanitize_id(hostile);
            assert!(
                got.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'),
                "{hostile} → {got}"
            );
            assert!(!got.contains('/'), "{hostile} kept a separator");
        }
        // a LEADING dash never survives — "@Work" would otherwise mint "-work",
        // a path component a later flag parser reads as an option
        assert_eq!(sanitize_id("-work"), "work");
        assert_eq!(sanitize_id("@Work"), "work");
        assert_eq!(sanitize_id("--force"), "force");
        // dash-only is not a usable directory name
        assert_eq!(sanitize_id(".."), "");
        assert_eq!(sanitize_id("///"), "");
        assert_eq!(sanitize_id(""), "");
    }

    #[test]
    fn profile_dir_refuses_an_id_it_did_not_sanitize() {
        // A hand-edited profiles.json cannot aim the config dir out of the root.
        assert!(profile_dir("../../etc").is_none());
        assert!(profile_dir("Work Rules").is_none(), "un-sanitized input is rejected, not silently fixed");
        assert!(profile_dir("").is_none());
        let ok = profile_dir("work").expect("a clean id resolves");
        assert!(ok.ends_with("profiles/work"));
    }

    #[test]
    fn the_unprofiled_sentinel_cannot_be_claimed_by_a_real_profile() {
        // A profile named "None" used to save, assign and even become the
        // default — and then resolve as "no profile" at every launch, silently.
        assert_eq!(new_id_from("None", &[]), "none-2");
        assert_eq!(new_id_from("none", &[]), "none-2");
        let p = Profile { id: RESERVED_ID.into(), name: "None".into(), ..Default::default() };
        assert!(save(p).is_err(), "save() must refuse the reserved id");
        // and the resolver still treats the spelling as an opt-out
        assert_eq!(resolve_profile_id_from(Some(RESERVED_ID), None, None, &["a".into()]), None);
    }

    #[test]
    fn new_ids_avoid_collisions_readably() {
        assert_eq!(new_id_from("Work", &[]), "work");
        assert_eq!(new_id_from("Work", &["work".into()]), "work-2");
        assert_eq!(new_id_from("Work", &["work".into(), "work-2".into()]), "work-3");
        // a name with nothing usable in it still yields a legal id
        assert_eq!(new_id_from("!!!", &[]), "profile");
        assert_eq!(new_id_from("!!!", &["profile".into()]), "profile-2");
    }

    #[test]
    fn profile_precedence_env_then_project_then_default() {
        let known = vec!["a".to_string(), "b".to_string(), "d".to_string()];
        let r = |e, a, d| resolve_profile_id_from(e, a, d, &known);
        assert_eq!(r(Some("a"), Some("b"), Some("d")).as_deref(), Some("a"));
        assert_eq!(r(None, Some("b"), Some("d")).as_deref(), Some("b"));
        assert_eq!(r(None, None, Some("d")).as_deref(), Some("d"));
        assert_eq!(r(None, None, None), None);
        // blank rungs are skipped, not fatal
        assert_eq!(r(Some("  "), Some("b"), None).as_deref(), Some("b"));
    }

    #[test]
    fn none_is_an_explicit_opt_out_that_stops_the_chain() {
        let known = vec!["a".to_string()];
        // `WORKTREES_PROFILE=none worktrees open foo` = a plain session, even
        // though both the project and the global default name a profile.
        assert_eq!(resolve_profile_id_from(Some("none"), Some("a"), Some("a"), &known), None);
        assert_eq!(resolve_profile_id_from(None, Some("none"), Some("a"), &known), None);
    }

    #[test]
    fn a_dangling_profile_id_degrades_to_unprofiled() {
        // Deleting a profile a repo still points at must not break `open`.
        let known = vec!["a".to_string()];
        assert_eq!(resolve_profile_id_from(None, Some("deleted"), None, &known), None);
        assert_eq!(resolve_profile_id_from(None, Some("deleted"), Some("a"), &known), None,
            "the assignment still WINS its rung — it just resolves to nothing");
    }

    #[test]
    fn ai_word_survives_an_env_prefixed_launch() {
        // The regression this whole struct exists to prevent.
        assert_eq!(ai_word_of("claude"), "claude");
        assert_eq!(ai_word_of("/opt/homebrew/bin/claude"), "claude");
        assert_eq!(ai_word_of("claude --model opus -r"), "claude");
        assert_eq!(ai_word_of(""), "claude");

        let l = AiLaunch {
            env: vec![("CLAUDE_CONFIG_DIR".into(), "/data/profiles/work".into())],
            cmd: "claude --append-system-prompt-file /data/profiles/work/rules.md".into(),
            match_word: ai_word_of("claude"),
        };
        assert_eq!(l.match_word, "claude", "adoption matches the program, not the env prefix");
        assert_eq!(l.shell_prefix(), "CLAUDE_CONFIG_DIR='/data/profiles/work' ");
        // …and the composed line still starts the program as pane0's foreground
        // process, which is what keeps pane_current_command == "claude".
        let line = format!("{}{}", l.shell_prefix(), l.cmd);
        assert!(line.starts_with("CLAUDE_CONFIG_DIR="));
        assert!(line.contains(" claude --append-system-prompt-file"));
    }

    #[test]
    fn pane0_body_puts_assignments_before_the_command() {
        let keep = "exec \"${SHELL:-/bin/sh}\"";
        // unprofiled: byte-for-byte what the bats suite has always pinned
        assert_eq!(
            AiLaunch::plain("claude -r").pane0_body(keep),
            "claude -r; exec \"${SHELL:-/bin/sh}\""
        );
        // profiled: assignments FIRST, or `pane_current_command` stops being
        // `claude` and tmux adoption breaks. Reversing the order in ops.rs used
        // to pass all 150 unit + 238 bats tests, because bats runs with no env.
        let l = AiLaunch {
            env: vec![("CLAUDE_CONFIG_DIR".into(), "/d/p/work".into())],
            cmd: "claude -r".into(),
            match_word: "claude".into(),
        };
        assert_eq!(
            l.pane0_body(keep),
            "CLAUDE_CONFIG_DIR='/d/p/work' claude -r; exec \"${SHELL:-/bin/sh}\""
        );
    }

    #[test]
    fn plain_launch_is_todays_behaviour_exactly() {
        let l = AiLaunch::plain("claude");
        assert!(l.env.is_empty());
        assert_eq!(l.shell_prefix(), "");
        assert_eq!(l.cmd, "claude");
        assert_eq!(l.match_word, "claude");
        // `ai_cmd = none` → plain shell, and the word still defaults sanely
        let none = AiLaunch::plain("");
        assert_eq!(none.cmd, "");
        assert_eq!(none.match_word, "claude");
    }

    #[test]
    fn env_values_with_shell_metacharacters_are_quoted() {
        let l = AiLaunch {
            env: vec![("CLAUDE_CONFIG_DIR".into(), "/tmp/a dir/it's".into())],
            cmd: "claude".into(),
            match_word: "claude".into(),
        };
        assert_eq!(l.shell_prefix(), "CLAUDE_CONFIG_DIR='/tmp/a dir/it'\\''s' ");
    }

    fn profiles_with(ids: &[&str]) -> Profiles {
        let mut ps = Profiles::default();
        for id in ids {
            ps.profiles.insert(
                (*id).to_string(),
                Profile { id: (*id).to_string(), name: (*id).to_string(), ..Default::default() },
            );
        }
        ps
    }

    #[test]
    fn probe_roots_cover_every_valid_profile_and_skip_the_rest() {
        // Built against a CONSTRUCTED Profiles rather than read_lenient(), so the
        // loop is actually exercised — on a machine with no profiles.json the
        // old version of this test asserted nothing at all.
        let ps = profiles_with(&["work", "oss", "Bad Id"]);
        let roots = claude_config_dirs_from(&ps);
        assert_eq!(roots[0], default_claude_dir(), "the user's own root comes first");
        assert!(roots.iter().any(|r| r.ends_with("profiles/work")));
        assert!(roots.iter().any(|r| r.ends_with("profiles/oss")));
        // a hand-edited id that does not round-trip is SKIPPED, never coerced
        // into some neighbouring directory
        assert!(!roots.iter().any(|r| r.to_string_lossy().contains("Bad Id")));
        assert!(!roots.iter().any(|r| r.ends_with("profiles/bad-id")));
        assert_eq!(roots.len(), 3);
        // duplicates would double-count probes and light two places from one session
        let mut d = roots.clone();
        d.sort();
        d.dedup();
        assert_eq!(d.len(), roots.len());
    }

    #[test]
    fn probes_and_launch_flip_together() {
        // THE invariant: while the launch side runs unprofiled, claude is still
        // writing to ~/.claude, so every probe must look there no matter what
        // profiles.json says. Getting this wrong loses auto-resume silently.
        if !launch_honors_profiles() {
            assert_eq!(
                claude_config_dir_for_repo("/any/repo"),
                default_claude_dir(),
                "probes must not honour a profile binding before the launch does"
            );
        }
    }

    // ── materialization ─────────────────────────────────────────────────────
    // Real files in a real temp dir, via MatPaths — no env mutation, so these
    // are safe under cargo's parallel test threads.

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        // thread id keeps parallel tests in the same process from colliding
        let t = std::env::temp_dir().join(format!(
            "wtprof-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        Tmp(t)
    }
    fn paths_in(root: &Path) -> MatPaths {
        MatPaths {
            dir: root.join("profile"),
            store: root.join("store"),
            user_claude: root.join("home/.claude"),
            user_claude_json: root.join("home/.claude.json"),
            worktrees_bin: None,
        }
    }
    /// Write a user `~/.claude.json`, optionally trusting some dirs.
    fn user_json(paths: &MatPaths, extra: serde_json::Value, trusted: &[&str]) {
        fs::create_dir_all(paths.user_claude_json.parent().unwrap()).unwrap();
        let mut v = extra;
        let projects: serde_json::Map<String, serde_json::Value> = trusted
            .iter()
            .map(|d| ((*d).to_string(), serde_json::json!({ "hasTrustDialogAccepted": true })))
            .collect();
        v["projects"] = serde_json::Value::Object(projects);
        fs::write(&paths.user_claude_json, v.to_string()).unwrap();
    }

    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
    }

    #[test]
    fn materialize_writes_rules_and_mcp() {
        let t = tmp("basic");
        let paths = paths_in(&t.0);
        let p = Profile {
            id: "work".into(),
            name: "Work".into(),
            rules: "be terse".into(),
            ..Default::default()
        };
        let out = materialize_with(&paths, &p, "/repo/.worktrees/feat", "/repo").unwrap();

        assert_eq!(read(out.rules.as_ref().unwrap()), "be terse");
        let mcp: serde_json::Value = serde_json::from_str(&read(out.mcp.as_ref().unwrap())).unwrap();
        assert!(mcp["mcpServers"].as_object().unwrap().is_empty());
        assert!(out.settings.is_none(), "no settings declared → no file to point --settings at");
    }

    #[test]
    fn trust_is_mirrored_from_the_user_never_invented() {
        // Enabling a profile must never be WEAKER than running claude normally.
        // The trust dialog is the only gate in front of a repo's own hooks,
        // .mcp.json and CLAUDE.md, so a freshly cloned repo must still be asked
        // about — we only carry across a decision the user already made.
        let t = tmp("trust-no");
        let paths = paths_in(&t.0);
        user_json(&paths, serde_json::json!({}), &[]);
        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        materialize_with(&paths, &p, "/repo/.worktrees/feat", "/repo").unwrap();
        let cj: serde_json::Value = serde_json::from_str(&read(&paths.dir.join(".claude.json"))).unwrap();
        assert!(
            cj["projects"].get("/repo/.worktrees/feat").is_none(),
            "an untrusted repo must not be silently pre-trusted"
        );

        // …but a repo the user already trusts carries over, so profiles do not
        // re-ask about codebases they have already accepted.
        let t2 = tmp("trust-yes");
        let paths2 = paths_in(&t2.0);
        user_json(&paths2, serde_json::json!({}), &["/repo"]);
        materialize_with(&paths2, &p, "/repo/.worktrees/feat", "/repo").unwrap();
        let cj2: serde_json::Value = serde_json::from_str(&read(&paths2.dir.join(".claude.json"))).unwrap();
        assert_eq!(cj2["projects"]["/repo/.worktrees/feat"]["hasTrustDialogAccepted"], serde_json::json!(true));
    }

    #[test]
    fn external_claude_md_imports_are_never_pre_approved() {
        // hasClaudeMdExternalIncludesApproved waves through `@`-imports that
        // reach OUTSIDE the repo — a hostile CLAUDE.md could pull
        // `@~/.ssh/config` into context with no prompt. Never set, not even for
        // a repo the user fully trusts.
        let t = tmp("imports");
        let paths = paths_in(&t.0);
        user_json(&paths, serde_json::json!({}), &["/repo"]);
        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        materialize_with(&paths, &p, "/repo/wt", "/repo").unwrap();
        let raw = read(&paths.dir.join(".claude.json"));
        assert!(!raw.contains("hasClaudeMdExternalIncludesApproved"), "{raw}");
        assert!(!raw.contains("hasClaudeMdExternalIncludesWarningShown"), "{raw}");
    }

    #[test]
    fn credential_shaped_keys_are_never_copied_into_a_profile() {
        // The module header claims worktrees never copies credential material.
        // This is what makes that a guarantee rather than a hope, given the seed
        // copies a file whose key set another vendor owns.
        let t = tmp("creds");
        let paths = paths_in(&t.0);
        user_json(
            &paths,
            serde_json::json!({
                "theme": "dark",
                "primaryApiKey": "sk-ant-SECRET",
                "customApiKeyResponses": { "approved": ["x"] },
                "oauthAccount": { "emailAddress": "a@b.c" },
                "userID": "uid",
                "bypassPermissionsModeAccepted": true
            }),
            &[],
        );
        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        let out = materialize_with(&paths, &p, "/repo/wt", "/repo").unwrap();
        let raw = read(&paths.dir.join(".claude.json"));
        assert!(!raw.contains("SECRET"), "a token reached the profile dir: {raw}");
        for k in ["primaryApiKey", "customApiKeyResponses", "oauthAccount", "userID", "bypassPermissionsModeAccepted"] {
            assert!(!raw.contains(k), "{k} was copied: {raw}");
        }
        assert!(raw.contains("theme"), "ordinary keys still come across");
        assert!(out.warnings.iter().any(|w| w.contains("not copied")), "{:?}", out.warnings);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(paths.dir.join(".claude.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "profile .claude.json must not be group/world readable");
        }
    }

    #[test]
    fn claude_json_is_seeded_wholesale_but_strips_mcp_and_other_repos() {
        let t = tmp("seed");
        let paths = paths_in(&t.0);
        fs::create_dir_all(paths.user_claude_json.parent().unwrap()).unwrap();
        fs::write(
            &paths.user_claude_json,
            serde_json::json!({
                "hasCompletedOnboarding": true,
                "theme": "dark",
                "someFutureDialogFlag": true,
                "mcpServers": { "global": { "command": "g" } },
                "projects": { "/other/repo": { "allowedTools": ["Bash(rm:*)"] } }
            })
            .to_string(),
        )
        .unwrap();

        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        materialize_with(&paths, &p, "/repo/wt", "/repo").unwrap();
        let cj: serde_json::Value = serde_json::from_str(&read(&paths.dir.join(".claude.json"))).unwrap();

        // inherited wholesale — including a flag this code has never heard of,
        // which is the entire point (cherry-picking left a dialog unseeded)
        assert_eq!(cj["hasCompletedOnboarding"], serde_json::json!(true));
        assert_eq!(cj["someFutureDialogFlag"], serde_json::json!(true));
        // but NOT the user's MCP servers (the profile owns MCP)…
        assert!(cj.get("mcpServers").is_none());
        // …and NOT another repo's entry, which would leak its allowedTools grants
        assert!(cj.get("projects").map(|p| p.get("/other/repo").is_none()).unwrap_or(true));
    }

    #[test]
    fn re_materializing_keeps_the_profiles_own_accumulated_state() {
        // The dir is STABLE and claude writes into it. Re-materializing on the
        // next launch must not wipe what claude recorded, or every relaunch
        // would look like a first run.
        let t = tmp("restate");
        let paths = paths_in(&t.0);
        user_json(&paths, serde_json::json!({}), &["/repo"]);
        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        materialize_with(&paths, &p, "/repo/a", "/repo").unwrap();

        let target = paths.dir.join(".claude.json");
        let mut cj: serde_json::Value = serde_json::from_str(&read(&target)).unwrap();
        cj["numStartups"] = serde_json::json!(7);
        fs::write(&target, cj.to_string()).unwrap();

        materialize_with(&paths, &p, "/repo/b", "/repo").unwrap();
        let after: serde_json::Value = serde_json::from_str(&read(&target)).unwrap();
        assert_eq!(after["numStartups"], serde_json::json!(7), "claude's own state survived");
        assert!(after["projects"].get("/repo/a").is_some(), "the first worktree stays trusted");
        assert!(after["projects"].get("/repo/b").is_some(), "and the new one is added");
    }

    #[test]
    fn global_mcp_is_merged_only_when_asked_and_never_shadows_the_profiles_own() {
        let t = tmp("mcp");
        let paths = paths_in(&t.0);
        fs::create_dir_all(paths.user_claude_json.parent().unwrap()).unwrap();
        fs::write(
            &paths.user_claude_json,
            serde_json::json!({ "mcpServers": {
                "shared": { "command": "THEIRS" },
                "globalonly": { "command": "g" }
            }})
            .to_string(),
        )
        .unwrap();

        let mut p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        p.mcp_servers.insert("shared".into(), serde_json::json!({ "command": "MINE" }));

        // off: the profile's set is the whole set — this is what makes
        // "remove a noisy global server" possible at all
        let out = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        let mcp: serde_json::Value = serde_json::from_str(&read(out.mcp.as_ref().unwrap())).unwrap();
        assert!(mcp["mcpServers"].get("globalonly").is_none());

        // on: globals fill in, but the profile's own definition wins a collision
        p.inherit_global_mcp = true;
        let out = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        let mcp: serde_json::Value = serde_json::from_str(&read(out.mcp.as_ref().unwrap())).unwrap();
        assert_eq!(mcp["mcpServers"]["globalonly"]["command"], serde_json::json!("g"));
        assert_eq!(mcp["mcpServers"]["shared"]["command"], serde_json::json!("MINE"));
    }

    #[test]
    fn skills_are_symlinked_and_stale_links_are_dropped() {
        let t = tmp("skills");
        let paths = paths_in(&t.0);
        for n in ["alpha", "beta"] {
            fs::create_dir_all(paths.store.join(n)).unwrap();
        }
        let mut p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        p.skills = vec!["alpha".into(), "beta".into()];
        materialize_with(&paths, &p, "/w", "/repo").unwrap();

        let link = paths.dir.join("skills/alpha");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(),
            "symlink, not a copy — editing a store skill must reach a RUNNING session");
        assert_eq!(fs::read_link(&link).unwrap(), paths.store.join("alpha"));

        // disabling one drops its link and leaves the other
        p.skills = vec!["alpha".into()];
        materialize_with(&paths, &p, "/w", "/repo").unwrap();
        assert!(fs::symlink_metadata(paths.dir.join("skills/beta")).is_err());
        assert!(fs::symlink_metadata(paths.dir.join("skills/alpha")).is_ok());
    }

    #[test]
    fn materialize_never_deletes_something_it_did_not_create() {
        // The sweep must only ever remove SYMLINKS. A plain file is the case
        // that actually exercises the guard: `remove_file` refuses a directory
        // on unix anyway, so a directory-only fixture passes even with the
        // is_symlink check deleted.
        let t = tmp("nodelete");
        let paths = paths_in(&t.0);
        fs::create_dir_all(paths.dir.join("skills")).unwrap();
        fs::write(paths.dir.join("skills/notes.txt"), "mine").unwrap();
        fs::create_dir_all(paths.dir.join("skills/handmade")).unwrap();
        fs::write(paths.dir.join("skills/handmade/SKILL.md"), "also mine").unwrap();

        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        materialize_with(&paths, &p, "/w", "/repo").unwrap();
        assert_eq!(read(&paths.dir.join("skills/notes.txt")), "mine");
        assert_eq!(read(&paths.dir.join("skills/handmade/SKILL.md")), "also mine");
    }

    #[test]
    fn a_skill_name_cannot_escape_the_skills_dir() {
        // Skill names are path components read from the same hand-editable file
        // as ids, and got none of the validation ids get. A "../" name used to
        // plant a symlink outside the profile dir, where the stale sweep (which
        // only reads skills/) could never reclaim it.
        let t = tmp("escape");
        let paths = paths_in(&t.0);
        fs::create_dir_all(paths.store.join("real")).unwrap();
        // a plausible traversal target that EXISTS, so `is_dir()` would pass
        fs::create_dir_all(t.0.join("victim")).unwrap();

        let mut p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        p.skills = vec!["../victim".into(), "a/b".into(), "..".into(), "".into(), "real".into()];
        let out = materialize_with(&paths, &p, "/w", "/repo").unwrap();

        // nothing landed outside skills/
        assert!(fs::symlink_metadata(paths.dir.join("victim")).is_err());
        assert!(fs::symlink_metadata(t.0.join("victim/real")).is_err());
        let entries: Vec<String> = fs::read_dir(paths.dir.join("skills"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["real".to_string()], "only the legal name linked");
        // and each rejection is reported rather than swallowed
        assert_eq!(out.warnings.iter().filter(|w| w.contains("refusing")).count(), 4, "{:?}", out.warnings);
    }

    #[test]
    fn a_non_object_user_claude_json_degrades_instead_of_failing_the_launch() {
        let t = tmp("weird");
        let paths = paths_in(&t.0);
        fs::create_dir_all(paths.user_claude_json.parent().unwrap()).unwrap();
        fs::write(&paths.user_claude_json, "[1,2,3]").unwrap();

        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        let out = materialize_with(&paths, &p, "/repo/wt", "/repo").expect("a weird user file must not strand the user");
        assert!(out.warnings.iter().any(|w| w.contains("not a JSON object")), "{:?}", out.warnings);
        // and the launch still produced a usable config
        let cj: serde_json::Value = serde_json::from_str(&read(&paths.dir.join(".claude.json"))).unwrap();
        assert!(cj.is_object());
    }

    #[test]
    fn a_corrupt_profile_claude_json_is_reseeded_loudly() {
        let t = tmp("corrupt");
        let paths = paths_in(&t.0);
        user_json(&paths, serde_json::json!({}), &["/repo"]);
        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        materialize_with(&paths, &p, "/repo/a", "/repo").unwrap();
        fs::write(paths.dir.join(".claude.json"), "{ truncated").unwrap();

        let out = materialize_with(&paths, &p, "/repo/b", "/repo").unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("re-seeded")), "{:?}", out.warnings);
        // recovery is real, not just a warning
        let cj: serde_json::Value = serde_json::from_str(&read(&paths.dir.join(".claude.json"))).unwrap();
        assert_eq!(cj["projects"]["/repo/b"]["hasTrustDialogAccepted"], serde_json::json!(true));
    }

    #[test]
    fn the_worktrees_mcp_stanza_points_at_the_resolved_binary() {
        let t = tmp("wtmcp");
        let mut paths = paths_in(&t.0);
        paths.worktrees_bin = Some(PathBuf::from("/opt/bin/worktrees"));
        let p = Profile { id: "work".into(), name: "W".into(), worktrees_mcp: true, ..Default::default() };
        let out = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        let mcp: serde_json::Value = serde_json::from_str(&read(out.mcp.as_ref().unwrap())).unwrap();
        assert_eq!(mcp["mcpServers"]["worktrees"]["command"], serde_json::json!("/opt/bin/worktrees"));
        assert_eq!(mcp["mcpServers"]["worktrees"]["args"], serde_json::json!(["mcp"]));

        // unresolvable → warn and carry on, never a failed launch
        paths.worktrees_bin = None;
        let out = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("no `worktrees` binary")), "{:?}", out.warnings);
        let mcp: serde_json::Value = serde_json::from_str(&read(out.mcp.as_ref().unwrap())).unwrap();
        assert!(mcp["mcpServers"].get("worktrees").is_none());
    }

    #[test]
    fn a_missing_skill_warns_instead_of_failing_the_launch() {
        let t = tmp("warn");
        let paths = paths_in(&t.0);
        let mut p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        p.skills = vec!["ghost".into()];
        let out = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("ghost")), "{:?}", out.warnings);
        // the launch still happens — a stale skill reference must not strand the
        // user with no session
        assert!(out.mcp.is_some());
    }

    #[test]
    fn clearing_the_rules_removes_the_file_rather_than_leaving_a_stale_one() {
        let t = tmp("rules");
        let paths = paths_in(&t.0);
        let mut p = Profile { id: "work".into(), name: "W".into(), rules: "old".into(), ..Default::default() };
        materialize_with(&paths, &p, "/w", "/repo").unwrap();
        assert!(paths.dir.join("rules.md").exists());
        p.rules = String::new();
        let out = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        assert!(out.rules.is_none());
        assert!(!paths.dir.join("rules.md").exists(), "stale rules must not keep applying");
    }

    #[test]
    fn the_profiled_launch_composes_env_and_flags_in_the_right_order() {
        let t = tmp("adapter");
        let paths = paths_in(&t.0);
        let p = Profile {
            id: "work".into(),
            name: "Work".into(),
            rules: "be terse".into(),
            model: Some("opus".into()),
            ..Default::default()
        };
        let m = materialize_with(&paths, &p, "/repo/wt", "/repo").unwrap();
        // base already carries the resume arg, as cmd_open would build it
        let base = AiLaunch::plain("claude -r");
        let l = claude_launch(&base, &p, &m);

        assert_eq!(l.match_word, "claude", "adoption still matches the program");
        assert_eq!(l.env, vec![("CLAUDE_CONFIG_DIR".to_string(), paths.dir.to_string_lossy().into_owned())]);

        // the resume arg stays adjacent to the ai word — three bats assertions
        // pin that, so profile flags must land AFTER it
        assert!(l.cmd.starts_with("claude -r "), "{}", l.cmd);
        assert!(l.cmd.contains("--append-system-prompt-file "), "{}", l.cmd);
        assert!(l.cmd.contains("--mcp-config "), "{}", l.cmd);
        assert!(l.cmd.contains("--strict-mcp-config"), "not inheriting globals → strict: {}", l.cmd);
        assert!(l.cmd.contains("--model 'opus'"), "{}", l.cmd);
        // and the env assignment still precedes the program in the composed line
        let body = l.pane0_body("KEEP");
        assert!(body.starts_with("CLAUDE_CONFIG_DIR="), "{body}");
        assert!(body.contains(" claude -r "), "{body}");
    }

    #[test]
    fn a_profile_with_settings_passes_the_settings_flag() {
        // The mirror of a_profile_with_no_rules_passes_no_rules_flag. Without
        // this, deleting the --settings push entirely passed every test — and a
        // profile's settings are where permission DENIES live.
        let t = tmp("settings");
        let paths = paths_in(&t.0);
        let p = Profile {
            id: "work".into(),
            name: "W".into(),
            settings: Some(serde_json::json!({ "permissions": { "deny": ["Bash(rm:*)"] } })),
            ..Default::default()
        };
        let m = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        let f = m.settings.clone().expect("settings were declared → a file is written");
        assert!(read(&f).contains("Bash(rm:*)"));
        let l = claude_launch(&AiLaunch::plain("claude"), &p, &m);
        assert!(l.cmd.contains("--settings "), "{}", l.cmd);
        assert!(l.cmd.contains(&shell_quote(&f.to_string_lossy())), "{}", l.cmd);
    }

    #[test]
    fn inheriting_global_mcp_drops_the_strict_flag() {
        // --strict-mcp-config is precisely what lets a profile REMOVE a global
        // server; with inherit on, keeping it would defeat the toggle.
        let t = tmp("strict");
        let paths = paths_in(&t.0);
        let mut p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        p.inherit_global_mcp = true;
        let m = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        let l = claude_launch(&AiLaunch::plain("claude"), &p, &m);
        assert!(l.cmd.contains("--mcp-config"));
        assert!(!l.cmd.contains("--strict-mcp-config"), "{}", l.cmd);
    }

    #[test]
    fn a_hostile_model_string_cannot_break_out_of_the_shell() {
        let t = tmp("hostile");
        let paths = paths_in(&t.0);
        let p = Profile {
            id: "work".into(),
            name: "W".into(),
            model: Some("x'; touch /tmp/PWNED; echo '".into()),
            ..Default::default()
        };
        let m = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        let l = claude_launch(&AiLaunch::plain("claude"), &p, &m);
        // `model` is user-typed text landing inside the `sh -ic` string; the
        // quoting must neutralise it rather than trusting the field.
        assert!(l.cmd.contains(r#"--model 'x'\''; touch /tmp/PWNED; echo '\'''"#), "{}", l.cmd);
        assert!(!l.cmd.contains("; touch /tmp/PWNED; echo ';"), "unquoted: {}", l.cmd);
    }

    #[test]
    fn a_profile_with_no_rules_passes_no_rules_flag() {
        // Pointing --append-system-prompt-file at a file we did not write would
        // fail the launch outright.
        let t = tmp("norules");
        let paths = paths_in(&t.0);
        let p = Profile { id: "work".into(), name: "W".into(), ..Default::default() };
        let m = materialize_with(&paths, &p, "/w", "/repo").unwrap();
        assert!(m.rules.is_none());
        let l = claude_launch(&AiLaunch::plain("claude"), &p, &m);
        assert!(!l.cmd.contains("--append-system-prompt-file"), "{}", l.cmd);
        assert!(!l.cmd.contains("--settings"), "no settings declared: {}", l.cmd);
    }

    #[test]
    fn unknown_keys_round_trip_so_an_older_build_cannot_clobber_a_newer_one() {
        let json = r#"{
            "version": 1,
            "default_id": "work",
            "profiles": {
                "work": { "id": "work", "name": "Work", "rules": "be terse",
                          "future_field": {"a": 1} }
            },
            "assignments": { "/repo": "work" },
            "top_level_future": true
        }"#;
        let ps: Profiles = serde_json::from_str(json).unwrap();
        assert_eq!(ps.default_id.as_deref(), Some("work"));
        assert_eq!(ps.assignments.get("/repo").map(|s| s.as_str()), Some("work"));
        let out = serde_json::to_string(&ps).unwrap();
        assert!(out.contains("future_field"), "per-profile unknown keys survive");
        assert!(out.contains("top_level_future"), "top-level unknown keys survive");
    }

    #[test]
    fn a_profile_defaults_to_inheriting_nothing() {
        // The locked semantics: a swap is a SWAP. Inheriting global skills/MCPs
        // is opt-in per profile, so a fresh profile is a clean slate (modulo the
        // user's global CLAUDE.md, which no mechanism can suppress).
        let p: Profile = serde_json::from_str(r#"{"id":"x","name":"X"}"#).unwrap();
        assert!(!p.inherit_global_skills);
        assert!(!p.inherit_global_mcp);
        assert!(!p.worktrees_mcp);
        assert!(p.skills.is_empty());
        assert!(p.model.is_none());
    }
}

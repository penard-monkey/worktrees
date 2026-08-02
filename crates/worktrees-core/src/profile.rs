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
const LAUNCH_HONORS_PROFILES: bool = false;

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
    if clean.is_empty() || clean != id {
        return None;
    }
    Some(profiles_data_root().join(clean))
}

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
        let s = sanitize_id(name);
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
        self.env.iter().map(|(k, v)| format!("{k}={} ", shell_quote(v))).collect()
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

/// Single-quote for POSIX sh: wrap in `'…'`, and end/re-open the quote around any
/// embedded `'`. Same trick as `tmux::sq`, kept local so profile paths are quoted
/// even when a caller forgets.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── storage ──────────────────────────────────────────────────────────────────

/// For display and resolution: a missing or unparseable file is an EMPTY set of
/// profiles, never an error. A typo in `profiles.json` must degrade to
/// "unprofiled launches", not lock the user out of their own tool — the same
/// leniency `config::cfg_toml_get` applies for the same reason.
pub fn read_lenient() -> Profiles {
    match fs::read(profiles_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Profiles::default(),
    }
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
        let lock = target.with_extension("json.lock");
        for _ in 0..100 {
            match fs::create_dir(&lock) {
                Ok(_) => return Ok(DirLock(lock)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock)
                        .and_then(|m| m.modified())
                        .map(|t| t.elapsed().map(|e| e.as_secs() > 15).unwrap_or(false))
                        .unwrap_or(false);
                    if stale {
                        let _ = fs::remove_dir_all(&lock);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                Err(e) => return Err(format!("lock error: {e}")),
            }
        }
        Err("could not acquire profiles-file lock".into())
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

//! The `ls --json` schema (v1) as typed serde structs — the single source of
//! truth shared by the CLI and the Tauri app.
//!
//! Field ORDER matches the bash `emit_place_json`/`emit_ls_json` exactly so the
//! compiled binary's `ls --json` is byte-identical (serde_json compact output +
//! struct field order). Every nullable field is `Option<T>` and is NOT skipped,
//! so `None` serializes as an explicit `null` (as the bash does).

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TmuxSession {
    pub name: String,
    pub up: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Place {
    pub schema_version: u32,
    pub slug: String,
    pub path: String,
    pub is_main: bool,
    pub registered: bool,
    pub branch: Option<String>,
    pub detached: Option<bool>,
    pub dirty: Option<bool>,
    pub dirty_files: Option<u32>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub upstream: Option<String>,
    pub created: Option<String>,
    pub created_epoch: Option<i64>,
    pub last_commit_epoch: Option<i64>,
    pub last_commit_subject: Option<String>,
    pub tmux_session: TmuxSession,
    pub claude_session_present: bool,
    pub claude_session_dir: Option<String>,
    pub install_cmd: Option<String>,
    /// Reserved for the infra phase (P3); the CLI emits `null`.
    pub stack: Option<serde_json::Value>,
    /// Declared state; the CLI emits `null` (live-only). The app overlays it.
    pub declared: Option<serde_json::Value>,
    /// Reconciled label. The CLI emits live-only (`active`|`closed`); the app
    /// recomputes with declared state merged in.
    pub lifecycle_effective: String,
}

/// A worktree git registers for this repo that lives OUTSIDE `.worktrees/` —
/// made by hand, or by another tool (`.dmux/worktrees/…`). Everything this
/// tool does is keyed on the place dir, so a stray is invisible to `ls`, `open`,
/// `rm`, doctor's per-place checks and the app, while still holding a branch
/// (and possibly uncommitted work). Reported, never moved: tmux sessions and
/// editors hold its cwd.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Stray {
    pub path: String,
    /// `None` when detached.
    pub branch: Option<String>,
    /// The slug adopting it would land on: `slugify(branch)`, else the dir name.
    pub slug: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LsJson {
    pub schema_version: u32,
    pub repo: String,
    pub prefix: String,
    pub places_file: String,
    pub places: Vec<Place>,
    /// Additive (v0.20): older consumers that deserialize this ignore it.
    #[serde(default)]
    pub strays: Vec<Stray>,
}

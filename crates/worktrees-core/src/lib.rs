//! worktrees-core — the engine shared by the `worktrees` CLI and the Tauri app.
//! A worktree is a durable PLACE; a branch is work that flows through it.
//!
//! Increment 0: read-only primitives (model, config, sysclock, git/tmux
//! wrappers, error). Later increments add project/place discovery, ops, the
//! declared store, and rendering. See MIGRATION.md.

pub mod config;
pub mod error;
pub mod git;
pub mod model;
pub mod ops;
pub mod project;
pub mod render;
pub mod store;
pub mod sysclock;
pub mod tmux;
pub mod ui;

pub use error::{Result, WtError};
pub use model::{LsJson, Place, TmuxSession, SCHEMA_VERSION};
pub use project::Project;
pub use ui::{CliUi, Ui};

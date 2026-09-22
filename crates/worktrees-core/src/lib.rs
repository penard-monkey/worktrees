//! worktrees-core — the engine shared by the `worktrees` CLI and the Tauri app.
//! A worktree is a durable PLACE; a branch is work that flows through it.
//!
//! Increment 0: read-only primitives (model, config, sysclock, git/tmux
//! wrappers, error). Later increments add project/place discovery, ops, the
//! declared store, and rendering. See MIGRATION.md.

pub mod agent;
pub mod automation;
pub mod config;
pub mod codex;
pub mod codexmcp;
pub mod derive;
pub mod diag;
pub mod docs;
pub mod error;
pub mod git;
pub mod health;
pub mod inbox;
pub mod init;
pub mod materialize;
pub mod mcpsetup;
pub mod mention;
pub mod model;
pub mod ops;
pub mod plan;
pub mod proc;
pub mod profile;
pub mod projcfg;
pub mod project;
pub mod provision;
pub mod render;
pub mod runs;
pub mod skillstore;
pub mod store;
pub mod sync;
pub mod sysclock;
pub mod tmux;
pub mod ui;

pub use error::{Result, WtError};
pub use model::{LsJson, Place, TmuxSession, SCHEMA_VERSION};
pub use project::Project;
pub use ui::{CliUi, Ui};

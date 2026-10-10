//! worktrees-core — the engine shared by the `worktrees` CLI and the Tauri app.
//! A worktree is a durable PLACE; a branch is work that flows through it.
//!
//! Increment 0: read-only primitives (model, config, sysclock, git/tmux
//! wrappers, error). Later increments add project/place discovery, ops, the
//! declared store, and rendering. See MIGRATION.md.

pub mod activity;
pub mod agent;
pub mod agentfiles;
pub mod choice;
pub mod automation;
pub mod config;
pub mod codex;
pub mod claude_usage;
pub mod clone;
pub mod codex_usage;
pub mod codexmcp;
pub mod derive;
pub mod diag;
pub mod docs;
pub mod error;
pub mod git;
pub mod github;
pub mod guidance;
pub mod harness;
pub mod health;
pub mod inbox;
pub mod logsink;
pub mod init;
pub mod materialize;
pub mod mcpsetup;
pub mod mcpmigrate;
pub mod mention;
pub mod messages;
pub mod model;
pub mod ops;
pub mod pi;
pub mod pimcp;
pub mod pimodels;
pub mod plan;
pub mod plancmd;
pub mod planning;
pub mod proc;
pub mod profile;
pub mod provider;
pub mod projcfg;
pub mod project;
pub mod provision;
pub mod quota;
pub mod reach;
pub mod registry;
pub mod render;
pub mod runs;
pub mod safepath;
pub mod skillstore;
pub mod store;
pub mod sync;
pub mod sysclock;
pub mod tmux;
pub mod tmux_server;
pub mod tmux_route;
pub mod trust;
pub mod ui;

pub use error::{Result, WtError};
pub use model::{LsJson, Place, TmuxSession, SCHEMA_VERSION};
pub use project::Project;
pub use ui::{CliUi, Ui};

//! A paired device's side of `worktrees serve`'s remote listener (Server
//! Edition Phase 2, `docs/proposals/server-edition-phase2.md` §2.2–§2.4,
//! §4.2–§4.4): pairing by link, the device key, the hosts it keeps, and the
//! pinned mutual-TLS WebSocket as a PIPE of text frames.
//!
//! The pipe parses nothing. The protocol over it is the loopback one,
//! handshake v1 included — keyed with the TLS session's exporter instead of
//! serve.json's token ([`tls::EXPORTER_SESSION`]) — so the page's
//! `app/src/ipcws.ts` stays its one client; [`call`] is the small Rust copy
//! `worktrees remote call` and the CI witness use.
//!
//! No tauri, no tmux, no engine: the desktop shim, the CLI and the iOS app (a
//! separate repository) all depend on this crate.

pub mod call;
pub mod dial;
pub mod hosts;
pub mod key;
pub mod pair;
pub mod pipe;
pub mod tls;

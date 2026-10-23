//! The remote door's compatibility rule (iOS MVP Q10, Phase 2 §4.5): a
//! protocol number for the wire and the handshake, separate from the release.
//!
//! Bump [`PROTO`] only for a BREAKING change to the wire or handshake v1,
//! never for a new row: a row the host lacks answers
//! `unknown command: <cmd>` ([`UNKNOWN_COMMAND`]), and a client says so in
//! words with [`unsupported_feature`]. The engine door (loopback, one build)
//! keeps exact release equality and carries none of this.
//!
//! - The client's hello carries `proto` (absent = 0). A host refuses one below
//!   [`PROTO_MIN`] only once its proof verifies, with a 1008 close whose
//!   reason is [`UPDATE_APP`].
//! - The host's proven `ok` carries `proto` and `proto_min`. The client
//!   refuses a host below [`HOST_PROTO_MIN`], or one whose `ok` says nothing
//!   of either, with [`update_host`]; one whose `proto_min` is above
//!   [`PROTO`] with [`UPDATE_APP`].
//!
//! `app/src/ipcws.ts` mirrors every constant here, and transport-check.mjs
//! holds them equal.

/// This implementation's wire and handshake contract.
pub const PROTO: u32 = 1;
/// The oldest client contract the host still serves.
pub const PROTO_MIN: u32 = 1;
/// The oldest host contract this client can safely drive: one without a
/// security property the client relies on is refused.
pub const HOST_PROTO_MIN: u32 = 1;

/// The host's refusal of a proven client below [`PROTO_MIN`] (a 1008 close
/// reason), and what a client says when the host needs a newer one.
pub const UPDATE_APP: &str = "Update the app";

/// What a client says of a host below [`HOST_PROTO_MIN`].
pub fn update_host(host: &str) -> String {
    format!("Update Worktrees on {host}")
}

/// How a host answers a row it does not have (serve's `dispatch::call`).
pub const UNKNOWN_COMMAND: &str = "unknown command: ";

/// A UI's words for `err` when it is a host's [`UNKNOWN_COMMAND`] answer, or
/// `None` for any other refusal. The refusal itself is passed on unchanged.
pub fn unsupported_feature(err: &str, feature: &str) -> Option<String> {
    err.starts_with(UNKNOWN_COMMAND).then(|| format!("This host doesn't support {feature} yet"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_unknown_command_reads_as_a_missing_feature() {
        assert_eq!(unsupported_feature("unknown command: agent_send", "sending a line").as_deref(), Some("This host doesn't support sending a line yet"));
        assert_eq!(unsupported_feature("denied", "sending a line"), None);
        assert_eq!(unsupported_feature("the prompt changed: unknown command: x", "x"), None);
    }
}

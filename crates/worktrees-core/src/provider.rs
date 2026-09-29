//! Provider identity and lifecycle policy shared by the CLI, MCP and app engine.
//! Add identity here and the harness's behaviour in `harness.rs` (one adapter
//! per row, same order).
//! User-configured arbitrary AI commands remain supported as before.

#[derive(Debug)]
pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    pub match_word: &'static str,
    pub sidecar_suffix: &'static str,
    /// Claude historically owns a canonical session even while it runs a shell.
    pub canonical_default: bool,
    /// Only Claude accepts --name; other launch-specific flags live in adapters.
    pub name_arg: Option<&'static str>,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "claude",
        label: "Claude",
        match_word: "claude",
        sidecar_suffix: "~agent~claude",
        canonical_default: true,
        name_arg: Some("--name"),
    },
    Provider {
        id: "codex",
        label: "Codex",
        match_word: "codex",
        sidecar_suffix: "~agent~codex",
        canonical_default: false,
        name_arg: None,
    },
];
pub const CLAUDE: &Provider = &PROVIDERS[0];
pub const CODEX: &Provider = &PROVIDERS[1];

pub fn by_id(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}
pub fn by_word(word: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.match_word == word)
}
pub fn ids() -> Vec<&'static str> {
    PROVIDERS.iter().map(|p| p.id).collect()
}
pub fn choices() -> String {
    ids().join(" or ")
}
pub fn is_sidecar(name: &str) -> bool {
    PROVIDERS.iter().any(|p| name.contains(p.sidecar_suffix))
}
/// The marker every provider sidecar carries (`<canonical>~agent~<id>`). `~`
/// cannot occur in a git ref, so no place's own session contains it.
pub const SIDECAR_MARKER: &str = "~agent~";

/// Which harness a pane in tmux session `session` is running, from its
/// `pane_current_command`.
///
/// 1. An exact program word (`codex`, `claude`) names its harness.
/// 2. A pane in a provider SIDECAR (`~agent~<id>`) belongs to that sidecar's
///    harness when its command is only a wrapper (`node`, a bare version) —
///    and to no harness at all when `<id>` is not one we know. The session
///    name is set by us at launch, so it is the stronger evidence: an npm
///    install runs Codex (and pi, and opencode) as `node`, which rule 3 alone
///    reads as Claude.
/// 3. Otherwise Claude's legacy node/version wrapper heuristic, which is all a
///    canonical (pre-sidecar) session has to go on.
pub fn for_pane(session: &str, command: &str) -> Option<&'static Provider> {
    if let Some(p) = by_word(command.rsplit('/').next().unwrap_or(command)) {
        return Some(p);
    }
    let wrapper = PROVIDERS.iter().find(|p| p.canonical_default && crate::tmux::is_ai_command(command, p.match_word));
    if session.contains(SIDECAR_MARKER) {
        return wrapper.and(PROVIDERS.iter().find(|p| session.contains(p.sidecar_suffix)));
    }
    wrapper
}
impl Provider {
    pub fn sidecar_name(&self, canonical: &str) -> String {
        format!("{canonical}{}", self.sidecar_suffix)
    }
    /// Compatibility with canonical sessions created before provider sidecars.
    /// The default provider prefers its existing sidecar; others prefer their
    /// legacy canonical session. These priorities intentionally differ.
    pub fn session_name(
        &self,
        canonical: &str,
        canonical_owner: &str,
        sidecar_exists: bool,
    ) -> String {
        if canonical_owner == self.id && (!self.canonical_default || !sidecar_exists) {
            canonical.to_string()
        } else {
            self.sidecar_name(canonical)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapper_pane_in_a_sidecar_is_that_sidecars_harness_never_claude() {
        let id = |s: &str, c: &str| for_pane(s, c).map(|p| p.id);
        // npm's codex runs as `node`: the sidecar names it.
        assert_eq!(id("repo-feat~agent~codex", "node"), Some("codex"));
        assert_eq!(id("repo-feat~agent~codex", "2.1.277"), Some("codex"));
        // A harness this build does not know is not Claude either.
        assert_eq!(id("repo-feat~agent~pi", "node"), None);
        assert_eq!(id("repo-feat~agent~pi", "zsh"), None);
        // Claude's own sidecar, and the legacy canonical session, still read as Claude.
        assert_eq!(id("repo-feat~agent~claude", "node"), Some("claude"));
        assert_eq!(id("repo-feat", "node"), Some("claude"));
        assert_eq!(id("repo-feat", "2.1.277"), Some("claude"));
        // Exact program words win wherever they run; a shell is nobody.
        assert_eq!(id("repo-feat", "codex"), Some("codex"));
        assert_eq!(id("repo-feat~agent~codex", "/usr/local/bin/codex"), Some("codex"));
        assert_eq!(id("repo-feat~agent~codex", "zsh"), None);
        assert_eq!(id("repo-feat", "zsh"), None);
    }
}

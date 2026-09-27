//! Provider identity and lifecycle policy shared by the CLI, MCP and app engine.
//! Add identity here and implement the provider's launch/activity adapters.
//! User-configured arbitrary AI commands remain supported as before.

#[derive(Debug)]
pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    pub match_word: &'static str,
    pub resume_arg: &'static str,
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
        resume_arg: "-r",
        sidecar_suffix: "~agent~claude",
        canonical_default: true,
        name_arg: Some("--name"),
    },
    Provider {
        id: "codex",
        label: "Codex",
        match_word: "codex",
        resume_arg: "resume --last",
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
/// Exact program matches precede Claude's legacy node/version wrapper heuristic.
pub fn for_pane(command: &str) -> Option<&'static Provider> {
    by_word(command.rsplit('/').next().unwrap_or(command)).or_else(|| {
        PROVIDERS
            .iter()
            .find(|p| p.canonical_default && crate::tmux::is_ai_command(command, p.match_word))
    })
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

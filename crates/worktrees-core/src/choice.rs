//! The harness × model shape (pi-harness §2.3, shared with the opencode
//! proposal): what a picker lists and what a lane is launched with.
//!
//! The registry (`provider::PROVIDERS`) deliberately has NO model dimension —
//! a harness × model product in a static table would go stale the moment
//! someone edits pi's `models.json`. Models are DERIVED here, per harness, and
//! carry why an unusable one is unusable: on the machine this was built on,
//! pi's default provider is not signed in and pi reports a dead model host as
//! ready, so a picker that cannot say *why* offers exactly the launches that
//! fail.

use serde::Serialize;

/// A model as the harness names it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ModelRef {
    /// `provider.rs` id.
    pub harness: String,
    /// pi/opencode's "provider" (`lm-studio`, `kimi-coding`). Named `backend`
    /// because `provider` already means the harness in this codebase. `None`
    /// for claude/codex.
    pub backend: Option<String>,
    pub model: String,
    /// Display only.
    pub label: Option<String>,
}

impl ModelRef {
    /// The harness's own spelling: `backend/model` for pi, `model` otherwise.
    pub fn arg(&self) -> String {
        match &self.backend {
            Some(b) => format!("{b}/{}", self.model),
            None => self.model.clone(),
        }
    }

    /// Parse the harness's spelling back. pi's is `backend/model[:thinking]`,
    /// split on the FIRST `/` (a model id can contain one: `qwen/qwen3-vl-30b`).
    pub fn parse(harness: &str, s: &str) -> ModelRef {
        let split = if harness == crate::provider::PI.id { s.split_once('/') } else { None };
        let (backend, model) = match split {
            Some((b, m)) => (Some(b.to_string()), m.to_string()),
            None => (None, s.to_string()),
        };
        ModelRef { harness: harness.to_string(), backend, model, label: None }
    }
}

/// Why a listed model cannot be launched right now.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// No credentials: an OAuth sign-in or an API key is missing (pi's `auth
    /// check` → `credentials_not_configured`).
    NoCredentials,
    /// The backend's `baseUrl` did not answer `GET /models` within the probe's
    /// deadline.
    EndpointUnreachable,
    /// The backend answered, and the declared model is not among what it
    /// serves (unloaded in LM Studio, say).
    NotServed,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::NoCredentials => "no_credentials",
            Reason::EndpointUnreachable => "endpoint_unreachable",
            Reason::NotServed => "not_served",
        }
    }
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_out: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
}

/// One row a picker lists: derived, cached, recomputed.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ModelOption {
    pub model: ModelRef,
    pub ready: bool,
    pub reason: Option<Reason>,
    /// `pi-list-models`, `pi-config` (declared but not listed), `claude-aliases`.
    pub source: String,
    pub meta: ModelMeta,
}

/// What a lane is launched with. `model: None` means the CLI's own default —
/// safe for claude and codex, never offered for pi (its default on this
/// machine is a provider that is not signed in, §2.2).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentChoice {
    pub harness: String,
    pub model: Option<ModelRef>,
    pub thinking: Option<String>,
}

/// The one gate a model string passes before argv. It is also `shell_quote`d
/// where it lands (as `Profile.model` is); this is the belt: a model id is data,
/// and `[A-Za-z0-9._/:-]+` is every id any harness here has been seen to use.
/// The catalog is NOT a gate — free text is allowed, as with `--model` today.
pub fn validate_model(s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 200 {
        return Err("model must be 1–200 characters".into());
    }
    if s.starts_with('-') {
        return Err(format!("model '{s}' looks like a flag"));
    }
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | ':' | '-')) {
        return Err(format!("model '{s}' may contain only letters, digits and . _ / : -"));
    }
    Ok(())
}

/// Claude's model list is not locally queryable; these are the aliases its
/// `--model` accepts, plus whatever the user types.
pub const CLAUDE_ALIASES: &[&str] = &["opus", "sonnet", "haiku", "fable"];

/// What a picker offers for `harness`. Claude: its aliases. Codex: nothing
/// listable (free text). pi: its catalog (`pimodels::options`), which shells
/// out and probes — cached, but not free.
pub fn options_for(harness: &str) -> Vec<ModelOption> {
    match harness {
        "claude" => CLAUDE_ALIASES
            .iter()
            .map(|m| ModelOption {
                model: ModelRef::parse("claude", m),
                ready: true,
                reason: None,
                source: "claude-aliases".into(),
                meta: ModelMeta::default(),
            })
            .collect(),
        "pi" => crate::pimodels::options(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_is_data_never_argv() {
        for ok in ["lm-studio/qwen3.6-27b", "opus", "gpt-5-codex", "qwen/qwen3-vl-30b", "sonnet:high", "a_b.c"] {
            assert!(validate_model(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-rf", "a b", "x'; touch /tmp/PWNED", "$(id)", "a\nb", "é", "a;b", "a|b"] {
            assert!(validate_model(bad).is_err(), "{bad:?}");
        }
        assert!(validate_model(&"a".repeat(201)).is_err());
    }

    #[test]
    fn pi_models_split_on_the_first_slash_and_round_trip() {
        let m = ModelRef::parse("pi", "lm-studio/qwen/qwen3-vl-30b");
        assert_eq!(m.backend.as_deref(), Some("lm-studio"));
        assert_eq!(m.model, "qwen/qwen3-vl-30b");
        assert_eq!(m.arg(), "lm-studio/qwen/qwen3-vl-30b");
        let c = ModelRef::parse("claude", "opus");
        assert_eq!((c.backend.clone(), c.arg()), (None, "opus".into()));
        // A pi model with no backend is kept whole (pi resolves bare patterns itself).
        assert_eq!(ModelRef::parse("pi", "qwen3.6-27b").backend, None);
        let r = serde_json::to_value(Reason::EndpointUnreachable).unwrap();
        assert_eq!(r, serde_json::json!("endpoint_unreachable"));
        assert_eq!(Reason::NotServed.as_str(), "not_served");
    }
}

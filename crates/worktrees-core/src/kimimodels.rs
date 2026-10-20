//! kimi's model catalog, from the `[models."<alias>"]` tables of its
//! `config.toml` — and from nothing else in that file.
//!
//! The same file holds kimi's provider credentials (`[providers.*]` and
//! `[services.*]` carry `api_key` and OAuth fields), and kimi's own JSON form
//! of the catalog (`kimi provider list --json`) prints them. So this never
//! runs that command, and never hands the whole file to a parser: lines are
//! sorted by the table they belong to FIRST, every table that is not
//! `models.*` is dropped unread, and only what is left is parsed. Nothing read
//! here is logged, and a failure says "config.toml unreadable" without
//! quoting a line or carrying the parser's message (which would). Tests use
//! synthetic text only; nothing reads the real file under test.
//!
//! Every listed model is `ready`: kimi signs in once for all of them, and
//! whether it is signed in lives in files worktrees does not read.

use std::path::PathBuf;

use crate::choice::{ModelMeta, ModelOption, ModelRef};

pub const UNREADABLE: &str = "config.toml unreadable";

pub fn config_path() -> PathBuf {
    crate::kimi::home().join("config.toml")
}

/// A `[header]` line's table path, or `None` for any other line.
/// `[[array.of.tables]]` reads as its own path, so it is never `models.*`.
fn header(line: &str) -> Option<&str> {
    let t = line.trim();
    if t.starts_with("[[") {
        return Some(t);
    }
    t.strip_prefix('[')?.split(']').next().map(str::trim)
}

fn is_models_table(path: &str) -> bool {
    path == "models" || path.starts_with("models.")
}

/// The kept half of a `config.toml`: every line under a `[models…]` header,
/// plus the root-level `default_model` line. Everything else is dropped here,
/// before any parsing.
pub fn models_only(text: &str) -> String {
    let mut out = String::new();
    let mut table: Option<bool> = None; // None: the root, before any header
    for line in text.lines() {
        if let Some(h) = header(line) {
            table = Some(is_models_table(h));
            if table == Some(true) {
                out.push_str(line);
                out.push('\n');
            }
            continue;
        }
        let keep = match table {
            Some(models) => models,
            None => line.split('=').next().map(str::trim) == Some("default_model"),
        };
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// One alias from the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiModel {
    pub alias: String,
    pub label: Option<String>,
    pub context: Option<u64>,
    pub thinking: bool,
    pub images: bool,
}

/// The catalog and kimi's default alias, from a `config.toml` text.
pub fn parse(text: &str) -> Result<(Vec<KimiModel>, Option<String>), &'static str> {
    let kept = models_only(text);
    let v: toml::Value = toml::from_str(&kept).map_err(|_| UNREADABLE)?;
    let default = v.get("default_model").and_then(|d| d.as_str()).map(str::to_string);
    let mut models: Vec<KimiModel> = v
        .get("models")
        .and_then(|m| m.as_table())
        .map(|t| {
            t.iter()
                .filter(|(_, m)| m.is_table())
                .map(|(alias, m)| {
                    let caps: Vec<&str> = m
                        .get("capabilities")
                        .and_then(|c| c.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                        .unwrap_or_default();
                    KimiModel {
                        alias: alias.clone(),
                        label: m.get("display_name").and_then(|d| d.as_str()).map(str::to_string),
                        context: m.get("max_context_size").and_then(|c| c.as_integer()).and_then(|c| u64::try_from(c).ok()),
                        thinking: caps.contains(&"thinking"),
                        images: caps.contains(&"image_in"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    models.sort_by(|a, b| a.alias.cmp(&b.alias));
    Ok((models, default))
}

/// `262144` → `256K`, `1048576` → `1M`: the picker's spelling.
pub fn human(n: u64) -> String {
    if n >= 1 << 20 && n % (1 << 20) == 0 {
        format!("{}M", n >> 20)
    } else if n >= 1024 {
        format!("{}K", n / 1024)
    } else {
        n.to_string()
    }
}

/// What a picker lists for kimi: the catalog, kimi's default first.
pub fn options_from(models: &[KimiModel], default: Option<&str>) -> Vec<ModelOption> {
    let mut v: Vec<&KimiModel> = models.iter().collect();
    v.sort_by_key(|m| Some(m.alias.as_str()) != default);
    v.into_iter()
        .map(|m| ModelOption {
            model: ModelRef {
                harness: crate::provider::KIMI.id.into(),
                backend: None,
                model: m.alias.clone(),
                label: m.label.clone(),
            },
            ready: true,
            reason: None,
            source: "kimi-config".into(),
            meta: ModelMeta {
                context: m.context.map(human),
                max_out: None,
                thinking: Some(m.thinking),
                images: Some(m.images),
            },
        })
        .collect()
}

/// The catalog on this machine; empty when kimi has no config or it is
/// unreadable (`catalog` says which).
pub fn options() -> Vec<ModelOption> {
    match catalog() {
        Ok((m, d)) => options_from(&m, d.as_deref()),
        Err(_) => Vec::new(),
    }
}

/// The catalog, or why there is none — "no config.toml" or `UNREADABLE`.
pub fn catalog() -> Result<(Vec<KimiModel>, Option<String>), &'static str> {
    let text = std::fs::read_to_string(config_path()).map_err(|_| "no config.toml")?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic, in kimi 2.2.0's layout. The secret values are made up — the
    /// point is that they never survive `models_only`.
    const CONFIG: &str = r#"default_model = "kimi-code/k3"
api_key = "ROOT-SECRET"

[providers."managed:kimi-code"]
type = "kimi"
api_key = "PROVIDER-SECRET"
base_url = "https://example.invalid/v1"

[providers."managed:kimi-code".oauth]
storage = "file"
key = "OAUTH-SECRET"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
capabilities = [ "thinking", "image_in", "tool_use" ]
display_name = "K3"

[models."kimi-code/fast"]
provider = "managed:kimi-code"
model = "fast"
max_context_size = 262144
capabilities = [
  "tool_use",
]
display_name = "Fast"

[services.moonshot_search]
api_key = "SERVICE-SECRET"

[thinking]
enabled = true
"#;

    #[test]
    fn only_the_models_tables_and_the_default_survive_the_cut() {
        let kept = models_only(CONFIG);
        for secret in ["ROOT-SECRET", "PROVIDER-SECRET", "OAUTH-SECRET", "SERVICE-SECRET", "example.invalid", "[thinking]"] {
            assert!(!kept.contains(secret), "{secret} leaked into:\n{kept}");
        }
        assert!(kept.starts_with("default_model = \"kimi-code/k3\"\n"));
        assert!(kept.contains("[models.\"kimi-code/fast\"]"));
    }

    #[test]
    fn the_catalog_lists_aliases_with_their_meta_default_first() {
        let (models, default) = parse(CONFIG).unwrap();
        assert_eq!(default.as_deref(), Some("kimi-code/k3"));
        assert_eq!(models.len(), 2);
        let opts = options_from(&models, default.as_deref());
        assert_eq!(opts[0].model.arg(), "kimi-code/k3", "default first");
        assert_eq!(opts[0].model.label.as_deref(), Some("K3"));
        assert_eq!(opts[0].meta.context.as_deref(), Some("1M"));
        assert_eq!((opts[0].meta.thinking, opts[0].meta.images), (Some(true), Some(true)));
        assert_eq!(opts[1].model.arg(), "kimi-code/fast");
        assert_eq!(opts[1].meta.context.as_deref(), Some("256K"));
        assert!(opts.iter().all(|o| o.ready && o.source == "kimi-config"));
        // An alias is handed to `-m` whole: it is not split like pi's.
        assert_eq!(crate::choice::ModelRef::parse("kimi", "kimi-code/k3").arg(), "kimi-code/k3");
    }

    #[test]
    fn a_broken_file_is_unreadable_and_quotes_nothing() {
        let broken = "[models.\"a\"]\nmodel = \nprovider = \"PROVIDER-SECRET-ish\"\n";
        assert_eq!(parse(broken), Err(UNREADABLE));
        // A broken SECRET table cannot break the catalog: it is never parsed.
        let ok = format!("[providers.x]\napi_key = \"unterminated\n{CONFIG}");
        assert!(parse(&ok).is_ok());
        assert_eq!(parse(""), Ok((vec![], None)));
    }

    #[test]
    fn sizes_read_the_way_the_picker_shows_them() {
        assert_eq!(human(1_048_576), "1M");
        assert_eq!(human(262_144), "256K");
        assert_eq!(human(1000), "1000");
    }
}

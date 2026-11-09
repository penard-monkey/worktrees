//! The pi model hosts worktrees manages (docs/proposals/pi-models-config.md).
//!
//! David's decision, 2026-11-09: worktrees keeps its own models config and
//! every pi worktrees starts uses it; pi's own `~/.pi` is never written. The
//! mechanism, measured on pi 1.5.0 (pi has no flag or env var for models.json
//! alone): a static extension, `pihosts_ext.js`, materialized under worktrees'
//! data dir and passed with `-e` by [`crate::pimodels::pi_command`] and
//! [`crate::pimodels::pane_ext_words`], reads [`data_path`] once at load and
//! calls `pi.registerProvider` for each host in it.
//!
//! What pi does with that, measured on 1.5.0:
//! - An extension provider composes ABOVE models.json: its model list
//!   REPLACES the list pi's file gives a provider of the same name, its
//!   `baseUrl` wins, and its key falls back to pi's file when it gives none.
//!   So an `override` of one of pi's hosts carries no key and pi keeps using
//!   its own, and it starts as a copy of pi's model list ([`override_host`]),
//!   or declaring one extra model would hide the others.
//! - A provider with no key is not listed, so a host worktrees ADDS gets its
//!   own id as a placeholder key — in the extension, not in this file: no row
//!   accepts a key and this file never holds one.
//! - `pi auth check` ignores `-e`: it calls our hosts `provider_not_found`, so
//!   the catalog never asks it about them.
//! - A RESUME without `-e` silently starts on another model when the session's
//!   model is one of ours. That is why EVERY pi spawn goes through the one
//!   builder (`app/scripts/pi-spawn-check.mjs` holds it), and why a session
//!   resumed by hand outside worktrees can change model without a word.
//! - A running pi reads the file only at start; `/model` re-reads models.json,
//!   not extensions, and pi has no hook when it opens. A change reaches a pi
//!   that is already running when it is restarted.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::pimodels::{Host, HostModel};

pub const FILE: &str = "pi-models.json";
const EXT_FILE: &str = "worktrees-models.js";
const EXT_SOURCE: &str = include_str!("pihosts_ext.js");

/// `$XDG_CONFIG_HOME/worktrees/pi-models.json`: the registry's directory, so
/// the CLI, serve and the MCP server build one view.
pub fn data_path() -> PathBuf {
    crate::profile::config_root_pub().join(FILE)
}

// ── the file ─────────────────────────────────────────────────────────────────

/// worktrees' own schema. Not pi's: no key, and `override` instead.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owned {
    #[serde(default = "one")]
    pub version: u32,
    /// Host id → host, in the order they were added.
    #[serde(default)]
    pub hosts: serde_json::Map<String, serde_json::Value>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedHost {
    pub base_url: String,
    #[serde(default = "openai_completions")]
    pub api: String,
    /// Replaces the host of the same id in pi's models.json; its key stays pi's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub r#override: bool,
    #[serde(default)]
    pub models: Vec<OwnedModel>,
}

fn openai_completions() -> String {
    "openai-completions".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
}

impl Owned {
    /// Each host, or why it is unusable (the extension leaves it out too).
    pub fn entries(&self) -> Vec<(String, Result<OwnedHost, String>)> {
        self.hosts
            .iter()
            .map(|(id, v)| (id.clone(), serde_json::from_value::<OwnedHost>(v.clone()).map_err(|e| e.to_string())))
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<OwnedHost> {
        self.hosts.get(id).and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    fn put(&mut self, id: &str, h: &OwnedHost) -> Result<(), String> {
        self.hosts.insert(id.to_string(), serde_json::to_value(h).map_err(|e| e.to_string())?);
        Ok(())
    }
}

/// The file at `path`: empty when it does not exist, `Err` with the reason
/// when it exists and cannot be used — the page shows that reason, since the
/// extension then registers nothing.
pub fn read_at(path: &Path) -> Result<Owned, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str::<Owned>(&t).map_err(|e| format!("{} is not valid: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Owned { version: 1, ..Default::default() }),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

pub fn read() -> Result<Owned, String> {
    read_at(&data_path())
}

/// worktrees' hosts as `pimodels::Host`, for the catalog and the page. `has_key`
/// is what pi will have: the placeholder for a host we added, pi's own for an
/// override (looked up in pi's file by the caller).
pub fn as_hosts(owned: &Owned, pi_has_key: &dyn Fn(&str) -> bool) -> Vec<Host> {
    owned
        .entries()
        .into_iter()
        .filter_map(|(id, h)| h.ok().map(|h| (id, h)))
        .map(|(id, h)| Host {
            has_key: if h.r#override { pi_has_key(&id) } else { true },
            api: Some(h.api.clone()),
            base_url: Some(h.base_url.clone()),
            models: h
                .models
                .iter()
                .map(|m| HostModel {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    context_window: m.context_window,
                    max_tokens: m.max_tokens,
                    reasoning: m.reasoning,
                    images: m.images,
                })
                .collect(),
            backend: id,
        })
        .collect()
}

// ── validation (the boundary) ────────────────────────────────────────────────

/// A host id: the model rule (`choice::validate_model`) without `/`, which pi
/// splits `host/model` at.
pub fn validate_host_id(id: &str) -> Result<(), String> {
    crate::choice::validate_model(id).map_err(|e| e.replacen("model", "host", 1))?;
    if id.contains('/') {
        return Err(format!("host '{id}' may not contain /"));
    }
    Ok(())
}

/// A base URL: `http://` or `https://`, a host, no credentials, no whitespace
/// or control characters. Returned without a trailing `/`.
pub fn validate_base_url(url: &str) -> Result<String, String> {
    let u = url.trim();
    if u.len() > 500 || u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the base URL must be one line with no spaces".into());
    }
    let rest = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
        .ok_or("the base URL must start with http:// or https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err("the base URL may not carry a user or password".into());
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    if host.is_empty() || host.starts_with('-') {
        return Err("the base URL names no host".into());
    }
    if u.contains(['?', '#']) {
        return Err("the base URL may not carry a query or fragment".into());
    }
    Ok(u.trim_end_matches('/').to_string())
}

/// The APIs a host worktrees adds may speak: the OpenAI-compatible ones a
/// local server offers (LM Studio, Ollama, vLLM, llama.cpp).
pub const APIS: &[&str] = &["openai-completions", "openai-responses"];

fn validate_api(api: &str) -> Result<(), String> {
    if APIS.contains(&api) {
        Ok(())
    } else {
        Err(format!("api must be one of: {}", APIS.join(", ")))
    }
}

/// A count a user typed: 1..=10M tokens.
fn validate_count(what: &str, n: Option<u64>) -> Result<(), String> {
    match n {
        Some(n) if n == 0 || n > 10_000_000 => Err(format!("{what} must be between 1 and 10000000")),
        _ => Ok(()),
    }
}

// ── edits ────────────────────────────────────────────────────────────────────

static EDIT: Mutex<()> = Mutex::new(());

/// Read, change and replace the file at `path` under one lock, refusing when
/// it changed on disk between the read and the swap. An unreadable file is
/// never overwritten.
///
/// The lock is `workspaces::edit_at`'s: a mutex for this process's threads
/// and an `flock` on a `.lock` sidecar for every process (the CLI, serve, an
/// MCP server). A hand edit takes no lock, which is what the compare before
/// the swap is for.
pub fn edit_at<R>(path: &Path, f: impl FnOnce(&mut Owned) -> Result<R, String>) -> Result<R, String> {
    use std::os::fd::AsRawFd;
    if path == data_path() && crate::registry::real_writes_forbidden() {
        return Err(format!("refused: a test reached the user's real {} — use edit_at with a temp path", FILE));
    }
    let _g = EDIT.lock().unwrap_or_else(|e| e.into_inner());
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let lock_path = dir.join(format!("{FILE}.lock"));
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    // SAFETY: flock gets a live descriptor; `lock` outlives the critical
    // section and closing it releases the lock on every exit path.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!("could not lock {}", lock_path.display()));
    }
    let before = std::fs::read(path).ok();
    let mut owned = read_at(path)?;
    owned.version = 1;
    let r = f(&mut owned)?;
    let bytes = serde_json::to_vec_pretty(&owned).map_err(|e| e.to_string())?;
    if std::fs::read(path).ok() != before {
        return Err(format!("{} changed while it was being edited; nothing was written. Try again.", path.display()));
    }
    crate::fsx::write_atomic(path, &bytes, None).map_err(|e| format!("{}: {e}", path.display()))?;
    drop(lock);
    Ok(r)
}

/// Add a host worktrees owns. Refused when pi's models.json already declares
/// that id (`pi_ids`): that is [`override_host`], which keeps pi's key.
pub fn add_host(o: &mut Owned, id: &str, base_url: &str, api: &str, pi_ids: &[String]) -> Result<(), String> {
    validate_host_id(id)?;
    let base_url = validate_base_url(base_url)?;
    validate_api(api)?;
    if o.hosts.contains_key(id) {
        return Err(format!("worktrees already has a host named {id}"));
    }
    if pi_ids.iter().any(|p| p == id) {
        return Err(format!("pi's own config declares {id}; use \"Override in worktrees\" on it instead"));
    }
    o.put(id, &OwnedHost { base_url, api: api.into(), r#override: false, models: Vec::new() })
}

/// Start overriding pi's host `id` from a COPY of what pi's file gives it —
/// its base URL, api and every model — never its key.
pub fn override_host(o: &mut Owned, pi_host: &Host) -> Result<(), String> {
    let id = pi_host.backend.as_str();
    validate_host_id(id)?;
    if o.hosts.contains_key(id) {
        return Err(format!("{id} is already overridden in worktrees"));
    }
    let base_url = validate_base_url(pi_host.base_url.as_deref().ok_or_else(|| {
        format!("{id} has no baseUrl in pi's config (a built-in provider); worktrees overrides only hosts with one")
    })?)?;
    let api = pi_host.api.clone().unwrap_or_else(openai_completions);
    validate_api(&api)?;
    let models = pi_host
        .models
        .iter()
        .filter(|m| crate::choice::validate_model(&m.id).is_ok())
        .map(|m| OwnedModel {
            id: m.id.clone(),
            name: m.name.clone(),
            context_window: m.context_window,
            max_tokens: m.max_tokens,
            reasoning: m.reasoning,
            images: m.images,
        })
        .collect();
    o.put(id, &OwnedHost { base_url, api, r#override: true, models })
}

fn host_mut(o: &mut Owned, id: &str, f: impl FnOnce(&mut OwnedHost) -> Result<(), String>) -> Result<(), String> {
    let mut h = o.get(id).ok_or_else(|| format!("worktrees has no host named {id}"))?;
    f(&mut h)?;
    o.put(id, &h)
}

pub fn set_base_url(o: &mut Owned, id: &str, base_url: &str) -> Result<(), String> {
    let url = validate_base_url(base_url)?;
    host_mut(o, id, |h| {
        h.base_url = url;
        Ok(())
    })
}

/// Remove a host, or an override (pi's own host then applies again).
pub fn remove_host(o: &mut Owned, id: &str) -> Result<(), String> {
    o.hosts.remove(id).map(|_| ()).ok_or_else(|| format!("worktrees has no host named {id}"))
}

/// The fields a model's row edits. `None` leaves pi's default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ModelFields {
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
}

pub fn declare_model(o: &mut Owned, id: &str, model: &str, fields: &ModelFields) -> Result<(), String> {
    crate::choice::validate_model(model)?;
    validate_count("context window", fields.context_window)?;
    validate_count("max output", fields.max_tokens)?;
    host_mut(o, id, |h| {
        if h.models.iter().any(|m| m.id == model) {
            return Err(format!("{id} already declares {model}"));
        }
        h.models.push(OwnedModel {
            id: model.into(),
            name: None,
            context_window: fields.context_window,
            max_tokens: fields.max_tokens,
            reasoning: None,
            images: None,
        });
        Ok(())
    })
}

pub fn update_model(o: &mut Owned, id: &str, model: &str, fields: &ModelFields) -> Result<(), String> {
    validate_count("context window", fields.context_window)?;
    validate_count("max output", fields.max_tokens)?;
    host_mut(o, id, |h| {
        let m = h.models.iter_mut().find(|m| m.id == model).ok_or_else(|| format!("{id} does not declare {model}"))?;
        m.context_window = fields.context_window;
        m.max_tokens = fields.max_tokens;
        Ok(())
    })
}

pub fn remove_model(o: &mut Owned, id: &str, model: &str) -> Result<(), String> {
    host_mut(o, id, |h| {
        let before = h.models.len();
        h.models.retain(|m| m.id != model);
        if h.models.len() == before {
            return Err(format!("{id} does not declare {model}"));
        }
        Ok(())
    })
}

// ── the extension ────────────────────────────────────────────────────────────

/// The extension's source for a data file at `data`: the static code with the
/// path set as a JSON string literal.
pub fn ext_source(data: &Path) -> String {
    let lit = serde_json::to_string(&data.to_string_lossy()).unwrap_or_else(|_| "\"\"".into());
    EXT_SOURCE.replacen("__DATA__", &lit, 1)
}

/// Write the extension under `dir`, in a directory named by a hash of its
/// whole source (the data path included), unless the file there already holds
/// exactly this source: compared on every call, never trusted because it
/// exists. Different sources never share a path, so a pi that loads its file
/// a moment after its launch line was built reads what that line meant — even
/// while an older build still running, or an engine with its own config dir
/// beside the shared data dir, writes its own.
pub fn materialize_at(dir: &Path, data: &Path) -> Result<PathBuf, String> {
    let src = ext_source(data);
    let dir = dir.join(format!("{:016x}", fnv1a(src.as_bytes())));
    let file = dir.join(EXT_FILE);
    if std::fs::read(&file).ok().as_deref() != Some(src.as_bytes()) {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        crate::fsx::write_atomic(&file, src.as_bytes(), None).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(file)
}

/// FNV-1a: stable across Rust versions, which a directory name must be.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// The extension every pi worktrees starts loads, or `None` when it cannot be
/// written (the pi then runs with pi's own hosts only; a launch never fails
/// on it). Under test it lives in a per-process temp dir, not the user's.
pub fn extension() -> Option<PathBuf> {
    #[cfg(test)]
    let (dir, data) = {
        let t = std::env::temp_dir().join(format!("wt-pihosts-test-{}", std::process::id()));
        (t.clone(), t.join(FILE))
    };
    #[cfg(not(test))]
    let (dir, data) = (crate::guidance::data_root().join("pi"), data_path());
    materialize_at(&dir, &data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-pihosts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn pi_lms() -> Host {
        crate::pimodels::parse_hosts(include_str!("../tests/fixtures/pi-models/models-commented.json")).unwrap().remove(0)
    }

    #[test]
    fn ids_and_urls_are_checked_at_the_boundary() {
        assert!(validate_host_id("lm-studio").is_ok());
        for bad in ["", "-x", "a/b", "a b", "a;b", "ü"] {
            assert!(validate_host_id(bad).is_err(), "{bad}");
        }
        assert_eq!(validate_base_url("http://127.0.0.1:1234/v1/").unwrap(), "http://127.0.0.1:1234/v1");
        assert!(validate_base_url("https://box.local/v1").is_ok());
        assert!(validate_base_url("http://[::1]:1234/v1").is_ok());
        for bad in [
            "ftp://h/v1",
            "file:///etc",
            "http://",
            "http://u:p@h/v1",
            "http://h/v1 -o x",
            "http://h/v1?k=1",
            "-ohttp://h",
            "http://-h/v1",
        ] {
            assert!(validate_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_added_host_takes_models_and_never_a_key_field() {
        let mut o = Owned::default();
        add_host(&mut o, "box", "http://h:1/v1", "openai-completions", &[]).unwrap();
        assert!(add_host(&mut o, "box", "http://h:1/v1", "openai-completions", &[]).is_err(), "twice");
        assert!(add_host(&mut o, "lm-studio", "http://h:1/v1", "openai-completions", &["lm-studio".into()]).is_err(), "pi's");
        assert!(add_host(&mut o, "x", "http://h:1/v1", "anthropic-messages", &[]).is_err());
        declare_model(&mut o, "box", "qwen3:8b", &ModelFields { context_window: Some(32768), max_tokens: None }).unwrap();
        assert!(declare_model(&mut o, "box", "qwen3:8b", &ModelFields::default()).is_err(), "twice");
        assert!(declare_model(&mut o, "box", "--x", &ModelFields::default()).is_err());
        assert!(declare_model(&mut o, "box", "m", &ModelFields { context_window: Some(0), max_tokens: None }).is_err());
        update_model(&mut o, "box", "qwen3:8b", &ModelFields { context_window: None, max_tokens: Some(4096) }).unwrap();
        let h = o.get("box").unwrap();
        assert_eq!((h.models[0].context_window, h.models[0].max_tokens), (None, Some(4096)));
        set_base_url(&mut o, "box", "http://other:2/v1").unwrap();
        let json = serde_json::to_string(&o).unwrap();
        assert!(!json.contains("apiKey") && !json.contains("override"), "{json}");
        remove_model(&mut o, "box", "qwen3:8b").unwrap();
        assert!(remove_model(&mut o, "box", "qwen3:8b").is_err());
        remove_host(&mut o, "box").unwrap();
        assert!(o.hosts.is_empty());
    }

    /// pi's model list is REPLACED by an override's (measured on 1.5.0), so an
    /// override starts as a copy of it — and carries no key.
    #[test]
    fn an_override_copies_pis_models_and_not_its_key() {
        let mut o = Owned::default();
        let lms = pi_lms();
        override_host(&mut o, &lms).unwrap();
        let h = o.get("lm-studio").unwrap();
        assert!(h.r#override);
        assert_eq!(h.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["qwen3-coder-next", "a//b"], "every model, in order");
        assert_eq!(h.models[0].context_window, Some(262144));
        let json = serde_json::to_string(&o).unwrap();
        assert!(!json.contains("security") && !json.contains("apiKey"), "{json}");
        assert!(override_host(&mut o, &lms).is_err(), "twice");
        let hosts = as_hosts(&o, &|id| id == "lm-studio");
        assert!(hosts[0].has_key, "an override uses pi's key");
        let mut built_in = lms.clone();
        built_in.backend = "anthropic".into();
        built_in.base_url = None;
        assert!(override_host(&mut o, &built_in).is_err());
    }

    #[test]
    fn the_file_is_edited_under_a_lock_and_never_over_a_bad_or_moved_one() {
        let d = tmp("edit");
        let p = d.join(FILE);
        assert_eq!(read_at(&p).unwrap().hosts.len(), 0, "missing is empty");
        edit_at(&p, |o| add_host(o, "box", "http://h:1/v1", "openai-completions", &[])).unwrap();
        assert!(read_at(&p).unwrap().get("box").is_some());
        // A refused edit writes nothing.
        let before = std::fs::read(&p).unwrap();
        assert!(edit_at(&p, |o| add_host(o, "box", "http://h:1/v1", "openai-completions", &[])).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), before);
        // A file changed on disk mid-edit is not overwritten.
        let r = edit_at(&p, |o| {
            std::fs::write(&p, b"{\"version\":1,\"hosts\":{}}").unwrap();
            remove_host(o, "box")
        });
        assert!(r.unwrap_err().contains("changed"), "lost update refused");
        assert_eq!(std::fs::read(&p).unwrap(), b"{\"version\":1,\"hosts\":{}}");
        // An unreadable file is reported and left alone.
        std::fs::write(&p, b"{not json").unwrap();
        assert!(read_at(&p).unwrap_err().contains("not valid"));
        assert!(edit_at(&p, |o| add_host(o, "x", "http://h:1/v1", "openai-completions", &[])).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"{not json");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Two engines with different data files (a scratch serve's own
    /// XDG_CONFIG_HOME beside the user's), or two builds, must never write
    /// one path: a pi that loads it a moment after its launch line was built
    /// would read the other's hosts.
    #[test]
    fn different_extension_sources_never_share_a_path() {
        let d = tmp("ext-two");
        let a = materialize_at(&d, &d.join("a/pi-models.json")).unwrap();
        let b = materialize_at(&d, &d.join("b/pi-models.json")).unwrap();
        assert_ne!(a, b, "two sources, two paths");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), ext_source(&d.join("a/pi-models.json")), "a kept its own source");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), ext_source(&d.join("b/pi-models.json")));
        assert_eq!(materialize_at(&d, &d.join("a/pi-models.json")).unwrap(), a, "the same source, the same path");
        assert!(a.ends_with(EXT_FILE));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_extension_is_rewritten_when_it_differs_and_names_its_data_file() {
        let d = tmp("ext");
        let data = d.join("my \"data\".json");
        let f = materialize_at(&d, &data).unwrap();
        let src = std::fs::read_to_string(&f).unwrap();
        assert!(src.contains(r#"const DATA = "#) && src.contains(&serde_json::to_string(&data.to_string_lossy()).unwrap()));
        assert!(!src.contains("__DATA__"));
        std::fs::write(&f, "tampered").unwrap();
        materialize_at(&d, &data).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), src, "not trusted because it exists");
        let _ = std::fs::remove_dir_all(&d);
    }
}

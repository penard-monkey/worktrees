//! pi's model catalog, its model hosts' reachability, and whether the pi a
//! pane will get can run at all — the three things that decide whether a pi
//! launch can do anything (pi-harness §2.1, §7, §9.4).
//!
//! What worktrees may touch of pi's, and what it may not:
//!
//! - `pi --list-models` is the PRIMARY source: it applies pi's own merge and
//!   credential rules, so they are not reimplemented here. It is a text table
//!   with no JSON form, pinned by fixture to the version this was built on.
//! - `pi auth check --provider X --json --no-refresh` explains a provider the
//!   list leaves out ("not signed in"). `--no-refresh` keeps it read-only, and
//!   `--credentials` is never passed.
//! - `models.json` is READ for two plain fields — a provider's `baseUrl` and
//!   its models' `id`s — and nothing else. It can hold `apiKey: "!command"`,
//!   which pi runs; worktrees never evaluates, prints or forwards it.
//! - `settings.json` is read for `defaultProvider`/`defaultModel`, to name the
//!   default in a picker even when it is not signed in.
//! - Never read: `models-store.json` (a cache, not a declaration) and
//!   `auth.json` (credentials). Never written: anything under `~/.pi`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::choice::{ModelMeta, ModelOption, ModelRef, Reason};

/// One row of `pi --list-models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub backend: String,
    pub model: String,
    pub context: String,
    pub max_out: String,
    pub thinking: bool,
    pub images: bool,
}

/// Parse `pi --list-models`' table (0.99.1, `fixtures/pi-models/`):
///
/// ```text
/// provider   model        context  max-out  thinking  images
/// lm-studio  qwen3.6-27b  128K     16.4K    no        no
/// ```
///
/// Columns are separated by runs of spaces and no cell contains a space. A
/// table whose header is not this one parses to nothing — a changed format
/// must read as "no catalog", never as garbage rows.
pub fn parse_list_models(text: &str) -> Vec<Listed> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<&str> = match lines.next() {
        Some(h) => h.split_whitespace().collect(),
        None => return Vec::new(),
    };
    if header != ["provider", "model", "context", "max-out", "thinking", "images"] {
        return Vec::new();
    }
    lines
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            if c.len() != 6 {
                return None;
            }
            Some(Listed {
                backend: c[0].into(),
                model: c[1].into(),
                context: c[2].into(),
                max_out: c[3].into(),
                thinking: c[4] == "yes",
                images: c[5] == "yes",
            })
        })
        .collect()
}

/// `pi auth check … --json`: `Some(None)` ready, `Some(Some(reason))` not
/// ready with pi's reason (`credentials_not_configured`,
/// `provider_not_found`), `None` for output this version does not produce.
pub fn parse_auth_check(json: &str) -> Option<Option<String>> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    match v.get("status")?.as_str()? {
        "ready" => Some(None),
        "not_ready" => Some(Some(v.get("reason").and_then(|r| r.as_str()).unwrap_or("unknown").to_string())),
        _ => None,
    }
}

/// A provider declared in `models.json`: its `baseUrl` and model ids, and
/// NOTHING else from its entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub backend: String,
    pub base_url: Option<String>,
    pub models: Vec<String>,
}

pub fn parse_models_json(text: &str) -> Vec<Declared> {
    parse_hosts(text)
        .unwrap_or_default()
        .into_iter()
        .map(|h| Declared { backend: h.backend, base_url: h.base_url, models: h.models.into_iter().map(|m| m.id).collect() })
        .collect()
}

pub fn declared() -> Vec<Declared> {
    std::fs::read_to_string(models_json_path()).map(|t| parse_models_json(&t)).unwrap_or_default()
}

pub fn models_json_path() -> PathBuf {
    crate::pi::agent_dir().join("models.json")
}

/// pi's own pre-parse (1.5.0 `utils/json.js`): `//` line comments and
/// trailing commas are dropped, string literals left alone. pi reads
/// models.json as `JSON.parse(stripJsonComments(stripBom(text)))`, so a file
/// pi loads can be one strict JSON refuses — read through this, or such a
/// file reads as declaring nothing. `/* */` is NOT pi's, and stays an error.
pub fn strip_json_comments(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // Pass 1: comments. Pass 2: trailing commas — two passes, as pi does, so a
    // comment between a comma and its `}` is gone before the comma is judged.
    let no_comments = scan_json(text, |rest| {
        rest.starts_with("//").then(|| rest.find('\n').unwrap_or(rest.len()))
    });
    scan_json(&no_comments, |rest| {
        let after = rest.strip_prefix(',')?;
        after.trim_start().starts_with(['}', ']']).then_some(1)
    })
}

/// Copy `text`, keeping string literals whole and dropping the bytes `cut`
/// claims (its answer is how many) at any position outside one.
fn scan_json(text: &str, cut: impl Fn(&str) -> Option<usize>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if rest.starts_with('"') {
            // A literal: up to the next unescaped quote. An unterminated one is
            // copied as is — the JSON parse then fails, as pi's does.
            let mut end = rest.len();
            let mut esc = false;
            for (j, c) in rest.char_indices().skip(1) {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    end = j + 1;
                    break;
                }
            }
            out.push_str(&rest[..end]);
            i += end;
        } else if let Some(n) = cut(rest) {
            i += n;
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// A model a provider in models.json declares: the fields Settings shows and
/// may one day edit. pi's defaults (1.5.0 `models-configuration.md`) apply
/// when a field is absent: context 128000, max output 16384.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HostModel {
    pub id: String,
    pub name: Option<String>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    pub reasoning: Option<bool>,
    /// `input` includes `"image"`.
    pub images: Option<bool>,
}

/// A provider models.json declares. `has_key` says whether an `apiKey` is
/// present, never what it is: the value may be a secret or a `!command`, and
/// it is not copied out of the parse.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Host {
    pub backend: String,
    pub base_url: Option<String>,
    pub api: Option<String>,
    pub has_key: bool,
    pub models: Vec<HostModel>,
}

/// Every provider models.json declares, or why the file cannot be read as
/// pi reads it. A provider entry that is not an object is skipped.
pub fn parse_hosts(text: &str) -> Result<Vec<Host>, String> {
    let v: serde_json::Value = serde_json::from_str(&strip_json_comments(text)).map_err(|e| format!("not JSON: {e}"))?;
    let providers = match v.get("providers") {
        None => return Ok(Vec::new()),
        Some(p) => p.as_object().ok_or("\"providers\" is not an object")?,
    };
    let s = |o: &serde_json::Value, k: &str| o.get(k).and_then(|u| u.as_str()).map(str::to_string);
    Ok(providers
        .iter()
        .filter(|(_, p)| p.is_object())
        .map(|(name, p)| Host {
            backend: name.clone(),
            base_url: s(p, "baseUrl"),
            api: s(p, "api"),
            has_key: p.get("apiKey").and_then(|k| k.as_str()).is_some_and(|k| !k.is_empty()),
            models: p
                .get("models")
                .and_then(|m| m.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|m| {
                            Some(HostModel {
                                id: s(m, "id")?,
                                name: s(m, "name"),
                                context_window: m.get("contextWindow").and_then(|n| n.as_u64()),
                                max_tokens: m.get("maxTokens").and_then(|n| n.as_u64()),
                                reasoning: m.get("reasoning").and_then(|b| b.as_bool()),
                                images: m
                                    .get("input")
                                    .and_then(|i| i.as_array())
                                    .map(|i| i.iter().any(|x| x.as_str() == Some("image"))),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect())
}

/// What pi said on stderr about a models.json it could not load
/// (`Warning: errors loading models.json:` and its reason, 1.5.0,
/// `fixtures/pi-models/load-error-1.5.0.txt`). pi then drops EVERY provider
/// the file declares, so this is the file's state, not one row's.
///
/// Only lines that name a PATH are kept. A parse failure's own text is
/// replaced: node's `JSON.parse` message quotes a snippet of the file, which
/// can be an `apiKey`.
pub fn parse_load_error(stderr: &str) -> Option<String> {
    let (_, rest) = stderr.split_once("errors loading models.json:")?;
    let mut why: Vec<String> = Vec::new();
    for l in rest.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()) {
        // Whatever follows a parse failure may be the rest of node's snippet.
        if l.starts_with("Failed to parse models.json") {
            return Some("Failed to parse models.json: it is not valid JSON, even allowing // comments and trailing commas.".into());
        } else if l.starts_with("Failed to load models.json") {
            return Some("Failed to load models.json: pi could not read the file.".into());
        } else if l.starts_with("Invalid models.json schema") || l.starts_with("  - ") || l.starts_with("Provider \"") {
            why.push(l.to_string());
        }
    }
    if why.is_empty() {
        why.push("pi reported an error loading models.json.".into());
    }
    let why = why.join("\n");
    Some(if why.chars().count() > 600 { why.chars().take(600).collect::<String>() + "…" } else { why })
}

/// pi's configured default (`defaultProvider`, `defaultModel`), if set.
pub fn default_model() -> Option<(String, String)> {
    let t = std::fs::read_to_string(crate::pi::agent_dir().join("settings.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).ok()?;
    Some((v.get("defaultProvider")?.as_str()?.to_string(), v.get("defaultModel")?.as_str()?.to_string()))
}

/// The pi this process would run: `$WORKTREES_PI_BIN` (development only), else
/// `pi` on PATH. The app's PATH is the login shell's (`fixup_gui_path`).
pub fn pi_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WORKTREES_PI_BIN").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("pi")).find(|p| p.is_file())
}

fn run_pi(args: &[&str], secs: u64) -> Option<std::process::Output> {
    let mut cmd = crate::childenv::command(pi_bin()?);
    cmd.args(args).env("PI_OFFLINE", "1");
    crate::proc::run_deadline(cmd, secs).ok()
}

// ── reachability (§7) ────────────────────────────────────────────────────────

/// What `GET <baseUrl>/models` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// It answered with a model list.
    Served(Vec<String>),
    /// It answered, but not with a list we can read (an auth wall, a 404):
    /// the host is up, and which models it serves is unknown.
    Up,
    /// No answer within the deadline, or the connection was refused.
    Down,
}

/// `pi auth check` and `pi --list-models` both call a DEAD host ready — they
/// check configuration, not reachability — and a blackholed host holds pi's
/// first turn on a spinner for minutes with nothing written. So worktrees asks
/// the host itself, from the app/CLI process (never the pane), with a short
/// deadline. `curl`, not an HTTP crate: same shell-out rule as git and tmux,
/// and no credentials are sent — an `apiKey` in models.json may be a
/// `!command`, which is never evaluated here.
pub fn probe_now(base_url: &str) -> Reach {
    let Some(url) = probe_url(base_url) else { return Reach::Down };
    let mut cmd = crate::childenv::command("curl");
    // `--url`, never a bare positional: the value is from a file pi reads and
    // worktrees does not own, and a positional starting with `-` is an option.
    cmd.args(["-s", "-m", "3", "--connect-timeout", "2", "-o", "-", "-w", "\n%{http_code}", "--url", &url]);
    let Ok(out) = crate::proc::run_deadline(cmd, 4) else { return Reach::Down };
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Reach::Down;
    }
    parse_models_reply(&text)
}

/// `<baseUrl>/models`, or `None` for anything but a plain http(s) URL — a
/// `file://`, a `-o …`-shaped value or whitespace is not a model host and is
/// never handed to curl. `None` reads as unreachable, which refuses the launch
/// with a reason the user can see rather than guessing.
pub fn probe_url(base_url: &str) -> Option<String> {
    let b = base_url.trim_end_matches('/');
    let ok = (b.starts_with("http://") || b.starts_with("https://")) && !b.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then(|| format!("{b}/models"))
}

/// `probe_now`'s output: the body, a newline, then the HTTP status.
pub fn parse_models_reply(text: &str) -> Reach {
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", text));
    let code: u16 = code.trim().parse().unwrap_or(0);
    if code == 0 {
        return Reach::Down;
    }
    if code != 200 {
        return Reach::Up;
    }
    let ids = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| {
        v.get("data")?.as_array().map(|a| a.iter().filter_map(|m| m.get("id")?.as_str().map(str::to_string)).collect::<Vec<_>>())
    });
    ids.map_or(Reach::Up, Reach::Served)
}

pub const REACH_TTL: Duration = Duration::from_secs(60);
static REACH: Mutex<Option<HashMap<String, (Instant, Reach)>>> = Mutex::new(None);

/// `probe_now`, cached per `baseUrl` for `REACH_TTL` and shared by every lane
/// — the app's poll re-lists every 30s, and one host serves many lanes.
pub fn probe(base_url: &str) -> Reach {
    {
        let g = REACH.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, r)) = g.as_ref().and_then(|m| m.get(base_url)) {
            if at.elapsed() < REACH_TTL {
                return r.clone();
            }
        }
    }
    let r = probe_now(base_url);
    let mut g = REACH.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(base_url.to_string(), (Instant::now(), r.clone()));
    r
}

/// The reason a model on a declared backend cannot run, from its host.
pub fn reach_reason(reach: &Reach, model: &str) -> Option<Reason> {
    match reach {
        Reach::Down => Some(Reason::EndpointUnreachable),
        Reach::Served(ids) if !ids.iter().any(|i| i == model) => Some(Reason::NotServed),
        _ => None,
    }
}

/// Why `model` (`backend/id`) would fail to launch now, or `None`. Only a
/// backend with a `baseUrl` in models.json is probed; a built-in provider's
/// endpoint is pi's business.
pub fn launch_reason(model: &str) -> Option<(Reason, String)> {
    let (backend, id) = model.split_once('/')?;
    let d = declared().into_iter().find(|d| d.backend == backend)?;
    let url = d.base_url?;
    let reason = reach_reason(&probe(&url), id.split(':').next().unwrap_or(id))?;
    let why = match reason {
        Reason::EndpointUnreachable => format!("{backend}'s host {url} did not answer within 3s"),
        Reason::NotServed => format!("{backend}'s host {url} is up but does not serve '{id}' (not loaded?)"),
        Reason::NoCredentials => format!("{backend} is not signed in"),
    };
    Some((reason, why))
}

// ── the catalog ──────────────────────────────────────────────────────────────

/// Merge pi's list, the declared-but-unlisted providers (with `auth check`'s
/// reason) and each declared host's reachability into picker rows. Pure: the
/// three lookups are passed in.
pub fn merge_options(
    listed: &[Listed],
    declared: &[Declared],
    default: Option<(String, String)>,
    auth: &dyn Fn(&str) -> Option<Option<String>>,
    reach: &dyn Fn(&str) -> Reach,
) -> Vec<ModelOption> {
    let base = |b: &str| declared.iter().find(|d| d.backend == b).and_then(|d| d.base_url.clone());
    let mut reach_by_url: HashMap<String, Reach> = HashMap::new();
    let mut out: Vec<ModelOption> = Vec::new();
    for l in listed {
        let reason = base(&l.backend).and_then(|u| {
            let r = reach_by_url.entry(u.clone()).or_insert_with(|| reach(&u));
            reach_reason(r, &l.model)
        });
        out.push(ModelOption {
            model: ModelRef { harness: "pi".into(), backend: Some(l.backend.clone()), model: l.model.clone(), label: None },
            ready: reason.is_none(),
            reason,
            source: "pi-list-models".into(),
            meta: ModelMeta {
                context: Some(l.context.clone()),
                max_out: Some(l.max_out.clone()),
                thinking: Some(l.thinking),
                images: Some(l.images),
            },
        });
    }
    // Providers pi knows of but did not list: declared ones, and the configured
    // default — explained by `auth check`, so the picker can say "not signed in"
    // instead of silently omitting the model pi would otherwise start on.
    let mut unlisted: Vec<(String, Vec<String>)> =
        declared.iter().filter(|d| !listed.iter().any(|l| l.backend == d.backend)).map(|d| (d.backend.clone(), d.models.clone())).collect();
    if let Some((b, m)) = default {
        if !listed.iter().any(|l| l.backend == b && l.model == m) {
            match unlisted.iter_mut().find(|(ub, _)| *ub == b) {
                Some((_, ms)) if !ms.contains(&m) => ms.push(m),
                Some(_) => {}
                None if !listed.iter().any(|l| l.backend == b) => unlisted.push((b, vec![m])),
                None => {}
            }
        }
    }
    for (backend, models) in unlisted {
        // pi lists every provider it has credentials for, so an unlisted one is
        // one it has none for. `auth check` is asked anyway: it is what tells a
        // provider pi has never heard of (`provider_not_found`, a typo in a
        // default) from one that is merely signed out — and the former is not a
        // model anyone can pick.
        if matches!(auth(&backend), Some(Some(r)) if r == "provider_not_found") {
            continue;
        }
        let reason = Reason::NoCredentials;
        for m in models {
            out.push(ModelOption {
                model: ModelRef { harness: "pi".into(), backend: Some(backend.clone()), model: m, label: None },
                ready: false,
                reason: Some(reason),
                source: "pi-config".into(),
                meta: ModelMeta::default(),
            });
        }
    }
    out
}

pub const CATALOG_TTL: Duration = Duration::from_secs(60);
/// The rows, and what pi said about a models.json it could not load.
type Catalog = (Instant, Vec<ModelOption>, Option<String>);
static CATALOG: Mutex<Option<Catalog>> = Mutex::new(None);

/// pi's picker rows, cached for `CATALOG_TTL`. Empty when pi is not installed.
pub fn options() -> Vec<ModelOption> {
    catalog().0
}

/// pi's reason it could not load models.json, from the same `--list-models`
/// run the catalog came from (`parse_load_error`).
pub fn load_error() -> Option<String> {
    catalog().1
}

fn catalog() -> (Vec<ModelOption>, Option<String>) {
    {
        let g = CATALOG.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, v, e)) = g.as_ref() {
            if at.elapsed() < CATALOG_TTL {
                return (v.clone(), e.clone());
            }
        }
    }
    let Some(out) = run_pi(&["--list-models", "--offline"], 10) else { return (Vec::new(), None) };
    let listed = parse_list_models(&String::from_utf8_lossy(&out.stdout));
    let load_error = parse_load_error(&String::from_utf8_lossy(&out.stderr));
    let declared = declared();
    let default = default_model();
    // Only providers pi did NOT list are asked about, all at once, under ONE
    // deadline: a picker waits on this, and N serial checks at 5s each is a
    // dialog that opens N×5s late. A check still running at the deadline is
    // simply unanswered (the row still says it is unusable).
    let mut unlisted: Vec<String> = declared.iter().map(|d| d.backend.clone()).chain(default.iter().map(|(b, _)| b.clone())).collect();
    unlisted.retain(|b| !listed.iter().any(|l| &l.backend == b));
    unlisted.dedup();
    let answers = auth_checks(&unlisted, AUTH_DEADLINE);
    let auth = |b: &str| answers.get(b).cloned().flatten();
    let v = merge_options(&listed, &declared, default, &auth, &probe);
    *CATALOG.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), v.clone(), load_error.clone()));
    (v, load_error)
}

// ── Settings → pi → Models: the hosts view ───────────────────────────────────

/// One declared provider as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HostView {
    #[serde(flatten)]
    pub host: Host,
    /// `served` (its `/models` answered with a list), `up` (it answered, not
    /// with a list we read), `down`, or `unprobed` (no http(s) `baseUrl`: a
    /// built-in provider's endpoint is pi's business).
    pub reach: &'static str,
    /// Ids the host's `/models` lists that this provider does not declare —
    /// models pi cannot use until they are declared.
    pub served_undeclared: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Hosts {
    pub path: String,
    /// `missing`, `ok`, or `unreadable` (`problem` says why).
    pub file: &'static str,
    pub problem: Option<String>,
    /// pi's own reason it drops the file (`load_error`): every provider in it
    /// is then unusable, whatever the rows below say.
    pub load_error: Option<String>,
    pub hosts: Vec<HostView>,
}

/// Ids in `served` that `declared` does not name, in the host's order.
pub fn served_undeclared(served: &[String], declared: &[HostModel]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in served {
        if !declared.iter().any(|d| &d.id == s) && !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

/// Join the parsed hosts with each one's reachability. Pure: `reach` is
/// passed in, asked once per distinct `baseUrl`.
pub fn host_views(hosts: Vec<Host>, reach: &dyn Fn(&str) -> Option<Reach>) -> Vec<HostView> {
    hosts
        .into_iter()
        .map(|h| {
            let r = h.base_url.as_deref().filter(|u| probe_url(u).is_some()).map(|u| reach(u));
            let (reach, served_undeclared) = match r {
                None => ("unprobed", Vec::new()),
                Some(None) | Some(Some(Reach::Down)) => ("down", Vec::new()),
                Some(Some(Reach::Up)) => ("up", Vec::new()),
                Some(Some(Reach::Served(ids))) => ("served", served_undeclared(&ids, &h.models)),
            };
            HostView { host: h, reach, served_undeclared }
        })
        .collect()
}

/// Settings → pi → Models: every provider models.json declares, each host's
/// state and what it serves undeclared, and pi's own load error. Reads the
/// file for the fields `Host` names and nothing else; probes every host at
/// once, under one deadline (`probe`'s cache is shared with the catalog).
pub fn hosts() -> Hosts {
    let path = models_json_path();
    let mut out = Hosts { path: path.to_string_lossy().into_owned(), file: "missing", problem: None, load_error: None, hosts: Vec::new() };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => {
            out.file = "unreadable";
            out.problem = Some(format!("cannot read it: {e}"));
            return out;
        }
    };
    let parsed = match parse_hosts(&text) {
        Ok(h) => h,
        Err(why) => {
            out.file = "unreadable";
            out.problem = Some(why);
            out.load_error = load_error();
            return out;
        }
    };
    let mut urls: Vec<String> = parsed.iter().filter_map(|h| h.base_url.clone()).filter(|u| probe_url(u).is_some()).collect();
    urls.sort();
    urls.dedup();
    let answers = gather(&urls, Duration::from_secs(5), probe);
    out.file = "ok";
    out.load_error = load_error();
    out.hosts = host_views(parsed, &|u| answers.get(u).cloned());
    out
}

/// Drop the catalog and reachability caches, so the next read asks pi and
/// the hosts again.
pub fn forget() {
    *CATALOG.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *REACH.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The whole budget for `auth check`s in one catalog read.
pub const AUTH_DEADLINE: Duration = Duration::from_secs(5);

/// `pi auth check` for each of `backends`, in parallel, answered within
/// `deadline` overall. A backend with no answer by then maps to `None`.
fn auth_checks(backends: &[String], deadline: Duration) -> HashMap<String, Option<Option<String>>> {
    let secs = deadline.as_secs().max(1);
    gather(backends, deadline, move |b| {
        run_pi(&["auth", "check", "--provider", b, "--json", "--no-refresh"], secs)
            .and_then(|o| parse_auth_check(&String::from_utf8_lossy(&o.stdout)))
    })
}

/// `check` for every key, concurrently, collecting what answered within
/// `deadline` of the start — the bound is on the WHOLE set.
fn gather<R: Send + 'static>(
    keys: &[String],
    deadline: Duration,
    check: impl Fn(&str) -> R + Send + Sync + 'static,
) -> HashMap<String, R> {
    let check = std::sync::Arc::new(check);
    let (tx, rx) = std::sync::mpsc::channel();
    for b in keys {
        let (tx, b, check) = (tx.clone(), b.clone(), check.clone());
        std::thread::spawn(move || {
            let r = check(&b);
            let _ = tx.send((b, r));
        });
    }
    drop(tx);
    let end = Instant::now() + deadline;
    let mut out = HashMap::new();
    while out.len() < keys.len() {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((b, r)) => {
                out.insert(b, r);
            }
            Err(_) => break,
        }
    }
    out
}

/// The ready rows, as `backend/model`, for an error that has to name them.
pub fn ready_names(opts: &[ModelOption]) -> Vec<String> {
    opts.iter().filter(|o| o.ready).map(|o| o.model.arg()).collect()
}

// ── preflight (§9.4) ─────────────────────────────────────────────────────────

/// Which pi, on which node, a PANE gets — measured by running the pane's own
/// shell the way the pane will (`$SHELL -ic`), because the node is whatever the
/// user's rc files leave first on PATH, and pi's launcher pins nothing unless
/// its installer chose a node.
#[derive(serde::Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Preflight {
    pub pi_path: Option<String>,
    pub pi_version: Option<String>,
    pub node_path: Option<String>,
    pub node_version: Option<String>,
    /// pi's own `engines.node` floor (`>=22.19.0` → `22.19.0`), read from the
    /// `package.json` beside the resolved binary, never hard-coded.
    pub node_floor: Option<String>,
    /// Why a launch must be refused; `None` when it may go ahead.
    pub problem: Option<String>,
}

/// The probe script. Every answer is on a `@@wt <key>=` line, so an rc file
/// that prints (a motd, nvm's banner) cannot be misread as one.
pub const PREFLIGHT_SCRIPT: &str = r#"p=$(command -v pi 2>/dev/null); echo "@@wt pi=$p"; [ -n "$p" ] && echo "@@wt piv=$(pi --version 2>/dev/null | head -n 1)"; n=$(command -v node 2>/dev/null); echo "@@wt node=$n"; [ -n "$n" ] && echo "@@wt nodev=$(node -p process.versions.node 2>/dev/null)"; pn="${XDG_DATA_HOME:-$HOME/.local/share}/pi-node/current/bin/node"; [ -x "$pn" ] && echo "@@wt pinode=$pn" && echo "@@wt pinodev=$("$pn" -p process.versions.node 2>/dev/null)"; true"#;

fn marked(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("@@wt ")?.split_once('='))
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (k.to_string(), v.trim().to_string()))
        .collect()
}

/// `(major, minor, patch)` of `22.19.0` / `v22.19.0` / `>=22.19.0`.
pub fn version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(">=").trim_start_matches('v');
    let mut it = s.split('.').map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

/// A managed install's launcher (`~/.pi/agent/bin/pi`, a sh script reading
/// `install/current-version`) → the real package dir. Anything else → the
/// resolved binary's own ancestors.
fn package_json_for(pi: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(pi).ok()?;
    let head = std::fs::read(&real).ok().map(|b| String::from_utf8_lossy(&b[..b.len().min(4096)]).into_owned());
    let start = if head.as_deref().is_some_and(|h| h.starts_with("#!/bin/sh") && h.contains("current-version")) {
        let agent = real.parent()?.parent()?;
        let v = std::fs::read_to_string(agent.join("install/current-version")).ok()?;
        std::fs::canonicalize(agent.join("install/releases").join(v.trim()).join("node_modules/.bin/pi")).ok()?
    } else {
        real
    };
    start.ancestors().skip(1).take(6).map(|d| d.join("package.json")).find(|p| {
        std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).is_some_and(|v| {
            v.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.ends_with("pi-coding-agent"))
        })
    })
}

fn engines_floor(pkg: &Path) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(pkg).ok()?).ok()?;
    let e = v.get("engines")?.get("node")?.as_str()?;
    e.strip_prefix(">=").map(|s| s.trim().to_string())
}

fn is_managed_launcher(pi: &Path) -> bool {
    std::fs::canonicalize(pi)
        .ok()
        .and_then(|r| std::fs::read(r).ok())
        .is_some_and(|b| String::from_utf8_lossy(&b[..b.len().min(4096)]).contains("pi-node/current/bin"))
}

/// Decide from the measured facts. Pure. `floor` is `engines.node`; `managed`
/// says whether pi's launcher will put its own `pi-node` ahead of the shell's
/// node (it does exactly that when that node exists).
pub fn judge(m: &HashMap<String, String>, floor: Option<String>, managed: bool) -> Preflight {
    let (node_path, node_version) = match (managed, m.get("pinode")) {
        (true, Some(p)) => (Some(p.clone()), m.get("pinodev").cloned()),
        _ => (m.get("node").cloned(), m.get("nodev").cloned()),
    };
    let mut pf = Preflight {
        pi_path: m.get("pi").cloned(),
        pi_version: m.get("piv").cloned(),
        node_path,
        node_version,
        node_floor: floor,
        problem: None,
    };
    pf.problem = if pf.pi_path.is_none() {
        Some("pi is not installed for your shell (no `pi` on its PATH). Install it with: curl -fsSL https://pi.dev/install.sh | sh".into())
    } else if pf.node_version.is_none() {
        Some(format!(
            "pi {} is installed but your shell gives it no node to run on. Re-run pi's installer (curl -fsSL https://pi.dev/install.sh | sh), which can install one.",
            pf.pi_version.as_deref().unwrap_or("?")
        ))
    } else {
        match (pf.node_version.as_deref().and_then(version), pf.node_floor.as_deref().and_then(version)) {
            (Some(have), Some(need)) if have < need => Some(format!(
                "pi {} needs node ≥ {}; your shell gives {} from {}. Below that floor pi can run and misbehave silently. Point your shell at a newer node (e.g. `nvm alias default {}`), or re-run pi's installer.",
                pf.pi_version.as_deref().unwrap_or("?"),
                pf.node_floor.as_deref().unwrap_or("?"),
                pf.node_version.as_deref().unwrap_or("?"),
                pf.node_path.as_deref().unwrap_or("?"),
                pf.node_floor.as_deref().unwrap_or("?"),
            )),
            _ => None,
        }
    };
    pf
}

pub fn preflight_now() -> Preflight {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
    let mut cmd = crate::childenv::command(shell);
    cmd.args(["-ic", PREFLIGHT_SCRIPT]);
    let m = match crate::proc::run_deadline(cmd, 5) {
        Ok(out) => marked(&String::from_utf8_lossy(&out.stdout)),
        Err(e) => {
            // Could not MEASURE is not "below the floor": say so, and do not refuse.
            return Preflight { problem: None, pi_version: Some(format!("(unmeasured: {e})")), ..Default::default() };
        }
    };
    let pi = m.get("pi").map(PathBuf::from);
    let floor = pi.as_deref().and_then(package_json_for).and_then(|p| engines_floor(&p));
    let managed = pi.as_deref().is_some_and(is_managed_launcher);
    judge(&m, floor, managed)
}

pub const PREFLIGHT_TTL: Duration = Duration::from_secs(600);
static PREFLIGHT: Mutex<Option<(Instant, Preflight)>> = Mutex::new(None);

/// `preflight_now`, cached for ten minutes: it runs an interactive shell.
pub fn preflight() -> Preflight {
    if let Some((at, p)) = PREFLIGHT.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if at.elapsed() < PREFLIGHT_TTL {
            return p.clone();
        }
    }
    let p = preflight_now();
    *PREFLIGHT.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), p.clone()));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_0_99_1: &str = include_str!("../tests/fixtures/pi-models/list-models-0.99.1.txt");

    #[test]
    fn the_list_models_table_parses_and_a_changed_one_parses_to_nothing() {
        let l = parse_list_models(LIST_0_99_1);
        assert_eq!(
            l,
            vec![Listed {
                backend: "lm-studio".into(),
                model: "qwen3.6-27b".into(),
                context: "128K".into(),
                max_out: "16.4K".into(),
                thinking: false,
                images: false
            }]
        );
        assert!(parse_list_models("id  provider\nx  y\n").is_empty(), "an unknown header is no catalog");
        assert!(parse_list_models("").is_empty());
        let two = format!("{LIST_0_99_1}kimi-coding  k3  256K  32K  yes  yes\n");
        assert_eq!(parse_list_models(&two).len(), 2);
    }

    const LIST_1_5_0: &str = include_str!("../tests/fixtures/pi-models/list-models-1.5.0.txt");
    const COMMENTED: &str = include_str!("../tests/fixtures/pi-models/models-commented.json");
    const LOAD_ERROR_1_5_0: &str = include_str!("../tests/fixtures/pi-models/load-error-1.5.0.txt");

    #[test]
    fn the_1_5_0_table_parses_too() {
        let l = parse_list_models(LIST_1_5_0);
        assert_eq!(l.iter().map(|r| r.model.as_str()).collect::<Vec<_>>(), ["qwen3-coder-next", "qwen3.8-27b-obliterated-6bit"]);
        assert!(l.iter().all(|r| r.backend == "lm-studio" && r.context == "128K"));
    }

    /// pi 1.5.0 loads a models.json with `//` comments and trailing commas
    /// (measured: `--list-models` lists both of this fixture's lm-studio
    /// models). Strict JSON read it as declaring nothing.
    #[test]
    fn a_models_json_pi_accepts_with_comments_and_trailing_commas_reads_whole() {
        let d = parse_models_json(COMMENTED);
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!(d[0].backend, "lm-studio");
        assert_eq!(d[0].base_url.as_deref(), Some("http://h:1234/v1"));
        assert_eq!(d[0].models, ["qwen3-coder-next", "a//b"], "a // inside a string is not a comment");
        let h = parse_hosts(COMMENTED).unwrap();
        assert_eq!(h[0].models[1].name.as_deref(), Some("slashes \"// in\" a string"));
        assert_eq!(
            h[0].models[0],
            HostModel {
                id: "qwen3-coder-next".into(),
                name: None,
                context_window: Some(262144),
                max_tokens: Some(32768),
                reasoning: Some(true),
                images: Some(true)
            }
        );
        assert!(h[0].has_key && !h[1].has_key);
        assert_eq!(h[1].api.as_deref(), Some("openai-completions"));
        assert!(!format!("{h:?}").contains("security"), "the apiKey never leaves the parse");
        // `/* */` is not pi's, and a broken file is an error, not "no hosts".
        assert!(parse_hosts("{/* x */\"providers\":{}}").is_err());
        assert!(parse_hosts("{\"providers\":[]}").is_err());
        assert_eq!(parse_hosts("{}").unwrap(), vec![]);
        assert_eq!(strip_json_comments("\u{feff}[1, 2 ,\n]"), "[1, 2 \n]");
    }

    #[test]
    fn pis_load_error_is_read_and_never_quotes_the_file() {
        let e = parse_load_error(LOAD_ERROR_1_5_0).unwrap();
        assert_eq!(e, "Invalid models.json schema:\n  - providers.lm-studio.apiKey: must not have fewer than 1 characters");
        assert_eq!(parse_load_error("provider  model\n"), None, "no warning, no error");
        // node's JSON.parse message quotes the file; only our sentence is kept.
        let parse = "Warning: errors loading models.json:\nFailed to parse models.json: Unexpected token 'x', \"\"apiKey\": \"sk-secret\"\" is not valid JSON\n  - sk-secret\n\nFile: /x";
        let e = parse_load_error(parse).unwrap();
        assert!(!e.contains("sk-secret") && e.starts_with("Failed to parse models.json"), "{e}");
    }

    #[test]
    fn a_host_view_says_what_it_serves_that_is_not_declared() {
        let h = parse_hosts(COMMENTED).unwrap();
        let served = |u: &str| match u {
            "http://h:1234/v1" => Some(Reach::Served(vec!["qwen3-coder-next".into(), "new-one".into(), "new-one".into()])),
            _ => Some(Reach::Down),
        };
        let v = host_views(h.clone(), &served);
        assert_eq!((v[0].reach, v[0].served_undeclared.clone()), ("served", vec!["new-one".to_string()]));
        assert_eq!((v[1].reach, v[1].served_undeclared.len()), ("down", 0));
        let mut built_in = h[0].clone();
        built_in.base_url = None;
        assert_eq!(host_views(vec![built_in], &|_| panic!("not probed"))[0].reach, "unprobed");
        assert_eq!(host_views(h.clone(), &|_| Some(Reach::Up))[0].reach, "up");
        assert_eq!(host_views(h, &|_| None)[0].reach, "down", "no answer by the deadline is down");
        let json = serde_json::to_value(&v[0]).unwrap();
        assert_eq!(json["has_key"], true, "flattened");
        assert!(json.get("apiKey").is_none() && !json.to_string().contains("security"));
    }

    #[test]
    fn auth_check_answers_parse() {
        assert_eq!(parse_auth_check(r#"{"status":"ready","provider":"lm-studio","authType":"api_key"}"#), Some(None));
        assert_eq!(
            parse_auth_check(r#"{"status":"not_ready","provider":"kimi-coding","reason":"credentials_not_configured"}"#),
            Some(Some("credentials_not_configured".into()))
        );
        assert_eq!(parse_auth_check("not json"), None);
    }

    #[test]
    fn models_json_yields_urls_and_ids_and_never_a_key() {
        let d = parse_models_json(
            r#"{"providers":{"lm-studio":{"baseUrl":"http://h:1234/v1","api":"openai-completions","apiKey":"!security find-generic-password -w","models":[{"id":"qwen3.6-27b"},{"name":"no id"}]}}}"#,
        );
        assert_eq!(
            d,
            vec![Declared { backend: "lm-studio".into(), base_url: Some("http://h:1234/v1".into()), models: vec!["qwen3.6-27b".into()] }]
        );
        assert!(!format!("{d:?}").contains("security"), "the apiKey never leaves the parse");
    }

    #[test]
    fn a_host_reply_reads_as_served_up_or_down() {
        assert_eq!(parse_models_reply("{\"data\":[{\"id\":\"a\"},{\"id\":\"b\"}]}\n200"), Reach::Served(vec!["a".into(), "b".into()]));
        assert_eq!(parse_models_reply("unauthorized\n401"), Reach::Up);
        assert_eq!(parse_models_reply("\n000"), Reach::Down);
        assert_eq!(parse_models_reply(""), Reach::Down);
        assert_eq!(reach_reason(&Reach::Down, "a"), Some(Reason::EndpointUnreachable));
        assert_eq!(reach_reason(&Reach::Served(vec!["a".into()]), "b"), Some(Reason::NotServed));
        assert_eq!(reach_reason(&Reach::Served(vec!["a".into()]), "a"), None);
        assert_eq!(reach_reason(&Reach::Up, "a"), None, "an auth wall is a live host");
        assert_eq!(probe_url("http://h:1234/v1/").as_deref(), Some("http://h:1234/v1/models"));
        assert_eq!(probe_url("https://api.x/v1").as_deref(), Some("https://api.x/v1/models"));
        for bad in ["file:///etc/passwd", "-o/tmp/x", "http://h/v1 -o /tmp/x", "ftp://h", "", "gopher://h"] {
            assert_eq!(probe_url(bad), None, "{bad}");
            assert_eq!(probe_now(bad), Reach::Down, "{bad} is never fetched");
        }
    }

    #[test]
    fn the_catalog_says_why_each_unusable_model_is_unusable() {
        let listed = parse_list_models(LIST_0_99_1);
        let declared = vec![
            Declared { backend: "lm-studio".into(), base_url: Some("http://h/v1".into()), models: vec!["qwen3.6-27b".into()] },
            Declared { backend: "other".into(), base_url: None, models: vec!["m1".into()] },
        ];
        let default = Some(("kimi-coding".into(), "kimi-for-coding".into()));
        let auth = |_: &str| Some(Some("credentials_not_configured".to_string()));
        let rows = |reach: Reach| {
            let r = reach.clone();
            merge_options(&listed, &declared, default.clone(), &auth, &move |_| r.clone())
        };
        let up = rows(Reach::Served(vec!["qwen3.6-27b".into()]));
        assert_eq!(up.len(), 3);
        assert!(up[0].ready && up[0].reason.is_none() && up[0].model.arg() == "lm-studio/qwen3.6-27b");
        assert_eq!((up[1].model.arg(), up[1].reason), ("other/m1".into(), Some(Reason::NoCredentials)));
        assert_eq!((up[2].model.arg(), up[2].ready), ("kimi-coding/kimi-for-coding".into(), false), "the default is named, not hidden");
        assert_eq!(rows(Reach::Down)[0].reason, Some(Reason::EndpointUnreachable), "pi calls a dead host ready; we do not");
        assert_eq!(rows(Reach::Served(vec![])) [0].reason, Some(Reason::NotServed));
        assert_eq!(ready_names(&up), vec!["lm-studio/qwen3.6-27b".to_string()]);
    }

    /// Every check runs at once and the deadline bounds the SET: three
    /// 300ms checks finish in about 300ms, and one that hangs is dropped at the
    /// deadline instead of holding the rest.
    #[test]
    fn auth_checks_run_together_under_one_deadline() {
        let keys: Vec<String> = ["a", "b", "c", "slow"].iter().map(|s| s.to_string()).collect();
        let t0 = Instant::now();
        let got = gather(&keys, Duration::from_millis(900), |k| {
            std::thread::sleep(Duration::from_millis(if k == "slow" { 5_000 } else { 300 }));
            k.len()
        });
        let took = t0.elapsed();
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(!got.contains_key("slow"));
        assert!(took < Duration::from_millis(1_500), "{took:?}: serial would be 900ms+ before the slow one");
    }

    #[test]
    fn preflight_refuses_below_pis_own_floor_and_prefers_pis_node_when_managed() {
        let m = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        let floor = Some("22.19.0".to_string());
        let base = [("pi", "/u/.local/bin/pi"), ("piv", "0.99.1"), ("node", "/u/.nvm/v22.13.0/bin/node"), ("nodev", "22.13.0")];
        let low = judge(&m(&base), floor.clone(), false);
        let why = low.problem.unwrap();
        assert!(why.contains("22.19.0") && why.contains("22.13.0") && why.contains(".nvm"), "{why}");
        // The same shell with pi's own node: the launcher puts it first.
        let mut with_pinode = base.to_vec();
        with_pinode.extend([("pinode", "/u/.local/share/pi-node/current/bin/node"), ("pinodev", "26.10.0")]);
        let ok = judge(&m(&with_pinode), floor.clone(), true);
        assert_eq!((ok.problem, ok.node_version.as_deref()), (None, Some("26.10.0")));
        // …but only a MANAGED launcher does that.
        assert!(judge(&m(&with_pinode), floor.clone(), false).problem.is_some());
        assert!(judge(&m(&[]), floor.clone(), false).problem.unwrap().contains("not installed"));
        assert!(judge(&m(&base[..2]), floor.clone(), false).problem.unwrap().contains("no node"));
        let exact = judge(&m(&[("pi", "p"), ("node", "n"), ("nodev", "22.19.0")]), floor, false);
        assert_eq!(exact.problem, None, "the floor itself is allowed");
        assert_eq!(version("v22.19.0"), Some((22, 19, 0)));
        assert_eq!(version(">=22.19"), Some((22, 19, 0)));
        assert!(version("26.10.0") > version("22.19.0"));
        assert_eq!(marked("Now using node v22\n@@wt pi=/x\n@@wt piv=\n"), m(&[("pi", "/x")]));
    }
}

//! A project's pull requests, read through the GitHub CLI.
//!
//! Design: `docs/proposals/pull-requests.md` (phase 1). Read-only toward GitHub
//! in every phase — this module runs `gh auth token` (offline) and ONE
//! `gh api graphql` query per project, and nothing else. No writes of any kind.
//!
//! Shape, in the order a fetch walks it:
//!
//! 1. [`parse_remote`] — `git@host:o/r`, `ssh://…`, `https://…` → `{host, owner, repo}`.
//! 2. [`resolve`] — which repo the PRs live on, mirroring `gh`'s own choice so
//!    the app never disagrees with `gh pr list` run in the same checkout:
//!    `remote.<n>.gh-resolved` (set by `gh repo set-default`) wins, then a remote
//!    named `upstream`, `github`, `origin`. The PUSH owner is origin's — for a
//!    fork that is the fork, and it is what a lane's PR head is matched on.
//! 3. [`probe_auth`] — `gh auth token -h <host>`, ~50ms and offline, so a missing
//!    binary or login never costs a network call to learn.
//! 4. [`fetch`] — the query, through `gh` (credentials stay `gh`'s; the token
//!    never enters this process).
//! 5. [`view`] — the snapshot joined to the project's places: each place's PR by
//!    its CURRENT LOCAL BRANCH (never its upstream — every lane here tracks
//!    `origin/main`), plus the per-PR chip state and the attention count, so the
//!    frontend renders and never re-decides.
//!
//! `WORKTREES_GH_BIN` is the seam (as in `agentfiles.rs`): no test reaches GitHub.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The `gh` binary: `$WORKTREES_GH_BIN`, else `gh` on PATH (which, in the app,
/// is the login-shell PATH `fixup_gui_path` installed before any command runs).
pub fn gh_bin() -> String {
    std::env::var("WORKTREES_GH_BIN").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "gh".into())
}

/// The query. One round trip: open PRs with their review/merge/CI state, plus
/// the last 20 merged/closed (which close the loop on a lane — a squash-merge is
/// invisible to git), plus the signed-in login. Measured at ~1.2s and one
/// rate-limit point (proposal §1). The CI rollup is a single `state`, never the
/// check list: `gh pr list --json statusCheckRollup` fetches every context.
pub const QUERY: &str = "query($o: String!, $r: String!) {
  viewer { login }
  repository(owner: $o, name: $r) {
    open: pullRequests(states: OPEN, first: 100, orderBy: {field: UPDATED_AT, direction: DESC}) {
      totalCount
      nodes {
        number title url isDraft updatedAt
        headRefName headRepositoryOwner { login } baseRefName
        author { login } reviewDecision mergeable mergeStateStatus
        commits(last: 1) { nodes { commit { oid statusCheckRollup { state } } } }
      }
    }
    recent: pullRequests(states: [MERGED, CLOSED], first: 20, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number title url state updatedAt mergedAt closedAt
        headRefName headRepositoryOwner { login } baseRefName author { login }
      }
    }
  }
}";

/// A deadline on every `gh` call: a wedged network must not hold a fetch (and
/// the app's in-flight slot for the project) forever.
const GH_DEADLINE_SECS: u64 = 30;
const AUTH_DEADLINE_SECS: u64 = 10;

// ── 1. remote → repo ────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

impl RepoRef {
    /// `https://<host>/<owner>/<repo>` — the repo's web home.
    pub fn web(&self) -> String {
        format!("https://{}/{}/{}", self.host, self.owner, self.repo)
    }
}

/// The https web base of a remote URL, or None for local paths and other
/// protocols. Kept byte-compatible with the app's former `normalize_remote`
/// (whose tests moved here): a path with more than two segments (GitLab
/// subgroups) still has a web base, it just is not a [`RepoRef`].
pub fn web_base(remote: &str) -> Option<String> {
    let r = remote.trim();
    let r = r.strip_suffix(".git").unwrap_or(r);
    // An http(s) remote is already a web URL: keep its scheme (a plain-http
    // Gitea is plain http) and drop only credentials, which a link must not carry.
    for scheme in ["https://", "http://"] {
        if let Some(rest) = r.strip_prefix(scheme) {
            let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
            let host = auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth);
            return Some(if path.is_empty() { format!("{scheme}{host}") } else { format!("{scheme}{host}/{path}") });
        }
    }
    let (host, path) = split_remote(r)?;
    Some(format!("https://{host}/{path}"))
}

/// `{host, owner, repo}` of a remote URL — `git@host:o/r(.git)`,
/// `ssh://git@host[:port]/o/r`, `http(s)://[user@]host[:port]/o/r`. None for
/// anything else, including a path that is not exactly `owner/repo`.
pub fn parse_remote(remote: &str) -> Option<RepoRef> {
    let (host, path) = split_remote(remote)?;
    let path = path.trim_matches('/');
    let mut it = path.split('/');
    let (owner, repo) = (it.next()?, it.next()?);
    if it.next().is_some() || owner.is_empty() || repo.is_empty() {
        return None;
    }
    // A web host never carries a port; an ssh one can (`ssh://git@h:2222/…`).
    let host = host.split(':').next().unwrap_or(&host).to_ascii_lowercase();
    Some(RepoRef { host, owner: owner.to_string(), repo: repo.to_string() })
}

/// (host, path) of a remote, `.git` stripped. The host may still carry a port
/// for http(s) remotes, as the old `normalize_remote` kept it.
fn split_remote(remote: &str) -> Option<(String, String)> {
    let r = remote.trim();
    let r = r.strip_suffix(".git").unwrap_or(r);
    if let Some(rest) = r.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        Some((host.to_string(), path.to_string()))
    } else if let Some(rest) = r.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map(|(_, h)| h).unwrap_or(rest);
        // the authority may carry a port (host:2222/owner/repo) — strip it
        let (auth, path) = rest.split_once('/')?;
        let host = auth.split(':').next().unwrap_or(auth);
        Some((host.to_string(), path.to_string()))
    } else if let Some(rest) = r.strip_prefix("https://").or_else(|| r.strip_prefix("http://")) {
        let (auth, path) = rest.split_once('/')?;
        // credentials in the URL (`https://user:tok@host/…`) are never echoed
        let host = auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth);
        Some((host.to_string(), path.to_string()))
    } else {
        None
    }
}

// ── 2. which repo the PRs live on ───────────────────────────────────────────

/// Where a project's PRs live, and whose branches are "ours".
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Resolved {
    /// The repo the PRs are listed from — the parent, for a fork.
    pub repo: RepoRef,
    /// The owner a lane's PR head must belong to: origin's (the fork, for a
    /// fork; the base repo's own owner otherwise).
    pub push_owner: String,
}

/// Pure half of [`resolve`]: `remotes` is `(name, url)`, `gh_resolved` is the
/// `(remote name, value)` pairs of `remote.<name>.gh-resolved`.
pub fn resolve_from(remotes: &[(String, String)], gh_resolved: &[(String, String)]) -> Option<Resolved> {
    let url_of = |name: &str| remotes.iter().find(|(n, _)| n == name).map(|(_, u)| u.as_str());
    // `gh repo set-default` writes `base` on the chosen remote; an older gh
    // wrote `owner/repo` there instead. Either way that remote's host is the host.
    let mut repo = None;
    for (name, val) in gh_resolved {
        let Some(r) = url_of(name).and_then(parse_remote) else { continue };
        let val = val.trim();
        repo = Some(match val.split_once('/') {
            Some((o, n)) if val != "base" && !o.is_empty() && !n.is_empty() => {
                RepoRef { host: r.host, owner: o.to_string(), repo: n.to_string() }
            }
            _ => r,
        });
        break;
    }
    let repo = repo.or_else(|| ["upstream", "github", "origin"].iter().find_map(|n| url_of(n).and_then(parse_remote)))?;
    let push_owner = url_of("origin").and_then(parse_remote).map(|r| r.owner).unwrap_or_else(|| repo.owner.clone());
    Some(Resolved { repo, push_owner })
}

/// [`resolve_from`] against a checkout's real git config.
pub fn resolve(root: &str) -> Option<Resolved> {
    let remotes: Vec<(String, String)> = crate::git::git_out(root, &["remote", "-v"])
        .unwrap_or_default()
        .lines()
        .filter(|l| l.ends_with("(fetch)"))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), it.next()?.to_string()))
        })
        .collect();
    let gh_resolved: Vec<(String, String)> =
        crate::git::git_out(root, &["config", "--get-regexp", r"^remote\..*\.gh-resolved$"])
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(' ')?;
                let name = k.strip_prefix("remote.")?.strip_suffix(".gh-resolved")?;
                Some((name.to_string(), v.to_string()))
            })
            .collect();
    resolve_from(&remotes, &gh_resolved)
}

// ── 3. auth ─────────────────────────────────────────────────────────────────

/// What `gh` can do for a host, learned offline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Auth {
    Ok,
    /// spawn failed: `gh` is not on PATH
    Missing,
    /// no login at all (`gh` exits 4: "please run: gh auth login")
    LoggedOut,
    /// logged in, but not to this host (exit 1, "no oauth token found for …")
    NoHostToken,
}

/// Classify `gh auth token -h <host>`'s result. `spawn_failed` is ENOENT.
pub fn classify_auth(spawn_failed: bool, code: Option<i32>, stderr: &str) -> Auth {
    if spawn_failed {
        return Auth::Missing;
    }
    match code {
        Some(0) => Auth::Ok,
        Some(4) => Auth::LoggedOut,
        _ if stderr.contains("gh auth login") && !stderr.contains("no oauth token found for") => Auth::LoggedOut,
        _ => Auth::NoHostToken,
    }
}

/// `gh auth token -h <host>`. Its stdout is `/dev/null`, so the token never
/// reaches this process at all — only the exit code and stderr are read.
/// ~50ms, offline (keyring / config file).
pub fn probe_auth(gh: &str, host: &str) -> Auth {
    let mut c = Command::new(gh);
    c.args(["auth", "token", "-h", host]);
    match run_with(c, AUTH_DEADLINE_SECS, false) {
        Err(RunErr::Spawn) => Auth::Missing,
        Err(RunErr::Timeout) => Auth::NoHostToken,
        Ok(o) => classify_auth(false, o.code, &o.stderr),
    }
}

/// Is this host GitHub? `github.com`, or any host `gh` holds a token for — that
/// covers Enterprise without a host list of our own. Anything else gets no tab,
/// no chip, no empty state.
pub fn is_github_host(host: &str, auth: &Auth) -> bool {
    host == "github.com" || *auth == Auth::Ok
}

// ── 4. fetch + parse ────────────────────────────────────────────────────────

/// The CI rollup of a PR's head commit, as GitHub reports it. None = no CI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Ci {
    Success,
    Failure,
    Error,
    Pending,
    Expected,
}

/// One open PR.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub draft: bool,
    pub updated_at: String,
    pub head: String,
    /// None when the head repository is gone (a deleted fork).
    pub head_owner: Option<String>,
    pub base: String,
    pub author: Option<String>,
    /// `APPROVED` / `CHANGES_REQUESTED` / `REVIEW_REQUIRED`, or None.
    pub review: Option<String>,
    /// `MERGEABLE` / `CONFLICTING` / `UNKNOWN` (still computing — "checking",
    /// never a conflict).
    pub mergeable: String,
    pub merge_state: String,
    pub ci: Option<Ci>,
    pub head_oid: Option<String>,
}

/// One recently merged or closed PR.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrRef {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// `MERGED` or `CLOSED`
    pub state: String,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
    pub head: String,
    pub head_owner: Option<String>,
    pub base: String,
    pub author: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub repo: RepoRef,
    /// The signed-in login, from the same query.
    pub viewer: Option<String>,
    pub open_total: u64,
    pub open: Vec<Pr>,
    pub recent: Vec<PrRef>,
}

/// Why a fetch produced no snapshot. Classified by stderr and the GraphQL
/// `errors` array, never by exit code alone: NOT_FOUND and "host unreachable"
/// both exit 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FetchError {
    Missing,
    LoggedOut,
    /// GitHub does not distinguish missing from private: the signed-in account
    /// cannot see the repo. `viewer` names the account, because a wrong active
    /// account is the likely cause.
    NotFound { viewer: Option<String> },
    /// Network: unreachable, timed out. The app keeps its last good result.
    Offline { message: String },
    Other { message: String },
}

// Wire shapes — private; [`parse`] flattens them.
#[derive(Deserialize)]
struct Wire {
    data: Option<WData>,
    #[serde(default)]
    errors: Vec<WErr>,
}
#[derive(Deserialize)]
struct WErr {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    message: String,
}
#[derive(Deserialize)]
struct WData {
    viewer: Option<WLogin>,
    repository: Option<WRepo>,
}
#[derive(Deserialize)]
struct WLogin {
    login: String,
}
#[derive(Deserialize)]
struct WRepo {
    open: WConn<WOpen>,
    recent: WConn<WRecent>,
}
#[derive(Deserialize)]
struct WConn<T> {
    #[serde(rename = "totalCount", default)]
    total: Option<u64>,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WOpen {
    number: u64,
    title: String,
    url: String,
    #[serde(default)]
    is_draft: bool,
    updated_at: String,
    head_ref_name: String,
    head_repository_owner: Option<WLogin>,
    base_ref_name: String,
    author: Option<WLogin>,
    review_decision: Option<String>,
    mergeable: Option<String>,
    merge_state_status: Option<String>,
    commits: Option<WConn<WCommitNode>>,
}
#[derive(Deserialize)]
struct WCommitNode {
    commit: WCommit,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WCommit {
    oid: Option<String>,
    status_check_rollup: Option<WRollup>,
}
#[derive(Deserialize)]
struct WRollup {
    state: Option<Ci>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WRecent {
    number: u64,
    title: String,
    url: String,
    state: String,
    updated_at: String,
    merged_at: Option<String>,
    closed_at: Option<String>,
    head_ref_name: String,
    head_repository_owner: Option<WLogin>,
    base_ref_name: String,
    author: Option<WLogin>,
}

/// Parse `gh api graphql`'s stdout (it prints the body on failure too).
pub fn parse(repo: &RepoRef, json: &str) -> Result<Snapshot, FetchError> {
    let w: Wire = serde_json::from_str(json).map_err(|e| FetchError::Other { message: format!("unreadable reply from gh: {e}") })?;
    let viewer = w.data.as_ref().and_then(|d| d.viewer.as_ref()).map(|v| v.login.clone());
    if w.errors.iter().any(|e| e.kind.as_deref() == Some("NOT_FOUND")) {
        return Err(FetchError::NotFound { viewer });
    }
    let Some(r) = w.data.and_then(|d| d.repository) else {
        let message = w.errors.first().map(|e| e.message.clone()).unwrap_or_else(|| "GitHub returned no repository".into());
        return Err(if w.errors.is_empty() { FetchError::NotFound { viewer } } else { FetchError::Other { message } });
    };
    let open: Vec<Pr> = r
        .open
        .nodes
        .into_iter()
        .flatten()
        .map(|n| {
            let head = n.commits.and_then(|c| c.nodes.into_iter().flatten().last()).map(|c| c.commit);
            Pr {
                number: n.number,
                title: n.title,
                url: n.url,
                draft: n.is_draft,
                updated_at: n.updated_at,
                head: n.head_ref_name,
                head_owner: n.head_repository_owner.map(|o| o.login),
                base: n.base_ref_name,
                author: n.author.map(|a| a.login),
                review: n.review_decision,
                mergeable: n.mergeable.unwrap_or_else(|| "UNKNOWN".into()),
                merge_state: n.merge_state_status.unwrap_or_else(|| "UNKNOWN".into()),
                ci: head.as_ref().and_then(|c| c.status_check_rollup.as_ref()).and_then(|r| r.state),
                head_oid: head.and_then(|c| c.oid),
            }
        })
        .collect();
    let recent = r
        .recent
        .nodes
        .into_iter()
        .flatten()
        .map(|n| PrRef {
            number: n.number,
            title: n.title,
            url: n.url,
            state: n.state,
            updated_at: n.updated_at,
            merged_at: n.merged_at,
            closed_at: n.closed_at,
            head: n.head_ref_name,
            head_owner: n.head_repository_owner.map(|o| o.login),
            base: n.base_ref_name,
            author: n.author.map(|a| a.login),
        })
        .collect();
    Ok(Snapshot { repo: repo.clone(), viewer, open_total: r.open.total.unwrap_or(0), open, recent })
}

/// Classify a failed `gh api graphql` from its stderr (stdout is tried first by
/// [`fetch`], since gh prints the GraphQL body on a query error).
pub fn classify_failure(code: Option<i32>, stderr: &str) -> FetchError {
    let s = stderr.trim();
    let low = s.to_ascii_lowercase();
    if code == Some(4) || low.contains("gh auth login") {
        FetchError::LoggedOut
    } else if low.contains("error connecting")
        || low.contains("could not resolve host")
        || low.contains("no such host")
        || low.contains("timeout")
        || low.contains("timed out")
        || low.contains("network is unreachable")
        || low.contains("connection refused")
    {
        FetchError::Offline { message: first_line(s) }
    } else {
        FetchError::Other { message: if s.is_empty() { format!("gh exited {}", code.unwrap_or(-1)) } else { first_line(s) } }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(300).collect()
}

/// Run the query for `repo` through `gh`. Network; ~1.2s. The caller probes
/// auth first ([`probe_auth`]) so the common failures never get this far.
pub fn fetch(gh: &str, repo: &RepoRef) -> Result<Snapshot, FetchError> {
    let mut c = Command::new(gh);
    c.args(["api", "graphql", "--hostname", &repo.host])
        .arg("-f")
        .arg(format!("query={QUERY}"))
        // `-f`, never `-F`: a typed field turns `2048`/`null`/`true` into JSON
        // non-strings (GraphQL then rejects `String!`), expands `{owner}`, and
        // reads a FILE for a value that starts with `@`. Names are data.
        .arg("-f")
        .arg(format!("o={}", repo.owner))
        .arg("-f")
        .arg(format!("r={}", repo.repo));
    match run(c, GH_DEADLINE_SECS) {
        Err(RunErr::Spawn) => Err(FetchError::Missing),
        Err(RunErr::Timeout) => Err(FetchError::Offline { message: format!("gh timed out after {GH_DEADLINE_SECS}s") }),
        Ok(o) if o.code == Some(0) => parse(repo, &o.stdout),
        Ok(o) => match parse(repo, &o.stdout) {
            // a GraphQL-level error carries a body worth more than stderr
            Err(e @ FetchError::NotFound { .. }) => Err(e),
            _ => Err(classify_failure(o.code, &o.stderr)),
        },
    }
}

// ── 5. the view: chip states, lane mapping, attention ──────────────────────

/// What a PR's dot and words say. One decision, made here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Chip {
    Merged,
    Closed,
    Draft,
    Conflicts,
    Failing,
    ChangesRequested,
    /// CI running, or `mergeable` still being computed
    Pending,
    /// CI green (or no CI) and mergeable
    Ready,
}

impl Chip {
    /// The dot's tone: `ok` / `warn` / `danger` / `accent` / `mute`. Colour lives
    /// in the dot only; the words are always `--txt-hi`/`--txt-dim`.
    pub fn tone(self) -> &'static str {
        match self {
            Chip::Ready => "ok",
            Chip::Pending => "warn",
            Chip::Conflicts | Chip::Failing | Chip::ChangesRequested => "danger",
            Chip::Merged => "accent",
            Chip::Draft | Chip::Closed => "mute",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Chip::Merged => "merged",
            Chip::Closed => "closed",
            Chip::Draft => "draft",
            Chip::Conflicts => "conflicts",
            Chip::Failing => "CI failing",
            Chip::ChangesRequested => "changes requested",
            Chip::Pending => "checking",
            Chip::Ready => "ready",
        }
    }
}

/// An open PR's chip. Order is severity: a conflict outranks a red CI run (the
/// CI result is for a merge that cannot happen), both outrank review state.
pub fn chip_of(pr: &Pr) -> Chip {
    if pr.draft {
        return Chip::Draft;
    }
    if pr.mergeable == "CONFLICTING" {
        return Chip::Conflicts;
    }
    if matches!(pr.ci, Some(Ci::Failure) | Some(Ci::Error)) {
        return Chip::Failing;
    }
    if pr.review.as_deref() == Some("CHANGES_REQUESTED") {
        return Chip::ChangesRequested;
    }
    if matches!(pr.ci, Some(Ci::Pending) | Some(Ci::Expected)) || pr.mergeable == "UNKNOWN" {
        return Chip::Pending;
    }
    Chip::Ready
}

pub fn chip_of_ref(r: &PrRef) -> Chip {
    if r.state == "MERGED" {
        Chip::Merged
    } else {
        Chip::Closed
    }
}

/// A place, as the mapping needs it: its slug and its CURRENT local branch.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlaceBranch {
    pub slug: String,
    pub branch: String,
}

fn owner_eq(a: Option<&str>, b: &str) -> bool {
    a.is_some_and(|a| a.eq_ignore_ascii_case(b))
}

/// Which PR is this branch's? Open first — head == branch and head owner == the
/// push owner; a PR whose head IS its base is never a lane's. Otherwise the
/// newest merged/closed one from `recent` (which is ordered newest-first), so a
/// merged PR stays on its lane for as long as GitHub lists it there.
///
/// Returns the PR number. Never matches on upstream: a lane branched from
/// `origin/main` tracks it, and "upstream's name" would map it onto every PR.
pub fn pr_for(branch: &str, push_owner: &str, snap: &Snapshot) -> Option<u64> {
    if branch.is_empty() {
        return None;
    }
    let mine = |head: &str, base: &str, owner: Option<&str>| head == branch && head != base && owner_eq(owner, push_owner);
    if let Some(p) = snap.open.iter().find(|p| mine(&p.head, &p.base, p.head_owner.as_deref())) {
        return Some(p.number);
    }
    snap.recent.iter().find(|r| mine(&r.head, &r.base, r.head_owner.as_deref())).map(|r| r.number)
}

/// One row of the view: an open or a recent PR, with what the UI shows for it.
#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub head: String,
    pub base: String,
    pub author: Option<String>,
    /// `open` / `merged` / `closed`
    pub state: &'static str,
    pub draft: bool,
    pub chip: Chip,
    pub tone: &'static str,
    pub label: &'static str,
    pub ci: Option<Ci>,
    pub review: Option<String>,
    pub mergeable: Option<String>,
    pub updated_at: String,
    /// merged_at / closed_at for a recent PR
    pub ended_at: Option<String>,
    /// The place whose branch this is, when one is.
    pub place: Option<String>,
    /// Needs you: failing CI, changes requested, conflicting, or merged while
    /// its place still exists. Only PRs in a place count — the "no place" tail
    /// is mostly other people's stale branches (proposal §3: seven of nine open
    /// PRs were conflicting), and a badge that is always red says nothing.
    pub attention: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub repo: RepoRef,
    pub web: String,
    pub viewer: Option<String>,
    /// open PRs (all of them, not just the first page)
    pub open_count: u64,
    /// how many rows have `attention`
    pub attention_count: u64,
    /// open PRs, newest update first
    pub open: Vec<Row>,
    /// merged/closed, newest first
    pub recent: Vec<Row>,
    /// slug → PR number, for every place that has one
    pub places: BTreeMap<String, u64>,
}

/// Join a snapshot to the project's places. Pure; cheap enough to run on every
/// call against a cached snapshot, so a branch switch re-maps without a fetch.
pub fn view(snap: &Snapshot, push_owner: &str, places: &[PlaceBranch]) -> View {
    let mut by_pr: BTreeMap<u64, String> = BTreeMap::new();
    let mut map: BTreeMap<String, u64> = BTreeMap::new();
    for p in places {
        if let Some(n) = pr_for(&p.branch, push_owner, snap) {
            map.insert(p.slug.clone(), n);
            // two places on one branch cannot happen (git refuses); first wins
            by_pr.entry(n).or_insert_with(|| p.slug.clone());
        }
    }
    let open: Vec<Row> = snap
        .open
        .iter()
        .map(|p| {
            let chip = chip_of(p);
            let place = by_pr.get(&p.number).cloned();
            Row {
                number: p.number,
                title: p.title.clone(),
                url: p.url.clone(),
                head: p.head.clone(),
                base: p.base.clone(),
                author: p.author.clone(),
                state: "open",
                draft: p.draft,
                chip,
                tone: chip.tone(),
                label: chip.label(),
                ci: p.ci,
                review: p.review.clone(),
                mergeable: Some(p.mergeable.clone()),
                updated_at: p.updated_at.clone(),
                ended_at: None,
                attention: place.is_some() && matches!(chip, Chip::Conflicts | Chip::Failing | Chip::ChangesRequested),
                place,
            }
        })
        .collect();
    let recent: Vec<Row> = snap
        .recent
        .iter()
        .map(|r| {
            let chip = chip_of_ref(r);
            let place = by_pr.get(&r.number).cloned();
            Row {
                number: r.number,
                title: r.title.clone(),
                url: r.url.clone(),
                head: r.head.clone(),
                base: r.base.clone(),
                author: r.author.clone(),
                state: if chip == Chip::Merged { "merged" } else { "closed" },
                draft: false,
                chip,
                tone: chip.tone(),
                label: chip.label(),
                ci: None,
                review: None,
                mergeable: None,
                updated_at: r.updated_at.clone(),
                ended_at: r.merged_at.clone().or_else(|| r.closed_at.clone()),
                attention: place.is_some() && chip == Chip::Merged,
                place,
            }
        })
        .collect();
    let attention_count = open.iter().chain(&recent).filter(|r| r.attention).count() as u64;
    View {
        web: snap.repo.web(),
        repo: snap.repo.clone(),
        viewer: snap.viewer.clone(),
        open_count: snap.open_total.max(snap.open.len() as u64),
        attention_count,
        open,
        recent,
        places: map,
    }
}

// ── diagnostics ─────────────────────────────────────────────────────────────

/// One line for the diagnostics block: version, and per host the login, token
/// source and whether it is active — from `gh auth status --json hosts`, which
/// VALIDATES each token against the API (~0.5s), so the caller gates it on the
/// Pull requests switch. Never prints a token.
pub fn diag_line(gh: &str) -> String {
    let mut v = Command::new(gh);
    v.arg("--version");
    let version = match run(v, AUTH_DEADLINE_SECS) {
        Err(RunErr::Spawn) => return "(not found)".into(),
        Err(RunErr::Timeout) => return "(timed out)".into(),
        Ok(o) => o.stdout.lines().next().unwrap_or("").trim().to_string(),
    };
    let mut s = Command::new(gh);
    s.args(["auth", "status", "--json", "hosts"]);
    let hosts = match run(s, AUTH_DEADLINE_SECS) {
        Ok(o) => auth_summary(&o.stdout),
        Err(_) => "auth status unavailable".into(),
    };
    format!("{version} · {hosts}")
}

/// `gh auth status --json hosts` → "github.com: login (keyring, active)".
pub fn auth_summary(json: &str) -> String {
    #[derive(Deserialize)]
    struct H {
        #[serde(default)]
        login: String,
        #[serde(default)]
        active: bool,
        #[serde(default, rename = "tokenSource")]
        source: String,
        #[serde(default)]
        state: String,
    }
    #[derive(Deserialize)]
    struct S {
        hosts: BTreeMap<String, Vec<H>>,
    }
    let Ok(s) = serde_json::from_str::<S>(json) else {
        return "auth status unreadable".into();
    };
    if s.hosts.values().all(|v| v.is_empty()) {
        return "not logged in".into();
    }
    s.hosts
        .iter()
        .flat_map(|(host, v)| {
            v.iter().map(move |h| {
                let mut tags = vec![h.source.clone()];
                if h.active {
                    tags.push("active".into());
                }
                if !h.state.is_empty() && h.state != "success" {
                    tags.push(h.state.clone());
                }
                format!("{host}: {} ({})", h.login, tags.into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", "))
            })
        })
        .collect::<Vec<_>>()
        .join("; ")
}

// ── process plumbing ────────────────────────────────────────────────────────

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}
enum RunErr {
    Spawn,
    Timeout,
}

/// Run with no stdin, no prompts, and a deadline. Pipes are drained on threads
/// so a large reply (the query is ~10KB, a busy repo far more) cannot fill the
/// pipe and wedge the child.
fn run(c: Command, deadline_secs: u64) -> Result<Out, RunErr> {
    run_with(c, deadline_secs, true)
}

/// `capture_stdout: false` sends the child's stdout to `/dev/null` — for a
/// command whose output must never enter this process (a token).
fn run_with(mut c: Command, deadline_secs: u64, capture_stdout: bool) -> Result<Out, RunErr> {
    c.env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(if capture_stdout { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|_| RunErr::Spawn)?;
    let so = child.stdout.take();
    let mut se = child.stderr.take().expect("piped");
    let to = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(mut so) = so {
            let _ = so.read_to_end(&mut b);
        }
        b
    });
    let te = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = se.read_to_end(&mut b);
        b
    });
    let deadline = Instant::now() + Duration::from_secs(deadline_secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunErr::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return Err(RunErr::Spawn),
        }
    };
    let stdout = String::from_utf8_lossy(&to.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&te.join().unwrap_or_default()).into_owned();
    Ok(Out { code: status.code(), stdout, stderr })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = include_str!("../tests/fixtures/github/worktrees-2026-10-07.json");
    const NOT_FOUND: &str = include_str!("../tests/fixtures/github/not-found.json");

    fn rr() -> RepoRef {
        RepoRef { host: "github.com".into(), owner: "penard-monkey".into(), repo: "worktrees".into() }
    }
    fn pb(slug: &str, branch: &str) -> PlaceBranch {
        PlaceBranch { slug: slug.into(), branch: branch.into() }
    }
    fn s(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    /// Moved from the app's `normalize_remote` test, cases unchanged: the remote
    /// spec is whatever `git remote get-url` says; the web base is what a
    /// browser can open, and everything else is None rather than a made-up link.
    #[test]
    fn a_remote_spec_becomes_one_https_base_or_none() {
        for (spec, want) in [
            ("git@github.com:acme/repo.git", "https://github.com/acme/repo"),
            ("git@github.com:acme/repo", "https://github.com/acme/repo"),
            ("ssh://git@github.com/acme/repo.git", "https://github.com/acme/repo"),
            ("ssh://git@gitea.local:2222/acme/repo.git", "https://gitea.local/acme/repo"),
            ("https://github.com/acme/repo.git", "https://github.com/acme/repo"),
            ("https://github.com/acme/repo", "https://github.com/acme/repo"),
            ("http://gitlab.internal/group/sub/repo.git", "http://gitlab.internal/group/sub/repo"),
            ("  git@github.com:acme/repo.git\n", "https://github.com/acme/repo"),
            // new: credentials never reach a link
            ("https://user:tok@github.com/acme/repo.git", "https://github.com/acme/repo"),
        ] {
            assert_eq!(web_base(spec).as_deref(), Some(want), "{spec}");
        }
        for spec in ["/Users/x/repo.git", "../sibling", "file:///tmp/repo", "git://github.com/acme/repo", "git@nocolon"] {
            assert_eq!(web_base(spec), None, "{spec}");
        }
    }

    #[test]
    fn parse_remote_handles_ssh_https_and_enterprise_hosts() {
        let r = |h: &str, o: &str, n: &str| Some(RepoRef { host: h.into(), owner: o.into(), repo: n.into() });
        assert_eq!(parse_remote("git@github.com:penard-monkey/worktrees.git"), r("github.com", "penard-monkey", "worktrees"));
        assert_eq!(parse_remote("ssh://git@ghe.example.com:2222/team/app.git"), r("ghe.example.com", "team", "app"));
        assert_eq!(parse_remote("https://GitHub.com/o/r"), r("github.com", "o", "r"));
        assert_eq!(parse_remote("https://user:secret@ghe.corp.net/o/r.git"), r("ghe.corp.net", "o", "r"));
        // not owner/repo
        assert_eq!(parse_remote("https://gitlab.com/group/sub/r.git"), None);
        assert_eq!(parse_remote("/srv/git/r.git"), None);
        assert_eq!(parse_remote("https://github.com/o"), None);
    }

    #[test]
    fn resolve_mirrors_gh_default_repo_order() {
        let fork = s(&[("origin", "git@github.com:me/r.git"), ("upstream", "https://github.com/parent/r.git")]);
        // upstream beats origin, push owner stays origin's
        let got = resolve_from(&fork, &[]).unwrap();
        assert_eq!((got.repo.owner.as_str(), got.push_owner.as_str()), ("parent", "me"));
        // gh-resolved=base on origin wins over upstream
        let got = resolve_from(&fork, &s(&[("origin", "base")])).unwrap();
        assert_eq!(got.repo.owner, "me");
        // the older owner/repo form
        let got = resolve_from(&fork, &s(&[("upstream", "other/thing")])).unwrap();
        assert_eq!((got.repo.owner.as_str(), got.repo.repo.as_str()), ("other", "thing"));
        // `github` beats `origin`
        let got = resolve_from(&s(&[("origin", "git@github.com:a/r"), ("github", "git@github.com:b/r")]), &[]).unwrap();
        assert_eq!((got.repo.owner.as_str(), got.push_owner.as_str()), ("b", "a"));
        // no usable remote
        assert_eq!(resolve_from(&s(&[("origin", "/srv/r.git")]), &[]), None);
        assert_eq!(resolve_from(&[], &[]), None);
    }

    #[test]
    fn auth_is_classified_as_gh_reports_it() {
        assert_eq!(classify_auth(true, None, ""), Auth::Missing);
        assert_eq!(classify_auth(false, Some(0), ""), Auth::Ok);
        assert_eq!(
            classify_auth(false, Some(4), "You are not logged into any GitHub hosts. To log in, run: gh auth login"),
            Auth::LoggedOut
        );
        assert_eq!(classify_auth(false, Some(1), "no oauth token found for ghe.example.com"), Auth::NoHostToken);
        assert!(is_github_host("github.com", &Auth::Missing));
        assert!(is_github_host("ghe.example.com", &Auth::Ok));
        assert!(!is_github_host("gitlab.com", &Auth::NoHostToken));
    }

    #[test]
    fn parses_the_captured_reply() {
        let s = parse(&rr(), REAL).unwrap();
        assert_eq!(s.viewer.as_deref(), Some("penard-monkey"));
        assert_eq!(s.open_total, 8);
        assert_eq!(s.open.len(), 8);
        let p350 = s.open.iter().find(|p| p.number == 350).unwrap();
        assert_eq!(p350.head, "codex-skills-mcps-close-out");
        assert_eq!(p350.mergeable, "CONFLICTING");
        assert_eq!(p350.ci, Some(Ci::Success));
        assert!(p350.head_oid.is_some());
        let p301 = s.open.iter().find(|p| p.number == 301).unwrap();
        assert_eq!(p301.head_owner.as_deref(), Some("t957095"));
        assert_eq!(p301.ci, None);
        assert_eq!(s.recent.len(), 20);
        let r442 = s.recent.iter().find(|r| r.number == 442).unwrap();
        assert_eq!((r442.state.as_str(), r442.head.as_str()), ("MERGED", "fix/codex-place-session"));
        assert!(r442.merged_at.is_some());
    }

    #[test]
    fn not_found_names_no_repo_and_failures_classify_by_text() {
        assert!(matches!(parse(&rr(), NOT_FOUND), Err(FetchError::NotFound { .. })));
        assert!(matches!(parse(&rr(), "not json"), Err(FetchError::Other { .. })));
        assert_eq!(classify_failure(Some(4), "please run: gh auth login"), FetchError::LoggedOut);
        assert!(matches!(
            classify_failure(Some(1), "error connecting to api.github.com\ncheck your internet connection"),
            FetchError::Offline { .. }
        ));
        assert!(matches!(classify_failure(Some(1), "HTTP 502: Bad Gateway"), FetchError::Other { .. }));
    }

    #[test]
    fn lanes_map_by_local_branch_never_by_upstream() {
        let s = parse(&rr(), REAL).unwrap();
        let v = view(
            &s,
            "penard-monkey",
            &[
                pb("codex-skills-mcps", "codex-skills-mcps-close-out"), // slug ≠ branch
                pb("roadmap-and-github-issues", "roadmap-and-github-issues"),
                pb("pr-panel", "pr-panel"), // merged #452
                pb("(main)", "main"),       // never a PR's head
                pb("fresh", "nothing-pushed"),
                // a fork's branch name in OUR repo must not claim its PR
                pb("combobox", "fix/combobox-hi-clamp"),
            ],
        );
        assert_eq!(v.places.get("codex-skills-mcps"), Some(&350));
        assert_eq!(v.places.get("roadmap-and-github-issues"), Some(&296));
        assert_eq!(v.places.get("pr-panel"), Some(&452));
        assert_eq!(v.places.get("(main)"), None);
        assert_eq!(v.places.get("fresh"), None);
        assert_eq!(v.places.get("combobox"), None);
        // the same branch name, owner = the fork → it is ours
        let fork = view(&s, "t957095", &[pb("combobox", "fix/combobox-hi-clamp")]);
        assert_eq!(fork.places.get("combobox"), Some(&301));
    }

    #[test]
    fn open_beats_recent_and_recent_is_newest_first() {
        let mut s = parse(&rr(), REAL).unwrap();
        // a reopened branch: an old closed PR plus a new open one
        let mut old = s.recent[0].clone();
        old.head = "roadmap-and-github-issues".into();
        old.number = 1;
        s.recent.push(old);
        assert_eq!(pr_for("roadmap-and-github-issues", "penard-monkey", &s), Some(296));
        // two closed PRs for one branch: the first listed (newest) wins
        let mut a = s.recent[3].clone();
        a.head = "twice".into();
        a.number = 900;
        let mut b = a.clone();
        b.number = 800;
        s.recent.insert(0, a);
        s.recent.push(b);
        assert_eq!(pr_for("twice", "penard-monkey", &s), Some(900));
    }

    #[test]
    fn chips_rank_by_severity() {
        let base = parse(&rr(), REAL).unwrap().open[0].clone();
        let with = |f: &dyn Fn(&mut Pr)| {
            let mut p = base.clone();
            p.draft = false;
            p.mergeable = "MERGEABLE".into();
            p.ci = Some(Ci::Success);
            p.review = None;
            f(&mut p);
            chip_of(&p)
        };
        assert_eq!(with(&|_| {}), Chip::Ready);
        assert_eq!(with(&|p| p.ci = None), Chip::Ready);
        assert_eq!(with(&|p| p.draft = true), Chip::Draft);
        assert_eq!(with(&|p| p.mergeable = "CONFLICTING".into()), Chip::Conflicts);
        assert_eq!(with(&|p| p.mergeable = "UNKNOWN".into()), Chip::Pending);
        assert_eq!(with(&|p| p.ci = Some(Ci::Pending)), Chip::Pending);
        assert_eq!(with(&|p| p.ci = Some(Ci::Error)), Chip::Failing);
        assert_eq!(
            with(&|p| {
                p.ci = Some(Ci::Failure);
                p.mergeable = "CONFLICTING".into()
            }),
            Chip::Conflicts
        );
        assert_eq!(with(&|p| p.review = Some("CHANGES_REQUESTED".into())), Chip::ChangesRequested);
        assert_eq!(Chip::Merged.tone(), "accent");
        assert_eq!(Chip::Conflicts.tone(), "danger");
    }

    #[test]
    fn attention_counts_only_prs_in_a_place() {
        let s = parse(&rr(), REAL).unwrap();
        // no places: seven conflicting PRs, and still no attention
        let none = view(&s, "penard-monkey", &[]);
        assert_eq!(none.attention_count, 0);
        assert_eq!(none.open_count, 8);
        // a conflicting lane + a merged lane need you; a no-PR lane does not
        let v = view(
            &s,
            "penard-monkey",
            &[pb("a", "roadmap-and-github-issues"), pb("b", "md-path-links"), pb("c", "elsewhere")],
        );
        assert_eq!(v.attention_count, 2);
        let row = v.recent.iter().find(|r| r.number == 451).unwrap();
        assert_eq!((row.state, row.place.as_deref(), row.attention), ("merged", Some("b"), true));
    }

    #[test]
    fn auth_summary_names_login_and_source_never_a_token() {
        let j = r#"{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com","login":"me","tokenSource":"keyring"}]}}"#;
        assert_eq!(auth_summary(j), "github.com: me (keyring, active)");
        assert_eq!(auth_summary(r#"{"hosts":{}}"#), "not logged in");
    }

    #[cfg(unix)]
    #[test]
    fn fetch_and_probe_go_through_the_gh_seam() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("wt-gh-seam-{}-{}", std::process::id(), crate::sysclock::now_epoch()));
        std::fs::create_dir_all(&dir).unwrap();
        let reply = dir.join("reply.json");
        std::fs::write(&reply, REAL).unwrap();
        // the fake records its argv, one arg per line: the seam is also where
        // the CALL is pinned, not only the reply
        let argv = dir.join("argv");
        let fake = dir.join("gh");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncase \"$1 $2\" in\n  'auth token') [ \"$4\" = github.com ] && {{ echo tok; exit 0; }}; echo 'no oauth token found for '\"$4\" >&2; exit 1;;\n  'api graphql') cat '{}';;\n  *) exit 2;;\nesac\n",
                argv.display(),
                reply.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let gh = fake.to_str().unwrap();
        assert_eq!(probe_auth(gh, "github.com"), Auth::Ok);
        assert_eq!(probe_auth(gh, "ghe.example.com"), Auth::NoHostToken);
        assert_eq!(probe_auth(dir.join("nope").to_str().unwrap(), "github.com"), Auth::Missing);
        let snap = fetch(gh, &rr()).unwrap();
        assert_eq!(snap.open.len(), 8);
        // owner/repo go RAW (`-f`): gh's `-F` types `2048`/`null`/`true` as
        // JSON, expands `{owner}`, and reads a file for a value starting `@`
        let args: Vec<String> = std::fs::read_to_string(&argv).unwrap().lines().map(str::to_string).collect();
        let pair = |flag: &str, val: &str| args.windows(2).any(|w| w[0] == flag && w[1] == val);
        assert!(pair("--hostname", "github.com"), "{args:?}");
        assert!(pair("-f", "o=penard-monkey"), "{args:?}");
        assert!(pair("-f", "r=worktrees"), "{args:?}");
        assert!(!args.iter().any(|a| a == "-F"), "no typed fields: {args:?}");
        // a repo named like a number still goes as a string
        let numeric = RepoRef { host: "github.com".into(), owner: "@etc".into(), repo: "2048".into() };
        let _ = fetch(gh, &numeric);
        let args: Vec<String> = std::fs::read_to_string(&argv).unwrap().lines().map(str::to_string).collect();
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == "r=2048"), "{args:?}");
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == "o=@etc"), "{args:?}");
        assert_eq!(fetch(dir.join("nope").to_str().unwrap(), &rr()), Err(FetchError::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

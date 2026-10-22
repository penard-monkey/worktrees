//! The resolver: every row's capability and objects, checked before its body.
//!
//! Server Edition Phase 2, R1 (`docs/proposals/server-edition-phase2.md` §3.3).
//! A row declares the capability it needs and the objects its arguments name
//! (`dispatch.rs`); [`authorize`] runs them, in order, against the grant of the
//! caller's (principal, device) pair:
//!
//! 1. the exposure — `ClientLocal` is never run, `HostConsole` and `Loopback`
//!    only for the console;
//! 2. the row's capability;
//! 3. each object: the argument is read, resolved to a project (a canonical
//!    registry root) and checked against the grant's scope, and the CANONICAL
//!    value is written back into the arguments, so the body acts on exactly
//!    what was checked.
//!
//! The body then runs on a `Ctx` narrowed to the same scope (`Ctx::scoped`),
//! so the api's own guard answers with the caller's roots: two checks, as R1
//! asked. Results and events are cut down to the scope in `scope.rs`.
//!
//! **A refusal says the same thing whether the object is missing or outside
//! the scope** — `not found or not in this device's scope` — so a scoped device
//! cannot probe which paths, projects or sessions exist. The real reason rides
//! in [`Refusal::detail`], for the host's eyes. The console is the exception,
//! and only in what it is TOLD: nothing is outside its scope, so it gets the
//! detail, which for a missing file or session is the error the body itself
//! would have given before this existed — loopback behaviour stays as it was.

use std::path::{Path, PathBuf};

use serde_json::Value;
use worktrees_core::authz::{self, Access, Capability, Caller, DeviceId, Grant, Requirement};
use worktrees_core::tmux::PaneList;

use super::dispatch::{Args, Call, Exposure, Row};
use super::scope::Filter;

/// What an optional project argument's ABSENCE means, per row: each absent
/// form is a different act, so the table says which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Absent {
    /// Nothing more to check (a global read with an optional focus project).
    Free,
    /// The host-wide form: the effective scope must be `"*"`.
    AllScope,
    /// The host-wide form, an `admin` act: `admin` and scope `"*"`.
    AdminAll,
    /// The console only (`sync_apply`'s adopt form, §8 gap 4).
    Console,
}

/// One object a row's arguments name. The `&str`s are argument names, spelled
/// as the row reads them (`a.get("repo")`).
#[derive(Clone, Copy, Debug)]
pub enum Obj {
    /// The argument canonicalizes to EXACTLY a registered root (§3.3) — stricter
    /// than `guard_under_projects`, which takes anything under one. A root that
    /// no longer exists on disk still matches its registry key verbatim, so a
    /// deleted project can be removed.
    Project(&'static str),
    /// [`Obj::Project`] when present (non-null, non-empty); [`Absent`] when not.
    OptProject(&'static str, Absent),
    /// `project(repo)` and a slug that is `(main)` or one path component.
    Place(&'static str, &'static str),
    /// A slug alone, of the project an earlier object resolved (an optional
    /// place: `doctor`'s whole-project form).
    Slug(&'static str),
    /// A PLACE root: a registered root, or a direct child of its `.worktrees/`.
    Root(&'static str),
    /// An existing path under a registered project, owned by the innermost
    /// project that contains it, and its path class's requirement (§7).
    File(&'static str),
    /// A tmux session that belongs to a place in scope (§8 gap 3).
    Session(&'static str),
    /// A tmux session of the project another argument named (that argument's
    /// object must come first in the row).
    SessionIn(&'static str, &'static str),
    /// A tmux session of one place: (session, repo, slug), the place resolved
    /// earlier in the row.
    SessionOf(&'static str, &'static str, &'static str),
    /// A tmux attach this connection opened (§8 gap 16).
    Attach(&'static str),
    /// [`Obj::Attach`] for a CLOSE, which is idempotent: closing an attach
    /// this connection does not own is answered `ok` without running the body
    /// — what closing a missing one always was. A pane replaced on reconnect
    /// closes its old id over the NEW socket (#525), and on a restarted serve
    /// that id may name someone else's attach; it must neither close it nor
    /// turn a routine stale close into an error.
    Close(&'static str),
    /// A dock shell: `place(repo, slug)` and a `u32` index.
    Shell(&'static str, &'static str, &'static str),
    /// An optional nested `{repo, slug, index}` (`resolve_term_paths.shell`).
    ShellRef(&'static str),
    /// A clone job THIS connection started: its id is namespaced per
    /// connection on the way in, so no other connection can name it.
    Job(&'static str),
    /// The id a new clone job will have, namespaced the same way.
    NewJob(&'static str),
    /// A device of the caller's own principal (§2.8). No row names one yet:
    /// slice 5's device rows will.
    Device(&'static str),
    /// A path where a project WILL be: `admin` (the row's) and scope `"*"`.
    NewPath(&'static str),
    /// `project_prs.places`: each `{slug}` of the row's project passes the
    /// slug rule.
    Places(&'static str),
    /// `reorder_projects.roots`: every element a registered root in scope.
    Roots(&'static str),
    /// Strings scraped from text that MAY name files (`resolve_*_paths.paths`).
    /// Not objects: the body resolves each against the caller's roots (the
    /// scoped `project_roots`), and one that resolves nowhere is a null, never
    /// a refusal of the whole batch — which is what it is for the console too.
    Candidates(&'static str),
    /// No project object. Allowed to any caller holding the capability, and only
    /// for a row on `GLOBAL_OK` (a test holds the two together).
    Global,
    /// A multi-project read whose RESULT goes through the filter.
    Filtered(Filter),
    /// Host-wide: the effective scope must be `"*"`.
    All,
    /// Answered for the console only; a remote caller gets
    /// `unavailable-remote` (§8 gap 7: a loopback URL means nothing off the
    /// host, and carries the docs server's token).
    NotRemote,
    /// The inner object when its argument is present (non-null, non-empty).
    Opt(&'static Obj),
}

impl Obj {
    /// The arguments this object covers, for the drift test.
    pub fn args(&self) -> Vec<&'static str> {
        use Obj::*;
        match *self {
            Project(a) | OptProject(a, _) | Slug(a) | Root(a) | File(a) | Session(a) | Attach(a) | Close(a) | ShellRef(a) | Job(a)
            | NewJob(a) | Device(a) | NewPath(a) | Places(a) | Roots(a) | Candidates(a) => vec![a],
            SessionIn(s, _) => vec![s],
            Place(r, s) => vec![r, s],
            SessionOf(s, _, _) => vec![s],
            Shell(r, s, i) => vec![r, s, i],
            Global | Filtered(_) | All | NotRemote => vec![],
            Opt(o) => o.args(),
        }
    }

    /// The argument whose absence makes an [`Obj::Opt`] skip.
    fn primary(&self) -> Option<&'static str> {
        self.args().first().copied()
    }

    pub fn is_global(&self) -> bool {
        matches!(self, Obj::Global)
    }

    pub fn filter(&self) -> Option<Filter> {
        match self {
            Obj::Filtered(f) => Some(*f),
            _ => None,
        }
    }
}

// ── refusals ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalKind {
    /// A `ClientLocal` row, over any socket.
    ClientLocal,
    /// A `HostConsole` row, from anything but the console.
    ConsoleOnly,
    /// A `Loopback` row, from anything but the loopback listener.
    LoopbackOnly,
    /// A row that answers only on the host (`open_docs_viewer`).
    UnavailableRemote,
    /// The grant lacks the capability, or the scope `"*"`, the act needs.
    Denied,
    /// The object does not exist, or is outside the scope — one answer.
    NotFound,
    /// An argument the resolver had to read is malformed. Its error is the one
    /// `Args::get` gives the body, and names nothing that exists.
    BadArgs,
    /// An idempotent act on something that is not there for this caller
    /// ([`Obj::Close`]): the dispatcher answers `ok` and does not run the body.
    Gone,
}

/// Why a request was not run. `detail` is the real reason, never sent to a
/// device; slice 3's audit record keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub kind: RefusalKind,
    pub detail: String,
}

pub const ERR_CONSOLE_ONLY: &str = "console-only";
pub const ERR_LOOPBACK_ONLY: &str = "loopback-only";
pub const ERR_UNAVAILABLE_REMOTE: &str = "unavailable-remote";
pub const ERR_DENIED: &str = "denied";
pub const ERR_NOT_FOUND: &str = "not found or not in this device's scope";

impl Refusal {
    fn new(kind: RefusalKind, detail: impl Into<String>) -> Refusal {
        Refusal { kind, detail: detail.into() }
    }

    fn not_found(detail: impl Into<String>) -> Refusal {
        Refusal::new(RefusalKind::NotFound, detail)
    }

    fn denied(detail: impl Into<String>) -> Refusal {
        Refusal::new(RefusalKind::Denied, detail)
    }

    /// The wire's `err` for a paired device: fixed words per kind.
    pub fn wire(&self) -> Value {
        Value::from(match self.kind {
            RefusalKind::ClientLocal => super::wire::CLIENT_LOCAL,
            RefusalKind::ConsoleOnly => ERR_CONSOLE_ONLY,
            RefusalKind::LoopbackOnly => ERR_LOOPBACK_ONLY,
            RefusalKind::UnavailableRemote => ERR_UNAVAILABLE_REMOTE,
            RefusalKind::Denied => ERR_DENIED,
            RefusalKind::NotFound => ERR_NOT_FOUND,
            RefusalKind::BadArgs => return Value::from(self.detail.clone()),
            RefusalKind::Gone => return Value::Null,
        })
    }

    /// The wire's `err` for THIS caller. The console has every capability and
    /// scope `"*"`, so a refusal can tell it nothing it could not list itself,
    /// and it gets the real reason — for a missing file or session, the error
    /// the body gave before the resolver existed.
    pub fn wire_for(&self, caller: &Caller) -> Value {
        if caller.is_console() && matches!(self.kind, RefusalKind::NotFound | RefusalKind::Denied) {
            return Value::from(self.detail.clone());
        }
        self.wire()
    }
}

// ── the world the resolver reads ─────────────────────────────────────────────

/// What resolution reads besides the request: the registered roots and the
/// tmux panes. A trait so the tests hand in fixtures rather than the user's
/// registry and tmux server (AGENTS.md: a throwaway tmux needs `-L`, and the
/// resolver's tests need none at all).
pub trait World: Send + Sync {
    /// Registered project roots, the registry's own keys, in nav order. ALL
    /// of them: the scope is applied by the resolver, not here.
    fn roots(&self) -> Vec<String>;
    /// One `list-panes -a` snapshot, or `None` when tmux is absent.
    fn panes(&self) -> Option<PaneList>;
}

/// The live world: the api's registry read (which merges the app's old list
/// once per process) and the tmux server.
pub struct Live<'a>(pub &'a worktrees_api::Ctx);

impl World for Live<'_> {
    fn roots(&self) -> Vec<String> {
        worktrees_api::projects::read_projects(self.0)
    }
    fn panes(&self) -> Option<PaneList> {
        PaneList::fetch()
    }
}

/// What the objects resolved to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    /// The registry key of every project an object resolved to, in row order.
    pub projects: Vec<String>,
}

/// Check `row` for `cx.caller` holding `grant`, and rewrite `args` to the
/// canonical objects. `Err` is a refusal: the body must not run. `world` is
/// [`Live`] in serve (`dispatch::call`) and a fixture in the tests.
pub fn authorize(row: &Row, cx: &Call, grant: &Grant, args: &mut Args, world: &dyn World) -> Result<Resolved, Refusal> {
    let caller = &cx.caller;
    match row.exposure {
        Exposure::ClientLocal => return Err(Refusal::new(RefusalKind::ClientLocal, "a client-local command")),
        Exposure::HostConsole if !caller.is_console() => {
            return Err(Refusal::new(RefusalKind::ConsoleOnly, format!("{} is a console act", row.name)))
        }
        Exposure::Loopback if caller.listener() != authz::Listener::Loopback => {
            return Err(Refusal::new(RefusalKind::LoopbackOnly, format!("{} is served on loopback only", row.name)))
        }
        _ => {}
    }
    if let Some(cap) = row.cap {
        if !grant.allows(cap) {
            return Err(Refusal::denied(format!("{} needs `{cap}`", row.name)));
        }
    }
    let mut st = St { cx, grant, world, roots: None, bound: Vec::new(), out: Resolved::default() };
    for obj in row.objects {
        st.resolve(row, obj, args)?;
    }
    Ok(st.out)
}

/// The path class's requirement for one file, on top of the row's capability.
/// THE seam for the path classifier (slice 2, `worktrees_core::pathclass`):
/// until it lands every path is plain, which is exactly today's behaviour.
pub fn file_requirement(_project_root: &Path, _rel: &Path, _access: Access) -> Requirement {
    Requirement::Row
}

// ── resolution ───────────────────────────────────────────────────────────────

struct St<'a> {
    cx: &'a Call,
    grant: &'a Grant,
    world: &'a dyn World,
    roots: Option<Vec<String>>,
    /// Argument name → the project key (and place dir) it resolved to, for
    /// the objects that refer back to an earlier one.
    bound: Vec<(&'static str, String, Option<PathBuf>)>,
    out: Resolved,
}

/// A JSON argument that counts as absent: missing, `null` or `""`.
fn absent(v: &Value) -> bool {
    v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty())
}

fn canon(p: &str) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

impl St<'_> {
    fn roots(&mut self) -> &[String] {
        if self.roots.is_none() {
            self.roots = Some(self.world.roots());
        }
        self.roots.as_deref().unwrap_or_default()
    }

    fn get<T: serde::de::DeserializeOwned>(&self, args: &Args, key: &str) -> Result<T, Refusal> {
        args.get(key).map_err(|e| Refusal::new(RefusalKind::BadArgs, e))
    }

    fn bind(&mut self, arg: &'static str, key: String, dir: Option<PathBuf>) {
        self.out.projects.push(key.clone());
        self.bound.push((arg, key, dir));
    }

    fn bound(&self, arg: &str) -> Option<&(&'static str, String, Option<PathBuf>)> {
        self.bound.iter().rev().find(|(a, _, _)| *a == arg)
    }

    fn need_all(&self, what: &str) -> Result<(), Refusal> {
        if self.grant.projects.is_all() {
            return Ok(());
        }
        Err(Refusal::denied(format!("{what} needs scope \"*\"")))
    }

    fn in_scope(&self, key: &str) -> Result<(), Refusal> {
        if self.grant.projects.covers_root(key) {
            return Ok(());
        }
        Err(Refusal::not_found(format!("project outside this device's scope: {key}")))
    }

    /// A registry key for `raw`: verbatim first (a deleted project still
    /// matches its key), then by canonical form, lazily so one dead mount
    /// among the roots costs nothing unless it is reached.
    fn match_root(&mut self, raw: &str) -> Option<String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if let Some(r) = self.roots().iter().find(|r| r.as_str() == raw) {
            return Some(r.clone());
        }
        let c = canon(raw)?;
        self.roots().iter().find(|r| canon(r).as_deref() == Some(c.as_path())).cloned()
    }

    /// The innermost registered project containing canonical `path`, as
    /// (registry key, canonical root).
    fn owner_of(&mut self, path: &Path) -> Option<(String, PathBuf)> {
        self.roots()
            .to_vec()
            .into_iter()
            .filter_map(|r| canon(&r).map(|c| (r, c)))
            .filter(|(_, c)| path.starts_with(c))
            .max_by_key(|(_, c)| c.components().count())
    }

    fn project(&mut self, arg: &'static str, args: &mut Args) -> Result<String, Refusal> {
        let raw: String = self.get(args, arg)?;
        let key = self.match_root(&raw).ok_or_else(|| Refusal::not_found(format!("not a registered project: {raw}")))?;
        self.in_scope(&key)?;
        args.set(arg, Value::from(key.clone()));
        let dir = canon(&key);
        self.bind(arg, key.clone(), dir);
        Ok(key)
    }

    fn slug(&mut self, arg: &'static str, args: &mut Args) -> Result<String, Refusal> {
        let raw: String = self.get(args, arg)?;
        let slug = slug_rule(&raw).map_err(Refusal::not_found)?;
        args.set(arg, Value::from(slug.clone()));
        Ok(slug)
    }

    fn resolve(&mut self, row: &Row, obj: &Obj, args: &mut Args) -> Result<(), Refusal> {
        match *obj {
            Obj::Project(a) => {
                self.project(a, args)?;
            }
            Obj::OptProject(a, when_absent) => {
                if !absent(&args.raw(a)) {
                    self.project(a, args)?;
                } else {
                    match when_absent {
                        Absent::Free => {}
                        Absent::AllScope => self.need_all(row.name)?,
                        Absent::AdminAll => {
                            if !self.grant.allows(Capability::Admin) {
                                return Err(Refusal::denied(format!("{} without a project needs `admin`", row.name)));
                            }
                            self.need_all(row.name)?;
                        }
                        Absent::Console => {
                            if !self.cx.caller.is_console() {
                                return Err(Refusal::new(RefusalKind::ConsoleOnly, format!("{} without a project is a console act", row.name)));
                            }
                        }
                    }
                }
            }
            Obj::Place(r, s) => {
                let key = self.project(r, args)?;
                let slug = self.slug(s, args)?;
                let dir = canon(&key).map(|root| if slug == "(main)" { root } else { root.join(".worktrees").join(&slug) });
                if let Some(b) = self.bound.iter_mut().rev().find(|(a, _, _)| *a == r) {
                    b.2 = dir;
                }
            }
            Obj::Slug(s) => {
                self.slug(s, args)?;
            }
            Obj::Root(a) => {
                let raw: String = self.get(args, a)?;
                let c = canon(&raw).ok_or_else(|| Refusal::not_found(format!("not a directory: {raw}")))?;
                let (key, root) = self.owner_of(&c).ok_or_else(|| Refusal::not_found(format!("path outside workspace: {raw}")))?;
                let is_place = c == root || c.parent() == Some(root.join(".worktrees").as_path());
                if !is_place {
                    return Err(Refusal::not_found(format!("not a place root: {raw}")));
                }
                self.in_scope(&key)?;
                args.set(a, Value::from(c.to_string_lossy().into_owned()));
                self.bind(a, key, Some(c));
            }
            Obj::File(a) => {
                let raw: String = self.get(args, a)?;
                // `guard_under_projects`' rule and its words, so the console's
                // error is the one it always had — over the WORLD's roots: the
                // owning project is the innermost, and it must be in scope.
                let c = std::fs::canonicalize(&raw).map_err(|e| Refusal::not_found(format!("{raw}: {e}")))?;
                let (key, root) = self.owner_of(&c).ok_or_else(|| Refusal::not_found(format!("path outside workspace: {raw}")))?;
                self.in_scope(&key)?;
                let access = if row.cap == Some(Capability::Read) { Access::Read } else { Access::Write };
                let rel = c.strip_prefix(&root).unwrap_or(Path::new(""));
                match file_requirement(&root, rel, access) {
                    Requirement::Row => {}
                    Requirement::Cap(cap) if self.grant.allows(cap) => {}
                    Requirement::Cap(cap) => return Err(Refusal::denied(format!("{} of {raw} needs `{cap}`", row.name))),
                    Requirement::Console if self.cx.caller.is_console() => {}
                    Requirement::Console => return Err(Refusal::denied(format!("{} of {raw} is a console act", row.name))),
                    Requirement::Never => return Err(Refusal::denied(format!("{raw} is never {}", if access == Access::Read { "read" } else { "written" }))),
                }
                args.set(a, Value::from(c.to_string_lossy().into_owned()));
                self.bind(a, key, None);
            }
            Obj::Session(a) => {
                let name: String = self.get(args, a)?;
                let key = self.session_project(&name, None)?;
                self.bind(a, key, None);
            }
            Obj::SessionIn(a, other) => {
                let name: String = self.get(args, a)?;
                let key = self.bound(other).map(|b| b.1.clone()).expect("SessionIn names an object resolved earlier in the row");
                self.session_project(&name, Some(&key))?;
            }
            Obj::SessionOf(a, r, s) => {
                let name: String = self.get(args, a)?;
                let (key, dir) = self.bound(r).map(|b| (b.1.clone(), b.2.clone())).expect("SessionOf names a place resolved earlier in the row");
                let slug: String = self.get(args, s)?;
                self.session_of_place(&name, &key, &slug, dir.as_deref())?;
            }
            Obj::Attach(a) => {
                let id: u32 = self.get(args, a)?;
                if !self.cx.frames.owns_term(id) {
                    return Err(Refusal::not_found("no such terminal"));
                }
            }
            Obj::Close(a) => {
                let id: u32 = self.get(args, a)?;
                if !self.cx.frames.owns_term(id) {
                    return Err(Refusal::new(RefusalKind::Gone, format!("terminal {id} is not this connection's")));
                }
            }
            Obj::Shell(r, s, i) => {
                self.resolve(row, &Obj::Place(r, s), args)?;
                let _: u32 = self.get(args, i)?;
            }
            Obj::ShellRef(a) => {
                let v = args.raw(a);
                if !v.is_null() {
                    let sh: worktrees_api::files::ShellRef =
                        serde_json::from_value(v).map_err(|e| Refusal::new(RefusalKind::BadArgs, format!("invalid args `{a}`: {e}")))?;
                    let mut inner = Args(serde_json::Map::new());
                    inner.set("repo", Value::from(sh.repo));
                    inner.set("slug", Value::from(sh.slug));
                    self.resolve(row, &Obj::Place("repo", "slug"), &mut inner)?;
                    let mut fixed = serde_json::Map::new();
                    fixed.insert("repo".into(), inner.raw("repo"));
                    fixed.insert("slug".into(), inner.raw("slug"));
                    fixed.insert("index".into(), Value::from(sh.index));
                    args.set(a, Value::Object(fixed));
                }
            }
            Obj::Job(a) | Obj::NewJob(a) => {
                let id: String = self.get(args, a)?;
                args.set(a, Value::from(format!("{}/{id}", self.cx.frames.tag())));
            }
            Obj::Device(a) => {
                let id: String = self.get(args, a)?;
                if !authz::owns_device(&*self.cx.registry, &self.cx.caller, &DeviceId::new(id.clone())) {
                    return Err(Refusal::not_found(format!("no device {id} of {}", self.cx.caller.principal())));
                }
            }
            Obj::NewPath(_) => self.need_all(row.name)?,
            Obj::Places(a) => {
                let v = args.raw(a);
                let list = v.as_array().cloned().unwrap_or_default();
                for el in &list {
                    let slug = el.get("slug").and_then(Value::as_str).unwrap_or_default();
                    slug_rule(slug).map_err(Refusal::not_found)?;
                }
            }
            Obj::Roots(a) => {
                let raw: Vec<String> = self.get(args, a)?;
                let mut keys = Vec::with_capacity(raw.len());
                for r in &raw {
                    let key = self.match_root(r).ok_or_else(|| Refusal::not_found(format!("not a registered project: {r}")))?;
                    self.in_scope(&key)?;
                    keys.push(key);
                }
                self.out.projects.extend(keys.iter().cloned());
                args.set(a, Value::from(keys));
            }
            Obj::Candidates(_) | Obj::Global | Obj::Filtered(_) => {}
            Obj::All => self.need_all(row.name)?,
            Obj::NotRemote => {
                if !self.cx.caller.is_console() {
                    return Err(Refusal::new(RefusalKind::UnavailableRemote, format!("{} answers on the host only", row.name)));
                }
            }
            Obj::Opt(inner) => {
                let present = inner.primary().is_some_and(|k| !absent(&args.raw(k)));
                if present {
                    self.resolve(row, inner, args)?;
                }
            }
        }
        Ok(())
    }

    /// The project (registry key) of a place session `name` belongs to,
    /// within scope — and within `only`, when given.
    ///
    /// A session belongs to a place by the evidence adoption uses: a pane of
    /// it cwd'd inside the place (`PaneList::session_in`), which also covers
    /// the provider sidecars and an adopted foreign-named session. Failing
    /// that, by NAME: the canonical `<prefix>-<slug>` of a place, or one of its
    /// sidecars, which still holds when every pane has `cd`'d away. A session
    /// with neither — the user's own tmux, a stranger's — is no place's, and
    /// the console cannot attach it through here either (§8 gap 3). One
    /// `list-panes` and no git in the common case; the name rule costs a
    /// `discover` per project in scope, and runs only when the panes say
    /// nothing.
    fn session_project(&mut self, name: &str, only: Option<&str>) -> Result<String, Refusal> {
        let missing = || Refusal::not_found(format!("no tmux session named {name}"));
        if name.trim().is_empty() {
            return Err(missing());
        }
        let grant = self.grant;
        let in_scope = |k: &str| grant.projects.covers_root(k) && only.is_none_or(|o| o == k);
        if let Some(panes) = self.world.panes() {
            let paths: Vec<String> = panes.session_paths(name).map(str::to_string).collect();
            for p in &paths {
                let c = canon(p).unwrap_or_else(|| PathBuf::from(p));
                if let Some((key, _)) = self.owner_of(&c) {
                    if in_scope(&key) {
                        return Ok(key);
                    }
                }
            }
        }
        let keys: Vec<String> = self.roots().iter().filter(|k| in_scope(k)).cloned().collect();
        for key in keys {
            let Ok(project) = worktrees_core::Project::discover(Path::new(&key)) else { continue };
            if project.place_index().iter().any(|p| names_of(&project, &p.slug).iter().any(|n| n == name)) {
                return Ok(key);
            }
        }
        Err(missing())
    }

    /// `name` is THIS place's session: its canonical name or a sidecar of it,
    /// or a session with a pane cwd'd in the place (for `(main)`, outside
    /// `.worktrees/`, which nests under it — adoption's own exclusion).
    fn session_of_place(&mut self, name: &str, key: &str, slug: &str, dir: Option<&Path>) -> Result<(), Refusal> {
        let missing = || Refusal::not_found(format!("no tmux session named {name} in {slug}"));
        if let Ok(project) = worktrees_core::Project::discover(Path::new(key)) {
            if names_of(&project, slug).iter().any(|n| n == name) {
                return Ok(());
            }
        }
        let Some(dir) = dir else { return Err(missing()) };
        let exclude = (slug == "(main)").then(|| dir.join(".worktrees"));
        let Some(panes) = self.world.panes() else { return Err(missing()) };
        let hit = panes.session_paths(name).any(|p| {
            let c = canon(p).unwrap_or_else(|| PathBuf::from(p));
            c.starts_with(dir) && !exclude.as_ref().is_some_and(|e| c.starts_with(e))
        });
        if hit {
            return Ok(());
        }
        Err(missing())
    }
}

/// Every session name a place answers to: its canonical one and each
/// provider's sidecar of it.
fn names_of(project: &worktrees_core::Project, slug: &str) -> Vec<String> {
    let canonical = project.session_name(slug);
    std::iter::once(canonical.clone())
        .chain(worktrees_core::provider::PROVIDERS.iter().map(|p| p.sidecar_name(&canonical)))
        .collect()
}

/// A place slug, as every caller must now send one (§3.5): `(main)`, or ONE
/// path component that core's `safe_arg` passes — no `/`, no leading `-`, no
/// `..`, no control characters. `place_dir` is `format!("{wt_root}/{slug}")`,
/// so `../x` used to leave `.worktrees/`.
pub fn slug_rule(raw: &str) -> Result<String, String> {
    if raw == "(main)" {
        return Ok(raw.to_string());
    }
    let s = worktrees_core::argv::safe_arg(raw, "slug")?;
    if s.contains(['/', '\\']) || s == "." || s.chars().any(char::is_control) {
        return Err(format!("not a place slug: {raw:?}"));
    }
    Ok(s)
}


/// What the resolver's tests and the dispatcher's share: a world of fixtures
/// (no registry file, no tmux server), a registry of fixture principals and
/// devices, and a `Call` as any caller.
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use worktrees_core::authz::{PrincipalId, Registry};

    #[derive(Default)]
    pub struct Fixture {
        pub roots: Vec<String>,
        /// `(session, pane cwd, pane command)`.
        pub panes: Vec<(String, String, String)>,
    }

    impl World for Fixture {
        fn roots(&self) -> Vec<String> {
            self.roots.clone()
        }
        fn panes(&self) -> Option<PaneList> {
            Some(PaneList::from_rows(self.panes.clone()))
        }
    }

    /// Principals and devices with grants, each device under one principal.
    #[derive(Default)]
    pub struct Devices {
        pub principals: HashMap<PrincipalId, Grant>,
        pub devices: HashMap<DeviceId, (PrincipalId, Grant)>,
    }

    impl Registry for Devices {
        fn principal_grant(&self, p: &PrincipalId) -> Option<Grant> {
            self.principals.get(p).cloned()
        }
        fn device_grant(&self, p: &PrincipalId, d: &DeviceId) -> Option<Grant> {
            self.devices.get(d).filter(|(owner, _)| owner == p).map(|(_, g)| g.clone())
        }
        fn device_principal(&self, d: &DeviceId) -> Option<PrincipalId> {
            self.devices.get(d).map(|(p, _)| p.clone())
        }
    }

    /// A fresh scratch directory, canonical (macOS's `/var` is `/private/var`).
    pub fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-resolve-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(&d).unwrap()
    }

    /// A git repository at `dir` (the session name rule `discover`s it).
    pub fn git_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        let ok = std::process::Command::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap().success();
        assert!(ok, "git init {}", dir.display());
    }

    pub fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    /// A `Call` from `caller`, over a scratch config dir, with a queue nobody
    /// drains.
    pub fn call_as(caller: Caller, registry: Arc<dyn Registry>) -> Call {
        struct Quiet;
        impl worktrees_api::Host for Quiet {
            fn emit(&self, _: &str, _: Value) {}
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // The receiver must outlive the call or every frame push fails; the
        // tests never read it, so it is leaked on purpose.
        std::mem::forget(rx);
        let ctx = worktrees_api::Ctx::new(std::env::temp_dir(), Arc::new(Quiet));
        Call { ctx, frames: super::super::wire::Frames::new(super::super::wire::Outbox(tx)), caller, registry, world: None }
    }

    pub fn args(v: Value) -> Args {
        Args(v.as_object().cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use worktrees_core::authz::{Caps, NoDevices, PrincipalId, Projects};

    use Capability::*;

    fn row(name: &str) -> &'static Row {
        super::super::dispatch::row(name).unwrap_or_else(|| panic!("no row {name}"))
    }

    /// Two registered projects, `a` and `b`, each a git repo with one place.
    struct Two {
        top: PathBuf,
        a: PathBuf,
        b: PathBuf,
    }

    fn two(tag: &str) -> Two {
        let top = scratch(tag);
        let (a, b) = (top.join("a"), top.join("b"));
        for r in [&a, &b] {
            git_repo(r);
            std::fs::create_dir_all(r.join(".worktrees/feat")).unwrap();
            std::fs::create_dir_all(r.join("src")).unwrap();
            std::fs::write(r.join("src/lib.rs"), "x").unwrap();
        }
        Two { top, a, b }
    }

    impl Two {
        fn world(&self) -> Fixture {
            Fixture { roots: vec![s(&self.a), s(&self.b)], panes: vec![] }
        }
    }

    impl Drop for Two {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.top);
        }
    }

    /// The owner's phone, scoped to `roots` with `caps`.
    fn phone(caps: &[Capability], roots: &[&Path]) -> (Call, Grant) {
        let mut reg = Devices::default();
        let g = Grant { capabilities: Caps::of(caps), projects: Projects::only(roots.iter().map(|r| s(r))) };
        reg.devices.insert(DeviceId::new("phone"), (PrincipalId::owner(), g));
        let caller = Caller::paired(PrincipalId::owner(), DeviceId::new("phone"));
        let reg: Arc<dyn worktrees_core::authz::Registry> = Arc::new(reg);
        let grant = authz::effective(&*reg, &caller);
        (call_as(caller, reg), grant)
    }

    fn console() -> (Call, Grant) {
        (call_as(Caller::console(), Arc::new(NoDevices)), Grant::everything())
    }

    fn try_row(name: &str, cx: &Call, g: &Grant, a: Value, w: &dyn World) -> Result<Args, Refusal> {
        let mut a = args(a);
        authorize(row(name), cx, g, &mut a, w).map(|_| a)
    }

    // ── project(…) ───────────────────────────────────────────────────────

    /// `repo` canonicalizes to EXACTLY a registered root, and the registry's
    /// key is what the body then receives — a trailing slash, a `..` detour
    /// or a symlink to the root all arrive as the key; a path INSIDE a root,
    /// which `guard_under_projects` would take, does not.
    #[test]
    fn a_project_is_exactly_a_registered_root_and_arrives_canonical() {
        let t = two("project");
        let (cx, g) = console();
        let w = t.world();
        let key = s(&t.a);
        for raw in [key.clone(), format!("{key}/"), format!("{key}/../a")] {
            let a = try_row("list_places", &cx, &g, json!({ "repo": raw }), &w).unwrap();
            assert_eq!(a.raw("repo"), json!(key), "{raw}");
        }
        std::os::unix::fs::symlink(&t.a, t.top.join("link")).unwrap();
        let a = try_row("list_places", &cx, &g, json!({ "repo": s(&t.top.join("link")) }), &w).unwrap();
        assert_eq!(a.raw("repo"), json!(key), "a link to a root is that root");
        for raw in [s(&t.a.join("src")), s(&t.top), String::new(), "/nonexistent/x".into()] {
            let e = try_row("list_places", &cx, &g, json!({ "repo": raw }), &w).unwrap_err();
            assert_eq!(e.kind, RefusalKind::NotFound, "{raw}");
        }
        // A registered project deleted from disk still matches its key, so it
        // can be removed from the nav.
        let w = Fixture { roots: vec![key.clone(), "/gone/project".into()], panes: vec![] };
        assert!(try_row("remove_project", &cx, &g, json!({ "root": "/gone/project" }), &w).is_ok());
    }

    /// §3.5, for EVERY caller, the console included: `set_note` wrote
    /// `.worktrees.places.json` into any directory it was given (gap 10).
    #[test]
    fn an_unregistered_repo_is_refused_even_to_the_console() {
        let t = two("unreg");
        let (cx, g) = console();
        let stranger = t.top.join("stranger");
        git_repo(&stranger);
        let e = try_row("set_note", &cx, &g, json!({ "repo": s(&stranger), "slug": "(main)", "note": "x" }), &t.world()).unwrap_err();
        assert_eq!(e.kind, RefusalKind::NotFound);
        assert_eq!(e.wire_for(&cx.caller), json!(format!("not a registered project: {}", s(&stranger))), "the console is told why");
        assert!(try_row("set_note", &cx, &g, json!({ "repo": s(&t.a), "slug": "(main)", "note": "x" }), &t.world()).is_ok());
    }

    /// gap 5: `create_initial_commit("")` ran git in serve's own cwd
    /// (`git -C ""` means HERE).
    #[test]
    fn an_empty_repo_is_no_project() {
        let t = two("empty");
        let (cx, g) = console();
        for raw in [json!(""), json!("   "), json!(null)] {
            assert!(try_row("create_initial_commit", &cx, &g, json!({ "repo": raw }), &t.world()).is_err(), "{raw}");
        }
    }

    // ── place(…) and the slug ───────────────────────────────────────────

    /// §3.5: a slug is ONE path component. `place_dir` is
    /// `format!("{wt_root}/{slug}")`, so `../x` left `.worktrees/`.
    #[test]
    fn a_slug_is_main_or_one_component() {
        let t = two("slug");
        let (cx, g) = console();
        let w = t.world();
        for ok in ["(main)", "feat", "feat-x.2"] {
            assert!(try_row("set_pin", &cx, &g, json!({ "repo": s(&t.a), "slug": ok, "on": true }), &w).is_ok(), "{ok}");
        }
        for bad in ["..", "../b", "a/b", "-x", ".", "", "x\ny", "..x"] {
            let e = try_row("set_pin", &cx, &g, json!({ "repo": s(&t.a), "slug": bad, "on": true }), &w).unwrap_err();
            assert_eq!(e.kind, RefusalKind::NotFound, "{bad:?}");
        }
        // The same rule in a dock shell's address and a nested one.
        assert!(try_row("shell_write", &cx, &g, json!({ "repo": s(&t.a), "slug": "../b", "index": 0, "data": [] }), &w).is_err());
        let e = try_row("resolve_term_paths", &cx, &g,
            json!({ "root": s(&t.a), "shell": { "repo": s(&t.a), "slug": "../b", "index": 0 }, "paths": [] }), &w).unwrap_err();
        assert_eq!(e.kind, RefusalKind::NotFound);
    }

    // ── file(…) and root(…) ──────────────────────────────────────────────

    /// A link inside an in-scope project that leads into an out-of-scope one
    /// is the file it leads to: refused, with the same words as a file that
    /// does not exist.
    #[test]
    fn a_symlink_out_of_the_scope_is_refused_like_a_missing_file() {
        let t = two("symlink");
        let (cx, g) = phone(&[Read], &[&t.a]);
        let w = t.world();
        std::os::unix::fs::symlink(t.b.join("src/lib.rs"), t.a.join("src/escape.rs")).unwrap();
        assert!(try_row("read_file", &cx, &g, json!({ "path": s(&t.a.join("src/lib.rs")) }), &w).is_ok());
        let out = try_row("read_file", &cx, &g, json!({ "path": s(&t.a.join("src/escape.rs")) }), &w).unwrap_err();
        let missing = try_row("read_file", &cx, &g, json!({ "path": s(&t.a.join("src/nope.rs")) }), &w).unwrap_err();
        let other = try_row("read_file", &cx, &g, json!({ "path": s(&t.b.join("src/lib.rs")) }), &w).unwrap_err();
        for e in [&out, &missing, &other] {
            assert_eq!(e.kind, RefusalKind::NotFound);
            assert_eq!(e.wire_for(&cx.caller), json!(ERR_NOT_FOUND), "one answer for missing and out of scope");
        }
        assert_ne!(out.detail, missing.detail, "the audit keeps the real reason");
    }

    #[test]
    fn a_root_is_a_place_root_and_nothing_else() {
        let t = two("root");
        let (cx, g) = console();
        let w = t.world();
        for ok in [t.a.clone(), t.a.join(".worktrees/feat")] {
            let a = try_row("changed_files", &cx, &g, json!({ "root": s(&ok) }), &w).unwrap();
            assert_eq!(a.raw("root"), json!(s(&ok)));
        }
        for bad in [t.a.join("src"), t.a.join(".worktrees"), t.top.clone()] {
            assert!(try_row("changed_files", &cx, &g, json!({ "root": s(&bad) }), &w).is_err(), "{}", bad.display());
        }
        // gap 5: resolve_doc_paths ran `git ls-files` in whatever `root` said.
        assert!(try_row("resolve_doc_paths", &cx, &g, json!({ "root": s(&t.top), "generation": 0, "paths": [] }), &w).is_err());
    }

    // ── session(…) ───────────────────────────────────────────────────────

    /// gap 3: a session is a PLACE's, in scope, by exact name. `api` and
    /// `api-fix` are different sessions however tmux would prefix-match them,
    /// and a session that is no place's — the user's own tmux — is refused.
    #[test]
    fn a_session_belongs_to_a_place_in_scope_by_its_exact_name() {
        let t = two("session");
        let (cx, g) = phone(&[Read, Stream], &[&t.b]);
        let w = Fixture {
            roots: vec![s(&t.a), s(&t.b)],
            panes: vec![
                ("api".into(), s(&t.a.join(".worktrees/feat")), "claude".into()),
                ("api-fix".into(), s(&t.b.join(".worktrees/feat")), "claude".into()),
                ("mine".into(), "/".into(), "zsh".into()),
            ],
        };
        let open = |name: &str| try_row("term_open", &cx, &g, json!({ "session": name, "cols": 80, "rows": 24 }), &w);
        assert!(open("api-fix").is_ok());
        for name in ["api", "api-", "mine", "nope", ""] {
            let e = open(name).unwrap_err();
            assert_eq!(e.wire(), json!(ERR_NOT_FOUND), "{name:?}");
        }
        // The console reaches both places' sessions, and still not `mine`.
        let (con, all) = console();
        let open = |name: &str| try_row("term_open", &con, &all, json!({ "session": name, "cols": 80, "rows": 24 }), &w);
        assert!(open("api").is_ok() && open("api-fix").is_ok());
        assert!(open("mine").is_err());
    }

    /// A canonical session whose panes have all `cd`'d away is still its
    /// place's, by name.
    #[test]
    fn a_session_is_found_by_its_canonical_name_when_the_panes_say_nothing() {
        let t = two("sessname");
        let (cx, g) = console();
        let project = worktrees_core::Project::discover(&t.a).unwrap();
        let name = project.session_name("feat");
        let w = Fixture { roots: vec![s(&t.a), s(&t.b)], panes: vec![(name.clone(), "/".into(), "zsh".into())] };
        assert!(try_row("plan_prompt", &cx, &g, json!({ "session": name }), &w).is_ok());
        assert!(try_row("plan_prompt", &cx, &g, json!({ "session": format!("{name}x") }), &w).is_err());
    }

    /// `drop_reference`'s `into_session` must be the RECEIVING place's.
    #[test]
    fn a_bound_session_must_be_its_places() {
        let t = two("bound");
        let (cx, g) = console();
        let w = Fixture {
            roots: vec![s(&t.a), s(&t.b)],
            panes: vec![
                ("into".into(), s(&t.b.join(".worktrees/feat")), "claude".into()),
                ("elsewhere".into(), s(&t.a), "claude".into()),
            ],
        };
        let drop = |sess: &str| {
            try_row("drop_reference", &cx, &g, json!({
                "repo": s(&t.a), "slug": "(main)", "into_repo": s(&t.b), "into_slug": "feat", "into_session": sess,
            }), &w)
        };
        assert!(drop("into").is_ok());
        assert!(drop("elsewhere").is_err(), "a session of another place");
        // (main)'s session is not one whose panes are in .worktrees/.
        let w2 = Fixture { roots: w.roots.clone(), panes: vec![("wt".into(), s(&t.a.join(".worktrees/feat")), "claude".into())] };
        assert!(try_row("close_place", &cx, &g, json!({ "repo": s(&t.a), "slug": "(main)", "yes": true, "session": "wt" }), &w2).is_err());
        assert!(try_row("close_place", &cx, &g, json!({ "repo": s(&t.a), "slug": "feat", "yes": true, "session": "wt" }), &w2).is_ok());
        assert!(try_row("close_place", &cx, &g, json!({ "repo": s(&t.a), "slug": "feat", "yes": true, "session": null }), &w2).is_ok());
    }

    // ── attach(…) and jobs ───────────────────────────────────────────────

    /// gap 16: an attach is its connection's. Another connection naming the
    /// same id gets what a missing id gets.
    #[test]
    fn an_attach_is_only_its_own_connections() {
        let (mine, g) = console();
        let (theirs, _) = console();
        mine.frames.note_term(7);
        let w = Fixture::default();
        assert!(try_row("term_write", &mine, &g, json!({ "id": 7, "data": [97] }), &w).is_ok());
        for cmd in ["term_write", "term_wheel", "term_resize"] {
            let e = try_row(cmd, &theirs, &g, json!({ "id": 7, "data": [97], "lines": 1, "cols": 1, "rows": 1 }), &w).unwrap_err();
            assert_eq!(e.kind, RefusalKind::NotFound, "{cmd}");
        }
        assert_eq!(try_row("term_close", &theirs, &g, json!({ "id": 7 }), &w).unwrap_err().kind, RefusalKind::Gone);
        mine.frames.forget_term(7);
        assert!(try_row("term_write", &mine, &g, json!({ "id": 7, "data": [97] }), &w).is_err(), "closed is gone");
    }

    /// (main)'s scenario for gap 16 (§9.1): a pane attached on serve A; serve
    /// restarts; on serve B another client's attach got the same id; the
    /// replaced pane closes its OLD id over its NEW socket (#525). The close
    /// must leave B's attach alone, and still answer ok, as a stale close
    /// always did — the page sends it with no `.catch`, and a rejection would
    /// be an error line in serve.log on every reconnect.
    #[test]
    fn a_stale_close_from_a_reconnected_pane_leaves_anothers_attach_alone() {
        let (other, g) = console();
        let (reconnected, _) = console();
        other.frames.note_term(5);
        let w = Fixture::default();
        assert_eq!(try_row("term_close", &reconnected, &g, json!({ "id": 5 }), &w).unwrap_err().kind, RefusalKind::Gone);
        assert_eq!(super::super::dispatch::call_in(&reconnected, "term_close", &args(json!({ "id": 5 })), &w), Ok(Value::Null));
        assert!(other.frames.owns_term(5), "B's attach is still B's: the body that closes it never ran");
        let e = try_row("term_write", &reconnected, &g, json!({ "id": 5, "data": [97] }), &w).unwrap_err();
        assert_eq!(e.kind, RefusalKind::NotFound, "and nothing else reaches it either");
    }

    /// A clone job's id is namespaced per connection on the way in, so a
    /// cancel from elsewhere names a job that does not exist.
    #[test]
    fn a_clone_job_is_only_its_own_connections() {
        let (mine, g) = console();
        let (theirs, _) = console();
        let w = Fixture::default();
        let started = try_row("clone_project", &mine, &g, json!({ "id": "j1", "url": "u", "parent": "/p" }), &w).unwrap();
        let cancel_mine = try_row("clone_cancel", &mine, &g, json!({ "id": "j1" }), &w).unwrap();
        let cancel_theirs = try_row("clone_cancel", &theirs, &g, json!({ "id": "j1" }), &w).unwrap();
        assert_eq!(started.raw("id"), cancel_mine.raw("id"));
        assert_ne!(started.raw("id"), cancel_theirs.raw("id"));
    }

    // ── exposure, capability, scope ──────────────────────────────────────

    /// Decision 6 and gap 7: a paired device holding EVERYTHING still cannot
    /// reach a console act, a loopback row or the docs viewer's URL.
    #[test]
    fn a_device_with_every_capability_is_still_not_the_console() {
        let t = two("console");
        let (cx, g) = phone(&Capability::ALL, &[&t.a]);
        let w = t.world();
        let e = |cmd: &str, a: Value| try_row(cmd, &cx, &g, a, &w).unwrap_err().wire();
        assert_eq!(e("update_cli", json!({ "tag": "v1" })), json!(ERR_CONSOLE_ONLY));
        assert_eq!(e("save_profile", json!({})), json!(ERR_CONSOLE_ONLY), "gap 2");
        assert_eq!(e("set_project_profile", json!({ "repo": s(&t.a), "id": "p" })), json!(ERR_CONSOLE_ONLY), "gap 2");
        assert_eq!(e("set_default_profile", json!({ "id": "p" })), json!(ERR_CONSOLE_ONLY), "gap 2");
        assert_eq!(e("sync_apply", json!({ "repo": null, "name": "x" })), json!(ERR_CONSOLE_ONLY), "gap 4: the adopt form");
        assert_eq!(e("ui_events_append", json!({ "events": [] })), json!(ERR_LOOPBACK_ONLY));
        assert_eq!(e("copy_text", json!({ "text": "x" })), json!(super::super::wire::CLIENT_LOCAL));
        let viewer = json!({ "repo": s(&t.a), "root": s(&t.a), "slug": "(main)", "path": null, "place": {} });
        assert_eq!(e("open_docs_viewer", viewer), json!(ERR_UNAVAILABLE_REMOTE), "gap 7");
        // The console, on the same rows, passes the resolver.
        let (con, all) = console();
        for (cmd, a) in [("update_cli", json!({})), ("ui_events_append", json!({})), ("sync_apply", json!({ "repo": null }))] {
            assert!(try_row(cmd, &con, &all, a, &w).is_ok(), "{cmd}");
        }
    }

    /// The capability is the row's, and `denied` names nothing.
    #[test]
    fn an_observe_device_reads_and_does_nothing_else() {
        let t = two("observe");
        let (cx, g) = phone(&[Read], &[&t.a]);
        let w = t.world();
        assert!(try_row("list_places", &cx, &g, json!({ "repo": s(&t.a) }), &w).is_ok());
        for (cmd, a) in [
            ("set_note", json!({ "repo": s(&t.a), "slug": "(main)", "note": "x" })),
            ("shell_open", json!({ "repo": s(&t.a), "slug": "(main)", "index": 0 })),
            ("init_write", json!({ "repo": s(&t.a) })),
            ("probe_dir", json!({ "dir": "/" })),
            ("diagnostics", json!({})),
        ] {
            let e = try_row(cmd, &cx, &g, a, &w).unwrap_err();
            assert_eq!(e.wire(), json!(ERR_DENIED), "{cmd}");
        }
        // gap 14: the arbitrary-path reads need `admin` AND scope "*".
        let (adm, ag) = phone(&[Read, Admin], &[&t.a]);
        assert_eq!(try_row("probe_dir", &adm, &ag, json!({ "dir": "/" }), &w).unwrap_err().wire(), json!(ERR_DENIED));
        assert_eq!(try_row("log_tail", &adm, &ag, json!({}), &w).unwrap_err().wire(), json!(ERR_DENIED));
        assert!(try_row("diagnostics", &adm, &ag, json!({}), &w).is_ok(), "a global admin row needs no scope");
    }

    /// §2.8's second-principal fixtures, through the resolver: a device grant
    /// wider than its principal's acts as the principal's, and a device of
    /// another principal is not an object, exactly as a missing one is not.
    #[test]
    fn a_second_principals_device_is_held_to_the_principal() {
        let t = two("principal");
        let bob = PrincipalId::new("bob");
        let mut reg = Devices::default();
        reg.principals.insert(bob.clone(), Grant { capabilities: Caps::of(&[Read]), projects: Projects::only([s(&t.a)]) });
        reg.devices.insert(DeviceId::new("bob-phone"), (bob.clone(), Grant::everything()));
        reg.devices.insert(DeviceId::new("my-phone"), (PrincipalId::owner(), Grant::everything()));
        let reg: Arc<dyn worktrees_core::authz::Registry> = Arc::new(reg);
        let caller = Caller::paired(bob.clone(), DeviceId::new("bob-phone"));
        let g = authz::effective(&*reg, &caller);
        let cx = call_as(caller, reg.clone());
        let w = t.world();
        assert!(try_row("list_places", &cx, &g, json!({ "repo": s(&t.a) }), &w).is_ok());
        assert_eq!(try_row("set_note", &cx, &g, json!({ "repo": s(&t.a), "slug": "(main)", "note": "x" }), &w).unwrap_err().wire(), json!(ERR_DENIED));
        assert_eq!(try_row("list_places", &cx, &g, json!({ "repo": s(&t.b) }), &w).unwrap_err().wire(), json!(ERR_NOT_FOUND));
        // `device(id)`: no row names a device until slice 5, so the kind is
        // driven through a row of its own.
        static DEVICE_ROW: Row = Row { name: "device_fixture", exposure: Exposure::Engine, cap: Some(Read), objects: &[Obj::Device("id")] };
        let dev = |cx: &Call, id: &str| authorize(&DEVICE_ROW, cx, &Grant::everything(), &mut args(json!({ "id": id })), &w);
        let me = call_as(Caller::paired(PrincipalId::owner(), DeviceId::new("my-phone")), reg.clone());
        assert!(dev(&me, "my-phone").is_ok());
        let theirs = dev(&me, "bob-phone").unwrap_err();
        let missing = dev(&me, "nobody").unwrap_err();
        assert_eq!(theirs.wire(), missing.wire());
        assert_eq!(theirs.kind, RefusalKind::NotFound);
        assert!(dev(&cx, "my-phone").is_err(), "and the other way round");
    }

    /// A row that needs scope `"*"` needs it on BOTH sides of the pair.
    #[test]
    fn scope_star_means_the_effective_scope() {
        let t = two("star");
        let bob = PrincipalId::new("bob");
        let mut reg = Devices::default();
        reg.principals.insert(bob.clone(), Grant { capabilities: Caps::ALL, projects: Projects::only([s(&t.a)]) });
        reg.devices.insert(DeviceId::new("d"), (bob.clone(), Grant::everything()));
        let reg: Arc<dyn worktrees_core::authz::Registry> = Arc::new(reg);
        let caller = Caller::paired(bob, DeviceId::new("d"));
        let g = authz::effective(&*reg, &caller);
        let cx = call_as(caller, reg);
        let e = try_row("reorder_projects", &cx, &g, json!({ "roots": [s(&t.a)] }), &t.world()).unwrap_err();
        assert_eq!(e.wire(), json!(ERR_DENIED));
    }

    #[test]
    fn nested_and_listed_objects_are_resolved_element_by_element() {
        let t = two("nested");
        let (cx, g) = phone(&[Read, Write], &[&t.a]);
        let w = t.world();
        let prs = |places: Value| try_row("project_prs", &cx, &g, json!({ "repo": s(&t.a), "max_age_secs": 0, "places": places }), &w);
        assert!(prs(json!([{ "slug": "(main)", "branch": "main" }, { "slug": "feat", "branch": "f" }])).is_ok());
        assert!(prs(json!([{ "slug": "feat", "branch": "f" }, { "slug": "../b", "branch": "x" }])).is_err(), "one bad element refuses the whole request");
        let (con, all) = console();
        let a = try_row("reorder_projects", &con, &all, json!({ "roots": [format!("{}/", s(&t.b)), s(&t.a)] }), &w).unwrap();
        assert_eq!(a.raw("roots"), json!([s(&t.b), s(&t.a)]), "each root arrives as its registry key");
        assert!(try_row("reorder_projects", &con, &all, json!({ "roots": [s(&t.a), s(&t.top)] }), &w).is_err());
    }
}

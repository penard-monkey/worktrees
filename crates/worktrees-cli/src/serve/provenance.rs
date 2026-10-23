//! What serve records about who did what, and the rules about agents and
//! devices (Server Edition Phase 2, `docs/proposals/server-edition-phase2.md`
//! §7.3, §7.5 and §6's LLM10 row).
//!
//! **Marked out of band (§7.3).** Nothing is written INTO a file: a header in
//! `AGENTS.md` would be stripped or forged, and an agent would read it as one
//! more instruction. The place's declared state carries the facts instead,
//! through the store's own writer:
//!
//! - `last_remote_instruction` — the resolver finds an instruction-class write
//!   from a device ([`Instruction`]), and [`stamped`] stamps it once the body
//!   has written it;
//! - `note_by` / `title_by` — the api's setters, from the `Ctx` a body runs on
//!   ([`acting`]);
//! - `launched_by` — core's launch, before the session exists, from the same
//!   `Ctx`.
//!
//! **A scoped device cannot widen a lane (§7.5, O11).** [`acting`] narrows a
//! launch from a device whose effective scope is a list to cross-project reach
//! `off`; the lane's MCP server reads that from `launched_by` when it starts.
//!
//! **At most four live device-launched lanes (§6, LLM10, O12).**
//! [`device_lane_gate`]. The hourly launch bucket is slice 4's (`budget.rs`,
//! #538), keyed on the same rows: it should read [`LAUNCH_ROWS`] when it lands.
//!
//! **An agent never reaches a Phase 2 control (§7.5, O10).** The test at the
//! bottom of this file reads `mcp.rs`.

use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde_json::Value;
use worktrees_core::authz::{Caller, Grant};
use worktrees_core::reach::Level;
use worktrees_core::store::{self, By, LaunchedBy, RemoteInstruction};

use super::dispatch::{Args, Call, Row};
use super::resolve::{Refusal, RefusalKind, World};

/// Who a request's body acts for. A device whose effective scope is a list
/// (its own grant intersected with its principal's, §2.8) launches lanes with
/// cross-project reach `off`: an agent it starts may not reach, through MCP,
/// projects the device itself cannot. A `"*"` device and the console leave
/// the lane's reach to the user's setting. Narrow-only: the MCP server takes
/// the lowest of this, the profile's and the user's.
pub fn acting(caller: &Caller, grant: &Grant) -> LaunchedBy {
    let scoped = !caller.is_console() && !grant.projects.is_all();
    LaunchedBy { by: By::of(caller), cross_project: scoped.then(|| Level::Off.as_str().to_string()) }
}

// ── the instruction stamp ────────────────────────────────────────────────────

/// An instruction-class write the resolver let a device make: the project's
/// registry key, the place it lands in, and the path below that place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub project: String,
    pub slug: String,
    pub path: String,
}

/// The place a project-relative CANONICAL path lies in, and the path below it:
/// `.worktrees/<slug>/<rest>` is `<slug>`'s, anything else is `(main)`'s.
/// `None` for a path that is not a file inside a place (`.worktrees` itself,
/// a place's own directory).
pub fn place_of(rel: &Path) -> Option<(String, String)> {
    let parts: Vec<&str> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [] => None,
        [".worktrees", ..] if parts.len() < 3 => None,
        [".worktrees", slug, rest @ ..] => Some((slug.to_string(), rest.join("/"))),
        _ => Some(("(main)".to_string(), parts.join("/"))),
    }
}

/// A body's result, with the instruction write it made stamped — only when it
/// returned `Ok`: a write the body refused (a stale `expected_mtime`) edited
/// nothing, and the stamp says "edited".
pub fn stamped(caller: &Caller, i: Option<&Instruction>, out: Result<Value, Value>) -> Result<Value, Value> {
    if let (Ok(_), Some(i)) = (&out, i) {
        record_instruction(caller, i);
    }
    out
}

/// Stamp `last_remote_instruction` on the place `i` names. A failed write is
/// logged, never the request's failure: the file IS written by now, and an
/// error would tell the device it was not.
pub fn record_instruction(caller: &Caller, i: &Instruction) {
    let stamp = RemoteInstruction { by: By::of(caller), path: i.path.clone(), at: worktrees_core::sysclock::now_epoch() };
    if let Err(e) = store::edit(&i.project, &i.slug, |d| d.last_remote_instruction = Some(stamp)) {
        worktrees_api::log::log("warn", &format!("serve: could not stamp {} in {}:{} for {caller}: {e}", i.path, i.project, i.slug));
    }
}

// ── the device-lane cap ──────────────────────────────────────────────────────

/// The rows that start an agent (§6, LLM10): a new place launches its
/// provider, `open_place` launches or resumes one, `rules_review` starts a
/// review lane. A row that starts an agent and is missing here escapes the
/// cap; the test below holds each name to the table.
pub const LAUNCH_ROWS: &[&str] = &["new_place", "open_place", "rules_review"];

/// Lanes launched from paired devices that may be live at once, host-wide.
pub const MAX_DEVICE_LANES: usize = 4;

/// What a device is told. It names no place: the live ones may be in projects
/// outside its scope. The refusal's detail has the list, for the host's log
/// and the audit record.
pub const ERR_DEVICE_LANES: &str =
    "device-lanes: 4 agents launched from paired devices are already running on this host (the limit) — close one before launching another";

/// Held from a device launch's count until its body returns, so the next one
/// counts the session this one made. Device launches are all serve's — one
/// engine per host — so a process lock serializes every one of them.
static DEVICE_LAUNCH: Mutex<()> = Mutex::new(());

/// Refuse a device's agent launch while [`MAX_DEVICE_LANES`] device-launched
/// lanes are live; otherwise hand back the lock for the caller to hold across
/// the body. `Ok(None)` for the console and for every other row. The refusal
/// is the host's ([`RefusalKind::DeviceLanes`]), so the dispatcher logs and
/// audits it as it does the resolver's.
///
/// `open_place`'s own target is not counted: reopening a place reuses its
/// lane, or replaces it with another agent in the same place, and never adds
/// one. `new_place` and `rules_review` cannot name theirs before the body
/// runs, and a reuse there is counted — the cap errs closed.
pub fn device_lane_gate(cx: &Call, row: &Row, args: &Args, world: &dyn World) -> Result<Option<MutexGuard<'static, ()>>, Refusal> {
    if cx.caller.is_console() || !LAUNCH_ROWS.contains(&row.name) {
        return Ok(None);
    }
    let held = DEVICE_LAUNCH.lock().unwrap_or_else(|e| e.into_inner());
    let target = (row.name == "open_place").then(|| (args.raw("repo"), args.raw("slug")));
    let target = target.as_ref().and_then(|(r, s)| Some((r.as_str()?, s.as_str()?)));
    let live = live_device_lanes(world, target);
    if live.len() >= MAX_DEVICE_LANES {
        return Err(Refusal { kind: RefusalKind::DeviceLanes, detail: format!("device lanes live: {}", live.join(", ")) });
    }
    Ok(Some(held))
}

/// Every place, in every registered project, whose agent was launched from a
/// paired device and whose session is up, as `<root>:<slug>`. Live is the
/// tmux reader's word (`PaneList::sessions_in`): a lane counts from
/// `new-session` until its session is closed, and a dock shell is not a lane.
/// No tmux: nothing can be live.
pub fn live_device_lanes(world: &dyn World, skip: Option<(&str, &str)>) -> Vec<String> {
    let Some(panes) = world.panes() else { return Vec::new() };
    let mut out = Vec::new();
    for root in world.roots() {
        let base = std::fs::canonicalize(&root).unwrap_or_else(|_| PathBuf::from(&root));
        for (slug, d) in store::read_lenient(&root).places {
            if !d.launched_by.as_ref().is_some_and(|l| l.by.is_device()) || skip == Some((root.as_str(), slug.as_str())) {
                continue;
            }
            let (dir, exclude) = match slug.as_str() {
                "(main)" => (base.clone(), Some(base.join(".worktrees"))),
                // A key a hand edit left that names no single directory.
                s if s.is_empty() || s.contains('/') || s == "." || s == ".." => continue,
                s => (base.join(".worktrees").join(s), None),
            };
            let exclude = exclude.as_ref().map(|e| e.to_string_lossy().into_owned());
            if !panes.sessions_in(&dir.to_string_lossy(), exclude.as_deref()).is_empty() {
                out.push(format!("{root}:{slug}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    use serde_json::json;
    use worktrees_core::authz::{Capability, Caps, DeviceId, Device, PrincipalId, Projects};

    use super::super::dispatch::{call_in, TABLE};
    use super::super::resolve::fixture::{call_as, git_repo, s, scratch, Devices, Fixture};

    fn args(v: Value) -> Args {
        Args(v.as_object().cloned().unwrap_or_default())
    }

    fn grant(caps: &[Capability], projects: Projects) -> Grant {
        Grant { capabilities: Caps::of(caps), projects }
    }

    /// Two principals' devices: the owner's phone scoped to `scope`, and bob's
    /// laptop with `"*"` under a bob whose own grant is `"*"`.
    fn devices(scope: Projects) -> Arc<Devices> {
        let mut reg = Devices::default();
        let all = [Capability::Read, Capability::Write, Capability::Instruct];
        reg.devices.insert(DeviceId::new("phone"), (PrincipalId::owner(), grant(&all, scope)));
        reg.principals.insert(PrincipalId::new("bob"), grant(&all, Projects::All));
        reg.devices.insert(DeviceId::new("bob-laptop"), (PrincipalId::new("bob"), grant(&all, Projects::All)));
        Arc::new(reg)
    }

    fn phone() -> Caller {
        Caller::paired(PrincipalId::owner(), DeviceId::new("phone"))
    }

    fn bob() -> Caller {
        Caller::paired(PrincipalId::new("bob"), DeviceId::new("bob-laptop"))
    }

    /// §7.5 / O11: a scoped device's launches narrow reach to `off`; a `"*"`
    /// device's and the console's leave it alone. A principal's list makes a
    /// `"*"` device scoped, because the effective scope is the intersection.
    #[test]
    fn only_a_scoped_devices_launch_narrows_reach() {
        let all = grant(&[Capability::Write], Projects::All);
        let listed = grant(&[Capability::Write], Projects::only(["/r/a"]));
        assert_eq!(acting(&Caller::console(), &all).cross_project, None);
        assert_eq!(acting(&phone(), &all).cross_project, None);
        let l = acting(&phone(), &listed);
        assert_eq!(l.cross_project.as_deref(), Some("off"));
        assert_eq!(l.cap(), Some(Level::Off));
        assert_eq!(l.by.device, Device::Paired(DeviceId::new("phone")));
        let bob_listed = acting(&bob(), &grant(&[Capability::Write], Projects::only(["/r/a"])));
        assert_eq!(bob_listed.by.principal, PrincipalId::new("bob"), "the second principal is recorded as itself");
        assert_eq!(bob_listed.cross_project.as_deref(), Some("off"));
    }

    #[test]
    fn a_path_lands_in_the_place_that_holds_it() {
        let p = |s: &str| place_of(Path::new(s));
        assert_eq!(p("AGENTS.md"), Some(("(main)".into(), "AGENTS.md".into())));
        assert_eq!(p("apps/web/CLAUDE.md"), Some(("(main)".into(), "apps/web/CLAUDE.md".into())));
        assert_eq!(p(".worktrees/feat/.planning/brief.md"), Some(("feat".into(), ".planning/brief.md".into())));
        assert_eq!(p(".worktrees/feat"), None);
        assert_eq!(p(".worktrees"), None);
        assert_eq!(p(""), None);
    }

    /// A project with one place, registered in the fixture world: the device
    /// writes through `write_file` exactly as the app's Files tab does.
    fn project(tag: &str) -> (PathBuf, Fixture) {
        let root = scratch(tag);
        git_repo(&root);
        std::fs::create_dir_all(root.join(".worktrees/feat/.planning")).unwrap();
        for f in ["AGENTS.md", "notes.md", ".worktrees/feat/.planning/brief.md", ".worktrees/feat/CLAUDE.md"] {
            std::fs::write(root.join(f), "before\n").unwrap();
        }
        let world = Fixture { roots: vec![s(&root)], panes: vec![] };
        (root, world)
    }

    /// What the resolver finds for a `write_file` of `path` by `caller`: the
    /// row is authorized against the fixture world exactly as `call_in` does.
    /// (The body is not run: its own guard reads the user's registry, which no
    /// test may touch. `stamped` is the step after it, tested on its own.)
    fn found(caller: Caller, reg: Arc<Devices>, world: &Fixture, path: &Path) -> Option<Instruction> {
        let cx = call_as(caller, reg);
        let grant = worktrees_core::authz::effective(&*cx.registry, &cx.caller);
        let row = super::super::dispatch::row("write_file").unwrap();
        let mut a = args(json!({"path": s(path), "content": "after\n"}));
        super::super::resolve::authorize(row, &cx, &grant, &mut a, world).expect("the write is allowed").instruction
    }

    fn stamp(root: &Path, slug: &str) -> Option<RemoteInstruction> {
        store::read_lenient(&s(root)).places.get(slug).and_then(|d| d.last_remote_instruction.clone())
    }

    /// §7.3: an instruction-class write from a device is found by the
    /// resolver as the place it lands in and the path below that place, and
    /// stamped `{principal, device, path, at}` there; a second principal's
    /// device is named under its own principal.
    #[test]
    fn a_devices_instruction_write_stamps_the_place() {
        let (root, world) = project("prov-stamp");
        let reg = devices(Projects::only([s(&root)]));
        let i = found(phone(), reg.clone(), &world, &root.join("AGENTS.md")).expect("an instruction write");
        assert_eq!(i, Instruction { project: s(&root), slug: "(main)".into(), path: "AGENTS.md".into() });
        stamped(&phone(), Some(&i), Ok(Value::Null)).unwrap();
        let st = stamp(&root, "(main)").expect("stamped");
        assert_eq!(st.by, By::of(&phone()));
        assert_eq!(st.path, "AGENTS.md");
        assert!((worktrees_core::sysclock::now_epoch() - st.at).abs() < 60, "{st:?}");

        let i = found(bob(), reg, &world, &root.join(".worktrees/feat/.planning/brief.md")).unwrap();
        assert_eq!((i.slug.as_str(), i.path.as_str()), ("feat", ".planning/brief.md"), "relative to the place, not the project");
        stamped(&bob(), Some(&i), Ok(Value::Null)).unwrap();
        let st = stamp(&root, "feat").expect("the place's own stamp");
        assert_eq!(st.by.principal, PrincipalId::new("bob"));
        assert_eq!(st.by.device, Device::Paired(DeviceId::new("bob-laptop")));
        assert_eq!(stamp(&root, "(main)").unwrap().path, "AGENTS.md", "another place's stamp is untouched");
    }

    /// The other side of the rule: the console's instruction writes and a
    /// device's PLAIN writes are not found, and a write the body refused is
    /// not stamped. "Edited from" must be true.
    #[test]
    fn no_stamp_without_a_devices_instruction_write() {
        let (root, world) = project("prov-nostamp");
        let reg = devices(Projects::All);
        assert_eq!(found(Caller::console(), reg.clone(), &world, &root.join("AGENTS.md")), None, "the console");
        assert_eq!(found(phone(), reg.clone(), &world, &root.join("notes.md")), None, "a plain file");
        let i = found(phone(), reg, &world, &root.join(".worktrees/feat/CLAUDE.md")).expect("CLAUDE.md in a place");
        let refused = Err(Value::from("file changed on disk since you opened it — reload to see the latest"));
        assert_eq!(stamped(&phone(), Some(&i), refused.clone()), refused, "the body's answer is passed through");
        assert_eq!(stamp(&root, "feat"), None, "a write the body refused edited nothing");
    }

    /// A link is classed as what it writes (§7.1): `notes-link.md ->
    /// AGENTS.md` written from a device is found as `AGENTS.md`. And a hook
    /// file (`executes`, which steers an agent harder than an instruction
    /// file) is found too.
    #[test]
    fn a_link_is_found_as_what_it_writes_and_a_hook_file_counts() {
        let (root, world) = project("prov-link");
        std::os::unix::fs::symlink(root.join("AGENTS.md"), root.join("notes-link.md")).unwrap();
        let reg = devices(Projects::All);
        assert_eq!(found(phone(), reg.clone(), &world, &root.join("notes-link.md")).map(|i| i.path).as_deref(), Some("AGENTS.md"));
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(root.join(".claude/settings.json"), "{}").unwrap();
        // A hook file is `executes`, which needs `stream` (2b): a device
        // without it is refused before any stamp, and one holding it is found.
        let hook = root.join(".claude/settings.json");
        let cx = call_as(phone(), reg.clone());
        let eff = worktrees_core::authz::effective(&*cx.registry, &cx.caller);
        let row = super::super::dispatch::row("write_file").unwrap();
        let mut a = args(json!({"path": s(&hook), "content": "after\n"}));
        assert!(super::super::resolve::authorize(row, &cx, &eff, &mut a, &world).is_err(), "a hook write needs `stream`");
        let mut with_stream = Devices::default();
        let caps = [Capability::Read, Capability::Write, Capability::Stream];
        with_stream.devices.insert(DeviceId::new("phone"), (PrincipalId::owner(), grant(&caps, Projects::All)));
        assert_eq!(found(phone(), Arc::new(with_stream), &world, &hook).map(|i| i.path).as_deref(), Some(".claude/settings.json"));
    }

    /// §7.3's notes and titles: serve's setters record `{principal, device}`,
    /// clearing the text clears the author, and the console is recorded as
    /// itself.
    #[test]
    fn notes_and_titles_record_who_set_them() {
        let (root, world) = project("prov-note");
        let reg = devices(Projects::only([s(&root)]));
        let set = |c: Caller, cmd: &str, key: &str, v: &str| {
            call_in(&call_as(c, reg.clone()), cmd, &args(json!({"repo": s(&root), "slug": "feat", key: v})), &world).unwrap();
        };
        let decl = || store::read_lenient(&s(&root)).places["feat"].clone();
        set(bob(), "set_note", "note", "from bob");
        set(phone(), "set_title", "title", "Auth");
        assert_eq!(decl().note_by, Some(By::of(&bob())));
        assert_eq!(decl().title_by, Some(By::of(&phone())));
        set(Caller::console(), "set_note", "note", "mine");
        assert_eq!(decl().note_by, Some(By::of(&Caller::console())));
        set(phone(), "set_title", "title", "  ");
        assert_eq!((decl().title, decl().title_by), (None, None), "a cleared title keeps no author");
    }

    /// The `Ctx` a body runs on acts for the caller, narrowed for a scoped
    /// device — what core's launch writes as `launched_by` (§7.5).
    #[test]
    fn a_body_acts_for_its_caller() {
        let g = grant(&[Capability::Write], Projects::only(["/r/a"]));
        let ctx = call_as(phone(), devices(Projects::All)).ctx.acting_for(acting(&phone(), &g));
        assert_eq!(ctx.launched_by().and_then(|l| l.cross_project.as_deref()), Some("off"));
        let again = ctx.acting_for(acting(&Caller::console(), &Grant::everything()));
        assert_eq!(again.by(), Some(By::of(&phone())), "a Ctx that acts for a device is never re-labelled");
    }

    // ── the device-lane cap ──────────────────────────────────────────────────

    /// `n` places of `root` stamped as launched by `who`, each with a session
    /// up in its directory.
    fn lanes(root: &Path, world: &mut Fixture, who: &Caller, slugs: &[&str]) {
        for slug in slugs {
            let dir = root.join(".worktrees").join(slug);
            std::fs::create_dir_all(&dir).unwrap();
            let by = LaunchedBy { by: By::of(who), cross_project: None };
            store::edit(&s(root), slug, |d| d.launched_by = Some(by)).unwrap();
            world.panes.push((format!("p-{slug}"), s(&dir), "claude".into()));
        }
    }

    /// No such harness: a launch body refuses it before anything else runs.
    const NO_HARNESS: &str = "no-such-harness";

    fn refused_for_lanes(r: &Result<Value, Value>) -> bool {
        r.as_ref().err().and_then(Value::as_str) == Some(ERR_DEVICE_LANES)
    }

    /// The gate alone, for the cases it LETS THROUGH: through `call_in` the
    /// launch body would run next, and a launch body reaches tmux.
    fn gate(c: Caller, reg: Arc<Devices>, world: &Fixture, cmd: &str, a: Value) -> Result<bool, Value> {
        let cx = call_as(c, reg);
        let row = super::super::dispatch::row(cmd).unwrap();
        device_lane_gate(&cx, row, &args(a), world).map(|held| held.is_some()).map_err(|r| r.wire())
    }

    /// §6, LLM10, O12: with four device-launched lanes live, host-wide and
    /// across principals, a device's launch is refused — through `call_in`,
    /// before any body — for every launch row; the console's is not. Lanes
    /// the console launched, and device lanes whose session is closed, are
    /// not counted.
    #[test]
    fn a_fifth_device_lane_is_refused() {
        let (root, mut world) = project("prov-cap");
        let other = scratch("prov-cap-other");
        git_repo(&other);
        world.roots.push(s(&other));
        let reg = devices(Projects::All);
        lanes(&root, &mut world, &phone(), &["a", "b"]);
        lanes(&other, &mut world, &bob(), &["c"]);
        lanes(&root, &mut world, &Caller::console(), &["mine1", "mine2"]);
        // Stamped by a device, but nothing is running there.
        std::fs::create_dir_all(root.join(".worktrees/gone")).unwrap();
        let by = LaunchedBy { by: By::of(&phone()), cross_project: None };
        store::edit(&s(&root), "gone", |d| d.launched_by = Some(by)).unwrap();

        assert_eq!(live_device_lanes(&world, None).len(), 3);
        let new = json!({"repo": s(&root), "branch": "x"});
        assert_eq!(gate(phone(), reg.clone(), &world, "new_place", new.clone()), Ok(true), "three live: the fourth may start, holding the lock");

        lanes(&other, &mut world, &bob(), &["d"]);
        assert_eq!(live_device_lanes(&world, None).len(), 4);
        // A provider that does not exist: should the gate ever let one of
        // these through, its body fails before it can reach git or tmux.
        let launch = json!({"repo": s(&root), "slug": "feat", "branch": "x", "provider": NO_HARNESS});
        for cmd in LAUNCH_ROWS {
            let r = call_in(&call_as(phone(), reg.clone()), cmd, &args(launch.clone()), &world);
            assert!(refused_for_lanes(&r), "{cmd}: {r:?}");
        }
        let r = call_in(&call_as(bob(), reg.clone()), "new_place", &args(launch), &world);
        assert!(refused_for_lanes(&r), "the count is host-wide, not per principal: {r:?}");
        assert!(!r.unwrap_err().as_str().unwrap().contains(&s(&other)), "the refusal names no place");
        assert_eq!(gate(Caller::console(), reg.clone(), &world, "new_place", new), Ok(false), "the console never meets it");
        assert_eq!(gate(phone(), reg, &world, "set_note", json!({})), Ok(false), "nor does a row that starts no agent");
    }

    /// Reopening one of the four live lanes adds none, so it is not refused.
    #[test]
    fn reopening_a_live_device_lane_is_not_a_fifth() {
        let (root, mut world) = project("prov-reopen");
        lanes(&root, &mut world, &phone(), &["a", "b", "c", "d"]);
        let reg = devices(Projects::All);
        let open = |slug: &str| json!({"repo": s(&root), "slug": slug});
        assert_eq!(gate(phone(), reg.clone(), &world, "open_place", open("a")), Ok(true), "its own lane");
        let mut feat = open("feat");
        feat["provider"] = json!(NO_HARNESS);
        let r = call_in(&call_as(phone(), reg), "open_place", &args(feat), &world);
        assert!(refused_for_lanes(&r), "a new one: {r:?}");
    }

    #[test]
    fn every_launch_row_is_in_the_table() {
        let names: HashSet<&str> = TABLE.iter().map(|r| r.name).collect();
        for r in LAUNCH_ROWS {
            assert!(names.contains(r), "{r} is not a row");
        }
    }

    // ── an agent never reaches a Phase 2 control (§7.5, O10) ─────────────────

    /// The MCP server's source, as built: everything above its `mod tests`.
    const MCP_RS: &str = include_str!("../mcp.rs");

    /// The live half of a Rust file: cut at the `#[cfg(test)]` that heads
    /// `mod tests`, not at the first `#[cfg(test)]` (a gated item above the
    /// test module would move a first-match cut, AGENTS.md).
    fn live(src: &str) -> &str {
        let mut at = 0;
        while let Some(i) = src[at..].find("#[cfg(test)]") {
            let start = at + i;
            let rest = src[start + "#[cfg(test)]".len()..].trim_start();
            if rest.starts_with("mod tests") {
                return &src[..start];
            }
            at = start + 1;
        }
        src
    }

    /// The families a Phase 2 control row is named in: pairing, devices, the
    /// server identity, step-up, scopes, the remote listener and the audit log.
    fn is_control(word: &str) -> bool {
        const PREFIXES: &[&str] = &["pair_", "pairing_", "device_", "devices_", "identity_", "stepup_", "audit_", "remote_access", "listener_"];
        PREFIXES.iter().any(|p| word.starts_with(p)) || word.ends_with("_scope") || word.contains("_scope_")
    }

    /// Every word in `src` that names a Phase 2 control: a table row in a
    /// control family, a control-shaped identifier or string (a tool named
    /// `device_list` is one before any row exists), a path into serve's
    /// dispatcher, or the device and audit files an agent could write.
    fn control_names(src: &str, rows: &[&str]) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for w in src.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty()) {
            if (rows.contains(&w) || is_control(w)) && !found.iter().any(|f| f == w) {
                found.push(w.to_string());
            }
        }
        for needle in ["serve::", "devices.json", "audit.jsonl"] {
            if src.contains(needle) {
                found.push(needle.to_string());
            }
        }
        found
    }

    fn control_rows() -> Vec<&'static str> {
        TABLE.iter().map(|r| r.name).filter(|n| is_control(n)).collect()
    }

    /// §7.5: pairing, devices, scopes, remote access and the audit log are
    /// dispatch rows, never MCP tools — an agent that could pair a device is
    /// one prompt injection away from a remote foothold (ASI02, ASI03).
    #[test]
    fn the_mcp_server_names_no_phase_2_control() {
        let src = live(MCP_RS);
        assert!(src.contains("fn cmd_mcp") && src.contains("\"place_status\""), "the slice holds the server");
        assert!(!src.contains("fn place_status_reports_who_launched_and_who_wrote"), "the slice stops at mod tests");
        assert_eq!(control_names(src, &control_rows()), Vec::<String>::new());
    }

    /// The check is shown red on planted names — one per family, and the
    /// rows slice 5 adds — so a pass means something.
    #[test]
    fn the_control_check_fails_on_a_planted_name() {
        let rows = ["device_narrow", "pair_begin"];
        let clean = live(MCP_RS);
        for plant in [
            "\"pair_begin\" => {}", "\"devices_list\"", "\"device_set_scope\"", "fn audit_tail()", "\"remote_access_set\"",
            "\"identity_rotate\"", "crate::serve::dispatch::call", "std::fs::write(\"devices.json\", b)", "\"listener_set\"",
        ] {
            let planted = format!("{clean}\n{plant}\n");
            assert!(!control_names(&planted, &rows).is_empty(), "{plant} slipped past");
        }
        // The words this server legitimately uses stay clean.
        assert!(control_names("user-scope server, remote_url, a device, scoped, pairs", &rows).is_empty());
    }
}

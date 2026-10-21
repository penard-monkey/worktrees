// The command layer must stay callable without a window.
//
//   node app/scripts/api-boundary-check.mjs      # exits non-zero on failure
//   API_BOUNDARY_LIB=<copy of lib.rs> node …      # the same rules against a mutated copy
//
// `worktrees-api` holds the bodies of the Tauri commands so that
// `worktrees serve` can call the same ones over a socket. The single thing
// that would undo that is a `tauri` dependency creeping back in — directly,
// or through something that pulls it. A comment in Cargo.toml says so; this
// is the part that cannot be edited away in passing, which is the same reason
// viewer-boundary-check.mjs exists for mermaid.
//
// Resolved from Cargo.lock, not from Cargo.toml: the toml says what we asked
// for and the lock says what we got, and a transitive edge is exactly the one
// a person would not notice.
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const LOCK = fileURLToPath(new URL("../../Cargo.lock", import.meta.url));
const ROOT = "worktrees-api";

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);

// Cargo.lock is TOML, but the shape we need is flat enough to read directly:
// a run of `[[package]]` blocks, each with a name and an optional deps list.
const pkgs = new Map();
for (const block of fs.readFileSync(LOCK, "utf8").split("[[package]]").slice(1)) {
  const name = /^\s*name = "([^"]+)"/m.exec(block)?.[1];
  if (!name) continue;
  const deps = /^dependencies = \[([\s\S]*?)^\]/m.exec(block)?.[1] ?? "";
  pkgs.set(name, [...deps.matchAll(/"([^"\s]+)/g)].map((m) => m[1]));
}

if (!pkgs.has(ROOT)) fail(`${ROOT} is not in Cargo.lock — has the crate been renamed?`);
else ok(`${ROOT} resolves in Cargo.lock`);

// Breadth-first over the real edges, keeping the path so a failure says HOW.
const seen = new Set([ROOT]);
const queue = [[ROOT, [ROOT]]];
const offenders = [];
while (queue.length) {
  const [name, path] = queue.shift();
  for (const d of pkgs.get(name) ?? []) {
    if (/^tauri(-|$)/.test(d)) offenders.push([...path, d].join(" -> "));
    if (seen.has(d)) continue;
    seen.add(d);
    queue.push([d, [...path, d]]);
  }
}

if (offenders.length) {
  for (const o of offenders) fail(`${ROOT} reaches tauri: ${o}`);
} else {
  ok(`${ROOT} reaches no tauri crate (${seen.size} crates in its tree)`);
}

// ── the shim is a CLIENT, not an engine ────────────────────────────────────
// One engine process per host (decision 2, `worktrees_api`'s crate doc): the
// nav-dot poll, the dock shells, the docs server and the engine's writers run
// in `worktrees serve`, and the app finds or starts it (src-tauri/src/engine.rs)
// and talks to it over a socket. These are facts about `run()` and the shim's
// command list that no Rust test reaches — each fails SILENTLY in the running
// app, as a second engine nobody can see, which is why they are here.
const LIB = process.env.API_BOUNDARY_LIB ?? fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url));
const IPC = fileURLToPath(new URL("../src/ipc.ts", import.meta.url));
const lib = fs.readFileSync(LIB, "utf8").split("\n#[cfg(test)]")[0];
// Code only: the shim's own prose names the things it no longer does.
const code = lib.split("\n").map((l) => l.replace(/\/\/.*$/, "")).join("\n");
const run = code.slice(code.indexOf("pub fn run()"));

// 1. The process preamble is still the command layer's, and FIRST: the shim
//    logs, can panic, and launches editors and terminals (and serve) that need
//    the repaired PATH. Nothing in the shim installs a sink or a hook of its own.
const boot = run.indexOf("worktrees_api::host::boot(");
const builder = run.indexOf("tauri::Builder::default()");
if (boot < 0) fail("run() never calls worktrees_api::host::boot — no log sink, no panic hook, no PATH repair, and serve would inherit launchd's bare PATH");
else if (builder >= 0 && boot > builder) fail("host::boot is called AFTER the Tauri builder starts — a startup panic or an early applog would be lost");
else ok("run() calls host::boot before anything else");
for (const stray of ["worktrees_api::log::install(", "std::panic::set_hook(", "worktrees_core::logsink::install(", "fixup_gui_path()", "fixup_gui_locale()"]) {
  if (code.includes(stray)) fail(`the shim still does ${stray} itself — that belongs to host::boot`);
}

// 2. Nothing of an engine is left in this process: no Ctx, no poll, no exit
//    sequence, no host policy, no docs viewer. Any one of these back is a
//    second engine beside serve — two nav-dot polls, shells serve cannot see,
//    two writers of the same files.
const ENGINE = [
  ["worktrees_api::host::start(", "the engine's poll"],
  ["worktrees_api::host::stop(", "the engine's exit sequence (serve runs it when the app stops it)"],
  ["Ctx::new(", "a Ctx"],
  ["worktrees_api::Ctx", "a Ctx"],
  ["fn ctx(", "a Ctx accessor"],
  ["hostpolicy::boot(", "host policy (cmd_serve loads it)"],
  ["viewer::refresh(", "a docs-viewer refresh (serve's poll owns the registrations)"],
  ["viewer::kill(", "a docs server"],
  ["kill_shell(", "dock shells"],
  ["CLONE_JOBS", "clone jobs"],
  ["impl worktrees_api::Host for", "an event host"],
  ["impl worktrees_api::ByteSink for", "a byte stream"],
];
const back = ENGINE.filter(([needle]) => code.includes(needle));
for (const [needle, what] of back) fail(`lib.rs has ${needle} — ${what}: the shim is serve's client, and an engine here is a second one`);
if (!back.length) ok(`lib.rs holds no engine: none of ${ENGINE.length} engine markers (Ctx, poll, exit sequence, host policy, viewer, shells, clones, Host, ByteSink)`);
if (/std::thread::spawn\(/.test(run)) fail("run() spawns a thread of its own — a loop here is an engine's, and the engine is serve");

// 3. The shim's commands are exactly what the door routes to it — serve's
//    client-local rows and its own two — and no other #[tauri::command] exists.
//    The routing list is ipc.ts's; engine-route-check.mjs holds IT against
//    dispatch.rs and generate_handler!.
const ipc = fs.readFileSync(IPC, "utf8");
const tsList = (name) => [...(new RegExp(`export const ${name}: readonly string\\[\\] = \\[([^\\]]*)\\]`).exec(ipc)?.[1] ?? "").matchAll(/"([a-z_0-9]+)"/g)].map((m) => m[1]);
const LOCAL = tsList("CLIENT_LOCAL"), SHIM = tsList("SHIM_COMMANDS");
const fns = [...code.matchAll(/#\[tauri::command\]\s*\n(?:pub )?async fn (\w+)\s*\(/g)].map((m) => m[1]);
const allowed = [...LOCAL, ...SHIM];
if (!LOCAL.length || !SHIM.length) fail("ipc.ts's CLIENT_LOCAL / SHIM_COMMANDS not found — the command rule would be vacuous");
const strayCmds = fns.filter((f) => !allowed.includes(f));
if (strayCmds.length) fail(`lib.rs defines commands the door never routes to it: ${strayCmds.join(", ")} — an engine command re-grown in the shim`);
else ok(`the shim's ${fns.length} #[tauri::command]s are all client-local or its own (${fns.join(", ")})`);
// …and the client-local ones still delegate their bodies, except the two
// with nothing to delegate to: the updater and page zoom act on THIS app.
const BODY_OK = new Set(["install_app_version", "set_zoom", ...SHIM]);
const inline = [];
for (const m of code.matchAll(/#\[tauri::command\]\s*\n(?:pub )?async fn (\w+)\s*\(/g)) {
  const next = code.indexOf("\n#[tauri::command]", m.index + 1);
  const body = code.slice(m.index, next < 0 ? undefined : next);
  if (!body.includes("worktrees_api::") && !BODY_OK.has(m[1])) inline.push(m[1]);
}
if (inline.length) fail(`these client-local commands have a body in lib.rs rather than a call into worktrees-api: ${inline.join(", ")}`);
else ok(`every client-local command delegates to worktrees-api (except ${[...BODY_OK].join(", ")})`);

// 4. ONE `.setup`. tauri's `Builder::setup` REPLACES the stored closure
//    (`self.setup = Box::new(setup)`, app.rs in the locked 2.11.5) rather than
//    chaining, so a second `.setup(` silently discards the first. #508 sat 9/9
//    green with its `Ctx` in the dead one, and every `ctx()` panicked "state()
//    called before manage()". The Engine is managed there now, so the same
//    mistake would leave `engine_connect` with nothing to answer from.
const setups = (run.match(/\.setup\(/g) ?? []).length;
if (setups !== 1) fail(`run() calls .setup( ${setups} times — tauri keeps only the LAST, so every earlier closure is dead code`);
else ok("run() has exactly one .setup( — tauri's Builder::setup replaces rather than chains");
const managed = (run.match(/app\.manage\(Arc::new\(engine::Engine::new\(/g) ?? []).length;
if (managed !== 1) fail(`the Engine is managed ${managed} times in run() — expected exactly 1, inside the .setup`);
else if (run.indexOf("app.manage(Arc::new(engine::Engine::new(") < run.indexOf(".setup(")) fail("the Engine is managed outside the .setup closure");
else ok("exactly one Engine is managed, inside the one .setup");

// 5. Nothing may look up as Tauri state a thing nobody manages: `state::<T>()`
//    compiles for any T and panics at RUNTIME, "state() called before
//    manage()". Two things are managed — the window tracker and the Engine.
const stateRe = /\.(?:try_)?state::<\s*([\w:<>]+?)\s*>(?:\(|\s)/g;
const asked = [...code.matchAll(stateRe)].map((m) => m[1]);
const strayState = [...new Set(asked)].filter((t) => !/^(winstate::)?Tracker$|^Arc<engine::Engine>$/.test(t));
const typedState = [...code.matchAll(/State<'_,\s*([\w:<>]+?)>\s*[,)]/g)].map((m) => m[1]).filter((t) => t !== "Arc<engine::Engine>");
if (strayState.length || typedState.length) fail(`lib.rs asks Tauri for state nothing manages: ${[...strayState, ...typedState].join(", ")}`);
else ok("lib.rs asks Tauri only for the window tracker and the Engine, the two things it manages");

// 6. Quitting stops the serve this app MANAGES — and only through the Engine,
//    which knows whether it started (or adopted) that serve. A bare kill here
//    could stop a serve someone else is using.
const exitAt = run.indexOf("tauri::RunEvent::Exit");
const exitArm = exitAt < 0 ? "" : run.slice(exitAt);
if (exitAt < 0) fail("run() has no Exit arm");
else if (!/handle\.state::<Arc<engine::Engine>>\(\)\.stop_owned\(\)/.test(exitArm)) fail("the Exit arm never calls the Engine's stop_owned() — a serve this app started would outlive it, dock shells and all");
else ok("the Exit arm stops the serve this app manages, through the Engine");
if (/libc::kill\(|Command::new\(/.test(code)) fail("lib.rs signals or spawns a process itself — starting and stopping serve is engine.rs's");

console.log(failed ? `\napi-boundary-check: ${failed} failure(s)` : "\napi-boundary-check: all good");
process.exit(failed ? 1 : 0);

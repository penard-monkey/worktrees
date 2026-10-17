// The command layer must stay callable without a window.
//
//   node app/scripts/api-boundary-check.mjs      # exits non-zero on failure
//
// `worktrees-api` holds the bodies of all 143 Tauri commands so that
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

// And the other direction: the app must not have re-grown a handler BODY.
// A wrapper is allowed — that is what the shim is — but a body is the
// extraction coming undone one command at a time. Three are deliberate
// exceptions, each for a thing a socket has no equivalent of.
const KEEP = new Set(["install_app_version", "set_zoom"]);
const lib = fs.readFileSync(
  fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url)), "utf8",
).split("\n#[cfg(test)]")[0];
const inline = [];
for (const m of lib.matchAll(/#\[tauri::command\]\s*\n(?:pub )?async fn (\w+)\s*\(/g)) {
  const name = m[1];
  const from = m.index;
  const next = lib.indexOf("\n#[tauri::command]", from + 1);
  const body = lib.slice(from, next < 0 ? undefined : next);
  if (!body.includes("worktrees_api::") && !KEEP.has(name)) inline.push(name);
}
if (inline.length) fail(`these commands have a body in lib.rs rather than a wrapper: ${inline.join(", ")}`);
else ok(`every command in lib.rs delegates to worktrees-api (except ${[...KEEP].join(", ")})`);

// ── the wiring in run(), which only the source can answer for ──────────────
// Three facts no Rust test can reach, because they are about the order and
// the arity of calls inside `run()` rather than any function's behaviour.
// Each fails SILENTLY in the running app, which is why they are here.
const run = lib.slice(lib.indexOf("pub fn run()"));

// 1. The sink must be installed before anything wants to log. The panic hook
//    is registered immediately after, and the startup line a few lines later;
//    install it late and both vanish, with nothing to show for it.
const install = run.indexOf("worktrees_api::log::install");
const hook = run.indexOf("std::panic::set_hook");
const firstLog = run.indexOf("applog(");
if (install < 0) fail("run() never installs the log sink — every applog in the command layer is a no-op");
else if (hook >= 0 && install > hook) fail("the log sink is installed AFTER the panic hook — a startup panic would not be logged");
else if (firstLog >= 0 && install > firstLog) fail("the log sink is installed after the first applog in run()");
else ok("run() installs the log sink before the panic hook and the first applog");

// 2. `ctx()` must hand out CLONES of the one managed Ctx. Building a fresh
//    one per call compiles, type-checks and passes every unit test — and
//    silently gives the poll different registries from the handlers, so the
//    nav dots never light. headless.rs asserts that a clone shares; this
//    asserts the app is actually cloning.
const ctxFn = /fn ctx\(app: &AppHandle\) -> worktrees_api::Ctx \{([^]*?)\n\}/.exec(lib)?.[1];
if (!ctxFn) fail("ctx(&AppHandle) is not where this check looks");
else if (/Ctx::new\(/.test(ctxFn)) fail("ctx() CONSTRUCTS a Ctx instead of cloning the managed one — the poll and the handlers would hold different registries");
else if (!/state::<worktrees_api::Ctx>\(\)/.test(ctxFn)) fail("ctx() no longer resolves the managed Ctx");
else ok("ctx() clones the one managed Ctx rather than building a new one");

// 3. …and there must be exactly one of it to clone.
const managed = (lib.match(/\.manage\(worktrees_api::Ctx::new/g) ?? []).length;
if (managed !== 1) fail(`the Ctx is managed ${managed} times — expected exactly 1`);
else ok("exactly one Ctx is managed for the process");
console.log(failed ? `\napi-boundary-check: ${failed} failure(s)` : "\napi-boundary-check: all good");
process.exit(failed ? 1 : 0);

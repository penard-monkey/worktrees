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

console.log(failed ? `\napi-boundary-check: ${failed} failure(s)` : "\napi-boundary-check: all good");
process.exit(failed ? 1 : 0);

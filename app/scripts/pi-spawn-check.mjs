// Every pi worktrees starts gets worktrees' model hosts (crates/worktrees-core/
// src/pihosts.rs, docs/proposals/pi-models-config.md). pi has no flag for
// models.json alone, so the hosts reach pi as an extension (`-e`) — and a pi
// started WITHOUT it lists a different catalog, and a resume of a session on
// one of worktrees' models silently starts on another model (measured, pi
// 1.5.0). So ONE builder adds it: `pimodels::pi_command[_for]` for a process,
// `pimodels::pane_ext_words` for a pane's launch line. This check makes that
// structural:
//
//   - every call of `pi_bin()` in non-test code is in ALLOWED below, with the
//     reason it is not a spawn that loads models (an unused row fails too);
//   - no `childenv::command("pi"…)`, which would name pi around the lookup;
//   - the builder adds `-e`, `run_pi` uses it, and pi's pane flags start with
//     the extension words.
//
//   node app/scripts/pi-spawn-check.mjs [<repo root>]   # exits non-zero on failure
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = process.argv[2] ?? fileURLToPath(new URL("../..", import.meta.url));
const DIRS = ["crates/worktrees-core/src", "crates/worktrees-api/src", "crates/worktrees-cli/src", "app/src-tauri/src"];

/** [file suffix, text on the line, why it is not a model-loading spawn]. */
const ALLOWED = [
  ["worktrees-core/src/pimodels.rs", "Some(pi_command_for(&pi_bin()?, hosts))", "the builder itself"],
  ["worktrees-core/src/harness.rs", "crate::pimodels::pi_bin()", "Adapter::installed: whether pi exists, no spawn"],
  ["worktrees-cli/src/mcp.rs", "pi_bin().is_none()", "create_worktree's 'pi is not installed' refusal, no spawn"],
  ["worktrees-core/src/pimcp.rs", "crate::pimodels::pi_bin().map(", "shows the path; `pi mcp` spawns go through pi_command_for"],
  ["worktrees-api/src/harness_updates.rs", "worktrees_core::pimodels::pi_bin()", "`--version` and `pi update`, as for every harness: no model loads"],
];

const rsFiles = (dir) =>
  fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) return rsFiles(p);
    return e.name.endsWith(".rs") ? [p] : [];
  });

// The non-test part, `//` comments blanked (childenv-check.mjs's rule).
const code = (src) => {
  const m = /^#\[cfg\(test\)\]\s*\n(pub\s+)?mod \w+/m.exec(src);
  const head = m ? src.slice(0, m.index) : src;
  return head.split("\n").map((l) => l.replace(/\/\/.*$/, ""));
};

let failed = 0;
const fail = (msg) => { failed++; console.log("FAIL " + msg); };
const used = ALLOWED.map(() => false);
const read = (rel) => fs.readFileSync(path.join(ROOT, rel), "utf8");

for (const d of DIRS) {
  const abs = path.join(ROOT, d);
  if (!fs.existsSync(abs)) { fail(`${d} does not exist under ${ROOT}`); continue; }
  for (const f of rsFiles(abs)) {
    const rel = path.relative(ROOT, f);
    code(fs.readFileSync(f, "utf8")).forEach((l, i) => {
      if (/\bpi_bin\(\)/.test(l) && !/fn pi_bin\(\)/.test(l)) {
        const k = ALLOWED.findIndex(([file, needle]) => rel.endsWith(file) && l.includes(needle));
        if (k < 0) fail(`${rel}:${i + 1}: pi_bin() outside the builder — spawn pi with pimodels::pi_command, or add a row with the reason: ${l.trim()}`);
        else used[k] = true;
      }
      if (/childenv::command\(\s*"pi"/.test(l)) fail(`${rel}:${i + 1}: pi named around the lookup and the builder: ${l.trim()}`);
    });
  }
}
ALLOWED.forEach(([file, needle], k) => { if (!used[k]) fail(`ALLOWED row matches nothing any more: ${file} "${needle}"`); });

const pm = code(read("crates/worktrees-core/src/pimodels.rs")).join("\n");
const body = (src, head) => { const i = src.indexOf(head); return i < 0 ? "" : src.slice(i, src.indexOf("\n}\n", i)); };
if (!/cmd\.args\(ext_args\(\)\)/.test(body(pm, "pub fn pi_command_for("))) fail("pi_command_for no longer adds ext_args()");
if (!/pi_command\(hosts\)/.test(body(pm, "fn run_pi("))) fail("run_pi no longer builds with pi_command");
if (!/run_pi\(&\["--list-models", "--offline"\], 10, true\)/.test(pm)) fail("--list-models no longer asks for the hosts extension");
const hz = code(read("crates/worktrees-core/src/harness.rs")).join("\n");
const piFlags = body(hz.slice(hz.indexOf("impl Adapter for Pi")), "fn place_flags(");
if (!/^\s*fn place_flags\(&self, wt: &str\) -> Vec<String> \{\n\s*let mut f = crate::pimodels::pane_ext_words\(\);/.test(piFlags)) {
  fail("pi's place_flags no longer starts with pimodels::pane_ext_words() — a launch or resume without the hosts");
}

if (failed) { console.log(`pi-spawn-check: ${failed} failure(s)`); process.exit(1); }
console.log("pi-spawn-check: all good");

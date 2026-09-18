#!/usr/bin/env node
// The nav drag can drop a place onto the terminal, which types a reference to
// that place into the Claude session there. Three invariants hold that up, and
// every one of them is invisible to `tsc` and to the unit tests — each would
// break the feature (or another one) while still compiling and still passing.
//
//   1. `TermSurface` is SHARED by `TerminalPane` and the dock's `ShellPane`.
//      Hardcoding `data-drop` in it would make every dock scratch shell a drop
//      target for a worktree reference, which is meaningless there.
//   2. `resolveDrop` must test the terminal BEFORE `[data-tier]`. It returns
//      null for a place that is not over a tier zone, so a mention branch
//      placed after that early return is dead code — and dead code that reads
//      as implemented.
//   3. The affordance is keyed on `[data-drop="mention"]`; the selector and the
//      attribute are written in two files and only agree by convention.
//
// Run: node app/scripts/dropzone-check.mjs
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const src = join(dirname(fileURLToPath(import.meta.url)), "..", "src");
const read = (f) => readFileSync(join(src, f), "utf8");
const fails = [];
const check = (ok, what) => {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) fails.push(what);
};

// ── 1. the shared component ────────────────────────────────────────────────
const pane = read("TerminalPane.tsx");
check(
  /<div className="term-wrap" data-drop=\{drop\}>/.test(pane),
  "`.term-wrap` takes data-drop from a PROP, not a literal (TermSurface is shared)",
);
// NOTE the boundary: these components' PROP TYPES contain `\n}` too
// (`} & TermFindProps) {`), so slicing to the first one cuts the body off
// before the JSX and the check reports a failure that is its own.
const body = (name) => {
  const i = pane.indexOf(`export function ${name}(`);
  if (i < 0) return "";
  const j = pane.indexOf("\n}\n", i);
  return pane.slice(i, j < 0 ? undefined : j);
};
check(/drop="mention"/.test(body("TerminalPane")), "TerminalPane marks itself a drop target");
check(!/drop=/.test(body("ShellPane")), "ShellPane does NOT — a dock scratch shell is not a place");

// ── 2. branch order in resolveDrop ─────────────────────────────────────────
const app = read("App.tsx");
const resolve = app.slice(app.indexOf("const resolveDrop ="));
const mentionAt = resolve.indexOf(`[data-drop="mention"]`);
const tierAt = resolve.indexOf(`closest<HTMLElement>("[data-tier]")`);
check(mentionAt > -1 && tierAt > -1, "resolveDrop still tests both the terminal and the tier zones");
check(
  mentionAt > -1 && tierAt > -1 && mentionAt < tierAt,
  "the terminal is tested BEFORE [data-tier] (after it, the branch is unreachable)",
);
// The cross-project guard: the MCP server is pinned to one repo, so a foreign
// slug cannot resolve and the token would be dead text.
const mentionBranch = resolve.slice(mentionAt, tierAt);
check(
  /item\.repo !== sel\.repo/.test(mentionBranch) && /kind: "reject"/.test(mentionBranch),
  "a cross-project drop is REJECTED rather than silently inserted",
);
check(
  /tmux_session\.up/.test(mentionBranch),
  "a place whose session is down is not a target (there is no pane to paste into)",
);

// ── 3. the affordance's selector matches the attribute ─────────────────────
const css = read("App.css");
check(
  /body\.dragging-place\s+\.term-wrap\[data-drop="mention"\]/.test(css),
  "the candidate outline is keyed on the same attribute the pane sets",
);
check(
  /body\.dragging-place\.drop-mention\s+\.term-wrap\[data-drop="mention"\]/.test(css),
  "…and the resolved state is a SEPARATE rule (a cross-project drag must not light up)",
);
check(
  /classList\.add\("dragging", `dragging-\$\{s\.item\.kind\}`\)/.test(read("navdrag.ts")),
  "navdrag publishes the dragged KIND, which is what `dragging-place` is",
);

console.log();
if (fails.length) {
  console.log(`FAILED (${fails.length}): ${fails.join("; ")}`);
  process.exit(1);
}
console.log("dropzone-check: all good");

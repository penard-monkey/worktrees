// Deterministic check that the dock's tab strip restores from SETTLED settings.
//
// The settings layout effect AWAITS an invoke. A place entered before it
// resolves reads DEFAULTS' empty `term_tabs` / `term_tab_names`, so
// TerminalTabs' restore unions nothing with the (also empty, post-restart)
// live shell list and falls back to a single unnamed `sh 1`. The real strip is
// untouched on disk — the restore deliberately never writes its fallback back —
// so the names are still there, just never on screen: nothing re-runs the
// restore, and they stay gone until you leave the place and come back.
//
// `hydratedTick` is the signal that already exists for this, added with the
// Files viewer's restore (PR #302) and carrying a comment that describes this
// exact failure. It had ONE consumer. This check pins the second.
//
// Static, like termfit-check.mjs: the thing that breaks is a dependency array,
// and a dep array is not observable at runtime without driving a real browser
// through a slow-settings restart (app/scripts has no browser). Reproduced that
// way while fixing it; pinned this way so it stays fixed.
//
//   node tabsrestore-check.mjs [path/to/App.tsx]   exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const SRC = process.argv[2] || fileURLToPath(new URL("../src/App.tsx", import.meta.url));
const raw = fs.readFileSync(SRC, "utf8");
let failed = 0;
const ok = (m) => console.log(`ok — ${m}`);
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const check = (c, m) => (c ? ok(m) : fail(m));

// 1. The component is handed the signal at all.
check(/<TerminalTabs[\s\S]{0,800}?hydratedTick=\{hydratedTick\}/.test(raw),
  "TerminalTabs is passed hydratedTick");

// 2. Its restore effect depends on it. Find the effect by the invoke it makes —
//    `list_shell_sessions` appears exactly once — then read the dep array that
//    closes it.
const at = raw.indexOf('invoke<{ index: number; dead: boolean }[]>("list_shell_sessions"');
if (at < 0) {
  fail("the tab-strip restore was not found (has list_shell_sessions moved?)");
} else {
  const deps = /\}, \[([^\]]*)\]\);/.exec(raw.slice(at));
  if (!deps) fail("no dependency array closes the restore effect");
  else {
    const list = deps[1].split(",").map((d) => d.trim()).filter(Boolean);
    check(list.includes("hydratedTick"),
      `the restore re-runs when settings land (deps: ${list.join(", ")})`);
    // The place identity must stay in there too, or switching places stops
    // re-reading the strip.
    check(list.includes("repo") && list.includes("slug"),
      "…and still re-runs on a place switch");
  }
}

// 3. The signal keeps its one producer. If hydration stops bumping it, both
//    restores that depend on it go quiet with no other symptom.
check(/setHydratedTick\(\(n\) => n \+ 1\)/.test(raw), "hydration still bumps the tick");

console.log(failed ? `\n${failed} failure(s)` : "\nall good");
process.exit(failed ? 1 : 0);

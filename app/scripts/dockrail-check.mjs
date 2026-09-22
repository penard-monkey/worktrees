// Checks the DOCK RAIL's two mirrors, and the one the Automations tab adds.
//
//   node app/scripts/dockrail-check.mjs        # exits non-zero on failure
//
// Three things that type-check perfectly while being wrong:
//
//  1. **`key` and `track` drift.** The comment above `DOCK_RAIL` in App.tsx says
//     they are pinned together, and `usage-check.mjs` already refuses an
//     INTERPOLATED `data-track` — which is what a `` `dock.${d.key}` `` there
//     would be, indistinguishable to that scanner from one built out of a place
//     name. So the pair is written out by hand, and hand-written pairs drift:
//     a fourth tab whose `track` still said `dock.plan` would file every one of
//     its events under another tab's key, silently, for as long as it shipped.
//  2. **The `dock_tab` union and the rail disagreeing.** The union is declared
//     TWICE in settings.ts (`PlacePanels` and `Settings`), and a tab added to
//     one and not the other compiles — the flat key is what `panelsFor` seeds
//     from, so the tab would work until it had to persist, and then silently
//     fall back. A tab missing from BOTH is worse: the rail renders it, the
//     ternary in the dock body shows it, and `pickDockTab` writes a value the
//     type says cannot exist.
//  3. **`PROPOSAL_TOOLS` drifting from `runs.rs`.** `automations.ts` mirrors the
//     closed set so the run view can label a button without a second parse.
//     The frontend's copy has no tests of its own that would notice — the same
//     shape as `dnd-check.mjs`'s mirror, and the same fix.
//
// Like `dnd-check.mjs`, this slices the REAL sources rather than a paraphrase,
// and it refuses to guess: a source that no longer matches its pattern is a
// FAILURE, not a skipped assertion, because a guard that cannot be wrong is a
// guard that cannot fail.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const APP = fileURLToPath(new URL("../src/App.tsx", import.meta.url));
const SETTINGS = fileURLToPath(new URL("../src/settings.ts", import.meta.url));
const AUTOMATIONS = fileURLToPath(new URL("../src/automations.ts", import.meta.url));
const RUNS_RS = fileURLToPath(new URL("../../crates/worktrees-core/src/runs.rs", import.meta.url));

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
const ok = (name, cond, detail = "") => {
  if (cond) return;
  failed++;
  console.log(`not ok — ${name}${detail ? `\n    ${detail}` : ""}`);
};

// ── 1. DOCK_RAIL: every entry's track is `dock.<key>` ───────────────────────
const app = fs.readFileSync(APP, "utf8");
// One declaration only. A second `const DOCK_RAIL` (a refactor that left the
// old one above the new one) would let this check assert the dead copy while
// the live one drifted — the `docsviewer-check.mjs` trap, named in CLAUDE.md.
const decls = [...app.matchAll(/const DOCK_RAIL\s*=\s*\[/g)];
ok("exactly one DOCK_RAIL declaration in App.tsx", decls.length === 1, `found ${decls.length}`);

let rail = [];
if (decls.length === 1) {
  const start = decls[0].index + decls[0][0].length;
  const end = app.indexOf("\n  ];", start);
  ok("the DOCK_RAIL literal is closed by `\\n  ];`", end > start);
  const body = app.slice(start, end);
  rail = [...body.matchAll(/\{\s*key:\s*"([a-z]+)"[^}]*?track:\s*"([^"]+)"/g)]
    .map((m) => ({ key: m[1], track: m[2] }));
  // The count is asserted against the raw `key:` occurrences, so an entry whose
  // shape the regex above does not match cannot simply vanish from the check.
  const keyCount = [...body.matchAll(/\bkey:\s*"/g)].length;
  eq("every DOCK_RAIL entry parsed", rail.length, keyCount);
  ok("DOCK_RAIL is not empty", rail.length > 0);
  for (const e of rail) eq(`track pins to key for "${e.key}"`, e.track, `dock.${e.key}`);
}
const railKeys = rail.map((e) => e.key).sort();

// ── 2. the dock_tab union, in BOTH places, equals the rail ──────────────────
const settings = fs.readFileSync(SETTINGS, "utf8");
// `[^;\n]` and not `[^;]`: the DEFAULTS object further down has a `dock_tab:`
// line of its own, and a class that can cross newlines runs past it to the next
// `;` dozens of lines below, swallowing the whole literal as a third "union".
const unions = [...settings.matchAll(/^\s*dock_tab:\s*([^;\n]+);/gm)].map((m) =>
  m[1].split("|").map((s) => s.trim().replace(/"/g, "")).sort());
// TWO: `PlacePanels` and `Settings`. Not "at least two" — a third copy is a
// third place to forget, and this check is where that is noticed.
eq("dock_tab is declared exactly twice in settings.ts", unions.length, 2);
unions.forEach((u, i) => eq(`dock_tab union #${i + 1} == DOCK_RAIL keys`, u, railKeys));

// ── 3. PROPOSAL_TOOLS mirrors runs.rs ───────────────────────────────────────
const js = (await transformWithEsbuild(fs.readFileSync(AUTOMATIONS, "utf8"), "automations.ts", {
  loader: "ts", format: "esm",
})).code;
const A = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

const runsRs = fs.readFileSync(RUNS_RS, "utf8");
// `mod tests` is sliced off first: its fixtures name these tools too, and a
// match in there would be asserting against a test's idea of the set.
const prod = runsRs.split(/\n#\[cfg\(test\)\]/)[0];
const m = /PROPOSAL_TOOLS:\s*\[&str;\s*(\d+)\]\s*=\s*\[([^\]]+)\]/.exec(prod);
ok("runs.rs still declares PROPOSAL_TOOLS", !!m);
if (m) {
  const tools = m[2].split(",").map((s) => s.trim().replace(/"/g, "")).filter(Boolean);
  // The declared arity and the literal must agree — a `[&str; 4]` holding five
  // entries does not compile, but reading both is what makes the count below
  // mean something.
  eq("PROPOSAL_TOOLS arity matches its literal", tools.length, Number(m[1]));
  eq("automations.ts PROPOSAL_TOOLS mirrors runs.rs", [...A.PROPOSAL_TOOLS].sort(), [...tools].sort());
  // Stated separately from the equality above, because this is the rule and
  // that is only its mechanism: `remove_worktree` is the one path in this
  // codebase that can destroy commits, and it is never a proposal.
  ok("remove_worktree is not a proposal tool", !tools.includes("remove_worktree"));
}

// ── the starters exist and are prose ────────────────────────────────────────
// Not decoration: a starter is the first thing a new user reads, and the design
// decision it carries is that the thing you edit is a PARAGRAPH (proposal §2).
// A starter that had been trimmed into a one-line command would teach the
// opposite, and nothing else would notice.
ok("three starters", A.STARTERS?.length === 3, `found ${A.STARTERS?.length}`);
for (const s of A.STARTERS ?? []) {
  ok(`starter "${s.name}" has a name`, !!s.name?.trim());
  const sentences = s.brief.split(/\.\s/).filter(Boolean).length;
  ok(`starter "${s.name}" is 3-5 sentences of prose`, sentences >= 3 && sentences <= 5, `${sentences} sentences`);
  for (const t of A.PROPOSAL_TOOLS)
    ok(`starter "${s.name}" names no tool (${t})`, !s.brief.includes(t));
}

if (failed) {
  console.log(`\n${failed} failure(s)`);
  process.exit(1);
}
console.log("ok — dock rail, dock_tab unions and PROPOSAL_TOOLS all agree");

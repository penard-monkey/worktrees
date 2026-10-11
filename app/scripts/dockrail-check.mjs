// Checks the DOCK RAIL's mirrors, and that a REMOVED tab cannot be restored.
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
//  3. **A persisted tab that no longer exists.** `dock_tab` lives in
//     ui-state.json, globally and per place, so removing a tab (Automations,
//     after 0.40.1) leaves values the union says cannot exist. Unsanitised,
//     the dock body's ternary falls through to the Terminal with no rail
//     button lit. `loadSettings` maps them to Files through `DOCK_TABS`, and
//     this drives the REAL `loadSettings` with such a blob.
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

// ── 3. a removed tab persisted in ui-state.json lands on Files ─────────────
const m = /export const DOCK_TABS = \[([^\]]+)\] as const;/.exec(settings);
ok("settings.ts still declares `export const DOCK_TABS = [...] as const`", !!m);
if (m) eq("DOCK_TABS == DOCK_RAIL keys", m[1].split(",").map((x) => x.trim().replace(/"/g, "")).filter(Boolean).sort(), railKeys);

// Load the real module, as `zoom-check.mjs` does: its relative imports are
// inlined (a data: URL has no base to resolve them against) and the Tauri
// bridge answers `get_settings` with the blob under test.
const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
const stripImport = (src, mod) => {
  const re = new RegExp(`^import \\{[^}]*\\} from "\\./${mod}";$`, "m");
  ok(`settings.ts imports from ./${mod} as expected (the inlining is not stale)`, re.test(src));
  return src.replace(re, "");
};
const src =
  inline("../src/afterglow.ts") + inline("../src/harness.ts") +
  stripImport(stripImport(settings, "afterglow"), "harness")
    .replace(/^import \{ invoke \}.*$/m, "const invoke = (cmd) => Promise.resolve(cmd === \"get_settings\" ? globalThis.__blob : undefined);");
const js = (await transformWithEsbuild(src, "settings.ts", { loader: "ts", format: "esm" })).code;
const S = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

globalThis.__blob = {
  // A current revision, so `migrate` has nothing to do and saves nothing.
  settings_rev: 999,
  dock_tab: "automations",
  place_panels: {
    "/r|old": { dock_open: true, dock_tab: "automations", dock_width: 400 },
    "/r|kept": { dock_open: true, dock_tab: "plan", dock_width: 400 },
    "/r|bare": { dock_open: true, dock_width: 400 },
  },
};
const s = await S.loadSettings();
eq("a persisted global dock_tab of a removed tab loads as files", s.dock_tab, "files");
eq("a place left on a removed tab loads as files", s.place_panels["/r|old"].dock_tab, "files");
eq("a place on a live tab keeps it", s.place_panels["/r|kept"].dock_tab, "plan");
// Absent stays absent: filling it in would freeze the global seed into a place
// that never set it (the `place_panels` seed trap in CLAUDE.md).
eq("a place with no dock_tab is not given one", "dock_tab" in s.place_panels["/r|bare"], false);
eq("panelsFor hands the old place Files", S.panelsFor(s, "/r|old").dock_tab, "files");
for (const t of S.DOCK_TABS) {
  globalThis.__blob = { settings_rev: 999, dock_tab: t };
  eq(`a live tab "${t}" survives a load`, (await S.loadSettings()).dock_tab, t);
}

if (failed) {
  console.log(`\n${failed} failure(s)`);
  process.exit(1);
}
console.log("ok — dock rail, dock_tab unions and DOCK_TABS agree; a removed tab loads as Files");

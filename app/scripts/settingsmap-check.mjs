// Guards the Settings regroup and its modes (docs/proposals/settings-reorg.md
// §3–§4, §6 item 7) against drift that every other suite would pass.
//
//   node app/scripts/settingsmap-check.mjs     # exits non-zero on failure
//
// `settingsMap.ts` says what is on which page, at which tier, owning which
// values; the page FILES (`pages/<id>.tsx`) render it; App, offers.ts and the
// remote store open the sheet by category ids, some of them from before the
// regroup. Each half can move without the other and look perfectly fine — a
// section on the wrong page, an old id that lands on General, a new blob key
// no section owns (so no mode ever shows it changed), a mode rule that hides
// a changed value. So:
//
//   1. ALIASES. Every category id the sheet has EVER had still resolves, to a
//      page, and a focus wins over the category it was sent with (§3.4).
//   2. FOCUS_HOME. Every section in the map is rendered by exactly its own
//      page's file, once; no page file renders a section the map puts
//      elsewhere; every page has a file and a view in SettingsSheet; every
//      offer's link lands on the page its focus lives on.
//   3. CLAIMS. Every key the blob has is claimed by exactly one section or one
//      of the map's stated lists, and every host value and policy getter by
//      exactly one section — or a key changed in Expert is hidden in Beginner
//      with no rule able to see it.
//   4. THE MODE RULES, evaluated: hidden is never off (`settings_mode` is read
//      only by the sheet), a changed setting is never hidden, a problem is
//      never hidden, an installed harness is never hidden, a deep link
//      reveals, and the "N more" footer counts exactly what is hidden.
//   5. MIGRATE REV 5, through the real loadSettings: an existing blob opens in
//      Expert, a fresh install in Beginner, a chosen mode is kept.
//   6. RESET SCOPE: the four scopes are disjoint and cover the blob, the
//      default never takes per-place memory or a silenced suggestion, and
//      Everything is their union.
//   7. ONE ARM: no component keeps its own two-click timer (useArmed.ts).
//   8. GRANTS: every row a section names for a remote window is one the
//      remote door can refuse (ipc.ts's tables), and exists in serve.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const read = (rel) => fs.readFileSync(here(rel), "utf8");
let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

const load = async (src, name) => {
  const js = (await transformWithEsbuild(src, name, { loader: "ts", format: "esm" })).code;
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
};

// The map: type imports only, which esbuild erases, so it loads as is.
const mapSrc = read("../src/settingsMap.ts");
if (/^import (?!type )/m.test(mapSrc)) fail("settingsMap.ts has a runtime import — it must load as a data: module, alone");
const M = await load(mapSrc, "settingsMap.ts");

// The real settings.ts, inlined the way layout-check does it.
let stored = null;
globalThis.__mapInvoke = (cmd) => Promise.resolve(cmd === "get_settings" ? structuredClone(stored) : null);
const settingsTs = read("../src/settings.ts");
const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
const stripImport = (src, mod) => {
  const re = new RegExp(`^import \\{[^}]*\\} from "\\./${mod}";$`, "m");
  if (!re.test(src)) fail(`settings.ts: no \`import { … } from "./${mod}"\` line — the inlining below is stale`);
  return src.replace(re, "");
};
if ([...settingsTs.matchAll(/^import [^\n]* from "\.\/([^"]+)";$/gm)].map((m) => m[1]).sort().join(",") !== "afterglow,harness,ipc") {
  fail("settings.ts gained a relative import — zoom-check and layout-check load it as a data: module that cannot resolve one");
}
const S = await load(
  inline("../src/afterglow.ts") + inline("../src/harness.ts") +
  stripImport(stripImport(settingsTs, "afterglow"), "harness")
    .replace(/^import \{ invoke \}.*$/m, "const invoke = (cmd, a) => globalThis.__mapInvoke(cmd, a);"),
  "settings.ts");
const D = S.DEFAULTS;
const pageIds = M.PAGES.map((p) => p.id);

// ── 1. aliases ──────────────────────────────────────────────────────────────
// Every id SettingsSheet's CATS ever had (v0.47.0), frozen here on purpose: a
// list derived from the current map could not notice one going missing.
const HISTORIC = ["appearance", "terminal", "navigation", "commands", "ai", "claude", "codex", "pi", "kimi", "guidance",
  "planning", "behavior", "hosts", "remote", "devices", "updates", "data", "shortcuts", "usage"];
for (const id of HISTORIC) {
  const r = M.resolveAt({ cat: id });
  if (!pageIds.includes(r.cat)) fail(`old category "${id}" resolves to "${r.cat}", not a page`);
  else if (r.cat === "general" && !["behavior"].includes(id)) fail(`old category "${id}" falls through to General — add it to LEGACY_CATS`);
}
ok(`all ${HISTORIC.length} historic category ids resolve to a page`);
for (const [id, a] of Object.entries(M.LEGACY_CATS)) {
  if (!pageIds.includes(a.cat)) fail(`LEGACY_CATS.${id} → "${a.cat}", not a page`);
  if (a.focus && M.FOCUS_HOME[a.focus] !== a.cat) fail(`LEGACY_CATS.${id} focuses "${a.focus}", which lives on "${M.FOCUS_HOME[a.focus]}", not "${a.cat}"`);
}
ok("every alias names a page, and its default focus lives on it");
// A focus wins over the category it came with: the old cross-project offer
// sent {guidance, cross-project}, and the section is on Agent policy now.
{
  const r = M.resolveAt({ cat: "guidance", focus: "cross-project" });
  if (r.cat !== "policy" || r.focus !== "cross-project") fail(`{guidance, cross-project} lands on ${r.cat}/${r.focus}, not policy/cross-project`);
  else ok("a focus moves the page to wherever its section lives (guidance/cross-project → policy)");
  const p = M.resolveAt({ cat: "planning" });
  if (p.cat !== "projects" || p.focus !== "planning") fail(`{planning} lands on ${p.cat}/${p.focus}`);
  else ok("an alias supplies its focus when the caller named only the category (planning → projects/planning)");
  const sub = M.resolveAt({ cat: "guidance", focus: "agent-guidance-changed" });
  if (sub.cat !== "guidance" || M.focusSection("agent-guidance-changed") !== "guidance-skill") fail("a band's focus does not reveal the section that holds it");
  else ok("a focus inside a section (agent-guidance-changed) reveals that section");
}

// ── 2. FOCUS_HOME and the page files ────────────────────────────────────────
const sheet = read("../src/SettingsSheet.tsx");
const views = sheet.match(/const PAGE_VIEW[^{]*\{([\s\S]*?)\n\};/)?.[1] ?? "";
const viewIds = [...views.matchAll(/^\s+([a-z]+): [A-Za-z]+Page,$/gm)].map((m) => m[1]);
if (viewIds.sort().join() !== [...pageIds].sort().join()) fail(`SettingsSheet's PAGE_VIEW (${viewIds.join(", ")}) is not PAGES (${pageIds.join(", ")})`);
else ok(`every page has a view (${pageIds.length})`);
const pageFiles = fs.readdirSync(here("../src/pages")).filter((f) => f.endsWith(".tsx"));
const rendered = {};
for (const f of pageFiles) {
  const src = fs.readFileSync(path.join(here("../src/pages"), f), "utf8");
  for (const m of src.matchAll(/<Sec id="([a-z0-9-]+)"/g)) (rendered[m[1]] ??= []).push(f.replace(/\.tsx$/, ""));
}
for (const p of pageIds) if (!pageFiles.includes(`${p}.tsx`)) fail(`page "${p}" has no file pages/${p}.tsx`);
for (const s of M.SECTIONS) {
  const at = rendered[s.id] ?? [];
  if (at.length === 0) fail(`section "${s.id}" (${s.page}) is rendered by no page file`);
  else if (at.length > 1) fail(`section "${s.id}" is rendered ${at.length} times (${at.join(", ")})`);
  else if (at[0] !== s.page) fail(`section "${s.id}" is on "${s.page}" in the map but rendered by pages/${at[0]}.tsx`);
}
for (const id of Object.keys(rendered)) if (!M.SECTION[id]) fail(`pages/${rendered[id][0]}.tsx renders <Sec id="${id}">, which the map does not have`);
const ids = M.SECTIONS.map((s) => s.id);
if (new Set(ids).size !== ids.length) fail("two sections share an id");
for (const [f, s] of Object.entries(M.SUB_FOCUS)) if (!M.SECTION[s]) fail(`SUB_FOCUS.${f} names "${s}", not a section`);
if (!bad) ok(`all ${M.SECTIONS.length} sections render on their own page, once`);
// A section's focus id is its Sec's alone: a panel that also puts it on its
// own element makes a deep link match two elements (the outer one wins the
// query, and anything that locates the section by it is ambiguous).
{
  const dupes = [];
  for (const f of fs.readdirSync(here("../src")).filter((f) => f.endsWith(".tsx"))) {
    const src = fs.readFileSync(path.join(here("../src"), f), "utf8");
    for (const m of src.matchAll(/(?:data-focus|focus)="([a-z0-9-]+)"/g)) if (M.SECTION[m[1]]) dupes.push(`${f}: ${m[1]}`);
  }
  if (dupes.length) fail(`a panel repeats a section's focus id, which its Sec already carries (${dupes.join(", ")})`);
  else ok("no panel repeats a section's focus id");
}
// offers.ts: each link's focus lives on the page it names.
for (const m of read("../src/offers.ts").matchAll(/to: \{ cat: "([a-z-]+)", focus: "([a-z-]+)" \}/g)) {
  const [, cat, focus] = m;
  if (M.FOCUS_HOME[focus] !== cat) fail(`offers.ts links ${cat}/${focus}, but "${focus}" lives on "${M.FOCUS_HOME[focus]}"`);
}
ok("every offer's link names the page its section is on");

// ── 3. claims ───────────────────────────────────────────────────────────────
// The blob's keys: everything DEFAULTS has, plus the optional ones the type
// declares (the host-policy mirror).
const typeBody = settingsTs.match(/export type Settings = \{([\s\S]*?)\n\};/)?.[1] ?? "";
const typeKeys = [...typeBody.matchAll(/^ {2}([a-z_]+)\??:/gm)].map((m) => m[1]);
const blobKeys = [...new Set([...Object.keys(D), ...typeKeys])];
const owners = {};
const claim = (k, who) => (owners[k] ??= []).push(who);
for (const s of M.SECTIONS) for (const k of s.keys ?? []) claim(k, `section ${s.id}`);
for (const [list, name] of [[M.MEMORY_KEYS, "MEMORY_KEYS"], [M.PLACE_MEMORY_KEYS, "PLACE_MEMORY_KEYS"], [M.MARKER_KEYS, "MARKER_KEYS"],
  [M.MIRROR_KEYS, "MIRROR_KEYS"], [M.SHEET_KEYS, "SHEET_KEYS"]]) for (const k of list) claim(k, name);
for (const k of blobKeys) {
  const o = owners[k] ?? [];
  if (o.length === 0) fail(`blob key \`${k}\` is claimed by no section and no list — a mode could hide it changed`);
  else if (o.length > 1) fail(`blob key \`${k}\` is claimed ${o.length} times (${o.join(", ")})`);
}
for (const k of Object.keys(owners)) if (!blobKeys.includes(k)) fail(`${owners[k].join(", ")} claims \`${k}\`, which the blob does not have`);
ok(`every blob key (${blobKeys.length}) is claimed exactly once`);
const hostOwners = {};
for (const s of M.SECTIONS) for (const h of s.host ?? []) (hostOwners[h] ??= []).push(s.id);
for (const h of M.HOST_KEYS) {
  if (!hostOwners[h]) fail(`host value \`${h}\` is claimed by no section`);
  else if (hostOwners[h].length > 1) fail(`host value \`${h}\` is claimed by ${hostOwners[h].join(", ")}`);
}
for (const h of Object.keys(hostOwners)) if (!M.HOST_KEYS.includes(h)) fail(`section ${hostOwners[h]} claims host value \`${h}\`, which HOST_KEYS does not have`);
ok(`every host value (${M.HOST_KEYS.length}) is claimed by exactly one section`);
// The policy getters serve has: a `get_*` row or a `*_settings` row is a
// value Settings shows, and must feed a host value — or say why not.
const dispatch = read("../../crates/worktrees-cli/src/serve/dispatch.rs").split(/\n#\[cfg\(test\)\]\s*\nmod tests\b/)[0]
  .split("\n").map((l) => l.replace(/\/\/.*$/, "")).join("\n");
const rows = [...dispatch.matchAll(/^ {4}([a-z_0-9]+): (Engine|HostConsole|ClientLocal|Loopback)\b/gm)].map((m) => m[1]);
const GETTER_EXEMPT = {
  get_settings: "the blob itself",
  get_ai_config: "read-only: the CLI's ai_cmd, shown by the ai-command section, set only by hand",
  get_changelog: "the release notes, not a setting",
};
const getters = rows.filter((r) => /^get_/.test(r) || (/_settings$/.test(r) && !/^set_/.test(r)));
const fed = new Set(Object.values(M.HOST_GETTERS));
for (const g of getters) {
  if (!fed.has(g) && !GETTER_EXEMPT[g]) fail(`serve's getter \`${g}\` feeds no host value in settingsMap.ts — a value it reads could be changed and never shown`);
}
for (const g of fed) if (!rows.includes(g)) fail(`HOST_GETTERS names \`${g}\`, which serve has no row for`);
ok(`every policy getter serve has (${getters.length}) feeds a host value, or is exempt with a reason`);

// ── 4. the mode rules ───────────────────────────────────────────────────────
// Rule 1: hidden is never off — nothing that ACTS reads the mode.
{
  const MODE_READERS = new Set(["settings.ts", "settingsMap.ts", "SettingsSheet.tsx", path.join("pages", "data.tsx")]);
  const walk = (dir, out = []) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { if (e.name !== "mock") walk(p, out); } else if (/\.tsx?$/.test(e.name)) out.push(p);
    }
    return out;
  };
  const src = here("../src");
  const stray = walk(src).filter((f) => fs.readFileSync(f, "utf8").includes("settings_mode") && !MODE_READERS.has(path.relative(src, f)));
  if (stray.length) fail(`settings_mode is read outside the sheet (${stray.map((f) => path.relative(src, f)).join(", ")}) — a mode must only decide what Settings draws`);
  else ok("hidden is never off: only the sheet reads settings_mode");
}
const freeze = (o) => { if (o && typeof o === "object") { Object.freeze(o); Object.values(o).forEach(freeze); } return o; };
const ctx = (over = {}) => freeze({
  mode: "beginner", settings: structuredClone(D), defaults: structuredClone(D), host: {}, problems: new Set(),
  installed: { claude: true, codex: false, pi: false, kimi: false }, revealed: new Set(), onHost: true, ...over,
});
/** A value for `k` that is not its default. */
const other = (k) => {
  const v = D[k];
  if (typeof v === "boolean") return !v;
  if (typeof v === "number") return v + 1;
  if (typeof v === "string") return v + "x";
  if (v === null) return "terminal";
  if (Array.isArray(v)) return [...v, "x"];
  if (typeof v === "object") return { ...v, x: "y" };
  return "x";
};
let changedOk = 0, problemOk = 0;
for (const mode of M.MODES) {
  for (const s of M.SECTIONS) {
    const base = { mode, settings: { ...structuredClone(D), theme: "system" } };
    for (const k of s.keys ?? []) {
      const c = ctx({ ...base, settings: { ...base.settings, [k]: other(k) } });
      if (!M.sectionApplies(s, c)) continue;
      if (M.sectionWhy(s, c) === null) fail(`rule 2: ${s.id} is hidden in ${mode} with \`${k}\` changed`); else changedOk++;
    }
    for (const h of s.host ?? []) {
      const c = ctx({ ...base, host: { [h]: true } });
      if (M.sectionWhy(s, c) === null) fail(`rule 2: ${s.id} is hidden in ${mode} with host value \`${h}\` changed`); else changedOk++;
    }
    const pc = ctx({ ...base, problems: new Set([s.id]) });
    if (M.sectionApplies(s, pc)) {
      if (M.sectionWhy(s, pc) === null) fail(`rule 3: ${s.id} is hidden in ${mode} with a problem`); else problemOk++;
      const rc = ctx({ ...base, revealed: new Set([s.id]) });
      if (M.sectionWhy(s, rc) === null) fail(`a deep link to ${s.id} does not reveal it in ${mode}`);
    }
  }
}
ok(`rule 2: a changed setting is drawn in every mode (${changedOk} cases)`);
ok(`rule 3: a problem is drawn in every mode (${problemOk} cases), and a deep link reveals its section`);
for (const h of ["claude", "codex", "pi", "kimi"]) {
  for (const installed of [true, undefined]) {
    const c = ctx({ installed: { [h]: installed } });
    if (!M.pageListed(h, c)) fail(`rule 4: the ${h} page is not listed in Beginner with its CLI ${installed ? "installed" : "unknown"}`);
    for (const s of M.SECTIONS.filter((x) => x.page === h && x.tier === "beginner")) {
      if (M.sectionWhy(s, c) === null) fail(`rule 4: ${s.id} is hidden in Beginner with ${h} ${installed ? "installed" : "unknown"}`);
    }
  }
  if (M.pageListed(h, ctx({ installed: { [h]: false } }))) fail(`the ${h} page is listed in Beginner with its CLI absent and nothing changed`);
}
ok("rule 4: an installed (or not yet known) harness's page and its Beginner sections are always drawn");
{
  const beg = ctx();
  const hidden = M.SECTIONS.filter((s) => M.sectionApplies(s, beg) && M.sectionWhy(s, beg) === null);
  if (hidden.length === 0) fail("Beginner hides nothing — the tiers are not doing anything");
  const exp = ctx({ mode: "expert" });
  const still = M.SECTIONS.filter((s) => M.sectionApplies(s, exp) && M.sectionWhy(s, exp) === null);
  if (still.length) fail(`Expert hides ${still.map((s) => s.id).join(", ")}`);
  // The footer counts exactly what the next mode would add, page by page.
  let footOk = 0;
  for (const p of pageIds) {
    const own = M.SECTIONS.filter((s) => s.page === p && M.sectionApplies(s, beg) && M.sectionWhy(s, beg) === null);
    const f = M.hiddenBelow([p], beg);
    const want = own.filter((s) => M.effectiveTier(s, beg) === "advanced").length || own.length;
    if ((f?.count ?? 0) !== (own.length ? want : 0)) fail(`page ${p}: footer says ${f?.count ?? 0}, ${own.length} sections are hidden`);
    else footOk++;
  }
  // Nothing in the map writes to what it is handed (the contexts are frozen).
  for (const scope of M.RESET_SCOPES) M.resetPatch(scope.id, beg);
  if (!bad) ok(`Beginner hides ${hidden.length} sections, Expert none, and every page's footer counts them (${footOk} pages)`);
}

// ── 5. migrate rev 5, through the real loadSettings ─────────────────────────
for (const [blob, want, why] of [
  [null, "beginner", "a fresh install opens in Beginner"],
  [{}, "beginner", "an empty blob is a fresh install"],
  [{ theme: "nord", settings_rev: 4 }, "expert", "a rev-4 blob opens in Expert: nothing the user had is hidden"],
  [{ theme: "nord" }, "expert", "a blob from before any revision opens in Expert"],
  [{ settings_rev: 5, settings_mode: "advanced" }, "advanced", "a chosen mode is kept"],
  [{ settings_rev: 5, settings_mode: "beginner" }, "beginner", "Beginner, once chosen, is kept on an existing blob"],
  [{ settings_rev: 5, settings_mode: "wizard" }, "expert", "a junk mode draws as Expert, the one that hides nothing"],
]) {
  stored = blob;
  const s = await S.loadSettings();
  if (s.settings_mode !== want) fail(`${why}: stored ${JSON.stringify(blob)} loaded as ${s.settings_mode}`);
  else ok(why);
}
if (S.SETTINGS_REV !== 5) fail(`SETTINGS_REV is ${S.SETTINGS_REV}, expected 5`);

// ── 6. reset scope ──────────────────────────────────────────────────────────
{
  const c = ctx({ mode: "beginner" });
  const keys = Object.fromEntries(["look", "behavior", "memory", "silenced", "above", "everything"].map((s) => [s, new Set(M.resetKeys(s, c))]));
  const keep = new Set(M.RESET_KEEPS);
  for (const k of Object.keys(D)) {
    if (keep.has(k)) { if (keys.everything.has(k)) fail(`Everything resets \`${k}\`, which RESET_KEEPS keeps`); continue; }
    const in4 = ["look", "behavior", "memory", "silenced"].filter((s) => keys[s].has(k));
    if (in4.length !== 1) fail(`blob key \`${k}\` is in ${in4.length ? in4.join(" and ") : "no"} reset scope`);
  }
  for (const k of keys.everything) if (!(k in D)) fail(`Reset names \`${k}\`, which DEFAULTS does not have`);
  for (const k of [...M.PLACE_MEMORY_KEYS, "offers_dismissed", "init_dismissed", "update_dismissed"]) {
    if (keys.look.has(k)) fail(`the default reset scope (look) clears \`${k}\` — a theme reset must not take per-place memory or a silenced suggestion`);
  }
  const patch = M.resetPatch("everything", { ...c, defaults: D });
  for (const k of keep) if (k in patch) fail(`resetPatch writes \`${k}\`, which it must keep`);
  for (const [k, v] of Object.entries(patch)) if (JSON.stringify(v) !== JSON.stringify(D[k])) fail(`resetPatch sets \`${k}\` to something other than its default`);
  const above = M.resetKeys("above", c);
  const aboveWant = M.SECTIONS.filter((s) => M.effectiveTier(s, c) !== "beginner").flatMap((s) => s.keys ?? []);
  if (above.sort().join() !== aboveWant.sort().join()) fail("\"Settings above my mode\" is not exactly the keys of the sections the mode hides");
  if (M.resetKeys("above", ctx({ mode: "expert" })).length) fail("in Expert, \"above my mode\" resets something");
  if (!bad) ok(`reset scopes: four disjoint scopes cover the blob (${Object.keys(D).length - keep.size} keys), the default takes no memory, Everything is their union`);
}

// ── 7. one arm ──────────────────────────────────────────────────────────────
{
  const offenders = [];
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { if (e.name !== "mock") walk(p); continue; }
      if (!/\.tsx?$/.test(e.name) || e.name === "useArmed.ts") continue;
      const src = fs.readFileSync(p, "utf8");
      if (/setTimeout\(\(\) => set[A-Z]\w*Armed\(/.test(src) || /const (?:[A-Z_]+_)?ARM_MS = /.test(src) || /\bset[A-Z]?\w*Armed\(true\)/.test(src)) offenders.push(e.name);
    }
  };
  walk(here("../src"));
  if (offenders.length) fail(`a component keeps its own two-click arm (${offenders.join(", ")}) — use useArmed, whose arm expires`);
  else ok("every two-click arm is useArmed's");
}

// ── 8. grants ───────────────────────────────────────────────────────────────
{
  const ipc = read("../src/ipc.ts");
  const table = (name) => (ipc.match(new RegExp(`export const ${name}: readonly string\\[\\] = \\[([\\s\\S]*?)\\];`))?.[1] ?? "")
    .match(/"[a-z_0-9]+"/g)?.map((x) => x.slice(1, -1)) ?? [];
  const door = new Set([...table("ADMIN_ROWS"), ...table("CONSOLE_ROWS")]);
  for (const s of M.SECTIONS) {
    for (const [what, row] of [["read", s.read], ["write", s.write]]) {
      if (!row) continue;
      if (!rows.includes(row)) fail(`section ${s.id}'s ${what} row \`${row}\` is not a row in serve's dispatch table`);
      else if (!door.has(row)) fail(`section ${s.id}'s ${what} row \`${row}\` is in neither ADMIN_ROWS nor CONSOLE_ROWS — a remote window would never be told it may not`);
    }
  }
  if (!bad) ok("every row a section names for a remote window is one the remote door can refuse");
}

console.log(bad ? `\n${bad} failure(s)` : "\nall good");
process.exit(bad ? 1 : 0);

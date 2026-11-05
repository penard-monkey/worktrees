// Guards the Settings header's search (settingsSearch.ts) against the real map:
// the ranking the owner chose, the cap, the filter, and that every section can
// be found by its own label and by each of its keywords.
//
//   node app/scripts/settingssearch-check.mjs [settingsSearch.ts]   # non-zero on failure
//
// Both modules are import-free at runtime, so they load as data: modules, and
// the matcher is handed the REAL sections, pages and groups — a fixture map
// would test a ranking against words nobody types. The optional argument runs
// the check against another copy of the matcher (how its failures were shown
// red before it went green).
//
//   1. RANK ORDER: name prefix > label word > synonym > page/group/key >
//      substring, and sheet order within a rank — on queries whose answer
//      crosses every boundary, and as a property over many queries.
//   2. THE OWNER'S SYNONYMS land where they were meant to: "dark mode" →
//      Theme, "permissions" → Codex permissions, "cap agents" / "max agents" →
//      Limits, "font" → both font sections.
//   3. THE CAP: at most SEARCH_LIMIT rows, and the filter runs BEFORE it, so a
//      remote window's list is still full.
//   4. EDGES: an empty query, or one with no letters, names nothing; a short
//      query matches word starts only; the input is normalised.
//   5. FINDABLE: every section is in the list for its own label and for each
//      of its keywords — a keyword shared so widely that its section falls off
//      the end finds nothing.
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
const searchPath = process.argv[2] ? path.resolve(process.argv[2]) : here("../src/settingsSearch.ts");
const searchSrc = fs.readFileSync(searchPath, "utf8");
const mapSrc = read("../src/settingsMap.ts");
for (const [name, src] of [["settingsSearch.ts", searchSrc], ["settingsMap.ts", mapSrc]]) {
  if (/^import (?!type )/m.test(src)) fail(`${name} has a runtime import — it must load as a data: module, alone`);
}
const M = await load(mapSrc, "settingsMap.ts");
const S = await load(searchSrc, "settingsSearch.ts");

const index = S.searchIndex({ sections: M.SECTIONS, pages: M.PAGES, groups: M.GROUPS });
const find = (q, opts) => S.searchSettings(index, q, opts);
const ids = (q, opts) => find(q, opts).map((h) => h.id);
const eq = (got, want, what) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
};
const all = { limit: Infinity };
const sheetOrder = M.PAGES.flatMap((p) => M.SECTIONS.filter((s) => s.page === p.id).map((s) => s.id));

// ── 1. rank order ───────────────────────────────────────────────────────────
{
  const before = bad;
  if (index.map((d) => d.id).join() !== sheetOrder.join()) fail("the index is not in sheet order (the rail's, top to bottom)");
  // name prefix, in sheet order: Agents › Plan usage, Agent policy › Plan
  // limits, Projects › Planning — ahead of every synonym ("plan meter", "plan files").
  eq(find("plan", all).slice(0, 3).map((h) => [h.id, h.rank]),
    [["plan-usage", "name"], ["plan-limits", "name"], ["planning", "name"]], "\"plan\": the three labels that start with it come first");
  // label word > synonym: the four MCP servers' labels hold "server"; the
  // agents' Connections only has it as a keyword ("mcp server").
  eq(find("server", all).map((h) => [h.id, h.rank]),
    [["mcp-server", "word"], ["codex-mcp", "word"], ["pi-mcp", "word"], ["kimi-mcp", "word"], ["connections", "synonym"]],
    "\"server\": label words, then the synonym");
  // synonym > page/group label: three sections carry a keyword starting
  // "remote" (Git's "refresh remotes" among them, in sheet order first); Paired
  // devices only lives in Remote access › Remote.
  eq(find("remote", all).map((h) => [h.id, h.rank]),
    [["auto-fetch", "synonym"], ["this-window-drives", "synonym"], ["remote-access", "synonym"], ["devices", "context"]],
    "\"remote\": synonyms, then the page's other section");
  // context > substring: "keyboard" holds "board" mid-word only.
  eq(find("board", all).map((h) => [h.id, h.rank]), [["shortcuts", "substring"]], "\"board\": a substring, last");
  // words spread over the label and the page.
  if (!ids("claude policy").includes("claude-policy")) fail("\"claude policy\" does not find Claude's Agent policy (label + page)");
  if (ids("server mcp")[0] !== "mcp-server") fail("\"server mcp\": query words in any order should still rank the label first");
  // The property, over every label word and keyword in the map.
  const queries = new Set();
  for (const s of M.SECTIONS) for (const w of [...S.norm(s.label).split(" "), ...s.keywords]) if (w) queries.add(w);
  for (const q of queries) {
    const hits = find(q, all);
    for (let i = 1; i < hits.length; i++) {
      const [a, b] = [hits[i - 1], hits[i]];
      const ra = S.RANKS.indexOf(a.rank), rb = S.RANKS.indexOf(b.rank);
      if (ra > rb || (ra === rb && sheetOrder.indexOf(a.id) > sheetOrder.indexOf(b.id))) {
        fail(`"${q}": ${a.id} (${a.rank}) is listed above ${b.id} (${b.rank})`);
        break;
      }
    }
  }
  if (bad === before) ok(`rank order holds: name > word > synonym > context > substring, sheet order within (${queries.size} queries)`);
}

// ── 2. the owner's synonyms ─────────────────────────────────────────────────
{
  const before = bad;
  eq(ids("dark mode"), ["theme", "theme-pair"], "\"dark mode\"");
  eq(ids("permissions")[0], "codex-permissions", "\"permissions\" first");
  eq(ids("cap agents")[0], "limits", "\"cap agents\" first");
  eq(ids("max agents")[0], "limits", "\"max agents\" first");
  eq(ids("font").slice(0, 2), ["ui-font", "term-font"], "\"font\": both font sections, first");
  if (bad === before) ok("\"dark mode\" → Theme, \"permissions\" → Codex permissions, \"cap/max agents\" → Limits, \"font\" → both fonts");
}

// ── 3. the cap, and the filter before it ───────────────────────────────────
{
  const before = bad;
  if (S.SEARCH_LIMIT < 6 || S.SEARCH_LIMIT > 10) fail(`SEARCH_LIMIT is ${S.SEARCH_LIMIT}; the list is meant to hold about eight rows`);
  const wide = find("a", all);
  if (wide.length <= S.SEARCH_LIMIT) fail(`"a" names only ${wide.length} sections — the cap below is not exercised`);
  eq(find("a").length, S.SEARCH_LIMIT, "\"a\" is capped at SEARCH_LIMIT");
  const top = find("a").map((h) => h.id);
  const drop = new Set(top.slice(0, 3));
  const filtered = ids("a", { include: (id) => !drop.has(id) });
  if (filtered.some((id) => drop.has(id))) fail("an excluded section is still listed");
  eq(filtered.length, S.SEARCH_LIMIT, "the filter runs before the cap (the list is still full)");
  eq(filtered, wide.map((h) => h.id).filter((id) => !drop.has(id)).slice(0, S.SEARCH_LIMIT), "the filtered list is the unfiltered one minus the excluded");
  // The sheet's own rule: a host-only section never on a remote window.
  const remote = { include: (id) => M.sectionApplies(M.SECTION[id], { settings: { theme: "system" }, onHost: false }) };
  if (ids("restore window", remote).includes("restore-window")) fail("a remote window lists Restore window, which it can never draw");
  if (!ids("restore window").includes("restore-window")) fail("the window's own host does not list Restore window");
  if (bad === before) ok(`at most ${S.SEARCH_LIMIT} rows, filtered before the cap; a remote window never lists a host-only section`);
}

// ── 4. edges ────────────────────────────────────────────────────────────────
{
  const before = bad;
  eq(find(""), [], "an empty query");
  eq(find("   "), [], "a blank query");
  eq(find("↔ —"), [], "a query with no letters or digits");
  eq(find("zzqqxx"), [], "a query nothing matches");
  eq(ids("  Dark-MODE "), ids("dark mode"), "case, punctuation and spacing are normalised");
  eq(ids("light dark pair"), ids("Light ↔ dark pair"), "the label's own symbols are not needed to find it");
  eq(S.norm("term_scrollback"), "term scrollback", "a setting key is searched as words");
  if (ids("bo", all).includes("shortcuts")) fail("a 2-letter query matched mid-word (\"bo\" in keyboard): short queries are word starts only");
  eq(JSON.stringify(find("agent")), JSON.stringify(find("agent")), "the same query twice lists the same rows");
  const t = find("theme")[0];
  eq([t.id, t.label, t.where], ["theme", "Theme", "Workspace › Appearance"], "a row names its section and where it lives");
  eq(find("presets")[0].where, "Agent policy", "a page named like its group says so once");
  eq(find("revoke")[0].where, "Remote access › Remote", "Paired devices lives in Remote access › Remote");
  if (bad === before) ok("empty and symbol-only queries name nothing; input is normalised; short queries match word starts only");
}

// ── 5. findable ─────────────────────────────────────────────────────────────
{
  const before = bad;
  for (const s of M.SECTIONS) {
    if (!ids(s.label).includes(s.id)) fail(`"${s.label}" does not list its own section ${s.id} in the first ${S.SEARCH_LIMIT}`);
    for (const k of s.keywords) if (!ids(k).includes(s.id)) fail(`section ${s.id}'s keyword "${k}" does not list it in the first ${S.SEARCH_LIMIT} (shared too widely?)`);
  }
  if (bad === before) ok(`every section (${M.SECTIONS.length}) is listed for its label and for each of its ${M.SECTIONS.reduce((n, s) => n + s.keywords.length, 0)} keywords`);
}

console.log(bad ? `\n${bad} failure(s)` : "\nall good");
process.exit(bad ? 1 : 0);

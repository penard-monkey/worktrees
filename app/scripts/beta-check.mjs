// Holds the beta features (app/src/beta.ts) to their rules.
//
//  1. One feature, one flag, one checkbox (the owner, 2026-11-14): every
//     entry of BETA_FEATURES has its own `beta_<name>` key, default off and
//     normalised, its own helper, and its own checkbox, which Settings →
//     General draws FROM the table. The Settings map owns exactly the table's
//     keys. Nothing but the helper reads a key.
//  2. Ideas (#632): with its flag OFF the main window shows no lightbulb in the
//     rail and takes no ⌘⇧I, turning it off closes the Ideas window, and the
//     shim's launch rule reads the same key. The workspace view is drawn with
//     Ideas OR workspace chats on, and each of its parts asks its own helper:
//     Plan and Ideas are Ideas', Memory and Chats are workspace chats'. With
//     both off a workspace's name folds its card.
//  3. Named workspaces LEFT the beta on 2026-11-14: no workspace surface sits
//     behind a beta helper any more (the switch, the menus, the list's fetch,
//     the view), and the words from before "All projects" existed are gone.
//
//   node app/scripts/beta-check.mjs     # exits non-zero on failure
//
// Like the other *-check.mjs it slices the REAL sources: App.tsx's gates
// (each found by the element it guards, then walked to the brace that
// encloses it), and the real beta.ts / settings.ts loaded as data: modules,
// so the defaults and the helpers' answers are evaluated, not grepped.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const read = (rel) => fs.readFileSync(here(rel), "utf8");
let bad = 0;
const ok = (cond, m) => { if (cond) console.log(`ok   ${m}`); else { console.log(`not ok  ${m}`); bad++; } };
const load = async (src, name) => {
  const js = (await transformWithEsbuild(src, name, { loader: "ts", format: "esm" })).code;
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
};

// ── 1. the table and the helpers, evaluated ──
const betaSrc = read("../src/beta.ts");
ok(!/^import (?!type )/m.test(betaSrc), "beta.ts has no runtime import (it loads alone)");
const B = await load(betaSrc, "beta.ts");
const F = B.BETA_FEATURES ?? [];
ok(Array.isArray(F) && F.length >= 1, `BETA_FEATURES lists the beta features (${F.length})`);
const keys = F.map((f) => f.key);
ok(new Set(keys).size === keys.length, "…each with its OWN key: no two features share a flag");
ok(new Set(F.map((f) => f.testId)).size === F.length, "…and its own checkbox test id");
for (const f of F) {
  ok(/^beta_[a-z][a-z_]*$/.test(f.key) && typeof f.label === "string" && f.label.length > 0 && typeof f.what === "string" && f.what.length > 20,
    `${f.key}: a beta_<name> key, a label and a line on what it adds`);
  // Its own helper: a function of exactly that key, answering only a real true.
  const helper = Object.entries(B).find(([, v]) => typeof v === "function" && new RegExp(`\\bs\\.${f.key} === true\\b`).test(String(v)));
  ok(!!helper, `${f.key}: has its own helper in beta.ts`);
  if (helper) {
    const [name, fn] = helper;
    ok(fn({ [f.key]: true }) === true, `${name}: on when ${f.key} is true`);
    for (const v of [false, undefined, "yes", 1, null]) ok(fn({ [f.key]: v }) === false, `${name}: off for ${JSON.stringify(v)}`);
  }
}
ok(F.find((f) => f.key === "beta_workspace_chats")?.label === "Workspace chats" && /still being built/i.test(F.find((f) => f.key === "beta_workspace_chats")?.what ?? "")
  && B.workspaceChatsBeta({ beta_ideas: true }) === false && B.ideasBeta({ beta_workspace_chats: true }) === false,
  "Workspace chats is a beta of its own, says it is still being built, and is neither Ideas' flag nor Ideas its");
ok(keys.includes("beta_ideas") && F.find((f) => f.key === "beta_ideas")?.label === "Ideas",
  "Ideas is a beta feature of its own, labelled Ideas (no longer \"Ideas and workspaces\")");

// ── 2. every key: default off, normalised, owned by the Settings section ──
let stored = null;
globalThis.__betaInvoke = (cmd) => Promise.resolve(cmd === "get_settings" ? structuredClone(stored) : null);
const settingsTs = read("../src/settings.ts");
const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
const S = await load(
  inline("../src/afterglow.ts") + inline("../src/harness.ts") +
  settingsTs
    .replace(/^import \{[^}]*\} from "\.\/afterglow";$/m, "")
    .replace(/^import \{[^}]*\} from "\.\/harness";$/m, "")
    .replace(/^import \{ invoke \}.*$/m, "const invoke = (cmd, a) => globalThis.__betaInvoke(cmd, a);"),
  "settings.ts");
for (const k of keys) {
  ok(S.DEFAULTS[k] === false, `DEFAULTS.${k} is false`);
  for (const [blob, want, what] of [
    [null, false, "a fresh install"],
    [{ settings_rev: 5 }, false, "a blob from before the key"],
    [{ settings_rev: 5, [k]: "true" }, false, "a junk value"],
    [{ settings_rev: 5, [k]: true }, true, "a stored true"],
  ]) {
    stored = blob;
    const s = await S.loadSettings();
    ok(s[k] === want, `loadSettings ${k}: ${what} → ${want}`);
  }
}
const map = read("../src/settingsMap.ts");
const sec = map.match(/^\s*\{ id: "beta",[^\n]*\},$/m)?.[0] ?? "";
const mapKeys = (sec.match(/keys: \[([^\]]*)\]/)?.[1] ?? "").match(/"[a-z_]+"/g)?.map((x) => x.slice(1, -1)) ?? [];
ok(/page: "general"/.test(sec) && /tier: "beginner"/.test(sec), "settingsMap: the Beta features section is on General, Beginner (group chat start design S2)");
ok(JSON.stringify([...mapKeys].sort()) === JSON.stringify([...keys].sort()),
  `settingsMap: the section owns exactly the table's keys (${mapKeys.join(", ")} vs ${keys.join(", ")})`);
ok(/label: "Beta features"/.test(sec) && /"beta"/.test(sec) && /"ideas"/.test(sec), "settingsMap: it is labelled Beta features and found by beta / ideas");
const general = read("../src/pages/general.tsx");
const secJsx = general.slice(general.indexOf('<Sec id="beta">'), general.indexOf("</Sec>", general.indexOf('<Sec id="beta">')));
ok(/import \{ BETA_FEATURES \} from "\.\.\/beta";/.test(general) && /BETA_FEATURES\.map\(\(f\) =>/.test(secJsx),
  "pages/general.tsx: one checkbox per feature, drawn from BETA_FEATURES");
ok(/checked=\{settings\[f\.key\] === true\}/.test(secJsx) && /onChange\(\{ \[f\.key\]: e\.currentTarget\.checked \}\)/.test(secJsx),
  "…each reads and writes its own feature's key");
ok(!/beta_[a-z]/.test(secJsx), "…and no checkbox names a key by hand");

// ── 3. one helper per key: nothing outside it reads a key ──
const srcFiles = (dir) => fs.readdirSync(here(dir), { withFileTypes: true }).flatMap((e) =>
  e.isDirectory() ? srcFiles(`${dir}/${e.name}`) : /\.(tsx?|mts)$/.test(e.name) ? [`${dir}/${e.name}`] : []);
const ALLOWED = new Set(["../src/beta.ts", "../src/settings.ts", "../src/settingsMap.ts", "../src/mock/install.ts"]);
for (const k of keys) {
  const readers = srcFiles("../src").filter((f) => !ALLOWED.has(f) && new RegExp(`\\b${k}\\b`).test(read(f)));
  ok(readers.length === 0, `only beta.ts answers "is ${k} on" (read nowhere else: ${readers.join(", ") || "none"})`);
}

// ── 4. App.tsx: Ideas behind `ideasOn`, workspaces behind nothing ──
const app = read("../src/App.tsx");
const code = app.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " ")).replace(/\/\/[^\n]*/g, (m) => " ".repeat(m.length));
ok(/import \{[^}]*\bideasBeta\b[^}]*\} from "\.\/beta";/.test(app) && /const ideasOn = ideasBeta\(settings\);/.test(code), "App.tsx: ideasOn = ideasBeta(settings)");

/** The `{ideasOn && … (` or `{ideasOn && <x … />}` regions, start to the
 *  matching close. Comments are blanked first, so a paren in one cannot
 *  unbalance the walk. */
const regionsOf = (flag) => {
  const out = [];
  for (const m of code.matchAll(new RegExp(`\\{${flag} && `, "g"))) {
    let depth = 0, i = m.index;
    for (; i < code.length; i++) {
      const c = code[i];
      if (c === "{") depth++;
      else if (c === "}" && --depth === 0) break;
    }
    out.push([m.index, i]);
  }
  return out;
};
const regions = regionsOf("ideasOn"), frameRegions = regionsOf("frameOn");
const gatedFrame = (needle) => { const a = at(needle); return a.length > 0 && a.every((p) => frameRegions.some(([x, y]) => p > x && p < y)); };
const at = (needle) => [...code.matchAll(new RegExp(needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "g"))].map((m) => m.index);
const gated = (needle) => { const a = at(needle); return a.length > 0 && a.every((p) => regions.some(([x, y]) => p > x && p < y)); };
const ungated = (needle) => { const a = at(needle); return a.length > 0 && a.every((p) => !regions.some(([x, y]) => p > x && p < y)); };
// Ideas: still the beta's.
ok(gated('data-testid="ideas-rail"'), "the rail's lightbulb renders only with Ideas on");
const chord = code.match(/if \([^\n]*e\.code === "KeyI"\)\) \{/)?.[0] ?? "";
ok(/^if \(keyRef\.current\.ideasOn && /.test(chord), "⌘⇧I is taken only with Ideas on");
ok(/keyRef\.current = \{[\s\S]*?\bideasOn\b[\s\S]*?\};/.test(code), "…and keyRef carries ideasOn");
ok((code.match(/openIdeas\(\)|onClick=\{openIdeas\}/g) ?? []).length === 3, "openIdeas has exactly its three callers, the chord, the rail and openIdeaDoc");
// The third: Float, on a document open in the main area, opens it in the
// Ideas window, and returns first with Ideas off.
ok(/const openIdeaDoc = \(id: string\) => \{\s*if \(!ideasOn\) return;/.test(code), "…and openIdeaDoc opens nothing with Ideas off");
// The workspace view: drawn with Ideas or workspace chats on, its parts by
// their own helper (wsview.ts, run here for every combination).
ok(/const wsChatsOn = workspaceChatsBeta\(settings\);/.test(code) && /const frameOn = wsFrameOn\(\{ ideas: ideasOn, chats: wsChatsOn \}\);/.test(code), "App.tsx: frameOn = Ideas or workspace chats, each from its own helper");
const V = await load(read("../src/wsview.ts").replace(/^import type .*$/gm, ""), "wsview.ts");
for (const [b, frame, tabs, chips] of [
  [{ ideas: false, chats: false }, false, "overview", ""],
  [{ ideas: true, chats: false }, true, "overview,plan,ideas", "plan,ideas"],
  [{ ideas: false, chats: true }, true, "overview,memory,chats", "memory"],
  [{ ideas: true, chats: true }, true, "overview,memory,plan,ideas,chats", "memory,plan,ideas"],
]) ok(V.wsFrameOn(b) === frame && V.wsTabs(b).join() === tabs && V.docChips(b).join() === chips, `Ideas ${b.ideas ? "on" : "off"}, chats ${b.chats ? "on" : "off"}: the view ${frame ? `has ${tabs}` : "is not drawn"}, the nav row ${chips || "nothing"}`);
ok(/const tabsOn = wsTabs\(\{ ideas: ideasOn, chats: wsChatsOn \}\);\s*const wsTabNow = tabOr\(wsTab, tabsOn\);/.test(code) && /tabs=\{tabsOn\}\s*tab=\{wsTabNow\}/.test(code), "its tabs are those, and a tab that is not drawn is never the one shown");
ok(/const wsShown = frameOn && !\(selected && sel\) && wsOpen \? wsById\(wsOpen\) : null;/.test(code) && /shown=\{wsShown !== null\}/.test(code), "the view is shown with either on, and with both off never");
ok(/const showWs = async \([\s\S]{0,160}?\{\s*if \(!frameOn \|\| id === EVERYTHING_ELSE_ID\) return false;/.test(code) && /const openWsOverview = \(id: string\) => \{ void showWsRef\.current\(id, "overview", \{ chat: null \}\); \};/.test(code), "showWs opens nothing with both off, nor for Everything Else, and the overview goes through it");
ok(/onOpen=\{frameOn \? \(\) => openWsOverview\(w\.id\) : undefined\}/.test(code) && /onClick=\{onOpen \?\? onToggle\}/.test(code), "with both off a workspace's name folds its card, as before phase 4");
ok(gatedFrame('data-testid="ws-overview-open"') && gatedFrame("<DocChips") && /chips=\{docChips\(\{ ideas: ideasOn, chats: wsChatsOn \}\)\.map/.test(code), "a workspace menu's Overview and its card's documents row render only with the view on, the row's chips by their own helpers");
const overview = read("../src/WorkspaceOverview.tsx");
ok(/const work = props\.ideasOn && <WorkCard/.test(overview) && /\{props\.ideasOn && <IdeasCard/.test(overview) && (overview.match(/<WorkCard|<IdeasCard/g) ?? []).length === 2, "the Overview's Work and Ideas are Ideas' alone");
ok(/\{props\.chatsOn\s*\? <Card name="Memory"/.test(overview) && /\{props\.chatsOn && <Card name="Chats"/.test(overview) && (overview.match(/<Card name="(Memory|Chats)"/g) ?? []).length === 2, "…its Memory and Chats are workspace chats' alone");
ok(/<WorkspaceOverview[\s\S]{0,200}?ideasOn=\{ideasOn\}\s*chatsOn=\{wsChatsOn\}/.test(code), "…and it is told each by its own");
// Chats are workspace chats' alone, wherever they show.
ok(/const wsChats = useWsChats\(wsChatsOn && !!wsList,/.test(code) && /if \(!on\) return;\s*for \(const id of key/.test(read("../src/useWsChats.ts")), "workspace_chats is asked for, and workspaces:sessions heard, only with workspace chats on");
ok(/\{wsChatsOn && \(\s*<NavChats/.test(code) && (code.match(/<NavChats\b/g) ?? []).length === 1, "a card's Chats group is drawn only with it on");
ok(/const wsMiddle = !wsShown \|\| !wsChatsOn \? null/.test(code) && (code.match(/<WorkspaceChatColumn\b/g) ?? []).length === 1 && /newChat=\{wsShown && wsChatsOn && /.test(code), "…and so are the middle column and New chat");
ok(/waits=\{wsChatsOn && chatWaits\}/.test(code) && /\.\.\.\(wsChatsOn \? wsChats\.rows\(w\.id\) : \[\]\)\.map/.test(code), "…and the switch's dot and ⌘K's chats");
// A document opens in the view's tabs. The pane may stay mounted behind the
// gate (a text that could not be saved waits in it), so what is held is that it
// SHOWS and OPENS nothing.
ok(/const openDocHere = \(id: string, startIn\?: "edit"\) => \{\s*if \(!ideasOn \|\| !wsOpen\) return;/.test(code), "openDocHere opens nothing with Ideas off");
ok(/const docShown = wsShown !== null && mainDoc !== null && wsShown\.id === mainDoc\.ws && wsTabNow === mainDoc\.tab && !\(memFile && memFile\.kind !== "text"\);/.test(code) && /<MainDocument\s[\s\S]{0,160}?shown=\{docShown\}/.test(code), "a document in the main area is shown only on its own tab, which its beta draws");
ok(/const docBeta = !mainDoc \|\| \(mainDoc\.source === ideaSource \? ideasOn : wsChatsOn\);/.test(code) && /if \(!docBeta \|\| !docWs\) void leaveMainDoc\(\);/.test(code), "…and is closed, once saved, when ITS beta goes off (Ideas' documents, or a memory's files)");
ok(/const wsMem = useWsMemory\(wsChatsOn && !!wsList,/.test(code) && /memory=\{!wsChatsOn \? null/.test(code), "workspace_memory is asked for, and the Overview's memory drawn, only with workspace chats on");
ok(/if \(to\.source === ideaSource\) \{\s*if \(!ideasOn\)/.test(code), "an idea: link in a memory file opens nothing with Ideas off");
ok((code.match(/<MainDocument\b/g) ?? []).length === 1 && (code.match(/setMainDoc\(\{/g) ?? []).length === 1, "…drawn in one place, and opened by showWs alone");
ok((code.match(/<WorkspaceFrame\b/g) ?? []).length === 1 && (code.match(/<WorkspaceOverview\b/g) ?? []).length === 1, "the view is drawn in one place");
// One parent for the document's life: the frame is mounted while a document is
// open, whatever is shown, and the pane is the last child of its one slot.
ok(/\{\(wsShown \|\| mainDoc\) && \(\s*<WorkspaceFrame\b/.test(code), "the frame stays mounted while a document is open, whatever is shown");
{
  const frame = read("../src/WorkspaceFrame.tsx");
  ok((frame.match(/\{props\.doc\}/g) ?? []).length === 1 && /\{props\.doc\}\s*<\/div>\s*<\/div>\s*<\/div>\s*<\/div>\s*<\/section>/.test(frame) && /<section className="wsf"[^>]*hidden=\{!props\.shown\}>/.test(frame) && !/key=\{(props\.)?ws/.test(frame),
    "…and the document's section has one parent there, hidden and never keyed to a workspace or a tab");
}
ok(/if \(was === true && !ideasOn\) \{\s*invoke\("close_companion"\)/.test(code), "turning Ideas off closes the Ideas window (close_companion)");
// Workspaces: out of the beta (2026-11-14).
ok(ungated("<NavViewSwitch"), "the Workspaces | All projects switch is behind no beta");
ok(ungated('testId="ctx-ws-add"') && ungated('data-testid="ctx-ws-remove"'), "a project's Add to workspace and Remove from workspace are behind no beta");
ok(/const navView: NavView = wsList && settings\.nav_view === "workspaces"/.test(code), "the view follows nav_view once the list has loaded, whatever the beta says");
ok(/navView === "workspaces" \? workspacesNav\(\)/.test(code), "the Workspaces view is drawn through navView");
const eff = code.match(/useEffect\(\(\) => \{\s*let alive = true;\s*invoke<WsList>\("workspaces_list"\)[\s\S]*?\}, \[wsRegKey\]\);/)?.[0] ?? "";
ok(eff.length > 0, "workspaces_list is fetched with no beta in its way (its effect keys on the registry alone)");
// The overview's own read (Ideas) keys on the list, so a changed plan is read
// again; it never writes it. That one effect apart, no line ties the two.
const ovEff = code.match(/useEffect\(\(\) => \{\s*if \(!ideasOn \|\| !wsOpen\) \{ setWsOv\(null\); return; \}[\s\S]*?\}, \[ideasOn, wsOpen, wsList, ideasTick\]\);/)?.[0] ?? "";
ok(/"workspace_overview"/.test(ovEff) && !/setWsList|workspaces_list/.test(ovEff), "the overview's effect reads workspace_overview and leaves the workspace list alone");
ok(!/ideasOn[^\n;]*\bwsList\b|\bwsList\b[^\n;]*ideasOn/.test(code.replace(ovEff, "")), "no line ties the workspace list to ideasOn");
// The words from before "All projects" existed are gone.
ok(/: "Remove from All projects"\}/.test(code) && !/ideasOn \? "Remove from All projects"/.test(code), "the project menu always says Remove from All projects");
ok(/"click again to remove from All projects" : "remove project"/.test(code) && !/"click again to remove from workspace"/.test(code), "the ✕'s armed tooltip always says remove from All projects");

// ── 5. the shim and the route tables (Ideas) ──
const lib = read("../src-tauri/src/lib.rs");
ok(/dev_open \|\| companion::reopens_at_launch\(&config_dir, /.test(lib), "lib.rs: the companion reopens at launch through companion::reopens_at_launch");
ok(/async fn close_companion\(/.test(lib) && /^\s+close_companion,$/m.test(lib), "lib.rs: close_companion is a shim command, registered");
const comp = read("../src-tauri/src/companion.rs");
ok(/pub const BETA: &str = "beta_ideas";/.test(comp), "companion.rs reads the same key");
ok(/"close_companion",/.test(read("../src/ipc.ts").match(/export const SHIM_COMMANDS[\s\S]*?\];/)?.[0] ?? ""), "ipc.ts: close_companion is in SHIM_COMMANDS");
const mock = read("../src/mock/install.ts");
ok(/case "close_companion":/.test(mock), "the mock answers close_companion");
ok(/get\("beta"\)\?\.split\(","\)\.includes\("ideas"\)/.test(mock) && /beta_ideas: true/.test(mock), "the mock: off by default, ?beta=ideas turns Ideas on");

if (bad) { console.log(`\n${bad} failed`); process.exit(1); }
console.log("\nbeta-check: all ok");

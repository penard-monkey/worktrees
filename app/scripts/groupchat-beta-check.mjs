// Holds group chat to its beta switch (beta.ts, `groupChatBeta`; the owner's
// decision of 2026-11-14: "merge staging to main and get the changes queued
// for a beta feature").
//
//  1. Every owner surface is behind it: the lane's Share bar and thread
//     (App.tsx), the corner's join asks and the `shares:changed` watch
//     (EngineGate's RemoteLayer), and Settings → Remote → Guests, whose
//     section applies only with the switch on (settingsMap) — the invite path
//     (`PairSheet guest=`) has no other caller.
//  2. Every guest surface is behind it: no guest connection at all while it is
//     off (main.tsx starts the guests only from the live switch, and stops
//     them when it goes off), and the nav's David's Places, Shared with me,
//     the toasts and the guest view (App.tsx).
//  3. Turning it off mid-session tears the guests down properly: every link
//     closed, the source forgotten, the open view closed, subscribers told.
//     A read in flight when it goes off adopts nothing.
//
//   node app/scripts/groupchat-beta-check.mjs     # exits non-zero on failure
//
// Like beta-check.mjs it slices the real sources, comments blanked first.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const read = (rel) => fs.readFileSync(here(rel), "utf8");
let bad = 0;
const ok = (cond, m) => { if (cond) console.log(`ok   ${m}`); else { console.log(`not ok  ${m}`); bad++; } };
const blank = (src) => src.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " ")).replace(/(^|[^:"'`])\/\/[^\n]*/g, (m, p) => p + " ".repeat(m.length - p.length));

/** The `{flag && …}` regions of `code`, start to the matching close. */
function regions(code, flag) {
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
}
const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const gatedIn = (code, flag, needle) => {
  const rs = regions(code, flag);
  const at = [...code.matchAll(new RegExp(esc(needle), "g"))].map((m) => m.index);
  return at.length > 0 && at.every((p) => rs.some(([x, y]) => p > x && p < y));
};

// ── 0. the feature, its key and its helper ──
const betaSrc = read("../src/beta.ts");
const B = await import("data:text/javascript;base64," + Buffer.from((await transformWithEsbuild(betaSrc, "beta.ts", { loader: "ts", format: "esm" })).code).toString("base64"));
const f = (B.BETA_FEATURES ?? []).find((x) => x.key === "beta_group_chat");
ok(!!f && f.label === "Group chat", "beta.ts: Group chat is a beta feature of its own (beta_group_chat)");
ok(typeof B.groupChatBeta === "function" && B.groupChatBeta({ beta_group_chat: true }) === true && B.groupChatBeta({}) === false,
  "beta.ts: groupChatBeta answers the switch");

// ── 1. owner surfaces ──
const app = blank(read("../src/App.tsx"));
ok(/import \{[^}]*\bgroupChatBeta\b[^}]*\} from "\.\/beta";/.test(app) && /const groupChatOn = groupChatBeta\(settings\);/.test(app),
  "App.tsx: groupChatOn = groupChatBeta(settings)");
ok(/useEffect\(\(\) => publishGroupChat\(groupChatOn\), \[groupChatOn\]\);/.test(app),
  "App.tsx: the switch is published to the windows' other surfaces (betaLive)");
for (const el of ["<LaneShareBar ", "<LaneThread "]) ok(gatedIn(app, "groupChatOn", el), `App.tsx: ${el.trim()} renders only with group chat on`);

const gate = blank(read("../src/EngineGate.tsx"));
ok(/const groupChat = useGroupChat\(\);/.test(gate), "EngineGate.tsx: RemoteLayer reads the live switch");
ok(/const asks = groupChat && /.test(gate), "…the corner shows join asks only with it on");
ok(gatedIn(gate, "groupChat", "<ShareAlerts />"), "…and ShareAlerts renders only with it on");
ok(/useEffect\(\(\) => \(groupChat \? watchShares\(\) : undefined\), \[groupChat\]\);/.test(gate),
  "…and shares:changed is watched only with it on, and unwatched when it goes off");
ok(!/useEffect\(\(\) => watchShares\(\), \[\]\);/.test(gate), "…no unconditional watchShares");

const map = read("../src/settingsMap.ts");
const guests = map.match(/^\s*\{ id: "guests",[^\n]*\},$/m)?.[0] ?? "";
ok(/applies: \(s, c\) => c\.groupChat\?\.\(s\) === true/.test(guests), "settingsMap: Settings → Remote → Guests applies only with group chat on, by the helper handed in");
ok(/groupChat: groupChatBeta\b/.test(read("../src/SettingsSheet.tsx")), "…and Settings hands it beta.ts's groupChatBeta (beta.ts: surfaces ask the helper)");
const pairCallers = fs.readdirSync(here("../src"), { recursive: true })
  .filter((p) => /\.tsx?$/.test(p) && !p.startsWith("mock"))
  .filter((p) => /<PairSheet[^>]*\bguest=/.test(read(`../src/${p}`)));
ok(JSON.stringify(pairCallers) === JSON.stringify(["GuestsPanel.tsx"]), `the invite path (PairSheet guest=) has one caller, the gated Guests section (${pairCallers.join(", ")})`);

// ── 2. guest surfaces ──
for (const el of ["<GuestUnder ", "<SharedWithMe ", "<GuestSpace />", "<GuestToasts />"]) ok(gatedIn(app, "groupChatOn", el), `App.tsx: ${el.replace(/ ?\/?>?$/, "")} renders only with group chat on`);
const main = blank(read("../src/main.tsx"));
const starts = [...main.matchAll(/startGuests\(/g)].length;
ok(starts === 1 && /onGroupChat\(\(on\) => \{?\s*if \(on\) startGuests\(/.test(main), "main.tsx: the guests start only from the live switch (no guest connection while it is off)");
ok(/else stopGuests\(\)/.test(main), "main.tsx: …and stop when it goes off");

// ── 3. the teardown ──
const guest = blank(read("../src/guest.ts"));
const stop = guest.match(/export function stopGuests\(\)[^{]*\{[\s\S]*?\n\}/)?.[0] ?? "";
ok(/for \(const l of links\.values\(\)\) l\.close\(\);/.test(stop) && /links\.clear\(\);/.test(stop), "guest.ts: stopGuests closes every link");
ok(/source = null;/.test(stop), "…forgets the source, so a later on starts again");
ok(/set\(\(\) => \(\{ hosts: \[\], toasts: \[\], open: null \}\)\)/.test(stop), "…and closes the open view through set(), so subscribers re-render");
ok(/export function resetGuestsForTest\(\) \{\s*stopGuests\(\);/.test(guest), "resetGuestsForTest is the same teardown");
const refresh = guest.match(/export async function refreshGuestHosts\(\)[\s\S]*?\n\}/)?.[0] ?? "";
ok(/const src = source;/.test(refresh) && /if \(source !== src\) return;/.test(refresh), "refreshGuestHosts: a read in flight when the switch goes off adopts nothing");

const live = fs.existsSync(here("../src/betaLive.ts")) ? read("../src/betaLive.ts") : "";
ok(/groupChatBeta\(/.test(live) && !/beta_group_chat/.test(live), "betaLive.ts asks the helper, never the key");

// ── 4. the harness: off by default, ?beta=group_chat turns it on ──
const mock = read("../src/mock/install.ts");
ok(/get\("beta"\)\?\.split\(","\)\.includes\("group_chat"\)/.test(mock) && /beta_group_chat: true/.test(mock), "the mock: off by default, ?beta=group_chat turns group chat on");
ok(/\?guest&beta=group_chat/.test(read("guest-thread-browser.mjs")), "guest-thread-browser drives the guest view with the switch on");

if (bad) { console.log(`\n${bad} failed`); process.exit(1); }
console.log("\ngroupchat-beta-check: all ok");

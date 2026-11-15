// Holds group chat's start design A (docs/proposals/group-chat-start.md §3.1,
// §4) to its security words, in the real sources:
//
//  1. Allow and Share stay two consents. The pairing window's one
//     implementation (PairWindow.tsx) never shares; the share dialog's Pair
//     step (`PairStep`) never calls `share_create` nor draws the share step,
//     and the one `share_create` in the dialog is `ShareStep`'s. Allow moves
//     the dialog to the Share step; it presses nothing there.
//  2. The pairing window has ONE implementation: PairSheet and the share
//     dialog both use `usePairWindow`, and nothing else calls the pairing
//     rows; closing either cancels a window still open.
//  3. The reach block turns on nothing by itself: `remote_enable` is called
//     only from the press that says so ("Turn on and show the link"), and the
//     window opens on its own only where remote access is listening already.
//     Its default is the first overlay candidate, else the first LAN one.
//  4. A join with the beta off is refused before any call (S5).
//  5. The entry points are behind the beta: *Share this session…* in the
//     header's ⋯ menu and the place's context menu, Home's *Join with a
//     link…*, and the dialog's host.
//  6. F7: the dialog's same-name rule is the host's (trim + lowercase).
//
//   node app/scripts/groupchat-start-check.mjs     # exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const read = (rel) => fs.readFileSync(here(rel), "utf8");
let bad = 0;
const ok = (cond, m) => { if (cond) console.log(`ok   ${m}`); else { console.log(`not ok  ${m}`); bad++; } };
const blank = (src) => src.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " ")).replace(/(^|[^:"'`])\/\/[^\n]*/g, (m, p) => p + " ".repeat(m.length - p.length));

/** The text of `function <name>(` up to the next top-level `function`/`export`. */
function fnBody(src, name) {
  const at = src.search(new RegExp(`\\n(?:export )?function ${name}\\b`));
  if (at < 0) return "";
  const rest = src.slice(at + 1);
  const next = rest.slice(1).search(/\n(?:export )?(?:async )?function |\n\/\*\* |\nexport (?:const|type|let) /);
  return next < 0 ? rest : rest.slice(0, next + 1);
}

const lane = blank(read("../src/LaneShare.tsx"));
const win = blank(read("../src/PairWindow.tsx"));
const sheet = blank(read("../src/PairSheet.tsx"));

// ── 1. two consents ──
ok(!/share_create|share_again|ShareStep/.test(win), "PairWindow.tsx never shares (no share_create, no ShareStep)");
const pair = fnBody(lane, "PairStep");
const share = fnBody(lane, "ShareStep");
const dialog = fnBody(lane, "ShareDialog");
ok(pair.length > 200 && share.length > 200 && dialog.length > 200, "LaneShare.tsx: ShareDialog, PairStep and ShareStep are found");
ok(!/share_create|<ShareStep/.test(pair), "PairStep never calls share_create nor draws the share step");
ok(/invoke\("share_create"/.test(share), "ShareStep calls share_create");
ok([...lane.matchAll(/"share_create"/g)].length === 1, "…and it is the one share_create in LaneShare.tsx");
const onPaired = dialog.slice(dialog.indexOf("onPaired={(rec) => {"), dialog.indexOf("}} />", dialog.indexOf("onPaired={(rec) => {")));
ok(onPaired.length > 20 && /setStep\("share"\)/.test(onPaired) && !/share_create|invoke\(/.test(onPaired),
  "Allow moves the dialog to the Share step and shares nothing (a separate press)");
ok(/data-testid="share-pair-allow"[^>]*onClick=\{\(\) => void allow\(\)\}/.test(pair) && /const rec = await w\.decide\(true\);\s*if \(rec\) onPaired\(rec\);/.test(pair),
  "PairStep's Allow is pair_decide and the hand-off, nothing else");
ok(/It shares nothing yet/.test(win), "the Allow screen says it shares nothing yet (§4)");

// ── 2. one window ──
ok(/usePairWindow\(\)/.test(sheet) && /usePairWindow\(\)/.test(dialog), "PairSheet and ShareDialog both use usePairWindow");
const pairRows = /\binvoke(?:<[^>()]*>)?\(\s*"pair_(begin|decide|status|cancel)"/;
for (const f of fs.readdirSync(here("../src"), { recursive: true }).filter((p) => /\.tsx?$/.test(p) && !p.startsWith("mock"))) {
  if (f === "PairWindow.tsx") continue;
  const src = blank(read(`../src/${f}`));
  if (pairRows.test(src)) ok(false, `${f} calls a pairing row itself: the window has one implementation (PairWindow.tsx)`);
}
ok(/useEffect\(\(\) => \(\) => \{ if \(liveRef\.current\) void invoke\("pair_cancel"\)/.test(win), "usePairWindow cancels a live window on unmount");
ok(/const close = \(\) => \{\s*w\.release\(\);\s*onClose\(\);/.test(dialog) && /w\.release\(\);/.test(sheet), "closing the dialog or the sheet releases (cancels) the window");

// ── 3. reach ──
const enables = [...lane.matchAll(/invoke\("remote_enable"/g)].length;
const turnOn = pair.match(/const turnOn = async \(\) => \{[\s\S]*?\n {2}\};/)?.[0] ?? "";
ok(enables === 1 && /invoke\("remote_enable"/.test(turnOn), "remote_enable is called once, in turnOn");
ok(/data-testid="share-reach-on"[^>]*onClick=\{\(\) => void turnOn\(\)\}>\s*\{turning \? "Turning on…" : "Turn on and show the link"\}/.test(pair),
  "…and turnOn runs only from the press that says \"Turn on and show the link\"");
ok(/r\.status\?\.state !== "listening"/.test(pair) && /void w\.begin\(args\);/.test(pair), "the window opens on its own only while remote access is listening");
const L = await import("data:text/javascript;base64," + Buffer.from((await transformWithEsbuild(
  read("../src/LaneShare.tsx").match(/export function reachDefault[\s\S]*?\n\}/)[0] + "\n" + read("../src/LaneShare.tsx").match(/export function sameGuestName[\s\S]*?\n\}/)[0],
  "x.ts", { loader: "ts", format: "esm" })).code).toString("base64"));
const c = (address, mode, cls = mode === "overlay" ? "cgnat" : "rfc1918") => ({ address, mode, class: cls, iface: "en0", prefix: 24, warning: null });
ok(L.reachDefault([c("10.0.0.2", "lan"), c("100.64.1.2", "overlay")])?.address === "100.64.1.2", "reachDefault: an overlay first");
ok(L.reachDefault([c("169.254.1.1", "lan", "link-local"), c("10.0.0.2", "lan")])?.address === "10.0.0.2", "reachDefault: else the first LAN one, never link-local");
ok(L.reachDefault([]) === null, "reachDefault: nothing to offer, nothing chosen");

// ── 4. beta off ──
const join = blank(read("../src/JoinHost.tsx"));
const joinFn = join.match(/const join = async \(\) => \{[\s\S]*?\n {2}\};/)?.[0] ?? "";
ok(/if \(!groupChat\) \{ setRefused\(true\); return; \}/.test(joinFn) && joinFn.indexOf("setRefused(true)") < joinFn.indexOf("remote_pair"),
  "a join with the beta off is refused before remote_pair is called (S5)");
ok(/Turn on Group chat in Settings → General → Beta features first/.test(read("../src/JoinHost.tsx")), "…in the owner's words");

// remote_pair is a shim command: its events need the local door's channel.
for (const f of ["../src/JoinHost.tsx", "../src/HostsPanel.tsx"]) {
  const src = blank(read(f));
  ok(/new ShimChannel<PairEv>\(/.test(src) && !/new Channel<PairEv>\(/.test(src), `${f.slice(7)}: remote_pair's events come on a ShimChannel`);
}

// ── 5. entry points ──
const app = blank(read("../src/App.tsx"));
for (const [what, re] of [
  ["the header's Share this session…", /\{groupChatOn && shareStart\(sel\.repo, sel\.slug\) && \(\s*<button className="pop-item" data-testid="topbar-share"/],
  ["the place menu's Share this session…", /\{groupChatOn && shareStart\(ctx\.repo, ctxPlace\.slug\) && \(\s*<button className="pop-item" data-testid="ctx-share"/],
  ["Home's Join with a link…", /\{groupChatOn && \(\s*<button className="ctrl home-import" data-testid="join-home"/],
  ["the dialog's host", /\{groupChatOn && <ShareDialogHost \/>\}/],
  ["the join dialog", /\{groupChatOn && joining && <JoinDialog /],
]) ok(re.test(app), `App.tsx: ${what} is behind groupChatOn`);
ok(/laneLive\(lanes, repo, slug\) === true && sharesOf\(s, repo, slug\)\.length === 0/.test(lane), "useShareStart: a live Claude or Codex, not shared yet (the bar's rule)");

// ── 6. F7 ──
ok(L.sameGuestName("Bret", " bret ") && !L.sameGuestName("Bret", "Brett"), "sameGuestName: trim + lowercase, as the host's same_guest_name");
ok(/a\.trim\(\)\.to_lowercase\(\) == b\.trim\(\)\.to_lowercase\(\)/.test(read("../../crates/worktrees-cli/src/serve/devices.rs")), "…which is the host's rule (devices.rs)");

if (bad) { console.log(`\n${bad} failed`); process.exit(1); }
console.log("\ngroupchat-start-check: all ok");

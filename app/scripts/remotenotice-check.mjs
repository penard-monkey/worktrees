// Guards where the notice that remote access is on lives (§4.1), and what it
// says.
//
//   node app/scripts/remotenotice-check.mjs     # exits non-zero on failure
//   SRC_App_tsx=<path> …   # another copy of any app/src file read here
//                          # (SRC_<its name, every non-word character as _>)
//
// WHY THIS EXISTS. The notice is App's footer item (`<footer
// className="statusbar">`), on a place AND on Home, and it no longer says
// "— quitting Worktrees turns it off" (Settings → Remote access says what
// quitting does). It used to be a fixed pill in EngineGate's bottom-right
// corner, `.remote-corner`, which §5.3's alerts still use. Nothing that runs
// in CI renders either placement, and the two ways back to the old shape both
// look fine in the harness:
//
//   1. Back in the corner, it renders as convincingly as in the footer.
//   2. In a footer gated only on what the footer USED to carry (the usage
//      meter, the status chip), it renders nowhere at all for the default
//      `usage_place: "strip"`: a notice that is "in the footer" and on no
//      screen, while §4.1 says it stays for as long as remote access is on.
//
// So this pins: the words (evaluated from devices.ts, the one source), that
// RemoteLayer's corner holds no notice, that every RemoteNotice in App sits
// inside a `.statusbar` footer whose own condition names the notice, and that
// Home has one. Slice-the-real-source, as race-check.mjs and offers-check.mjs.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// vite's esbuild re-export — see race-check.mjs: bare "esbuild" does not resolve
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
/** An app/src file, or the copy `SRC_<its name, . as _>` names. */
const src = (file) => fs.readFileSync(process.env[`SRC_${file.replace(/\W/g, "_")}`] ?? here(`../src/${file}`), "utf8");

let failed = 0;
let passed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) { passed++; return; }
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};

// ── 1. the words: devices.ts's remoteNoticeWords, evaluated ────────────────
// devices.ts imports nothing at runtime (devices-check.mjs loads it the same way).
const D = await import(
  "data:text/javascript;base64," +
    Buffer.from((await transformWithEsbuild(src("devices.ts"), "devices.ts", { loader: "ts", format: "esm" })).code).toString("base64")
);
eq("devices.ts has remoteNoticeWords", typeof D.remoteNoticeWords, "function");
const words = (st) => (typeof D.remoteNoticeWords === "function" ? D.remoteNoticeWords(st) : "(missing)");
const registered = { state: "registered", host: "wt-x.local" };
const CASES = [
  [{ state: "listening", advert: registered }, "Remote access is on · findable on this network"],
  [{ state: "listening", advert: { state: "off" } }, "Remote access is on"],
  [{ state: "listening", advert: { state: "not-advertised", why: "overlay" } }, "Remote access is on"],
  [{ state: "paused", advert: registered }, "Remote access is paused"],
  [{ state: "error" }, "Remote access is on, not listening"],
  [{ state: "starting" }, "Remote access is on, not listening"],
  [{ state: "off" }, null],
  [{ state: "unreadable" }, null],
];
for (const [st, want] of CASES) {
  for (const owner of ["app", null]) {
    eq(`words for ${st.state}${st.advert ? ` (advert ${st.advert.state})` : ""}, owner ${owner}`, words({ ...st, owner }), want);
  }
}
eq("no status, no words", words(null), null);

// ── 2. EngineGate.tsx: the corner holds the alerts, never the notice ───────
const gate = src("EngineGate.tsx");
// esbuild drops comments, so what is left is code and the words it renders.
const gateCode = (await transformWithEsbuild(gate, "EngineGate.tsx", { loader: "tsx", jsx: "preserve" })).code;
eq("EngineGate renders nothing about quitting (Settings says what quitting does)", /quit/i.test(gateCode), false);
const fnBody = (text, name) => {
  const i = text.indexOf(`export function ${name}(`);
  return i < 0 ? "" : text.slice(i, text.indexOf("\n}\n", i));
};
const layer = fnBody(gate, "RemoteLayer");
eq("RemoteLayer is found", layer.length > 0, true);
eq("RemoteLayer's corner renders no RemoteNotice", /<RemoteNotice\b/.test(layer), false);
const notice = fnBody(gate, "RemoteNotice");
eq("RemoteNotice is exported for App's footer", notice.length > 0, true);
eq("RemoteNotice's words are remoteNoticeWords(st), nothing added", /<span className="remote-notice-txt">\{remoteNoticeWords\(st\)\}<\/span>/.test(notice), true);
eq("RemoteNotice still opens Settings → Remote access", /onClick=\{\(\) => openRemoteSettings\("remote"\)\}/.test(notice), true);

// ── 3. App.tsx: every notice is in a .statusbar footer that exists for it ──
const app = src("App.tsx");
const FOOTER = '<footer className="statusbar">';
const footers = [];
for (let i = app.indexOf(FOOTER); i >= 0; i = app.indexOf(FOOTER, i + 1)) {
  const end = app.indexOf("</footer>", i);
  const lineStart = app.lastIndexOf("\n", i);
  const condLine = app.slice(app.lastIndexOf("\n", lineStart - 1) + 1, lineStart);
  footers.push({ at: i, end, body: app.slice(i, end), cond: condLine });
}
const notices = [...app.matchAll(/<RemoteNotice\b/g)].map((m) => m.index);
eq("App renders RemoteNotice twice (a place's footer and Home's)", notices.length, 2);
eq("every RemoteNotice sits inside a .statusbar footer", notices.every((n) => footers.some((f) => n > f.at && n < f.end)), true);
const hosting = footers.filter((f) => /<RemoteNotice\b/.test(f.body));
eq("…each footer that hosts it is rendered for it (its condition names remoteNotice)",
  hosting.map((f) => /\bremoteNotice\b/.test(f.cond)), hosting.map(() => true));
eq("…and remoteNotice is useRemoteNotice()'s", /const remoteNotice = useRemoteNotice\(\);/.test(app), true);
const briefing = app.indexOf('<div className="briefing">');
const mainEnd = app.indexOf("</main>", briefing);
eq("Home has its own footer for the notice, after the briefing and inside .main",
  hosting.some((f) => f.at > briefing && f.end < mainEnd), true);
eq("…and a place's footer is the other one", hosting.some((f) => f.end < briefing), true);
eq("Home's briefing scrolls in .home-scroll, so its footer stays on screen",
  /<div className="home-scroll">\s*<div className="briefing">/.test(app), true);

// ── 4. CSS: a button sized by flex is a WebKit trap ────────────────────────
const css = src("EngineGate.css");
const rule = (sel) => new RegExp(`(?:^|\\n)${sel.replace(/[.]/g, "\\.")} \\{([^}]*)\\}`).exec(css)?.[1] ?? null;
eq(".remote-notice is found", rule(".remote-notice") !== null, true);
eq(".remote-notice (the <button>) is not a flex container; its inner span is",
  [/display:\s*(?:inline-)?flex/.test(rule(".remote-notice") ?? ""), /display:\s*flex/.test(rule(".remote-notice-in") ?? "")], [false, true]);

if (failed) {
  console.log(`\n${failed} failed, ${passed} passed`);
  process.exit(1);
}
console.log(`ok ${passed} remote notice checks`);

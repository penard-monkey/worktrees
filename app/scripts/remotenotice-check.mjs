// Guards where the notice that remote access is on lives (§4.1), and what it
// says.
//
//   node app/scripts/remotenotice-check.mjs     # exits non-zero on failure
//   SRC_App_tsx=<path> …   # another copy of any app/src file read here
//                          # (SRC_<its name, every non-word character as _>)
//
// WHY THIS EXISTS. The notice is an item in the bar at the bottom: the one
// the status chip uses, by the chip's own rule (`remoteNoticeHost` beside
// `statusChipHost` in App.tsx). That is the window-wide `.usage-strip`, the
// only bar that is there on Home and through a restored selection, unless
// the usage meter lives in a place's footer, which then takes both. It no
// longer says "— quitting Worktrees turns it off" (Settings → Remote access
// says what quitting does). It used to be a fixed pill in EngineGate's
// corner, `.remote-corner`, which §5.3's alerts still use. Nothing that runs
// in CI renders any of this, and every wrong placement looks fine in the
// harness:
//
//   1. Back in the corner, it renders as convincingly as in a bar.
//   2. In a footer gated only on what the footer USED to carry, it renders
//      nowhere at all for the default `usage_place: "strip"`.
//   3. In a footer of its own (the first cut of this, aaeb4da), it adds a
//      SECOND bar above the strip, which is the bar the user actually sees.
//
// So this pins the words (evaluated from devices.ts, the one source), that
// RemoteLayer's corner holds no notice, and the rule itself: the host
// expressions and every render condition on the way to the notice are sliced
// out of App.tsx and evaluated over every placement × selection × meter ×
// chip × notice, so the notice is in exactly one bar, the one its host
// names, beside the chip, and there are never two bars at the bottom.
// Slice-the-real-source, as race-check.mjs and offers-check.mjs.
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
eq("RemoteNotice is exported for App's bars", notice.length > 0, true);
eq("RemoteNotice's words are remoteNoticeWords(st), nothing added", /<span className="remote-notice-txt">\{remoteNoticeWords\(st\)\}<\/span>/.test(notice), true);
eq("RemoteNotice still opens Settings → Remote access", /onClick=\{\(\) => openRemoteSettings\("remote"\)\}/.test(notice), true);

// ── 3. App.tsx: the host rule, evaluated ───────────────────────────────────
const app = src("App.tsx");
eq("remoteNotice is useRemoteNotice()'s", /const remoteNotice = useRemoteNotice\(\);/.test(app), true);
const SCOPE = ["settings", "usage", "selected", "claudeStatus", "statusOnTile", "statusChipHost", "remoteNotice", "remoteNoticeHost"];
/** An expression from App.tsx, as a function of SCOPE; `fallback` if absent.
 *  The text is this repository's own source — the code the app runs — read
 *  from disk by a dev-only check, never input from anywhere else. */
const fn = (expr, fallback) => {
  if (expr == null) return () => fallback;
  try {
    const f = new Function(...SCOPE, `return (${expr});`);
    return (s) => f(...SCOPE.map((k) => s[k]));
  } catch (e) {
    eq(`App.tsx expression parses: ${expr.slice(0, 60)}`, String(e), "");
    return () => fallback;
  }
};
const declared = (name) => new RegExp(`const ${name}(?::[^=]+)?=\\s*([^;]+);`).exec(app)?.[1] ?? null;
eq("App.tsx declares remoteNoticeHost", declared("remoteNoticeHost") !== null, true);
const onTile = fn(declared("statusOnTile"), null);
const chipHost = fn(declared("statusChipHost"), null);
const noticeHost = fn(declared("remoteNoticeHost"), undefined);

/** A bar: its render condition, the far-end group's condition, and the
 *  notice's own guard, each sliced out of the bar's block. */
function bar(opener, close) {
  const m = new RegExp(`\\{\\((.+)\\) && \\(\\s*${opener}`).exec(app);
  if (!m) return null;
  const block = app.slice(m.index, app.indexOf(close, m.index + m[0].length));
  const group = /\{\((.+)\) && \(\s*<span className="bar-end">/.exec(block);
  const guard = /\{([^{}\n]+?) && <RemoteNotice\b/.exec(block);
  const chipAt = block.indexOf("<StatusChip"), noticeAt = block.indexOf("<RemoteNotice");
  const groupEnd = group ? block.indexOf("</span>", group.index) : -1;
  return {
    shown: fn(m[1], false),
    group: group ? fn(group[1], false) : () => true,
    guard: guard ? fn(guard[1], false) : () => noticeAt >= 0,
    hasNotice: noticeAt >= 0,
    // The chip, then the notice, both inside the far-end group.
    besideChip: !!group && chipAt > group.index && noticeAt > chipAt && noticeAt < groupEnd,
  };
}
const FOOTER = '<footer className="statusbar">';
eq("App has ONE .statusbar footer, a place's (Home's bar is the strip)", app.split(FOOTER).length - 1, 1);
const bars = { footer: bar(FOOTER, "</footer>"), strip: bar('<div className="usage-strip">', "</div>") };
eq("both bars are found", [!!bars.footer, !!bars.strip], [true, true]);
eq("App renders RemoteNotice twice, once in each bar and nowhere else",
  [[...app.matchAll(/<RemoteNotice\b/g)].length, bars.footer?.hasNotice, bars.strip?.hasNotice], [2, true, true]);
eq("…beside the chip in each bar's far-end group (.bar-end), the notice last",
  [bars.footer?.besideChip, bars.strip?.besideChip], [true, true]);

const bad = { twoBars: [], notExactlyOne: [], wrongBar: [], notChipRule: [], strayNotice: [] };
const SEL = { slug: "x" }, INFO = { at: 1 }, STATUS = { level: "degraded" }, ON = { state: "listening" };
for (const usage_place of ["strip", "footer", "rail", "off"])
  for (const selected of [SEL, null])
    for (const info of [INFO, null])
      for (const claudeStatus of [STATUS, null])
        for (const remoteNotice of [ON, null]) {
          const s = { settings: { usage_place }, usage: { info }, selected, claudeStatus, remoteNotice };
          s.statusOnTile = onTile(s);
          s.statusChipHost = chipHost(s);
          s.remoteNoticeHost = noticeHost(s);
          const label = JSON.stringify({ usage_place, selected: !!selected, info: !!info, chip: !!claudeStatus, notice: !!remoteNotice });
          if (!bars.footer || !bars.strip) continue;
          // The footer is in the place branch, which renders on `selected && sel`.
          const shown = { footer: !!selected && !!bars.footer.shown(s), strip: !!bars.strip.shown(s) };
          const inBar = {};
          for (const k of ["footer", "strip"]) inBar[k] = shown[k] && !!bars[k].group(s) && !!bars[k].guard(s);
          if (shown.footer && shown.strip) bad.twoBars.push(label);
          if (!remoteNotice) {
            if (inBar.footer || inBar.strip) bad.strayNotice.push(label);
            continue;
          }
          if (Number(inBar.footer) + Number(inBar.strip) !== 1) bad.notExactlyOne.push(label);
          if (!inBar[s.remoteNoticeHost]) bad.wrongBar.push(`${label} host=${s.remoteNoticeHost}`);
          // The chip's rule, without the tile (a 44px rail has no room for a sentence).
          const rule = chipHost({ ...s, claudeStatus: STATUS, statusOnTile: null });
          if (s.remoteNoticeHost !== rule) bad.notChipRule.push(`${label} host=${s.remoteNoticeHost} chip rule=${rule}`);
        }
const first = (a) => (a.length ? `${a.length} cases, e.g. ${a[0]}` : "none");
eq("never two bars at the bottom", first(bad.twoBars), "none");
eq("with remote access on, the notice is in exactly one bar", first(bad.notExactlyOne), "none");
eq("…the one remoteNoticeHost names", first(bad.wrongBar), "none");
eq("…which is the status chip's host rule, minus the tile", first(bad.notChipRule), "none");
eq("with remote access off, no notice anywhere", first(bad.strayNotice), "none");

// ── 4. CSS ─────────────────────────────────────────────────────────────────
const css = src("EngineGate.css");
const rule = (sheet, sel) => new RegExp(`(?:^|\\n)${sel.replace(/[.]/g, "\\.")} \\{([^}]*)\\}`).exec(sheet)?.[1] ?? null;
eq(".remote-notice is found", rule(css, ".remote-notice") !== null, true);
eq(".remote-notice (the <button>) is not a flex container; its inner span is (WebKit)",
  [/display:\s*(?:inline-)?flex/.test(rule(css, ".remote-notice") ?? ""), /display:\s*flex/.test(rule(css, ".remote-notice-in") ?? "")], [false, true]);
// One far-end group, so a wrapped row keeps the chip and the notice at the
// far end of their line (two `auto` siblings left the notice at the near end).
const barEnd = rule(src("App.css"), ".bar-end") ?? "";
eq(".bar-end keeps the far end on any line it wraps onto",
  [/margin-left:\s*auto/.test(barEnd), /flex-wrap:\s*wrap/.test(barEnd), /justify-content:\s*flex-end/.test(barEnd)], [true, true, true]);

if (failed) {
  console.log(`\n${failed} failed, ${passed} passed`);
  process.exit(1);
}
console.log(`ok ${passed} remote notice checks`);

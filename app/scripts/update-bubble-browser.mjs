// Optional browser gate for the update bubble and the gear's dot, in Chromium
// AND WebKit (the app is WKWebView; the harness is Chrome — AGENTS.md).
//
// Install Playwright in scratch space with both engines, then run against the
// mock harness on a port that is not 1420:
//   VITE_MOCK=1 vite --port 1471 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     UPDATE_MOCK_URL=http://localhost:1471 node app/scripts/update-bubble-browser.mjs
// UPDATE_SCREENSHOTS=<dir> saves the bubble in all six themes (WebKit).
//
// What it holds, and why each needs a browser rather than updatepoll-check.mjs:
//   - geometry: the bubble sits beside the gear with its tail on the gear's
//     centre, on whichever side the rail is (mirrored Places), at 900 and
//     1280px, and every button is REACHABLE (elementFromPoint, not a rect);
//   - the error/undo float-stack, which anchors at the same bottom-left
//     corner, is lifted above the bubble instead of painting over it;
//   - dismissal per VERSION across reloads; "Update…" and visiting Settings →
//     Updates by any route retire it; reopening Settings elsewhere does NOT
//     (the stale-category edge — it failed before SettingsSheet reset `cat` on
//     close); Escape does not touch it (not modal: Esc belongs to the pty);
//   - the real poll on a FAKE clock: a release published mid-session is found
//     on the hour, and an offline poll keeps the last tag (no blink);
//   - contrast in all six themes: words in neutral tokens, the accent only as
//     the 6px dot, which must clear 3:1 against what it sits on.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
import fs from "node:fs";
const BASE = process.env.UPDATE_MOCK_URL ?? "http://localhost:1471";
const SHOTS = process.env.UPDATE_SCREENSHOTS;
// The gear's dot predates this change and wears --accent, which in tokyo-day is
// 2.4–3.1:1 against that theme's surfaces app-wide (AGENTS.md: check a number
// against the app's own tokens before calling it a defect). A token change is a
// THEME change; here the dot is held to being exactly --accent, and the
// measured ratio is still printed.
const ACCENT_LOW = new Set(["tokyo-day"]);
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const BUBBLE = "[data-testid=update-bubble]";

async function geometry(page) {
  return page.evaluate(() => {
    const bb = document.querySelector("[data-testid=update-bubble]"), g = document.querySelector(".rail-icon.upd");
    const b = bb.getBoundingClientRect(), r = g.getBoundingClientRect();
    const tailBottom = parseFloat(getComputedStyle(bb, "::before").bottom);
    const reach = [...bb.querySelectorAll("button")].every((btn) => {
      const q = btn.getBoundingClientRect();
      return btn.contains(document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2));
    });
    return { b: { l: b.left, r: b.right, t: b.top, bot: b.bottom }, g: { l: r.left, r: r.right, mid: r.top + r.height / 2 }, tailMid: b.bottom - tailBottom - 5, reach, vw: innerWidth };
  });
}

for (const [engine, type] of Object.entries({ chromium, webkit })) {
  console.log(`── ${engine}`);
  const browser = await type.launch({ headless: true });

  // ── geometry, both sides, two widths ──
  for (const [w, side] of [[1280, "left"], [900, "left"], [1280, "right"], [900, "right"]]) {
    const ctx = await browser.newContext({ viewport: { width: w, height: 800 } });
    const page = await ctx.newPage();
    await page.addInitScript((side) => {
      if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify({ places_side: side }));
    }, side);
    await page.goto(`${BASE}/?latest=v9.9.9&cli=0.1.0`);
    await page.waitForSelector(BUBBLE, { timeout: 10000 });
    await page.waitForTimeout(400); // the pop animation
    const m = await geometry(page);
    const gap = side === "left" ? m.b.l - m.g.r : m.g.l - m.b.r;
    ok(gap > 4 && gap < 16, `${w} ${side}: bubble beside the gear (gap ${gap.toFixed(1)}px)`);
    ok(Math.abs(m.tailMid - m.g.mid) <= 1.5, `${w} ${side}: tail on the gear's centre (${m.tailMid.toFixed(1)} vs ${m.g.mid.toFixed(1)})`);
    ok(m.b.l >= 0 && m.b.r <= m.vw, `${w} ${side}: inside the viewport`);
    ok(m.reach, `${w} ${side}: every button reachable (elementFromPoint)`);
    await ctx.close();
  }

  // ── flows ──
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await ctx.newPage();
  const has = () => page.locator(BUBBLE).count();
  const dot = () => page.locator(".rail-icon.upd").count();
  await page.goto(`${BASE}/`); await page.waitForTimeout(4500);
  ok(await has() === 0 && await dot() === 0, "default mock is up to date: no bubble, no dot");
  await page.goto(`${BASE}/?offline`); await page.waitForTimeout(4500);
  ok(await has() === 0 && await dot() === 0, "?offline: no bubble, no dot");
  await page.goto(`${BASE}/?cli=none`); await page.waitForTimeout(4500);
  ok(await has() === 0 && await dot() === 1, "CLI not installed: the dot, never the bubble (not a release)");
  await page.goto(`${BASE}/?latest=v9.9.9`); await page.waitForTimeout(2000);
  ok(await has() === 0, "nothing before the first check (3s)");
  await page.waitForTimeout(2500);
  ok(await has() === 1 && await dot() === 1, "bubble + dot after the first check");

  await page.evaluate(() => {
    const I = window.__TAURI_INTERNALS__, orig = I.invoke.bind(I);
    I.invoke = (c, a, o) => (c === "plugin:opener|open_url" ? Promise.reject("open_url: forced failure for the stack test") : orig(c, a, o));
  });
  await page.click(".upd-bubble-link"); await page.waitForTimeout(400);
  const lay = await page.evaluate(() => {
    const f = document.querySelector(".float-stack"), bb = document.querySelector(".upd-bubble");
    return f && { floatBottom: f.getBoundingClientRect().bottom, bubbleTop: bb.getBoundingClientRect().top };
  });
  ok(!!lay && lay.floatBottom <= lay.bubbleTop, `float-stack lifted above the bubble (${JSON.stringify(lay)})`);
  if (lay) { await page.click(".err-float"); await page.waitForTimeout(200); }

  await page.keyboard.press("Escape"); await page.waitForTimeout(200);
  ok(await has() === 1, "Escape leaves the bubble alone (it belongs to the terminal)");
  await page.click(".upd-bubble-x"); await page.waitForTimeout(300);
  ok(await has() === 0 && await dot() === 1, "× hides the bubble; the gear keeps its dot");
  await page.reload(); await page.waitForTimeout(4500);
  ok(await has() === 0, "a dismissed tag stays dismissed across reload");
  await page.goto(`${BASE}/?latest=v9.9.10`); await page.waitForTimeout(4500);
  ok(await has() === 1, "a NEWER release re-shows the bubble");
  await page.click(".upd-bubble .enter-btn"); await page.waitForTimeout(400);
  const cat = await page.evaluate(() => document.querySelector(".settings-cat.on")?.textContent ?? "");
  ok(cat.startsWith("Updates") && await has() === 0, `Update… opens Settings → Updates (${cat})`);
  await page.keyboard.press("Escape"); await page.waitForTimeout(400);
  ok(await page.locator(".settings-cats").count() === 0 && await has() === 0, "closing Settings does not bring it back");
  await page.goto(`${BASE}/?latest=v9.9.11`); await page.waitForTimeout(4500);
  await page.click(".rail-icon.upd"); await page.waitForTimeout(300);
  await page.locator(".settings-cat", { hasText: "Updates" }).click(); await page.waitForTimeout(300);
  await page.keyboard.press("Escape"); await page.waitForTimeout(300);
  ok(await has() === 0, "clicking to Updates inside Settings counts as seen");

  // ── the real poll on a fake clock ──
  {
    const q = await ctx.newPage();
    await q.clock.install();
    await q.goto(`${BASE}/?latest=v9.9.20`);
    await q.waitForSelector(".rail-icon"); await q.waitForTimeout(500);
    await q.clock.runFor(4000); await q.waitForTimeout(500);
    const hasQ = () => q.locator(BUBBLE).count();
    ok(await hasQ() === 1, "[clock] launch check shows v9.9.20");
    await q.click(".upd-bubble .enter-btn"); await q.waitForTimeout(200);
    await q.keyboard.press("Escape"); await q.waitForTimeout(200);
    await q.evaluate(() => window.__mock.release("v9.9.21"));
    await q.clock.runFor(59 * 60_000); await q.waitForTimeout(300);
    ok(await hasQ() === 0, "[clock] no re-check inside the hour");
    await q.clock.runFor(2 * 60_000); await q.waitForTimeout(300);
    ok(await hasQ() === 1, "[clock] the hourly poll finds a mid-session release");
    await q.click(".rail-icon.upd"); await q.waitForTimeout(200);
    const at = await q.evaluate(() => document.querySelector(".settings-cat.on")?.textContent ?? "");
    await q.keyboard.press("Escape"); await q.waitForTimeout(200);
    ok(await hasQ() === 1, `[clock] reopening Settings at ${at} does not retire it (stale category)`);
    await q.evaluate(() => window.__mock.release(null));
    await q.clock.runFor(61 * 60_000); await q.waitForTimeout(300);
    ok(await hasQ() === 1 && await q.locator(".rail-icon.upd").count() === 1, "[clock] an offline poll keeps the last tag (no blink)");
    await q.close();
  }

  await page.evaluate(() => {
    const s = JSON.parse(sessionStorage.getItem("wt-mock-ui-state")); s.update_auto_check = false;
    sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  });
  await page.goto(`${BASE}/?latest=v9.9.30`); await page.waitForTimeout(4500);
  ok(await has() === 0 && await dot() === 0, "auto-check off: no check, so no bubble and no dot");
  await ctx.close();

  // ── contrast, six themes ──
  const c2 = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
  const p2 = await c2.newPage();
  await p2.goto(`${BASE}/?latest=v9.9.9&cli=0.1.0`);
  await p2.waitForSelector(BUBBLE); await p2.waitForTimeout(400);
  for (const theme of THEMES) {
    await p2.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    await p2.waitForTimeout(50);
    const c = await p2.evaluate(() => {
      function rgba(s) {
        if (s.startsWith("color(srgb")) { const v = s.slice(11, -1).trim().split(/[\s/]+/).map(Number); return [v[0] * 255, v[1] * 255, v[2] * 255, v[3] ?? 1]; }
        const v = s.match(/[\d.]+/g)?.map(Number) ?? [0, 0, 0, 0]; return [v[0], v[1], v[2], v[3] ?? 1];
      }
      const over = (a, b) => [0, 1, 2].map((i) => a[i] * a[3] + b[i] * (1 - a[3])).concat(1);
      const bgOf = (e) => { const L = []; for (let n = e; n; n = n.parentElement) L.push(rgba(getComputedStyle(n).backgroundColor)); return L.reverse().reduce((b, a) => over(a, b), [255, 255, 255, 1]); };
      const lum = (c) => c.slice(0, 3).map((v) => v / 255).map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)).reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
      const ratio = (f, b) => { const x = lum(f), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
      const sw = document.createElement("span"); document.body.append(sw);
      const tok = (t) => { sw.style.color = `var(${t})`; return getComputedStyle(sw).color; };
      const neutral = ["--txt-hi", "--txt"].map(tok);
      const bb = document.querySelector(".upd-bubble"), panel = bgOf(bb);
      const text = [".upd-bubble-title", ".upd-bubble-body", ".upd-bubble-link"].map((s) => {
        const e = document.querySelector(s), cs = getComputedStyle(e);
        return { s, ratio: ratio(over(rgba(cs.color), panel), panel), neutral: neutral.includes(cs.color) };
      });
      const dotIn = rgba(getComputedStyle(document.querySelector(".upd-bubble-dot")).backgroundColor);
      const gear = document.querySelector(".rail-icon.upd"), railBg = bgOf(gear);
      const dotRail = rgba(getComputedStyle(gear, "::after").backgroundColor);
      const accent = rgba(tok("--accent"));
      sw.remove();
      return { text, dotBubble: ratio(dotIn, panel), dotRail: ratio(dotRail, railBg), sameDot: dotIn.join() === dotRail.join(), isAccent: dotIn.join() === accent.join() };
    });
    ok(c.text.every((t) => t.neutral), `${theme}: bubble words use neutral tokens`);
    ok(c.text.every((t) => t.ratio >= 4.5), `${theme}: text ≥ 4.5:1 (${c.text.map((t) => t.ratio.toFixed(1)).join(" / ")})`);
    const dots = `on the bubble ${c.dotBubble.toFixed(1)}:1, on the rail ${c.dotRail.toFixed(1)}:1`;
    if (ACCENT_LOW.has(theme)) ok(c.isAccent, `${theme}: dot is exactly --accent (known-low accent theme; ${dots})`);
    else ok(c.dotBubble >= 3 && c.dotRail >= 3, `${theme}: dot ≥ 3:1 (${dots})`);
    ok(c.sameDot, `${theme}: the bubble's dot and the gear's dot are one colour`);
    if (SHOTS && engine === "webkit") {
      fs.mkdirSync(SHOTS, { recursive: true });
      await p2.screenshot({ path: `${SHOTS}/bubble-${theme}.png`, clip: { x: 0, y: 600, width: 420, height: 200 } });
    }
  }
  await c2.close();
  await browser.close();
}
if (failed) { console.error(`\n${failed} failure(s)`); process.exit(1); }
console.log("\nupdate-bubble-browser: all passed");

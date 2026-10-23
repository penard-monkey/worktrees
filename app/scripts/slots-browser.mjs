// Optional browser gate for Settings → Behavior → Heavy jobs (SlotsPanel.tsx),
// in Chromium AND WebKit (the app is WKWebView; the harness is Chrome — AGENTS.md).
//
// Install Playwright in scratch space with both engines, then run against the
// mock harness on a port that is not 1420:
//   VITE_MOCK=1 vite --port 1473 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     SLOTS_MOCK_URL=http://localhost:1473 node app/scripts/slots-browser.mjs
// SLOTS_SCREENSHOTS=<dir> saves the section per engine (and per theme in WebKit).
//
// What it holds, and why each needs a browser:
//   - every control is REACHABLE: − / + / Reset are hit-tested with
//     elementFromPoint at their centre (a rect inside a clipping ancestor is
//     not a click), at 1280 and 900px, and nothing in a row overflows it;
//   - the round trip the mock mirrors from core: + writes `[slots] gate`, Reset
//     removes it and the row falls back to the machine (or `default`, once that
//     is set), − stops at 1 and + at the reply's `max`;
//   - the busy count, the machine line, the built-in fallback and the
//     problems line render;
//   - contrast in all six themes: words in neutral tokens (--txt-hi /
//     --txt-dim), ≥ 4.5:1 for the name and the count, and the busy dot the one
//     place the accent appears.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
import fs from "node:fs";
const BASE = process.env.SLOTS_MOCK_URL ?? "http://localhost:1473";
const SHOTS = process.env.SLOTS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
const SECTION = "[data-testid=slots-settings]";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

async function open(page, query = "") {
  await page.goto(`${BASE}/${query}`);
  await page.waitForSelector("[data-track=settings]");
  await page.click("[data-track=settings]");
  await page.locator(".settings-cat", { hasText: "Behavior" }).click();
  await page.waitForSelector(`${SECTION} [data-testid=slot-gate]`);
  await page.locator(SECTION).scrollIntoViewIfNeeded();
}

/** Every slot row as the page shows it. */
const rows = (page) => page.evaluate(() => [...document.querySelectorAll(".slot-row")].map((r) => ({
  name: r.dataset.testid.slice(5),
  cap: r.querySelector(".slot-cap").textContent,
  source: r.querySelector("[data-testid^=slot-source-]").textContent,
  meta: r.querySelector(".slot-meta").textContent,
  reset: r.querySelector(".slot-reset")?.textContent ?? null,
})));

/** Each button in the section: reachable at its centre, and whether it is disabled. */
const reach = (page) => page.evaluate((sel) => [...document.querySelectorAll(`${sel} button`)].map((b) => {
  b.scrollIntoView({ block: "center" });
  const q = b.getBoundingClientRect();
  const hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2);
  return { id: b.dataset.testid, hit: !!hit && b.contains(hit), w: q.width, h: q.height };
}), SECTION);

const overflow = (page) => page.evaluate(() => [...document.querySelectorAll(".slot-row")].filter((r) => r.scrollWidth > r.clientWidth + 1).map((r) => r.dataset.testid));

const row = async (page, name) => (await rows(page)).find((r) => r.name === name);

for (const [engine, type] of Object.entries({ chromium, webkit })) {
  console.log(`── ${engine}`);
  const browser = await type.launch({ headless: true });
  for (const width of [1280, 900]) {
    const ctx = await browser.newContext({ viewport: { width, height: 800 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    await open(page);
    const r = await rows(page);
    ok(r.map((x) => x.name).join() === "gate,default", `[${width}] rows are gate then default (${r.map((x) => x.name)})`);
    const hits = await reach(page);
    ok(hits.length >= 5 && hits.every((h) => h.hit), `[${width}] every button is reachable (${hits.filter((h) => !h.hit).map((h) => h.id).join(", ") || hits.length + " hit"})`);
    ok(hits.filter((h) => /slot-(inc|dec)/.test(h.id ?? "")).every((h) => h.w >= 20 && h.h >= 20), `[${width}] steppers are at least 20px square`);
    ok((await overflow(page)).length === 0, `[${width}] no row overflows`);
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await page.locator(SECTION).screenshot({ path: `${SHOTS}/slots-${engine}-${width}.png` });
    }
    await ctx.close();
  }

  // ── the round trip ──
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await ctx.newPage();
  await open(page);
  let g = await row(page, "gate"), d = await row(page, "default");
  ok(g.cap === "3" && g.source === "config [slots] gate", `gate starts at 3 from config (${g.cap}, ${g.source})`);
  ok(g.meta.includes("1 running, 1 waiting"), `gate shows what runs and waits (${g.meta})`);
  ok(g.reset === "Reset to machine default (3)", `gate's Reset names what it leaves (${g.reset})`);
  ok(d.cap === "3" && d.source === "machine (10 cores, 36 GiB)" && d.reset === null, `default is the machine's, with nothing to reset (${d.source})`);
  const machine = await page.locator("[data-testid=slots-machine]").textContent();
  ok(machine.includes("10 cores and 36 GiB give 3"), `the machine line (${machine})`);
  await page.click("[data-testid=slot-inc-gate]");
  await page.waitForFunction(() => document.querySelector("[data-testid=slot-cap-gate]")?.textContent === "4");
  ok((await row(page, "gate")).source === "config [slots] gate", "+ sets gate to 4 in config");
  await page.click("[data-testid=slot-reset-gate]");
  await page.waitForFunction(() => !document.querySelector("[data-testid=slot-reset-gate]"));
  g = await row(page, "gate");
  ok(g.cap === "3" && g.source === "machine (10 cores, 36 GiB)", `Reset puts gate back on the machine (${g.cap}, ${g.source})`);
  await page.click("[data-testid=slot-inc-default]");
  await page.waitForFunction(() => document.querySelector("[data-testid=slot-cap-default]")?.textContent === "4");
  g = await row(page, "gate"); d = await row(page, "default");
  ok(g.cap === "4" && g.source === "config [slots] default", `gate follows [slots] default (${g.cap}, ${g.source})`);
  ok(d.reset === "Reset to machine default (3)", `default's Reset goes to the machine (${d.reset})`);
  await page.click("[data-testid=slot-inc-gate]");
  await page.waitForFunction(() => document.querySelector("[data-testid=slot-reset-gate]"));
  ok((await row(page, "gate")).reset === "Reset to default (4)", "with [slots] default set, gate's Reset names it");
  for (let i = 0; i < 6; i++) {
    if (await page.locator("[data-testid=slot-dec-gate]").isDisabled()) break;
    const before = await page.locator("[data-testid=slot-cap-gate]").textContent();
    await page.click("[data-testid=slot-dec-gate]");
    await page.waitForFunction((b) => document.querySelector("[data-testid=slot-cap-gate]")?.textContent !== b, before);
  }
  ok((await row(page, "gate")).cap === "1" && await page.locator("[data-testid=slot-dec-gate]").isDisabled(), "− stops at 1");
  await ctx.close();

  // ── the edges: + at max, built-in, problems ──
  const c3 = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const p3 = await c3.newPage();
  await open(p3, "?slots=gate=64,e2e=1&slotsmachine=off&slotsbad=1");
  ok(await p3.locator("[data-testid=slot-inc-gate]").isDisabled(), "+ stops at the reply's max (64)");
  const names = (await rows(p3)).map((x) => x.name).join();
  ok(names === "gate,e2e,default", `a slot the config names gets a row (${names})`);
  const dd = await row(p3, "default");
  ok(dd.cap === "2" && dd.source === "built-in", `an unreadable machine is the built-in 2 (${dd.cap}, ${dd.source})`);
  ok((await p3.locator("[data-testid=slots-machine]").textContent()).includes("built-in"), "the machine line says why");
  ok(await p3.locator("[data-testid=slots-problems]").count() === 1, "a hand-edit core ignored is shown");
  await c3.close();

  // ── contrast, six themes ──
  const c2 = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
  const p2 = await c2.newPage();
  await open(p2);
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
      const neutral = ["--txt-hi", "--txt", "--txt-dim"].map(tok);
      const r = document.querySelector("[data-testid=slot-gate]"), panel = bgOf(r);
      const sheet = bgOf(r.parentElement);
      const text = [".slot-name", ".slot-cap", ".slot-meta", ".slot-reset", ".slot-step button"].map((s) => {
        const e = r.querySelector(s), cs = getComputedStyle(e);
        const bg = bgOf(e);
        return { s, ratio: ratio(over(rgba(cs.color), bg), bg), neutral: neutral.includes(cs.color) };
      });
      const dot = rgba(getComputedStyle(r.querySelector(".slot-dot")).backgroundColor);
      const accent = rgba(tok("--accent"));
      sw.remove();
      const card = Math.max(...[0, 1, 2].map((i) => Math.abs(panel[i] - sheet[i])));
      return { text, dot: ratio(dot, panel), isAccent: dot.join() === accent.join(), card };
    });
    ok(c.text.every((t) => t.neutral), `${theme}: words use neutral tokens`);
    const main = c.text.filter((t) => [".slot-name", ".slot-cap", ".slot-step button"].includes(t.s));
    ok(main.every((t) => t.ratio >= 4.5), `${theme}: name, count and steppers ≥ 4.5:1 (${main.map((t) => t.ratio.toFixed(1)).join(" / ")})`);
    const dim = c.text.filter((t) => [".slot-meta", ".slot-reset"].includes(t.s));
    ok(dim.every((t) => t.ratio >= 3), `${theme}: source line and Reset ≥ 3:1 (${dim.map((t) => t.ratio.toFixed(1)).join(" / ")})`);
    ok(c.isAccent, `${theme}: the busy dot is the accent (${c.dot.toFixed(1)}:1 on the row)`);
    ok(c.card > 0, `${theme}: a row is a distinct surface from the sheet (max channel distance ${c.card})`);
    if (SHOTS && engine === "webkit") await p2.locator(SECTION).screenshot({ path: `${SHOTS}/slots-webkit-${theme}.png` });
  }
  await c2.close();
  await browser.close();
}
console.log(failed ? `\n${failed} FAILED` : "\nall ok");
process.exit(failed ? 1 : 0);

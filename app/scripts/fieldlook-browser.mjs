// An editable field must LOOK editable, in all six themes, in Chromium AND
// WebKit (the app is WKWebView; the harness is Chrome — AGENTS.md).
//
//   VITE_MOCK=1 vite --port 1471 --strictPort --force            (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     FIELDLOOK_MOCK_URL=http://localhost:1471 node app/scripts/fieldlook-browser.mjs
// FIELDLOOK_SCREENSHOTS=<dir> saves Settings → Remote ("Join with a link") per
// theme and engine, which is the surface the owner reported (2026-11-24: "not
// visually obvious that these fields are not disabled").
//
// The rule, per text control (input text/number/search/url/password, textarea,
// select) drawn in Settings and its header search:
//   - FILL differs from the first opaque ancestor it sits on (alpha composited
//     first: a translucent background read raw measures 1.0 everywhere, and a
//     resolved color-mix() comes back as `color(srgb 0-1 / a)`, not rgb());
//   - BOUNDARY (its border, else its fill edge) is >= 3:1 against that surface
//     (WCAG 1.4.11 non-text contrast). --line-strong was 1.3-2.8:1 everywhere;
//   - in a LIGHT theme the fill is lighter than the surface, never a darker
//     grey (that is the language of "disabled" there);
//   - FOCUS shows: a ring or an edge change that is itself >= 3:1;
//   - DISABLED differs from enabled: not-allowed cursor, no fill, and a weaker
//     boundary. The Join form's name field is `disabled={busy}`, so both exist.
// The surface list is --bg-panel (cards, dialogs), --bg-tree and --bg-abyss
// (what a field in a rail or the app's backdrop meets) — the check reads the
// REAL ancestor of each field, and a synthetic probe sits a field on each of
// the three so a theme that only passes on one host fails.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
import fs from "node:fs";
const BASE = process.env.FIELDLOOK_MOCK_URL ?? "http://localhost:1471";
const SHOTS = process.env.FIELDLOOK_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
const LIGHT = new Set(["tokyo-day", "catppuccin-latte"]);
const PAGES = ["remote", "general", "agents"];

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

// Everything below runs in the page: it needs getComputedStyle.
const measure = (themes) => {
  const parse = (c) => {
    let m = /^rgba?\(([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
    m = /^color\(srgb ([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; }
    return [0, 0, 0, 0];
  };
  const over = (top, under) => { const a = top[3]; return [0, 1, 2].map((i) => top[i] * a + under[i] * (1 - a)).concat(1); };
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const surfaceOf = (el) => {
    const chain = [];
    for (let n = el.parentElement; n; n = n.parentElement) chain.push(parse(getComputedStyle(n).backgroundColor));
    let acc = parse(getComputedStyle(document.body).backgroundColor);
    if (acc[3] < 1) acc = over(acc, [255, 255, 255, 1]);
    for (const c of chain.reverse()) if (c[3] > 0) acc = over(c, acc);
    return acc;
  };
  const edge = (el, host) => {
    const cs = getComputedStyle(el);
    const fill = over(parse(cs.backgroundColor), host);
    const w = ["Top", "Right", "Bottom", "Left"].reduce((s, k) => s + (parseFloat(cs[`border${k}Width`]) || 0) * (cs[`border${k}Style`] === "none" ? 0 : 1), 0);
    const bc = over(parse(cs.borderTopColor), host);
    const line = w > 0 ? ratio(bc, host) : 0;
    return { fill, fillRatio: ratio(fill, host), line, boundary: Math.max(line, ratio(fill, host)), cursor: cs.cursor, color: parse(cs.color), shadow: cs.boxShadow, bw: w, outline: cs.outlineStyle };
  };
  const SEL = 'input:not([type=checkbox]):not([type=radio]):not([type=range]):not([type=button]):not([type=submit]):not([type=file]):not([type=color]):not([type=hidden]), textarea, select';
  const out = [];
  for (const el of document.querySelectorAll(SEL)) {
    const r = el.getBoundingClientRect();
    if (r.width < 8 || r.height < 8 || getComputedStyle(el).visibility === "hidden") continue;
    if (el.closest("[hidden], [inert], .sr-only")) continue;
    const host = surfaceOf(el);
    const was = el.disabled;
    el.disabled = false;
    const en = edge(el, host);
    el.focus({ preventScroll: true });
    const fo = edge(el, host);
    const ring = fo.shadow !== "none" || fo.outline !== "none" ? 1 : 0;
    el.blur();
    el.disabled = true;
    const di = edge(el, host);
    el.disabled = was;
    const lightFill = lum(en.fill) > lum(host);
    out.push({
      what: `${el.tagName.toLowerCase()}${el.type && el.tagName === "INPUT" ? "[" + el.type + "]" : ""}${el.className ? "." + String(el.className).split(" ")[0] : ""}${el.getAttribute("aria-label") ? ` "${el.getAttribute("aria-label")}"` : ""}`,
      fill: en.fillRatio, line: en.line, boundary: en.boundary, lightFill,
      focusEdge: Math.max(fo.line, ring ? 3 : 0), focusChanged: fo.shadow !== en.shadow || fo.outline !== en.outline || JSON.stringify(fo.fill) !== JSON.stringify(en.fill) || fo.line !== en.line,
      disCursor: di.cursor, disFill: di.fillRatio, disBoundary: di.boundary, enFill: en.fillRatio,
    });
  }
  return out;
};

// A synthetic probe: one unstyled-by-us field on each host, so the TOKENS are
// held against every surface, not only the ones Settings happens to draw.
const probe = () => {
  const parse = (c) => { const m = /^rgba?\(([^)]+)\)/.exec(c); if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; } const k = /^color\(srgb ([^)]+)\)/.exec(c); if (k) { const p = k[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; } return [0, 0, 0, 0]; };
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const root = getComputedStyle(document.documentElement);
  const tok = (n) => { const d = document.createElement("div"); d.style.background = `var(${n})`; document.body.appendChild(d); const c = parse(getComputedStyle(d).backgroundColor); d.remove(); return c; };
  const line = tok("--line-field"), fill = tok("--bg-field");
  void root;
  return ["--bg-panel", "--bg-tree", "--bg-abyss"].map((h) => ({ host: h, line: ratio(line, tok(h)), fill: ratio(fill, tok(h)), same: JSON.stringify(fill) === JSON.stringify(tok(h)) }));
};

async function run(browserType, name) {
  const browser = await browserType.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  await page.goto(`${BASE}/`);
  await page.waitForSelector("[data-track=settings]");
  await page.click("[data-track=settings]");
  await page.waitForSelector(".settings-body .page-title");
  // Settings is Beginner by default; Expert draws every page's fields.
  await page.click("[data-testid=settings-mode] [data-mode=expert]").catch(() => {});
  await page.waitForTimeout(150);
  for (const theme of THEMES) {
    await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    const probes = await page.evaluate(probe);
    for (const p of probes) {
      ok(p.line >= 3, `[${name}] ${theme}: --line-field on ${p.host} ${p.line.toFixed(2)}:1 (>= 3)`);
      ok(!p.same, `[${name}] ${theme}: --bg-field is not ${p.host} itself (${p.fill.toFixed(2)}:1)`);
    }
    let seen = 0;
    for (const cat of PAGES) {
      await page.click(`.settings-cat[data-cat=${cat}]`).catch(() => {});
      await page.waitForSelector(`.settings-body[data-page=${cat}]`).catch(() => {});
      await page.waitForTimeout(120);
      if (SHOTS && cat === "remote") {
        fs.mkdirSync(SHOTS, { recursive: true });
        await page.screenshot({ path: `${SHOTS}/join-${theme}-${name}.png` });
      }
      const fields = await page.evaluate(measure, THEMES);
      seen += fields.length;
      for (const f of fields) {
        const tag = `[${name}] ${theme} ${cat} ${f.what}`;
        ok(f.fill > 1.0 && f.boundary >= 3, `${tag}: boundary ${f.boundary.toFixed(2)}:1 (border ${f.line.toFixed(2)}, fill ${f.fill.toFixed(2)}) (>= 3)`);
        ok(f.fill >= 1.03, `${tag}: fill differs from its surface (${f.fill.toFixed(2)}:1)`);
        if (LIGHT.has(theme)) ok(f.lightFill, `${tag}: fill is LIGHTER than its surface`);
        ok(f.focusChanged && f.focusEdge >= 3, `${tag}: focus shows (edge ${f.focusEdge.toFixed(2)}:1)`);
        ok(f.disCursor === "not-allowed" && f.disBoundary < f.boundary && f.disFill < f.enFill + 0.001 && f.disFill < 1.03, `${tag}: disabled differs (cursor ${f.disCursor}, boundary ${f.disBoundary.toFixed(2)} < ${f.boundary.toFixed(2)}, fill ${f.disFill.toFixed(2)})`);
      }
    }
    ok(seen > 0, `[${name}] ${theme}: the survey found fields (${seen})`);
  }
  await browser.close();
}

await run(chromium, "chromium");
await run(webkit, "webkit");
console.log(failed ? `\n${failed} failed` : "\nall ok");
process.exit(failed ? 1 : 0);

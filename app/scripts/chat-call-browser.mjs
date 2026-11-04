// Headless drive of the mock harness in Chromium AND WebKit (the app's engine):
// a chat tool call opens on click to what it was given. Run against
// `VITE_MOCK=1 vite --port 1472 --force` in app/, with PLAYWRIGHT_MODULE
// pointing at a local Playwright installation. CHAT_CALL_SHOTS names a
// directory for screenshots. Geometry is read, not eyeballed: a rect is not
// reachability (`elementFromPoint`), and a clipped row is not a broken one.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.CHAT_CALL_URL ?? "http://localhost:1472";
// CHAT_CALL_ENGINE runs one engine.
const shots = process.env.CHAT_CALL_SHOTS;
let failures = 0;
const check = (ok, m) => { console.log(`${ok ? "ok" : "not ok"} — ${m}`); if (!ok) failures++; };
const near = (a, b, by = 1.5) => Math.abs(a - b) <= by;

async function open(page, query = "") {
  await page.goto(`${base}/?turns=tools&agentprompt=off${query}`);
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator('li.row[data-slug="messaging"]').click();
  await page.locator('[data-testid="chat-list"] > li').first().waitFor();
  await page.waitForTimeout(250);
}
const row = (page, text) => page.locator('[data-testid="chat-call"]').filter({ hasText: text }).first();
const opener = (page, text) => row(page, text).locator('[data-testid="chat-call-open"]');
const chip = (page, text) => page.locator('.chat-tools [data-testid="chat-call-open"]', { hasText: text }).first();
const scrollTop = (page) => page.evaluate(() => Math.round(document.querySelector(".chat-scroll").scrollTop));
const fromBottom = (page) => page.evaluate(() => { const e = document.querySelector(".chat-scroll"); return Math.round(e.scrollHeight - e.clientHeight - e.scrollTop); });
const topOf = (l) => l.evaluate((e) => e.getBoundingClientRect().top - document.querySelector(".chat-scroll").getBoundingClientRect().top);
const settled = (page) => page.waitForTimeout(120);
/** The hue a translucent background composites to, over the first opaque ancestor. */
const CONTRAST = (el) => {
  const c = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
  const rgba = (s) => { c.clearRect(0, 0, 1, 1); c.fillStyle = s; c.fillRect(0, 0, 1, 1); return [...c.getImageData(0, 0, 1, 1).data]; };
  const lum = ([r, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
  const layers = [];
  for (let e = el; e; e = e.parentElement) { const b = rgba(getComputedStyle(e).backgroundColor); if (b[3]) { layers.push(b); if (b[3] === 255) break; } }
  let base = [255, 255, 255];
  for (const b of layers.reverse()) base = base.map((v, i) => v + (b[i] - v) * (b[3] / 255));
  const fg = lum(rgba(getComputedStyle(el).color)), bg = lum(base);
  return (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05);
};

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  if (process.env.CHAT_CALL_ENGINE && process.env.CHAT_CALL_ENGINE !== name) continue;
  const browser = await engine.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  page.setDefaultTimeout(10_000);
  console.log(`# ${name}`);
  await open(page);

  // ── closed: as it always was, and nothing is open by itself ──
  check((await page.locator('[data-testid="chat-call-full"]').count()) === 0, `${name}: nothing is open by itself, a failed call included`);
  const closed = await page.evaluate(() => {
    const m = (sel) => { const li = [...document.querySelectorAll('[data-testid="chat-call"]')].find((r) => r.querySelector(sel)); const s = li?.querySelector(".chat-call-sum"); const n = li?.querySelector(".chat-call-name"); const list = li?.closest(".chat-calls"); if (!s) return null; const c = getComputedStyle(s); return { h: li.getBoundingClientRect().height, nameLeft: n.getBoundingClientRect().left - list.getBoundingClientRect().left, sumWidth: s.getBoundingClientRect().width, ws: c.whiteSpace, ov: c.textOverflow, font: c.fontSize + c.fontFamily, color: c.color, rowWidth: li.getBoundingClientRect().width, listWidth: list.getBoundingClientRect().width }; };
    return { button: m(".chat-call-btn"), plain: m("div.chat-call") };
  });
  check(closed.button && closed.plain, `${name}: the fixture has a row of each kind`);
  for (const k of ["h", "nameLeft"]) check(near(closed.button[k], closed.plain[k], 0.5), `${name}: a button row's ${k} is a plain row's (${closed.button[k]} / ${closed.plain[k]})`);
  for (const k of ["ws", "ov", "font", "color"]) check(closed.button[k] === closed.plain[k], `${name}: a button row's line has the plain row's ${k}`);
  check(closed.button.ws === "nowrap" && closed.button.ov === "ellipsis", `${name}: the closed line is one ellipsised row`);
  check(closed.button.sumWidth > 20 && closed.button.rowWidth === closed.button.listWidth, `${name}: WebKit's button sizing leaves the line its width (${closed.button.sumWidth}px)`);

  // ── a multi-line command: open, whole, closed again ──
  await page.locator('[data-testid="chat-calls-more"]').first().click();
  const b = opener(page, "set -e");
  check((await b.getAttribute("aria-expanded")) === "false", `${name}: closed says so`);
  await b.click();
  const text = row(page, "set -e").locator('[data-testid="chat-call-text"]');
  await text.waitFor();
  const cmd = await text.evaluate((e) => e.textContent);
  check(cmd.includes("--exact\n\tcargo clippy") && cmd.endsWith("printf 'all done'"), `${name}: the whole command, its line breaks and tab kept`);
  check((await b.getAttribute("aria-expanded")) === "true", `${name}: open says so`);
  const css = await text.evaluate((e) => { const c = getComputedStyle(e); return { ws: c.whiteSpace, ov: c.overflowY, max: c.maxHeight, mono: c.fontFamily === getComputedStyle(e.closest("li").querySelector(".chat-call-sum")).fontFamily }; });
  check(css.ws === "pre-wrap" && css.ov === "visible" && css.max === "none" && css.mono, `${name}: monospace pre-wrap, with no scroller of its own`);
  const hits = await page.evaluate(() => {
    const at = (el) => { const r = el.getBoundingClientRect(); return document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); };
    const li = [...document.querySelectorAll('[data-testid="chat-call"]')].find((r) => r.querySelector('[data-testid="chat-call-text"]'));
    return { head: !!at(li.querySelector(".chat-call-btn"))?.closest(".chat-call-btn"), text: at(li.querySelector("pre"))?.tagName };
  });
  check(hits.head && hits.text === "PRE", `${name}: head and text are what a pointer reaches`);

  // Selecting and copying leave it open.
  const box = await text.boundingBox();
  await page.mouse.move(box.x + 4, box.y + 6);
  await page.mouse.down();
  await page.mouse.move(box.x + 180, box.y + 6, { steps: 4 });
  await page.mouse.up();
  const picked = await page.evaluate(() => getSelection().toString());
  check(picked.length > 3 && (await text.count()) === 1, `${name}: selecting text leaves it open (${JSON.stringify(picked.slice(0, 16))})`);
  await page.evaluate(() => getSelection().removeAllRanges());

  // The keyboard.
  await b.focus();
  await page.keyboard.press("Enter");
  await settled(page);
  check((await row(page, "set -e").locator('[data-testid="chat-call-full"]').count()) === 0, `${name}: Enter closes it`);
  await page.keyboard.press("Space");
  await text.waitFor();
  check(true, `${name}: Space opens it`);

  // ── the poll, and the fold ──
  await page.evaluate(() => window.__mock.agentReply("/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo/.worktrees/messaging", "A new reply."));
  await page.locator(".chat-bubble-agent", { hasText: "A new reply." }).waitFor();
  await page.evaluate(() => document.querySelector('[data-testid="chat-refresh"]')?.click());
  await settled(page);
  check((await text.count()) === 1 && (await b.getAttribute("aria-expanded")) === "true", `${name}: a re-read keeps the row open`);
  await page.locator('[data-testid="chat-calls-more"]').first().click();
  await settled(page);
  check((await page.locator('[data-testid="chat-call-text"]').count()) === 0, `${name}: folding hides the open row`);
  await page.locator('[data-testid="chat-calls-more"]').first().click();
  await text.waitFor();
  check(true, `${name}: unfolding finds it still open, where it was`);
  await b.click();

  // ── scrolling: the row stays under the pointer, and a tall one opens from its top ──
  const tall = opener(page, "set -euo pipefail");
  await tall.scrollIntoViewIfNeeded();
  await page.evaluate(() => { document.querySelector(".chat-scroll").scrollTop -= 60; });
  await settled(page);
  let before = await topOf(tall), st = await scrollTop(page);
  await tall.click();
  await row(page, "set -euo pipefail").locator('[data-testid="chat-call-text"]').waitFor();
  await settled(page);
  let after = await topOf(tall);
  check(before > 100 && after >= 0 && after <= 9, `${name}: opening a tall command low in the view brings its row to the top, its text under it (${Math.round(before)} → ${Math.round(after)}, height ${Math.round((await row(page, "set -euo pipefail").boundingBox()).height)})`);
  const scrollers = await page.evaluate(() => [...document.querySelectorAll('[data-testid="chat-call-full"], [data-testid="chat-call-full"] *')].filter((e) => e.scrollHeight > e.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(e).overflowY)).length);
  check(scrollers === 0, `${name}: no nested scroller at any height`);
  before = await topOf(tall);
  await tall.click();
  await settled(page);
  after = await topOf(tall);
  check(near(before, after, 2) || after > before, `${name}: closing it holds its row (${Math.round(before)} → ${Math.round(after)})`);

  // Scrolled up, a command that fits under its row moves nothing at all.
  const small = opener(page, "Survey the transcript fixtures");
  await small.scrollIntoViewIfNeeded();
  await page.evaluate(() => { document.querySelector(".chat-scroll").scrollTop -= 120; });
  await settled(page);
  st = await scrollTop(page);
  before = await topOf(small);
  const fits = before > 0 && before < 600;
  check(fits && (await fromBottom(page)) > 48, `${name}: the fixture is a row in view, scrolled up (${Math.round(before)})`);
  await small.click();
  await row(page, "Survey the transcript fixtures").locator('[data-testid="chat-call-text"]').waitFor();
  await settled(page);
  check(near(await scrollTop(page), st, 1) && near(await topOf(small), before, 1), `${name}: opening one that fits does not scroll (${st} → ${await scrollTop(page)})`);
  await small.click();
  await settled(page);
  check(near(await topOf(small), before, 1), `${name}: nor does closing it`);

  // At the very bottom on a short window, with a good deal to open: following
  // would show its tail and push its row off the top.
  await page.setViewportSize({ width: 1280, height: 460 });
  await settled(page);
  await page.evaluate(() => { const e = document.querySelector(".chat-scroll"); e.scrollTop = e.scrollHeight; });
  await settled(page);
  const batch = chip(page, "browser_batch");
  check((await fromBottom(page)) <= 1 && (await topOf(batch)) >= 0, `${name}: the pill is on screen, at the bottom (${Math.round(await topOf(batch))})`);
  await batch.click();
  const batchText = page.locator('.chat-tool-open [data-testid="chat-call-text"]');
  await batchText.waitFor();
  await settled(page);
  const place = await page.evaluate(() => { const e = document.querySelector(".chat-scroll").getBoundingClientRect(), o = document.querySelector(".chat-tool-open .chat-call-full").getBoundingClientRect(); return { over: o.bottom - e.bottom }; });
  const pillTop = await topOf(batch);
  check(pillTop >= 0 && (place.over <= 1 || pillTop <= 9), `${name}: opening it there keeps its row on screen and shows the text, from its start (row at ${Math.round(pillTop)}, ${Math.round(place.over)}px past the view)`);
  check((await fromBottom(page)) > 48, `${name}: and stops following, while it is being read`);
  await batch.click();
  await settled(page);
  check((await fromBottom(page)) <= 48, `${name}: closing it is back at the bottom`);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.evaluate(() => { const e = document.querySelector(".chat-scroll"); e.scrollTop = e.scrollHeight; });
  await settled(page);
  await page.evaluate(() => window.__mock.agentReply("/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo/.worktrees/messaging", "Followed."));
  await page.locator(".chat-bubble-agent", { hasText: "Followed." }).waitFor();
  await settled(page);
  check((await fromBottom(page)) <= 1, `${name}: and it follows a new reply from there`);

  // ── MCP pills ──
  const pill = chip(page, "browser_batch");
  const pillLook = await page.evaluate(() => {
    const b = [...document.querySelectorAll(".chat-tools .chat-tool-btn")][0];
    const p = [...document.querySelectorAll(".chat-tools li.chat-tool")][0];
    const c = (e) => { const s = getComputedStyle(e); return [s.height, s.fontSize, s.fontFamily, s.color, s.borderRadius, s.paddingLeft, s.backgroundColor, s.lineHeight].join("|"); };
    return { b: c(b), p: c(p), name: b.textContent, plainName: p.textContent };
  });
  check(pillLook.b === pillLook.p, `${name}: a closed pill looks like a plain one (${pillLook.b} / ${pillLook.p})`);
  check(pillLook.name === "mcp__claude-in-chrome__browser_batch" && pillLook.plainName === "mcp__x__ping", `${name}: pills keep the whole name; an empty input is no button`);
  await pill.click();
  const json = page.locator('.chat-tool-open [data-testid="chat-call-text"]');
  await json.waitFor();
  const shown = await json.evaluate((e) => e.textContent);
  check(JSON.parse(shown).tabId === 7 && shown.startsWith('{\n  "tabId": 7,'), `${name}: an MCP call opens to its input, pretty-printed`);
  const wide = await page.evaluate(() => { const o = document.querySelector(".chat-tool-open").getBoundingClientRect(); const u = document.querySelector(".chat-tools").getBoundingClientRect(); return near1(o.width, u.width); function near1(a, b) { return Math.abs(a - b) < 2; } });
  check(wide, `${name}: it takes the whole row under the pill`);
  if (shots) await page.screenshot({ path: `${shots}/${name}-mcp-open.png` });

  // ── the themes ──
  for (const theme of ["tokyo-night", "tokyo-day", "catppuccin-latte", "nord"]) {
    await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    await settled(page);
    const ratio = await json.evaluate(CONTRAST);
    check(ratio >= 4.5, `${name}: the text reads in ${theme} (${ratio.toFixed(2)}:1)`);
    const diff = await page.evaluate(() => { const p = document.querySelector(".chat-call-full"), s = document.querySelector(".chat-scroll"); return getComputedStyle(p).backgroundColor !== getComputedStyle(s).backgroundColor; });
    check(diff, `${name}: the block's surface differs from the chat's in ${theme}`);
    if (shots && (theme === "tokyo-night" || theme === "tokyo-day")) await page.screenshot({ path: `${shots}/${name}-${theme}.png` });
  }
  await page.evaluate(() => { delete document.documentElement.dataset.theme; });

  // ── a line cut only by its width ──
  await page.setViewportSize({ width: 1600, height: 800 });
  await settled(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  const cut = row(page, "playwright test nav-row.spec.ts");
  const cutOpen = cut.locator('[data-testid="chat-call-open"]');
  const clipped = await cut.locator(".chat-call-sum").evaluate((e) => e.scrollWidth > e.clientWidth);
  check((await cutOpen.count()) === (clipped ? 1 : 0), `${name}: a row with no \`detail\` is a button exactly when its line is cut (${clipped})`);
  const short = row(page, "app/src/App.css");
  check((await short.locator('[data-testid="chat-call-open"]').count()) === 0, `${name}: a short path with nothing more is no button`);
  if (clipped) {
    await cutOpen.click();
    const full = await cut.locator('[data-testid="chat-call-text"]').evaluate((e) => e.textContent);
    check(full.endsWith("--repeat-each=3"), `${name}: it opens to its whole line`);
    await cutOpen.click();
  }

  // ── what the host can answer ──
  for (const [mode, testid, words] of [
    ["gone", "chat-call-gone", "No longer available"],
    ["old", "chat-call-old", "This host doesn't support showing a whole tool call yet."],
    ["fail", "chat-call-failed", "Couldn’t read this call"],
    ["cut", "chat-call-cut", "first 64 KB of 70 KB"],
  ]) {
    await open(page, `&toolcall=${mode}`);
    await page.locator('[data-testid="chat-calls-more"]').first().click();
    await opener(page, "set -e").click();
    const note = row(page, "set -e").locator(`[data-testid="${testid}"]`);
    await note.waitFor();
    check((await note.textContent()).includes(words), `${name}: ${mode} says so: ${await note.textContent()}`);
    if (mode !== "cut") check((await row(page, "set -e").locator('[data-testid="chat-call-text"]').textContent()) === "set -e", `${name}: ${mode} keeps the line it had`);
  }
  await open(page, "&toolcall=slow");
  await page.locator('[data-testid="chat-calls-more"]').first().click();
  await opener(page, "set -e").click();
  check((await row(page, "set -e").locator('.chat-call-note[role="status"]').count()) === 1, `${name}: a slow answer says it is reading`);
  await browser.close();
}
console.log(failures ? `\nchat-call-browser: ${failures} failure(s)` : "\nchat-call-browser: all good");
process.exit(failures ? 1 : 0);

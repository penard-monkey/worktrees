// Headless drive of the isolated mock harness on port 1455, in Chromium AND
// WebKit (the app's engine): does the chat keep its scroll when a reply
// arrives? Run against `VITE_MOCK=1 vite --port 1455 --force` in app/, with
// PLAYWRIGHT_MODULE pointing at a local Playwright installation.
//
// `?turns=long` gives the place 24 turns, so `agent_turns`' 20-turn window
// SLIDES on every new turn: the oldest leaves the top as the newest arrives.
// Each case records geometry (scrollTop, scrollHeight, clientHeight, the
// distance from the bottom, and the item at the top of the view with its
// offset) before the update, every frame for a second after it, and once it
// has settled, then checks the rule:
//   - at the bottom, the view ends at the bottom on EVERY frame (no flash);
//   - scrolled up, the item at the top of the view stays there, to the pixel,
//     and "New messages" is offered for what arrived below.
// `--report` prints the numbers and does not fail (the "before" run).
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");

const base = process.env.CHAT_SCROLL_URL ?? "http://localhost:1455";
const report = process.argv.includes("--report");
const traceAll = process.argv.includes("--trace");
const only = process.argv.find((a) => a.startsWith("--engine="))?.slice(9);
let failures = 0;

function check(ok, message) {
  if (ok) console.log(`ok — ${message}`);
  else if (report) console.log(`not ok — ${message}`);
  else { failures++; console.log(`not ok — ${message}`); }
}

/** Installs `window.__geom`: the chat scroller's geometry, and the item at
 *  the top of its view. */
const INSTALL = () => { window.__geom = () => {
  const el = document.querySelector(".chat-scroll");
  const box = el.getBoundingClientRect();
  const rows = [...document.querySelectorAll('[data-testid="chat-list"] > li')];
  const top = rows.find((li) => li.getBoundingClientRect().bottom > box.top + 1);
  const r = top?.getBoundingClientRect();
  return {
    st: Math.round(el.scrollTop), sh: el.scrollHeight, ch: el.clientHeight,
    fromBottom: Math.round(el.scrollHeight - el.clientHeight - el.scrollTop),
    top: top ? top.textContent.replace(/\s+/g, " ").trim().slice(0, 48) : null,
    topOff: r ? Math.round(r.top - box.top) : null,
    rows: rows.length,
    fresh: !!document.querySelector('[data-testid="chat-new"]'),
  };
}; };

/** Sample `__geom` every animation frame until `window.__stopTrace()`. */
const TRACE = () => {
  const geom = window.__geom;
  const out = [];
  let on = true;
  const t0 = performance.now();
  const tick = () => { if (!on) return; out.push({ t: Math.round(performance.now() - t0), ...geom() }); requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  window.__stopTrace = () => { on = false; return out; };
};

async function geom(page) { return page.evaluate(() => window.__geom()); }
async function frames(page, n = 2) {
  await page.evaluate((n) => new Promise((res) => { let i = 0; const f = () => (++i >= n ? res() : requestAnimationFrame(f)); requestAnimationFrame(f); }), n);
}
async function settle(page) {
  // Two quiet frames in a row, at most ~1 s.
  let last = JSON.stringify(await geom(page));
  for (let i = 0; i < 60; i++) {
    await frames(page, 2);
    const now = JSON.stringify(await geom(page));
    if (now === last) return JSON.parse(now);
    last = now;
  }
  return JSON.parse(last);
}

async function open(page) {
  await page.goto(`${base}/?turns=long&activity=3&agentprompt=off`);
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator('li.row[data-slug="messaging"]').click();
  await page.locator('[data-testid="chat-list"] > li').first().waitFor();
  await page.evaluate(INSTALL);
  await page.waitForTimeout(300);
  return settle(page);
}

/** Run `act`, wait for `landed` to hold in the page, and trace the frames between. */
async function during(page, act, landed) {
  const before = await settle(page);
  await page.evaluate(TRACE);
  await act();
  await page.waitForFunction(landed, null, { timeout: 8000 });
  await page.waitForTimeout(600);
  const trace = await page.evaluate(() => window.__stopTrace());
  const after = await settle(page);
  return { before, trace, after };
}

const fmt = (g) => `st=${g.st} sh=${g.sh} ch=${g.ch} fromBottom=${g.fromBottom} rows=${g.rows} top=${JSON.stringify(g.top)}@${g.topOff}${g.fresh ? " [New messages]" : ""}`;
function show(name, r) {
  console.log(`  ${name}`);
  console.log(`    before: ${fmt(r.before)}`);
  const worst = r.trace.reduce((m, g) => (g.fromBottom > m.fromBottom ? g : m), r.trace[0] ?? r.after);
  console.log(`    worst frame: ${fmt(worst)} (t=${worst.t}ms, of ${r.trace.length} frames)`);
  console.log(`    after:  ${fmt(r.after)}`);
  if (traceAll) {
    let prev = "";
    for (const g of r.trace) {
      const k = fmt(g);
      if (k !== prev) console.log(`      t=${g.t}: ${k}`);
      prev = k;
    }
  }
}

async function send(page, text) {
  await page.locator('[data-testid="chat-field"]').fill(text);
  await page.locator('[data-testid="chat-send"]').click();
}
const replyCount = () => document.querySelectorAll('[data-testid="chat-agent"]').length;
const path = "/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo/.worktrees/messaging";

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  if (only && only !== name) continue;
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    page.setDefaultTimeout(12_000);
    console.log(`# ${name}`);
    const opened = await open(page);
    console.log(`  opened: ${fmt(opened)}`);
    check(opened.fromBottom <= 1, `${name}: opens at the bottom`);

    // 1. At the bottom, a reply lands in the last turn (no slide, count grows).
    let n = await page.evaluate(replyCount);
    let r = await during(page,
      () => page.evaluate((p) => window.__mock.agentReply(p, "A short reply at the bottom."), path),
      (n) => document.querySelectorAll('[data-testid="chat-agent"]').length > n && document.body.textContent.includes("A short reply at the bottom."));
    show("1. at the bottom: a reply joins the last turn", r);
    check(r.after.fromBottom <= 1, `${name}: (1) ends at the bottom`);
    check(r.trace.every((g) => g.fromBottom <= 1), `${name}: (1) no frame leaves the bottom`);

    // 2. At the bottom, a message is sent: the window slides as the new turn
    //    appears (the oldest turn leaves the top), the pending one settles,
    //    and three seconds later the reply arrives.
    r = await during(page,
      () => send(page, "Slide the window, please."),
      () => [...document.querySelectorAll('[data-testid="chat-agent"]')].some((e) => e.textContent.includes("On it. (A scripted reply")));
    show("2. at the bottom: send, the window slides, the reply arrives", r);
    check(r.after.fromBottom <= 1, `${name}: (2) ends at the bottom`);
    check(r.trace.every((g) => g.fromBottom <= 1), `${name}: (2) no frame leaves the bottom`);

    // 3. At the bottom, a LONG reply lands.
    const long = Array.from({ length: 14 }, (_, i) => `Long reply paragraph ${i + 1}: enough words to wrap across the column and push the bottom far down the page.`).join("\n\n");
    r = await during(page,
      () => page.evaluate(([p, t]) => window.__mock.agentReply(p, t), [path, long]),
      () => document.body.textContent.includes("Long reply paragraph 14"));
    show("3. at the bottom: a long reply lands", r);
    check(r.after.fromBottom <= 1, `${name}: (3) ends at the bottom`);
    check(r.trace.every((g) => g.fromBottom <= 1), `${name}: (3) no frame leaves the bottom`);

    // 4. Scrolled up: send, so the window slides (turns drop off the top) and
    //    the reply arrives below. The item at the top must not move.
    const box = await page.locator(".chat-scroll").boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.wheel(0, -1800);
    await page.waitForTimeout(400);
    r = await during(page,
      () => send(page, "Slide it again while I read."),
      () => [...document.querySelectorAll('[data-testid="chat-agent"]')].filter((e) => e.textContent.includes("On it. (A scripted reply")).length >= 2);
    show("4. scrolled up: send, the window slides, the reply arrives below", r);
    check(r.after.top === r.before.top && Math.abs(r.after.topOff - r.before.topOff) <= 1, `${name}: (4) the item at the top stays put (${JSON.stringify(r.before.top)}@${r.before.topOff} → ${JSON.stringify(r.after.top)}@${r.after.topOff})`);
    check(r.trace.every((g) => g.top === r.before.top && Math.abs(g.topOff - r.before.topOff) <= 1), `${name}: (4) on every frame`);
    check(r.after.fresh, `${name}: (4) "New messages" is offered`);

    // 5. Still scrolled up: a reply joins the last turn, below the view.
    await page.mouse.wheel(0, -300);
    await page.waitForTimeout(400);
    r = await during(page,
      () => page.evaluate((p) => window.__mock.agentReply(p, "Another reply, below the reader."), path),
      () => document.body.textContent.includes("Another reply, below the reader."));
    show("5. scrolled up: a reply lands below the view", r);
    check(r.after.top === r.before.top && Math.abs(r.after.topOff - r.before.topOff) <= 1, `${name}: (5) the item at the top stays put (${JSON.stringify(r.before.top)}@${r.before.topOff} → ${JSON.stringify(r.after.top)}@${r.after.topOff})`);
    check(r.after.fresh, `${name}: (5) "New messages" is offered`);

    // 6. "New messages" takes the reader to the bottom, and the next reply
    //    is followed again.
    await page.locator('[data-testid="chat-new"]').click();
    await settle(page);
    r = await during(page,
      () => page.evaluate((p) => window.__mock.agentReply(p, "Followed again."), path),
      () => document.body.textContent.includes("Followed again."));
    show("6. after New messages: a reply at the bottom", r);
    check(r.after.fromBottom <= 1 && !r.after.fresh, `${name}: (6) ends at the bottom, no New messages`);

    // 7. Scrolled up, the window narrows: every row above reflows, and the
    //    row at the top must not move (native anchoring is off, so this is
    //    the ResizeObserver's hold).
    await page.mouse.wheel(0, -1500);
    await page.waitForTimeout(400);
    let before = await settle(page);
    await page.setViewportSize({ width: 980, height: 800 });
    await page.waitForTimeout(300);
    let after = await settle(page);
    console.log(`  7. scrolled up: the window narrows\n    before: ${fmt(before)}\n    after:  ${fmt(after)}`);
    check(after.top === before.top && Math.abs(after.topOff - before.topOff) <= 1, `${name}: (7) the item at the top stays put through a reflow (${JSON.stringify(before.top)}@${before.topOff} → ${JSON.stringify(after.top)}@${after.topOff})`);
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.waitForTimeout(300);

    // 8. At the bottom, the composer grows to several lines: the bottom
    //    stays in view.
    await page.mouse.wheel(0, 20000);
    await page.waitForTimeout(400);
    before = await settle(page);
    await page.locator('[data-testid="chat-field"]').fill(Array.from({ length: 8 }, (_, i) => `line ${i + 1}`).join("\n"));
    await page.waitForTimeout(300);
    after = await settle(page);
    console.log(`  8. at the bottom: the composer grows\n    before: ${fmt(before)}\n    after:  ${fmt(after)}`);
    check(after.ch < before.ch && after.fromBottom <= 1, `${name}: (8) the view shrank by ${before.ch - after.ch}px and kept the bottom`);
    await page.locator('[data-testid="chat-field"]').fill("");
    await page.waitForTimeout(300);

    // 9. At the bottom, a prompt comes up that fits: it is in view, at the bottom.
    await settle(page);
    await page.evaluate((p) => window.__mock.agentPrompt(p, "permission"), path);
    await page.locator('[data-testid="chat-prompt"]').waitFor();
    after = await settle(page);
    const card = await page.evaluate(() => {
      const el = document.querySelector(".chat-scroll"), c = document.querySelector(".chat-card");
      const b = el.getBoundingClientRect(), r = c.getBoundingClientRect();
      return { top: Math.round(r.top - b.top), bottom: Math.round(r.bottom - b.top), h: Math.round(r.height), ch: el.clientHeight };
    });
    console.log(`  9. a prompt that fits comes up\n    after:  ${fmt(after)} card=${JSON.stringify(card)}`);
    check(after.fromBottom <= 1 && card.top >= 0 && card.bottom <= card.ch, `${name}: (9) the card is in view, at the bottom`);

    // 10. Scrolled up, a prompt TALLER than the view comes up: it is brought
    //     into view at its top, so what it asks is read before its choices.
    await page.mouse.wheel(0, -1500);
    await page.waitForTimeout(400);
    await page.evaluate((p) => window.__mock.agentPrompt(p, "tall"), path);
    await page.waitForFunction(() => { const c = document.querySelector(".chat-card"), el = document.querySelector(".chat-scroll"); return c && c.offsetHeight > el.clientHeight; });
    await settle(page);
    const tall = await page.evaluate(() => {
      const el = document.querySelector(".chat-scroll"), c = document.querySelector(".chat-card");
      return { top: Math.round(c.getBoundingClientRect().top - el.getBoundingClientRect().top), h: c.offsetHeight, ch: el.clientHeight };
    });
    console.log(`  10. scrolled up: a prompt taller than the view comes up\n    card=${JSON.stringify(tall)}`);
    check(tall.top >= 0 && tall.top <= 16, `${name}: (10) the tall card opens at its top (${tall.top}px from the top of the view)`);
  } finally {
    await browser.close();
  }
}
// 11. A BUSY place ("Claude is working" below the list, the send queued):
//     at the bottom, send, the window slides, the reply arrives.
for (const [name, engine] of Object.entries({ chromium, webkit })) {
  if (only && only !== name) continue;
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    page.setDefaultTimeout(12_000);
    console.log(`# ${name}, busy`);
    await page.goto(`${base}/?turns=long&activity=1&agentprompt=off`);
    await page.locator('[data-testid="places-rail"]').click();
    await page.locator('li.row[data-slug="messaging"]').click();
    await page.locator('[data-testid="chat-typing"]').waitFor();
    await page.evaluate(INSTALL);
    await page.waitForTimeout(300);
    const r = await during(page,
      () => send(page, "Queue this behind the turn."),
      () => [...document.querySelectorAll('[data-testid="chat-agent"]')].some((e) => e.textContent.includes("On it. (A scripted reply")));
    show("11. busy, at the bottom: send (queued), the window slides, the reply arrives", r);
    check(r.after.fromBottom <= 1, `${name}: (11) ends at the bottom`);
    check(r.trace.every((g) => g.fromBottom <= 1), `${name}: (11) no frame leaves the bottom`);
  } finally {
    await browser.close();
  }
}
if (failures) { console.log(`${failures} failed`); process.exit(1); }

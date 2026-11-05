// Headless drive of the mock harness in Chromium AND WebKit (the app's engine):
// a message of several lines, with real keys. Shift+Enter and Cmd+Enter add a
// line, Enter sends, the field grows to its limit and then scrolls with no
// sideways bar, and a host that takes one line says so and offers the way out.
//
//   VITE_MOCK=1 vite --port 1457 --force      # in app/, the mock harness
//   PLAYWRIGHT_MODULE=<…/playwright/index.mjs> node app/scripts/chat-lines-browser.mjs
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.CHAT_LINES_URL ?? "http://localhost:1457";
const only = process.env.CHAT_LINES_ENGINE;
let failures = 0;
const check = (ok, m, said = "") => { console.log(`${ok ? "ok" : "not ok"} — ${m}${ok || !said ? "" : `: ${said}`}`); if (!ok) failures++; };
const FIELD = '[data-testid="chat-field"]';
const LINE = '[data-testid="chat-line"]';
const SEND = '[data-testid="chat-send"]';

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  if (only && only !== name) continue;
  const browser = await engine.launch({ headless: true });
  const open = async (query = "", viewport = { width: 1280, height: 900 }) => {
    const page = await (await browser.newContext({ viewport })).newPage();
    page.setDefaultTimeout(12_000);
    await page.goto(`${base}/?agentprompt=off${query}`);
    await page.locator('[data-testid="places-rail"]').click();
    await page.locator('li.row[data-slug="messaging"]').click();
    await page.locator(FIELD).waitFor();
    await page.waitForTimeout(200);
    return page;
  };
  const value = (page) => page.locator(FIELD).inputValue();
  const words = (page) => page.locator(LINE).innerText();
  const box = (page) => page.locator(FIELD).evaluate((e) => {
    const cs = getComputedStyle(e);
    return { h: e.offsetHeight, ch: e.clientHeight, sh: e.scrollHeight, cw: e.clientWidth, sw: e.scrollWidth, y: cs.overflowY, x: cs.overflowX, line: parseFloat(cs.lineHeight),
      chat: e.closest(".chat").clientHeight };
  });
  /** What is at the send button's centre: a rect is not reachability. */
  const reachable = (page) => page.locator(SEND).evaluate((b) => {
    const r = b.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return !!hit && (hit === b || b.contains(hit));
  });
  const bubbles = (page) => page.locator('[data-testid="chat-user"]').count();
  try {
    // Keys.
    let page = await open();
    const before = await bubbles(page);
    await page.locator(FIELD).click();
    await page.keyboard.type("one");
    await page.keyboard.press("Shift+Enter");
    await page.keyboard.type("two");
    await page.keyboard.press("Meta+Enter");
    await page.keyboard.type("three");
    check(await value(page) === "one\ntwo\nthree", `${name}: Shift+Enter and Cmd+Enter each add a line`, JSON.stringify(await value(page)));
    check(await bubbles(page) === before, `${name}: neither sends`);
    check(/Shift\+Enter adds a line/.test(await words(page)), `${name}: the line under the field says how`, await words(page));
    // A mid-text break goes at the caret.
    await page.locator(FIELD).evaluate((e) => e.setSelectionRange(3, 3));
    await page.keyboard.press("Meta+Enter");
    check(await value(page) === "one\n\ntwo\nthree", `${name}: Cmd+Enter breaks at the caret`, JSON.stringify(await value(page)));
    await page.keyboard.press("ControlOrMeta+z");
    await page.waitForTimeout(60);
    // WebKit undoes the whole typing run in one step, Chromium the break alone.
    check(!(await value(page)).includes("\n\n"), `${name}: Cmd+Enter is undoable`, JSON.stringify(await value(page)));
    await page.locator(FIELD).fill("one\ntwo");
    // IME: Enter that confirms a composition is not a send or a break.
    await page.locator(FIELD).evaluate((e) => {
      e.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: false, isComposing: true, bubbles: true, cancelable: true }));
    });
    await page.waitForTimeout(60);
    check(await bubbles(page) === before && (await value(page)).includes("one"), `${name}: Enter during IME composition does nothing`);
    await page.locator(FIELD).fill("one\ntwo\nthree");
    await page.keyboard.press("Enter");
    await page.waitForFunction((n) => document.querySelectorAll('[data-testid="chat-user"]').length > n, before);
    check(await value(page) === "", `${name}: Enter sends and clears the draft`);
    const bubble = page.locator('[data-testid="chat-user"]').last();
    const shown = await bubble.evaluate((b) => { const p = b.querySelector("p"); return { text: b.innerText, h: p.getBoundingClientRect().height, line: parseFloat(getComputedStyle(p).lineHeight) }; });
    check(/one\ntwo\nthree/.test(shown.text), `${name}: the bubble keeps the breaks`, JSON.stringify(shown.text));
    check(shown.h >= shown.line * 2.5, `${name}: the bubble draws three rows`, `${shown.h}px at ${shown.line}px`);
    check(await page.locator('.chat-bubble-pending').count() === 0, `${name}: nothing is left pending beside its record`);

    // Later lines may start with what the first may not.
    await page.locator(FIELD).fill("look\n/clear\n@file\n#x\n!ls");
    check(!/alert/.test(await page.locator(LINE).getAttribute("class")), `${name}: a later / @ # ! is text`, await words(page));
    await page.locator(FIELD).fill("@file\nlook");
    check(/can’t start with @/.test(await words(page)), `${name}: the first line still may not`, await words(page));
    await page.locator(FIELD).fill("a\tb\nc");
    check(/tabs/.test(await words(page)), `${name}: a tab is still refused, in words`, await words(page));
    await page.locator(FIELD).fill("x".repeat(16 * 1024));
    check(!/alert/.test(await page.locator(LINE).getAttribute("class")) && /16,384 of 16,384/.test(await words(page)), `${name}: 16 KB is allowed and counted`, await words(page));
    await page.locator(FIELD).fill("x".repeat(16 * 1024 + 1));
    check(/Too long/.test(await words(page)), `${name}: past it is refused`, await words(page));

    // The field grows, then scrolls: #590's bars must stay away below the cap.
    page = await open();
    for (const lines of [2, 6, 12]) {
      await page.locator(FIELD).fill(Array.from({ length: lines }, (_, i) => `row ${i}`).join("\n"));
      await page.waitForTimeout(60);
      const b = await box(page);
      check(b.y === "hidden" && b.sh <= b.ch && b.sw <= b.cw, `${name}: ${lines} lines fit with no scroll bar`, JSON.stringify(b));
    }
    await page.locator(FIELD).fill(Array.from({ length: 40 }, (_, i) => `row ${i}`).join("\n"));
    await page.waitForTimeout(60);
    let b = await box(page);
    check(b.y === "auto" && b.sw <= b.cw, `${name}: 40 lines scroll down, never sideways`, JSON.stringify(b));
    check(b.h <= b.line * 12 + 40 && b.h >= b.line * 11, `${name}: the field stops at 12 rows`, JSON.stringify(b));
    check(await reachable(page), `${name}: Send is reachable beside a full field`);
    // A short window: the share is the limit, and refits as the window moves.
    await page.setViewportSize({ width: 1280, height: 420 });
    await page.waitForTimeout(150);
    b = await box(page);
    check(b.h <= b.chat * 0.4 + 2, `${name}: a short window caps it at 40% of the chat`, JSON.stringify(b));
    check(await reachable(page), `${name}: Send is reachable in a short window`);

    // A host that says nothing is an older host: one line, in words.
    page = await open("&remote");
    await page.locator(FIELD).click();
    await page.keyboard.type("one");
    check(/takes one line here/.test(await words(page)), `${name}: an older host's composer says one line`, await words(page));
    await page.keyboard.press("Shift+Enter");
    await page.keyboard.press("Meta+Enter");
    check(await value(page) === "one", `${name}: no key adds a line there`, JSON.stringify(await value(page)));
    await page.locator(FIELD).fill("one\ntwo");
    check(/Use one line/.test(await words(page)), `${name}: a pasted break is refused in words`, await words(page));
    await page.getByRole("button", { name: "Join into one line" }).click();
    check(await value(page) === "one two", `${name}: and joining is one click`, JSON.stringify(await value(page)));

    // What it advertised is what the composer takes.
    page = await open(`&remote&sendcaps=${encodeURIComponent(JSON.stringify({ line_breaks: false, max_bytes: 4096, harnesses: { claude: { line_breaks: true, max_bytes: 16384 } } }))}`);
    await page.locator(FIELD).fill("one\ntwo");
    check(!/alert/.test(await page.locator(LINE).getAttribute("class")), `${name}: a host's send_caps let line breaks through`, await words(page));

    // A stale client meets a refusal: words and a way out, not a dead end.
    page = await open("&agentsend=oldhost");
    await page.locator(FIELD).fill("one\ntwo");
    await page.keyboard.press("Enter");
    await page.waitForTimeout(150);
    check(/takes one line at a time/.test(await words(page)), `${name}: a refused break is worded`, await words(page));
    check(await value(page) === "one\ntwo", `${name}: the draft stays`);
    await page.getByRole("button", { name: "Join into one line" }).click();
    check(await value(page) === "one two", `${name}: and can be joined`, JSON.stringify(await value(page)));

    // Stopped part way, or someone typed: said, whole draft kept, terminal offered.
    for (const [mode, re] of [["stopped", /about 512 of 2048 bytes/], ["foreign", /something other than this message/]]) {
      page = await open(`&agentsend=${mode}`);
      const n = await bubbles(page);
      await page.locator(FIELD).fill("one\ntwo");
      await page.keyboard.press("Enter");
      await page.waitForTimeout(150);
      const said = await words(page);
      check(re.test(said) && /Nothing was submitted/.test(said), `${name}: ${mode}: the words say what happened`, said);
      check(await value(page) === "one\ntwo", `${name}: ${mode}: the whole draft stays`);
      check(await bubbles(page) === n && await page.locator(".chat-bubble-pending").count() === 0, `${name}: ${mode}: no bubble claims it went`);
      check(await page.getByRole("button", { name: "Open the terminal" }).count() === 1, `${name}: ${mode}: the terminal is offered`);
    }
  } finally {
    await browser.close();
  }
}
if (failures) { console.error(`${failures} failed`); process.exit(1); }

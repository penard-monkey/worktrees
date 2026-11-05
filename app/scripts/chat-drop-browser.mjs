// Headless drive of the mock harness in Chromium AND WebKit (the app's engine):
// dropping a place from the nav onto the chat puts its reference in the draft,
// at the cursor, and sends nothing. Real pointer input (Playwright's mouse goes
// through hit-testing, a synthetic drag does not: AGENTS.md).
//
//   VITE_MOCK=1 vite --port 1457 --force      # in app/, the mock harness
//   PLAYWRIGHT_MODULE=<…/playwright/index.mjs> node app/scripts/chat-drop-browser.mjs
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.CHAT_DROP_URL ?? "http://localhost:1457";
const only = process.env.CHAT_DROP_ENGINE;
let failures = 0;
const check = (ok, m, said = "") => { console.log(`${ok ? "ok" : "not ok"} — ${m}${ok || !said ? "" : `: ${said}`}`); if (!ok) failures++; };

const errors = [];
const FIELD = '[data-testid="chat-field"]';
const CARGO = 'li.row[data-slug="messaging"]';

async function open(browser, query = "") {
  // A fresh context each time: the rail's state persists across a reload.
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 900 } })).newPage();
  page.setDefaultTimeout(12_000);
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}/?agentprompt=off${query}`);
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator(CARGO).click();
  await page.locator(FIELD).waitFor();
  await page.waitForTimeout(250);
  return page;
}

/** Press on a nav row, travel to `to` ({x, y}), and return what is under the
 *  pointer and what the page shows mid-drag; `drop` releases, else Escape. */
async function drag(page, slug, to, drop = true) {
  const box = await page.locator(`li.row[data-slug="${slug}"]`).first().boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 12, box.y + box.height / 2 + 12, { steps: 3 });
  await page.mouse.move(to.x, to.y, { steps: 6 });
  await page.waitForTimeout(80);
  const seen = await page.evaluate(({ x, y }) => {
    const el = document.elementFromPoint(x, y);
    const chat = el?.closest(".chat");
    return {
      inChat: !!chat,
      place: document.body.classList.contains("dragging-place"),
      lit: document.body.classList.contains("drop-mention"),
      outline: chat ? getComputedStyle(chat).outlineStyle : "",
      why: document.querySelector(".drag-why")?.textContent ?? "",
    };
  }, to);
  await page.mouse.up();
  await page.waitForTimeout(150);
  return seen;
}
const centre = async (page, sel) => {
  const b = await page.locator(sel).first().boundingBox();
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
};
const value = (page) => page.locator(FIELD).inputValue();
const focused = (page) => page.evaluate((s) => document.activeElement === document.querySelector(s), FIELD);
const pasted = (page) => page.evaluate(() => window.__mock.drops().length);

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  if (only && only !== name) continue;
  const browser = await engine.launch({ headless: true });
  try {
    errors.length = 0;
    // Over the transcript, into an empty draft.
    let page = await open(browser);
    const scroll = await centre(page, ".chat-scroll");
    let seen = await drag(page, "billing-refactor", scroll);
    check(seen.inChat, `${name}: the transcript is what the pointer hits (elementFromPoint)`);
    check(seen.place && seen.lit, `${name}: the drop highlight is on over the transcript`, JSON.stringify(seen));
    check(seen.outline === "solid", `${name}: the chat draws the accent outline`, seen.outline);
    check(/message/i.test(seen.why), `${name}: the chip says it lands in the message`, seen.why);
    let v = await value(page);
    check(/^Look at @worktrees:place:\/\/billing-refactor $/.test(v), `${name}: an empty draft takes the lead-in, so it can be sent`, JSON.stringify(v));
    check(await focused(page), `${name}: focus lands in the composer`);
    check(await pasted(page) === 0, `${name}: nothing was pasted into the pane`);
    const line = await page.locator('[data-testid="chat-line"]').getAttribute("class");
    check(!/alert/.test(line ?? ""), `${name}: the draft it leaves has no refusal to show`, line);

    // Over the composer, at the cursor, spaces as needed.
    page = await open(browser);
    await page.locator(FIELD).fill("abc def");
    await page.locator(FIELD).evaluate((e) => e.setSelectionRange(3, 3));
    await drag(page, "billing-refactor", await centre(page, FIELD));
    v = await value(page);
    check(v === "abc @worktrees:place://billing-refactor def", `${name}: over the composer it goes in at the cursor, with no lead-in`, JSON.stringify(v));
    const caret = await page.locator(FIELD).evaluate((e) => [e.selectionStart, e.selectionEnd]);
    check(caret[0] === caret[1] && v.slice(0, caret[0]).endsWith("billing-refactor"), `${name}: the caret ends after the reference`, JSON.stringify(caret));

    // A selection is replaced, and the cursor at the very start still needs the lead-in.
    page = await open(browser);
    await page.locator(FIELD).fill("hello there");
    await page.locator(FIELD).evaluate((e) => e.setSelectionRange(0, 0));
    await drag(page, "billing-refactor", scroll);
    v = await value(page);
    check(v === "Look at @worktrees:place://billing-refactor hello there", `${name}: a drop at the start of a draft keeps it sendable`, JSON.stringify(v));

    // Another project: refused in words while reach is off, draft untouched.
    page = await open(browser);
    await page.locator(FIELD).fill("keep me");
    const other = "feat-redesign";
    {
      seen = await drag(page, other, scroll);
      check(!seen.lit, `${name}: another project with reach off does not light up`, JSON.stringify(seen));
      check(await value(page) === "keep me", `${name}: a refused drop leaves the draft alone`, await value(page));
      check(/reach is off/.test(seen.why), `${name}: the chip says why`, seen.why);
      // … and with reach on it is the plain-text address.
      page = await open(browser, "&reach=read");
      seen = await drag(page, other, scroll);
      v = await value(page);
      check(/^place [^ ]+:[^ ]+ $/.test(v), `${name}: another project with reach on is an address, no lead-in`, JSON.stringify(v));
    }

    // The terminal's own drop still pastes.
    page = await open(browser);
    await page.locator('[data-testid="agent-view-terminal"]').click();
    await page.waitForTimeout(150);
    await drag(page, "billing-refactor", await centre(page, ".term-wrap"));
    check(await pasted(page) === 1, `${name}: over the terminal the drop still pastes into the pane`);

    // xterm's Viewport races its renderer at mount, in the mock, with no drag at all.
    const own = errors.filter((e) => !/dimensions/.test(e));
    check(own.length === 0, `${name}: no page errors`, own.join("; "));
  } finally {
    await browser.close();
  }
}
if (failures) { console.error(`${failures} failed`); process.exit(1); }

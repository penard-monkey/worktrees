// Headless drive of the mock harness in Chromium AND WebKit (the app's engine):
// a bubble the host shortened is read in full from "Show full reply". Run
// against `VITE_MOCK=1 vite --port 1478 --force` in app/ (HMR is dead inside
// .worktrees/, so restart it after an edit), with PLAYWRIGHT_MODULE pointing
// at a local Playwright installation. CHAT_FULL_SHOTS names a directory for
// screenshots; CHAT_FULL_ENGINE runs one engine.
//
// `?turns=cut` gives the place turns the host shortens (`mockHostShort`): a
// 10,000-character reply, a turn whose later text steps are dropped (the
// question is the last), and a 5,000-character prompt. `?turntext=old|gone|
// fail|slow|cut` plays the row's other answers. A rect is not reachability:
// the button is hit-tested with `elementFromPoint`.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.CHAT_FULL_URL ?? "http://localhost:1478";
const shots = process.env.CHAT_FULL_SHOTS;
const PLACE = "/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo/.worktrees/messaging";
const LONG_END = "That is the last sentence of the long reply.";
const ASKED = "One question before step 2: which branch should the change land on?";
const PROMPT_END = "END-OF-THE-LONG-PROMPT";
let failures = 0;
const check = (ok, m) => { console.log(`${ok ? "ok" : "not ok"} — ${m}`); if (!ok) failures++; };

async function open(page, query = "") {
  await page.goto(`${base}/?turns=cut&agentprompt=off${query}`);
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator('li.row[data-slug="messaging"]').click();
  await page.locator('[data-testid="chat-list"] > li').first().waitFor();
  await page.waitForTimeout(250);
}
const list = (page) => page.locator('[data-testid="chat-list"]');
const listText = (page) => list(page).evaluate((e) => e.textContent);
const buttons = (page, words) => page.locator('[data-testid="chat-show-full"]', { hasText: words });
const button = (page, words) => buttons(page, words).first();
/** The button is what a click at its centre reaches. */
const reachable = (btn) => btn.evaluate((b) => { b.scrollIntoView({ block: "center" }); const r = b.getBoundingClientRect(); const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); return !!hit && (hit === b || b.contains(hit)) && r.width > 20 && r.height > 8; });

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  if (process.env.CHAT_FULL_ENGINE && process.env.CHAT_FULL_ENGINE !== name) continue;
  const browser = await engine.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  page.setDefaultTimeout(10_000);
  console.log(`# ${name}`);
  await open(page);

  // ── the three shortened bubbles each offer a button; none is whole ──
  let t = await listText(page);
  check(!t.includes(LONG_END) && !t.includes(ASKED) && !t.includes(PROMPT_END), `${name}: the short read has neither the long reply's last sentence, the question nor the prompt's end`);
  check((await page.locator('[data-testid="chat-show-full"]').count()) === 3, `${name}: three shortened turns offer three buttons`);
  check((await buttons(page, "Show full message").count()) === 1 && (await buttons(page, "Show full reply").count()) === 2, `${name}: one says Show full message (the prompt), two Show full reply`);
  const first = button(page, "Show full reply");
  check(await reachable(first), `${name}: the button is reachable (elementFromPoint at its centre)`);
  if (shots) await page.screenshot({ path: `${shots}/${name}-1-shortened.png` });

  // ── the long reply: click, and its last sentence is there ──
  await first.click();
  await page.waitForFunction((s) => document.querySelector('[data-testid="chat-list"]').textContent.includes(s), LONG_END);
  t = await listText(page);
  check(t.includes(LONG_END), `${name}: the 10,000-character reply's last sentence is on screen`);
  check((await page.locator('[data-testid="chat-show-full"]').count()) === 2, `${name}: its own button is gone (2 left)`);
  check(!(await page.locator('[data-testid="chat-agent"]').first().textContent()).includes("Shortened by the host"), `${name}: and its note with it`);
  if (shots) await page.screenshot({ path: `${shots}/${name}-2-long-reply-open.png` });

  // ── a refresh of agent_turns does not snap it back ──
  await page.evaluate((p) => window.__mock.agentReply(p, "A reply written after it was opened."), PLACE);
  await page.waitForFunction(() => document.querySelector('[data-testid="chat-list"]').textContent.includes("written after it was opened"));
  await page.waitForTimeout(400);
  t = await listText(page);
  check(t.includes(LONG_END), `${name}: after the chat re-read agent_turns the long reply is still whole`);

  // ── the turn whose later text steps the host dropped: the question comes back ──
  const second = button(page, "Show full reply");
  check(await reachable(second), `${name}: the next turn's button is reachable`);
  await second.click();
  await page.waitForFunction((s) => document.querySelector('[data-testid="chat-list"]').textContent.includes(s), ASKED);
  check((await listText(page)).includes(ASKED), `${name}: the dropped last block — the question — is shown`);
  const order = await list(page).evaluate((e, [a, q]) => { const s = e.textContent; return s.indexOf("Then the plan.") < s.indexOf(q) && s.indexOf("First, the survey.") < s.indexOf("Then the plan."); }, ["", ASKED]);
  check(order, `${name}: its blocks are in order, the question last`);
  check((await page.locator('[data-testid="chat-call-open"], .chat-tools, .chat-calls').count()) >= 1, `${name}: and the tool it called is still between them`);

  // ── the cut prompt ──
  const msg = button(page, "Show full message");
  check(await reachable(msg), `${name}: the message button is reachable`);
  await msg.click();
  await page.waitForFunction((s) => document.querySelector('[data-testid="chat-list"]').textContent.includes(s), PROMPT_END);
  check((await page.locator('[data-testid="chat-show-full"]').count()) === 0, `${name}: nothing is left to expand`);
  check((await page.locator('[data-testid="chat-shortened"]').count()) === 0, `${name}: and no note is left`);
  if (shots) await page.screenshot({ path: `${shots}/${name}-3-all-open.png` });

  // ── the older host: today's note, no button ──
  await open(page, "&turntext=old");
  check((await page.locator('[data-testid="chat-show-full"]').count()) === 3, `${name}: an older host's buttons are offered until one is tried`);
  await button(page, "Show full reply").click();
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="chat-show-full"]').length === 0);
  const notes = await page.locator('[data-testid="chat-shortened"]').allTextContents();
  check(notes.length === 3 && notes.every((n) => /Shortened by the host: the full (reply|message) is in the terminal\./.test(n)), `${name}: every bubble says today's note and offers no button (${notes.length})`);
  check(!(await listText(page)).includes(LONG_END), `${name}: and nothing was shown`);
  if (shots) await page.screenshot({ path: `${shots}/${name}-4-older-host.png` });

  // ── gone ──
  await open(page, "&turntext=gone");
  await button(page, "Show full reply").click();
  await page.waitForFunction(() => document.querySelector('[data-testid="chat-shortened"]')?.textContent.includes("left what the host reads"));
  const gone = await page.locator('[data-testid="chat-shortened"]').allTextContents();
  check(gone.some((n) => /has left what the host reads: the full text is in the terminal/.test(n)) && (await page.locator('[data-testid="chat-show-full"]').count()) === 2, `${name}: gone says it is in the terminal, and only that bubble loses its button`);
  if (shots) await page.screenshot({ path: `${shots}/${name}-5-gone.png` });

  // ── a failure asks again; slow shows Loading… ──
  await open(page, "&turntext=fail");
  await button(page, "Show full reply").click();
  await page.waitForFunction(() => document.querySelector('[data-testid="chat-show-full"]')?.textContent.startsWith("Try again"));
  check(/session file/.test(await page.locator('[data-testid="chat-shortened"] [role="alert"]').first().textContent()), `${name}: a failure says what the host said and offers Try again`);
  await open(page, "&turntext=slow");
  await button(page, "Show full reply").click();
  const loading = page.locator('[data-testid="chat-show-full"]', { hasText: "Loading…" }).first();
  await loading.waitFor();
  check(await loading.isDisabled(), `${name}: while it loads the button says Loading… and is disabled`);
  await page.waitForFunction((s) => document.querySelector('[data-testid="chat-list"]').textContent.includes(s), LONG_END);

  // ── past the host's own cap: the note stays, no button ──
  await open(page, "&turntext=cut");
  await button(page, "Show full reply").click();
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="chat-show-full"]').length === 2);
  const capped = await page.locator('[data-testid="chat-shortened"]').first().textContent();
  check(/Shortened by the host: the full reply is in the terminal/.test(capped), `${name}: a turn cut again at the host's cap keeps the note, with no button`);
  await browser.close();
}
console.log(failures ? `\n${failures} not ok` : "\nall ok");
process.exit(failures ? 1 : 0);
